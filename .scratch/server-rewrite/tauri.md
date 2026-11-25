# Tauri v2 + Svelte 5 implementation notes for Audio Share Server

Status: verified on 2026-11-09 against the current registries and a real compile/run probe.
Scope: tickets 08-14 (`/home/w0n9/workspaces/audio-share/.scratch/server-rewrite/issues/08-tauri-tray-first-shell.md` through `14-linux-packaging.md`).

## 0. What was verified, and what was not

Verification environment:

- Ubuntu 24.04 amd64, Rust 1.98.1, Node 22.18.0, npm 10.9.3, Xvfb installed.
- Installed Tauri build/runtime deps: `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libsoup-3.0-dev`, `libayatana-appindicator3-dev`, `libpipewire-0.3-0t64`.
- Cargo uses a USTC sparse mirror (`~/.cargo/config.toml`); npm uses `registry.npmmirror.com`.
- Probe source: `/home/w0n9/.claude/jobs/d82859a4/tmp/tauri-probe/`
- Probe logs: `/home/w0n9/.claude/jobs/d82859a4/tmp/tauri-probe/check.log`, `check-release.log`, `check-windows.log`, `build.log`, `deb-build.log`, `appimage-build.log`, `smoke.log`, `appimage-smoke.log`
- Svelte probe: `/home/w0n9/.claude/jobs/d82859a4/tmp/svelte-probe/`

Verified:

- `cargo check` (dev) and `cargo check --release` of a Tauri v2 app with `tray-icon`, all four plugins, capabilities, and the full `tauri.conf.json` below: 0 errors, 0 warnings. Dev check 1m10s cold, release check 48.52s.
- `cargo build --features custom-protocol`: binary at `target/debug/AudioShareServer`.
- `cargo check --target x86_64-pc-windows-msvc --features custom-protocol` of the same probe, including the Windows-only WebView2 preflight: 0 errors in 41.56s. There is no MSVC linker on this machine, so Windows `cargo build`/`test` is still not possible.
- `tauri build --debug --no-bundle` (no workspace) and `tauri build --debug` with `targets: []` (workspace): `Built application at: <cargo-target-dir>/debug/AudioShareServer`.
- `tauri build --bundles deb`: release deb built, installed with `dpkg -i`, run headlessly, removed with `dpkg -r`.
- `tauri build --bundles appimage`: 80.11 MiB AppImage built; runs directly with FUSE2 and with `APPIMAGE_EXTRACT_AND_RUN=1`.
- Headless Xvfb run: window titled `Audio Share Server`, size 420x640, `SMOKE visible=Ok(true)`, process exit 0.
- Svelte 5 + Vite 8 + TypeScript 6: `vite build` and `svelte-check` pass, 0 errors (one harmless unused-CSS warning in the probe).

Not verified here:

- Real Windows runtime behavior: the code type-checks, but WebView2 detection, MSVC resource embedding, and toast notifications were not executed on Windows. Cross-target `cargo check` emits two `GNU compiler is not supported for this target` warnings from `tauri-build` resource embedding; they do not affect type-checking and should disappear in a native `windows-latest` MSVC build. `rustup target add` fails against the mirror (404); the Windows std in this environment was installed manually into the toolchain sysroot (see `environment.md`).
- `tauri dev` under Xvfb (the built binary was tested instead).
- Actual visual rendering/tray panel interaction (window existence and clean exit were tested; no screenshot comparison).
- Windows toast notification appearance for a portable exe.

## 1. Exact versions that are available and compatible

Queried with `cargo add --dry-run` in `/home/w0n9/.claude/jobs/d82859a4/tmp/tauri-probe` and `npm view`; resolved versions confirmed in `Cargo.lock` and `npm ls`.

### Rust crates

| Crate | Version | Notes |
|---|---:|---|
| `tauri` | 2.12.5 | needs feature `tray-icon` for the tray API |
| `tauri-build` | 2.7.4 | build dependency; `tauri_build::build()` |
| `tauri-plugin-single-instance` | 2.5.5 | Rust-only, no npm package |
| `tauri-plugin-autostart` | 2.7.0 | Rust + `@tauri-apps/plugin-autostart` |
| `tauri-plugin-opener` | 2.7.2 | Rust + `@tauri-apps/plugin-opener` |
| `tauri-plugin-notification` | 2.8.0 | Rust + `@tauri-apps/plugin-notification` |
| `tauri-runtime-wry` | 2.12.3 | resolved transitive |
| `wry` | 0.57.1 | resolved transitive |
| `tray-icon` | 0.25.1 | resolved transitive via `tauri/tray-icon` |
| `muda` | 0.20.0 | resolved transitive menu implementation |
| `winreg` | 0.56.0 | Windows-only preflight; compiled via cross-target `cargo check` |
| `windows-sys` | 0.61.2 | Windows-only preflight; compiled via cross-target `cargo check` |

MSRV: `tauri` 2.12.5, `tauri-build` 2.7.4, `tauri-plugin-single-instance` 2.5.5, autostart, opener, and notification all declare `rust-version = "1.90"`. Rust 1.98.1 is fine. Pin MSRV 1.90 in CI documentation, not 1.98.1, unless you use newer language features.

### npm packages

| Package | Version | Notes |
|---|---:|---|
| `@tauri-apps/api` | 2.12.4 | crate is 2.12.5; CLI accepted this patch mismatch |
| `@tauri-apps/cli` | 2.12.4 | `tauri-cli 2.12.4` |
| `@tauri-apps/plugin-autostart` | 2.7.0 | matches Rust plugin |
| `@tauri-apps/plugin-opener` | 2.7.2 | matches Rust plugin |
| `@tauri-apps/plugin-notification` | 2.8.0 | matches Rust plugin |
| `svelte` | 5.57.3 | |
| `vite` | 8.3.6 | |
| `@sveltejs/vite-plugin-svelte` | 7.3.1 | peers: `vite ^8.0.0`, `svelte ^5.46.4` |
| `svelte-check` | 4.7.8 | peers: `typescript ^5.0.0 || ^6.0.0` |
| `typescript` | 6.0.3 | DO NOT use latest 7.0.2; see below |

Important compatibility findings:

- `@tauri-apps/plugin-single-instance` does not exist on npm. The single-instance plugin is Rust-only and needs no capability/permission (the docs page confirms this).
- Latest `typescript` is 7.0.2, but `svelte-check@4.7.8` declares `peerDependencies.typescript = "^5.0.0 || ^6.0.0"`. `npm install` fails with ERESOLVE. Pin `typescript@6.0.3` (latest 6.x).
- Vite 8.3.6 engines: `^20.19.0 || >=22.12.0`. Node 22.18.0 is compatible.
- `@sveltejs/vite-plugin-svelte@7.3.1` engines: `^20.19 || ^22.12 || >=24`. Node 22.18.0 is compatible.

### `Cargo.toml` for the GUI crate

Paths assume `server-rs/gui/src-tauri/Cargo.toml` inside a workspace.

```toml
[package]
name = "audio-share-server-gui"
version = "0.4.0"
description = "Audio Share Server GUI"
authors = ["Audio Share maintainers"]
edition = "2021"
rust-version = "1.90"

[[bin]]
name = "AudioShareServer"
path = "src/main.rs"

[features]
# Enables embedding frontendDist. `tauri build` enables tauri/custom-protocol itself;
# this feature is for bare-cargo production builds.
custom-protocol = ["tauri/custom-protocol"]

[build-dependencies]
tauri-build = { version = "2.7.4", features = [] }

[dependencies]
tauri = { version = "2.12.5", features = ["tray-icon"] }
tauri-plugin-single-instance = "2.5.5"
tauri-plugin-autostart = "2.7.0"
tauri-plugin-opener = "2.7.2"
tauri-plugin-notification = "2.8.0"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[target.'cfg(windows)'.dependencies]
# Only for the optional pre-start WebView2 preflight in section 8.
winreg = "0.56"
windows-sys = { version = "0.61", features = [
  "Win32_Foundation",
  "Win32_UI_WindowsAndMessaging",
  "Win32_UI_Shell",
] }
```

`tauri-build` must run in `build.rs` on every platform:

```rust
fn main() {
    tauri_build::build()
}
```

## 2. Workspace layout and target directory

Recommended layout:

```text
server-rs/
  Cargo.toml                 # [workspace] members = ["core", "cli", "gui/src-tauri"]
  core/
  cli/
  gui/
    package.json
    package-lock.json
    vite.config.ts
    svelte.config.js
    tsconfig.json
    index.html
    src/
    src-tauri/
      Cargo.toml
      build.rs
      tauri.conf.json
      tauri.linux.conf.json
      capabilities/default.json
      icons/
      src/main.rs
```

Workspace `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["core", "cli", "gui/src-tauri"]
```

When the GUI crate is a workspace member, Cargo puts artifacts in the workspace root, not in `src-tauri`:

- Without a workspace root: `src-tauri/target/release/AudioShareServer`.
- With a workspace root: `<workspace-root>/target/release/AudioShareServer`.
- CI artifact paths must use the workspace-root path. This was observed when a workspace wrapper was added to the probe: `tauri build` changed its output from `src-tauri/target/debug/...` to `/home/w0n9/.claude/jobs/d82859a4/tmp/tauri-probe/target/debug/...`.

## 3. `tauri.conf.json` v2

### Base config (portable exe by default)

`bundle.targets: []` means "build the executable, no installer/bundle". This was verified on Linux with `tauri build --debug --config '{"bundle":{"targets":[]}}'`: it printed only `Built application at: ...` and produced no bundle. Platform config files then add Linux bundles.

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "AudioShareServer",
  "version": "0.4.0",
  "identifier": "io.github.<owner>.audioshare",
  "mainBinaryName": "AudioShareServer",
  "build": {
    "frontendDist": "../dist",
    "devUrl": "http://localhost:1420",
    "beforeDevCommand": "npm run dev",
    "beforeBuildCommand": "npm run build"
  },
  "app": {
    "windows": [
      {
        "label": "main",
        "title": "Audio Share Server",
        "width": 420,
        "height": 640,
        "minWidth": 380,
        "minHeight": 560,
        "resizable": true,
        "visible": true,
        "center": true
      }
    ],
    "security": {
      "csp": null
    }
  },
  "bundle": {
    "active": true,
    "targets": [],
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ],
    "windows": {
      "webviewInstallMode": {
        "type": "downloadBootstrapper",
        "silent": true
      }
    }
  }
}
```

### Linux platform config

Tauri merges `tauri.linux.conf.json` (or `.toml`) over the base config with JSON Merge Patch. Objects merge key by key; arrays are replaced entirely. Verified: base `targets: []` plus `tauri.linux.conf.json` with `targets: ["deb"]` produced a deb.

```json
{
  "bundle": {
    "targets": ["deb", "appimage"],
    "linux": {
      "deb": {
        "depends": ["libpipewire-0.3-0"]
      }
    }
  }
}
```

Do not list `libwebkit2gtk-4.1-0`, `libgtk-3-0`, or `libayatana-appindicator3-1` in `depends` unless you want duplicates: Tauri appends the configured list to its own defaults. Observed control file with all four declared:

```text
Depends: libwebkit2gtk-4.1-0, libgtk-3-0, libayatana-appindicator3-1, libpipewire-0.3-0,
         libayatana-appindicator3-1, libwebkit2gtk-4.1-0, libgtk-3-0
```

Duplicate fields are harmless to dpkg but noisy. Declare only the extra runtime dependency (`libpipewire-0.3-0`).

### Field semantics and defaults (from `tauri-utils-2.10.2/src/config.rs`)

- `productName` must match `^[^/\:*?"<>|]+$`; it feeds the deb package name, desktop file, bundle artifact name, and `PackageInfo.name`.
- `identifier` must be reverse-DNS and unique; default `com.tauri.dev` is rejected by `tauri build`. It also determines the Linux single-instance DBus name.
- `version`: semver `0.4.0`; keep it equal to the Cargo package version and `package.json` version.
- `mainBinaryName`: Tauri CLI renames the Cargo output during `tauri build`; it does not affect plain `cargo build`. Set `[[bin]] name = "AudioShareServer"` as well.
- Window defaults: `label` defaults to `main`; `width` 800, `height` 600; `resizable`, `visible`, `decorations` default true; `center` default false. All listed fields were accepted by the compiled probe.
- `bundle.active` defaults false; set true.
- `bundle.targets` defaults `"all"`; valid values include `deb`, `rpm`, `appimage`, `msi`, `nsis`, `app`, `dmg`. Arrays of objects are replaced by platform config, not appended.
- `bundle.windows.webviewInstallMode` defaults to `{ "type": "downloadBootstrapper", "silent": true }`; it only affects MSI/NSIS installers.
- `app.security.csp` defaults to unset/off. `csp: null` compiled and ran.
- `bundle.icon` defaults to `[]`; include PNGs, `.ico`, and `.icns`. `tauri icon <source.png>` generated all required files (verified).

### `frontendDist` and workspace builds (important)

Tauri's production asset embedding is gated by the `tauri/custom-protocol` feature:

- `tauri build` runs exactly (verified with `tauri build --no-bundle -v`):
  `cargo build --bins --features tauri/custom-protocol --release`
- Bare `cargo build` / `cargo check` does not enable it. Tauri's build script sets `dev = true`, and `tauri-codegen` then uses `devUrl` instead of `frontendDist`. Verified: `cargo check --release` and a workspace `cargo check -p audio-share-server-gui` pass with no `dist/` directory.
- With `custom-protocol` enabled and `dist/` missing, compilation panics at `tauri::generate_context!`:

```text
error: proc macro panicked
  --> src/main.rs:219:14
   |
   = help: message: The `frontendDist` configuration is set to `"../dist"` but this path doesn't exist
```

Consequences for the Rust workspace:

- `cargo build --workspace` will not fail before the npm build, but its GUI binary is a dev binary that tries to load `http://localhost:1420`. Do not ship it.
- `tauri build` runs `beforeBuildCommand` (`npm run build`) first and then enables `custom-protocol`, so `dist/` exists.
- For a bare-cargo production build, run `npm run build` first, then:
  `cargo build -p audio-share-server-gui --release --features audio-share-server-gui/custom-protocol`
  (verified from a workspace root: adding this feature makes the missing-`dist` panic appear).
- Optional: set `[workspace] default-members = ["core", "cli"]` so a root `cargo build` does not build the GUI at all.

### CSP

Start with `"csp": null` for v0.4.0 if you want zero risk of breaking WebKit/Vite behavior. The compiled probe used `null`.

If hardening, the Tauri docs' pattern with IPC sources is:

```json
{
  "app": {
    "security": {
      "csp": "default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' asset: http://asset.localhost data:; style-src 'self' 'unsafe-inline'",
      "devCsp": "default-src 'self'; connect-src ipc: http://ipc.localhost ws://localhost:1420 http://localhost:1420; img-src 'self' asset: http://asset.localhost data:; style-src 'self' 'unsafe-inline'"
    }
  }
}
```

Tauri hashes bundled scripts and nonces styles automatically. `style-src 'unsafe-inline'` is still the pragmatic setting for Svelte/Vite and WebKitGTK. Vite HMR needs the `ws://localhost:1420` source in `devCsp`.

## 4. Rust shell: tray, window, commands, events

The code below is a condensed version of the compiled probe `/home/w0n9/.claude/jobs/d82859a4/tmp/tauri-probe/src-tauri/src/main.rs`. It compiled cleanly against tauri 2.12.5.

### Imports, state, showing the window

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent,
};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerStatus {
    running: bool,
    host: String,
    port: u16,
    clients: u32,
}

struct AppState {
    running: AtomicBool,
    status: Mutex<ServerStatus>,
}

/// Menu items kept in managed state so commands can enable/disable them.
struct TrayItems {
    start: MenuItem<tauri::Wry>,
    stop: MenuItem<tauri::Wry>,
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
```

`show_main_window` is the Windows second-launch/tray-click behavior. `unminimize` + `show` + `set_focus` was compiled; on Windows, foreground-focus restrictions can still cause `set_focus` to be best-effort.

### Tray icon, Show/Start/Stop/Exit menu, dynamic enable/disable

```rust
fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show_i = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
    let start_i = MenuItem::with_id(app, "start", "Start", true, None::<&str>)?;
    let stop_i = MenuItem::with_id(app, "stop", "Stop", false, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let exit_i = MenuItem::with_id(app, "exit", "Exit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_i, &start_i, &stop_i, &sep, &exit_i])?;

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("bundle.icon must include a PNG");

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("Audio Share Server")
        .menu(&menu)
        // Windows: left click is handled below; right click opens the menu.
        // Linux: Tauri does not emit tray icon events; right click opens the menu.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "start" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = start_server(app).await;
                });
            }
            "stop" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = stop_server(app).await;
                });
            }
            "exit" => app.exit(0), // do NOT call window.close(): that hits the close-to-tray handler
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Not emitted on Linux. Windows only.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    if window.is_visible().unwrap_or(false) {
                        let _ = window.hide();
                    } else {
                        show_main_window(app);
                    }
                }
            }
        })
        .build(app)?;

    app.manage(TrayItems { start: start_i, stop: stop_i });
    Ok(())
}

fn publish_status(app: &AppHandle, running: bool) {
    let state = app.state::<AppState>();
    state.running.store(running, Ordering::SeqCst);
    let snapshot = {
        let mut status = state.status.lock().unwrap();
        status.running = running;
        status.clone()
    };
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.start.set_enabled(!running);
        let _ = items.stop.set_enabled(running);
    }
    let _ = app.emit("server:state-changed", snapshot);
}
```

Notes:

- `MenuItem::with_id(app, id, text, enabled, accelerator)` and `MenuItem::set_enabled(bool)` are the dynamic-enable API. `MenuItem<tauri::Wry>` is `Send + Sync`; storing it in managed state compiles.
- Menu/tray callbacks run on the main thread. Never block them on server start/stop; `tauri::async_runtime::spawn` or `spawn_blocking` instead (shown below).
- `app.default_window_icon()` requires `bundle.icon` to contain a PNG (Linux) or the embedded app icon resource (Windows). Loading a PNG manually via `Image::from_bytes` would additionally require the `image-png` feature; the default-icon path needs no extra feature.
- On Linux, Tauri documents that tray mouse events are not emitted even though the icon is shown; the context menu still opens on right click. Therefore left-click-to-toggle is Windows-only; Linux users get Show from the menu. This matches spec user story 36.
- On Windows, the menu by default opens on both left and right click. `show_menu_on_left_click(false)` is required if left click should toggle the window.
- `app.exit(0)` bypasses `CloseRequested`; this is the correct Exit path.

### Close-to-tray and window event

```rust
fn install_close_handler(app: &tauri::App) {
    if let Some(window) = app.get_webview_window("main") {
        let window_for_event = window.clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Default policy: close-to-tray. Branch on the onClose setting here.
                api.prevent_close();
                let _ = window_for_event.hide();
            }
        });
    }
}
```

`api.prevent_close()` + `hide()` is the documented close-to-tray mechanism. `WindowEvent::CloseRequested` is `#[non_exhaustive]`; always match with `..`. If `onClose == "exit"`, skip `prevent_close` and let the window close (or call `app.exit(0)`).

### Async commands, shared state, blocking core

```rust
#[tauri::command]
async fn get_status(state: State<'_, AppState>) -> Result<ServerStatus, String> {
    let status = state.status.lock().map_err(|e| e.to_string())?;
    Ok(status.clone())
}

#[tauri::command]
async fn start_server(app: AppHandle) -> Result<ServerStatus, String> {
    let app_for_job = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Placeholder for core::Server::start(): device enumeration and
        // capture setup may block. Never do this on the main thread.
        std::thread::sleep(std::time::Duration::from_millis(10));
        publish_status(&app_for_job, true);
    })
    .await
    .map_err(|e| e.to_string())?;

    get_status(app.state::<AppState>()).await
}

#[tauri::command]
async fn stop_server(app: AppHandle) -> Result<ServerStatus, String> {
    let app_for_job = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Placeholder for a cancellable core stop.
        publish_status(&app_for_job, false);
    })
    .await
    .map_err(|e| e.to_string())?;

    get_status(app.state::<AppState>()).await
}
```

Key rules:

- Commands without `async` run on the main thread. Use `async fn` (or `#[tauri::command(async)]`) for anything that can block.
- Tauri runs async commands on its async runtime, not the UI thread. `spawn_blocking` is appropriate for WASAPI/PipeWire setup. `spawn_blocking` cannot be aborted; implement stop as a cancellation token/atomic checked by the core, then join the worker.
- `State<'_, AppState>` in an async command compiled on tauri 2.12.5, but do not hold a `std::sync::MutexGuard` across `.await`. Lock, copy, drop.
- `app.manage(AppState::default())` registers state; `app.state::<T>()` / `app.try_state::<T>()` retrieve it.
- Return `Result<T, String>` for simple readable errors; a custom error type implementing `Serialize` is also fine and surfaces as a rejected `invoke` Promise.
- `tauri::generate_handler![get_status, start_server, stop_server]` must be called once; later calls replace earlier handlers.

### Emitting events to the frontend

```rust
// Rust; requires `use tauri::Emitter`.
let _ = app.emit("server:state-changed", snapshot); // all webviews
// or: let _ = app.emit_to("main", "server:state-changed", snapshot);
```

Event names may contain alphanumerics, `-`, `/`, `:`, `_`. Payloads are JSON and must be `Serialize` (ours derives `Serialize` with `camelCase`). Emission is fire-and-forget and async; it never blocks the tray callback.

### Plugin initialization and setup (exact order)

```rust
fn main() {
    tauri::Builder::default()
        // Single-instance MUST be the first plugin registered.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
            let _ = app.emit("app:second-instance", ());
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hide"]),
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            get_status,
            start_server,
            stop_server
        ])
        .setup(|app| {
            build_tray(app)?;
            install_close_handler(app);

            // Autostart launches with --hide; normal launches show the window.
            if std::env::args().any(|arg| arg == "--hide") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            // Optional headless CI probe, see section 9.
            if std::env::args().any(|arg| arg == "--smoke-test") {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(4));
                    if let Some(window) = handle.get_webview_window("main") {
                        println!(
                            "SMOKE visible={:?} size={:?} title={:?}",
                            window.is_visible(),
                            window.inner_size(),
                            window.title()
                        );
                    }
                    handle.exit(0);
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Audio Share Server");
}
```

## 5. Plugins in detail

### Single instance

- Crate: `tauri-plugin-single-instance = "2.5.5"`. No npm package, no JS API, no capability permission needed.
- It must be the first plugin registered. Registering it inside `.setup()` after other builder plugins is not "first"; prefer the builder chain above.
- Callback signature: `(AppHandle, args: Vec<String>, cwd: String)`.
- On second launch, the new process exits and the callback runs in the first process. To show/focus the existing window:

```rust
tauri_plugin_single_instance::init(|app, _args, _cwd| {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
})
```

- Linux implementation uses the session DBus. Service name is `org.{identifier with . and - replaced by _}.SingleInstance`. For identifier `io.github.audioshare.server`, that is `org.io_github_audioshare_server.SingleInstance`. Deb, RPM, and AppImage work out of the box. Snap/Flatpak need DBus permissions; those are out of scope.
- Nuance: when autostart launches a second instance with `--hide`, do not steal focus. Check `_args` for `--hide` and return silently.

### Autostart

- Crate 2.7.0 + npm 2.7.0. Init:

```rust
.plugin(tauri_plugin_autostart::init(
    tauri_plugin_autostart::MacosLauncher::LaunchAgent, // ignored on Windows/Linux
    Some(vec!["--hide"]),                               // args used at login
))
```

- Runtime control (inside `setup` or a command):

```rust
use tauri_plugin_autostart::ManagerExt;

let autolaunch = app.autolaunch();
if should_autostart {
    // In a command returning Result<_, String>, map the plugin error:
    autolaunch.enable().map_err(|e| e.to_string())?;
    // In setup (Box<dyn Error>), plain `autolaunch.enable()?;` also works.
} else {
    autolaunch.disable().map_err(|e| e.to_string())?;
}
let enabled = autolaunch.is_enabled().map_err(|e| e.to_string())?;
```

- Windows: the plugin uses `auto-launch 0.6.0`. Default mode `Dynamic` tries `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run`, falls back to `HKCU\...\Run` on access denied. Value name is the app name (`PackageInfo.name`, i.e. `productName` = `AudioShareServer`); value data is `<exe path> --hide`.
- Linux: default XDG autostart. Writes `~/.config/autostart/AudioShareServer.desktop` with `Exec=<path> --hide`, `Type=Application`, `Terminal=false`, `StartupNotify=false`.
- AppImage: if the `APPIMAGE` environment variable is set, the plugin registers the `.AppImage` path instead of the extracted executable. Moving the AppImage breaks the entry.
- Gotcha: auto-launch 0.6.0 joins path and args without quoting on both Windows and Linux (`"{} {}"` / `args.join(" ")`). A portable exe under a path with spaces, e.g. `C:\Users\Foo\Audio Share\AudioShareServer.exe`, produces a broken Run key / Exec line. Either keep the exe in a space-free directory, or write the Run key/desktop file yourself with quotes.
- The `--hide` check must happen before showing the window. `visible: true` in config means a possible flash; the clean alternative is `visible: false` plus `show()` in setup when `--hide` is absent.
- Permissions for frontend calls: `autostart:default` (allow-enable, allow-disable, allow-is-enabled).

### Opener

- Crate 2.7.2 + npm 2.7.2.
- Rust use:

```rust
use tauri_plugin_opener::OpenerExt;

// In a command returning Result<_, String>:
app.opener()
    .open_url("https://github.com/.../releases/latest", None::<&str>)
    .map_err(|e| e.to_string())?;
// In setup (Box<dyn Error>), plain `...open_url(url, None::<&str>)?;` works.
```

- JS use: `import { openUrl, openPath } from "@tauri-apps/plugin-opener"`.
- Capability: `opener:default` allows `mailto:`, `tel:`, `https:`, `http:` URLs and reveal-in-file-explorer. `openPath` is not in the default set; add `opener:allow-open-path` with a path scope if you need to open a log/config file.
- The update check should call `open_url` for the release page; no Tauri updater plugin.

### Notification

- Crate 2.8.0 + npm 2.8.0.
- Rust use:

```rust
use tauri_plugin_notification::NotificationExt;

app.notification()
    .builder()
    .title("Audio Share Server")
    .body("A new version is available")
    .show()
    .map_err(|e| e.to_string())?;
```

- JS: check `isPermissionGranted()`, then `requestPermission()`, then `sendNotification({ title, body })`.
- Capability: `notification:default`.
- Caveats: the Tauri docs state Windows notifications only work fully for installed apps; during development they show the PowerShell icon/name. A portable exe is not "installed" in that sense. Do not make the update-check feature depend on an OS notification; show an in-app banner and/or tray menu entry, and use notifications as a bonus. On Linux a notification daemon must be running (normal desktop session provides one; Xvfb does not).
- The Windows plugin sets the AppUserModelID to the Tauri identifier only when the exe is not under `target/debug` or `target/release`; this is intended to improve unpackaged-app behavior but does not make toast notifications fully reliable for a portable exe.

## 6. Frontend: Svelte 5 + Vite + TypeScript

### Package and tooling

`/home/w0n9/.claude/jobs/d82859a4/tmp/svelte-probe/package.json` (verified):

```json
{
  "name": "audio-share-server-gui",
  "private": true,
  "version": "0.4.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "check": "svelte-check --tsconfig ./tsconfig.json",
    "tauri": "tauri"
  },
  "dependencies": {
    "@tauri-apps/api": "2.12.4",
    "@tauri-apps/plugin-autostart": "2.7.0",
    "@tauri-apps/plugin-opener": "2.7.2",
    "@tauri-apps/plugin-notification": "2.8.0"
  },
  "devDependencies": {
    "@sveltejs/vite-plugin-svelte": "7.3.1",
    "@tauri-apps/cli": "2.12.4",
    "svelte": "5.57.3",
    "svelte-check": "4.7.8",
    "typescript": "6.0.3",
    "vite": "8.3.6"
  }
}
```

`vite.config.ts`:

```ts
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false, // keep Rust/Tauri logs visible
  server: {
    port: 1420,
    strictPort: true, // fail instead of silently moving off devUrl
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: { target: "es2021" }, // WebView2 and WebKitGTK 4.1 both support this
});
```

`svelte.config.js`:

```js
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

export default {
  preprocess: vitePreprocess(),
};
```

`tsconfig.json` (worked with svelte-check 4.7.8 and TS 6.0.3):

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "resolveJsonModule": true,
    "strict": true,
    "verbatimModuleSyntax": true,
    "isolatedModules": true,
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "types": ["svelte", "vite/client"],
    "skipLibCheck": true,
    "noEmit": true
  },
  "include": ["src/**/*.ts", "src/**/*.svelte"]
}
```

### Entry point

`index.html` loads `/src/main.ts`; `src/main.ts` uses the Svelte 5 mount API, not `new App()`:

```ts
import { mount } from "svelte";
import App from "./App.svelte";

const app = mount(App, { target: document.getElementById("app")! });
export default app;
```

### File structure and prototype-A view split

```text
src/
  main.ts
  App.svelte                  # shell + header + main/settings switch
  views/
    ServerView.svelte         # status hero, Start/Stop, endpoint, encoding, host/port, footer
    SettingsView.svelte       # autostart, on-launch, on-close, updates, language, firewall, About
  lib/
    ipc.ts                    # typed invoke/listen wrappers
    store.svelte.ts           # $state server store
    i18n.svelte.ts            # locale + t()
    platform.ts               # isWindows via a Rust command or navigator data
  locales/
    en-US.json
    zh-CN.json
```

Prototype A information architecture to implement:

- Main view, top to bottom: brand/header with settings gear; status dot plus `Server running` / `Server stopped`; listening address `host:port`; full-width primary `Start server` / `Stop server` (turns red while running); Audio endpoint select; Encoding select; Host + Port row; footer `Settings` and `Quit`.
- Secondary settings view replaces the main content in the same window (not a tab bar): Start at login, On launch (nothing/start/last), When closing (exit/tray), Auto update check, Language, Repair firewall (Windows only; Linux shows an explanation link), About (version, homepage, license), footer Back and Quit.
- No log viewer in v0.4.0. No channels/sample-rate controls in the GUI.
- Window is a compact 420x640 single window; close goes to tray by default.

Visual tokens copied from the prototype A CSS (`git show prototype/server-gui:prototype/server-gui/index.html`):

```css
:root {
  --bg: #e9e9f0;
  --surface: #ffffff;
  --surface-2: #f6f6f9;
  --text: #1b1b1f;
  --muted: #6b6b76;
  --line: #dcdce3;
  --accent: #0a84ff;
  --green: #34c759;
  --red: #ff453a;
  --sans: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
    "Helvetica Neue", Arial, "PingFang SC", "Microsoft YaHei", sans-serif;
  --mono: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
```

Controls use 8-10px radii; the window/popover uses 14-16px; the status dot is 9px with a green glow when running; primary button is accent blue, red when stopping; rows are separated by 1px `--line` top borders; address text uses `--mono`.

### Typed IPC wrappers (`src/lib/ipc.ts`)

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface ServerStatus {
  running: boolean;
  host: string;
  port: number;
  clients: number;
}

export interface Settings {
  host: string;
  port: number;
  endpoint: string;
  encoding: string;
}

export const api = {
  getStatus: () => invoke<ServerStatus>("get_status"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  start: () => invoke<ServerStatus>("start_server"),
  stop: () => invoke<ServerStatus>("stop_server"),
};

export function onServerStateChanged(
  handler: (status: ServerStatus) => void,
): Promise<UnlistenFn> {
  return listen<ServerStatus>("server:state-changed", (event) => handler(event.payload));
}

export function onAppSecondInstance(handler: () => void): Promise<UnlistenFn> {
  return listen("app:second-instance", () => handler());
}
```

Tauri v2 imports are `@tauri-apps/api/core` and `@tauri-apps/api/event` (not the v1 `@tauri-apps/api/tauri` path). Rust command argument names are snake_case; JS argument keys are camelCase by default, so `save_settings(settings: Settings)` is invoked with `{ settings }`. Use `#[tauri::command(rename_all = "snake_case")]` if you prefer snake_case keys.

### Svelte 5 rune store (`src/lib/store.svelte.ts`)

```ts
import { api, type ServerStatus, type Settings } from "./ipc";

export type View = "main" | "settings";

class ServerStore {
  status = $state<ServerStatus>({
    running: false,
    host: "0.0.0.0",
    port: 65530,
    clients: 0,
  });
  settings = $state<Settings>({
    host: "0.0.0.0",
    port: 65530,
    endpoint: "default",
    encoding: "default",
  });
  view = $state<View>("main");
  busy = $state(false);
  error = $state<string | null>(null);

  async refresh(): Promise<void> {
    this.status = await api.getStatus();
    this.settings = await api.getSettings();
  }

  async start(): Promise<void> {
    this.busy = true;
    this.error = null;
    try {
      this.status = await api.start();
    } catch (error) {
      this.error = String(error);
    } finally {
      this.busy = false;
    }
  }

  // stop() and save() follow the same pattern.
}

export const server = new ServerStore();
```

Runes only work in `.svelte` or `.svelte.ts`/`.svelte.js` files. Naming this file `store.ts` breaks `$state`.

### i18n with en-US / zh-CN JSON

`src/locales/en-US.json` and `zh-CN.json` use flat, namespaced keys:

```json
{
  "app.title": "Audio Share Server",
  "status.running": "Server running",
  "status.stopped": "Server stopped",
  "status.listening": "Listening on {address}",
  "action.start": "Start server",
  "action.stop": "Stop server",
  "field.host": "Host",
  "field.port": "Port",
  "field.endpoint": "Audio endpoint",
  "field.encoding": "Encoding"
}
```

`src/lib/i18n.svelte.ts`:

```ts
import enUS from "../locales/en-US.json";
import zhCN from "../locales/zh-CN.json";

export type Locale = "en-US" | "zh-CN";

const resources: Record<Locale, Record<string, string>> = {
  "en-US": enUS,
  "zh-CN": zhCN,
};

let current = $state<Locale>("en-US");

export function setLocale(locale: Locale): void {
  current = locale;
}

export function t(key: string, params?: Record<string, string | number>): string {
  const template = resources[current][key] ?? resources["en-US"][key] ?? key;
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (_, name: string) =>
    name in params ? String(params[name]) : `{${name}}`,
  );
}
```

Language is applied at startup only, per spec. The clean sequence is: mount a small bootstrap, `await api.getSettings()`, call `setLocale(settings.language)` before rendering the shell, then `mount(App, ...)`. Runtime switching can still work through the rune, but the spec excludes it. Keep both JSON files key-complete; a CI script can diff key sets.

### App shell and lifecycle listener cleanup (`src/App.svelte`)

```svelte
<script lang="ts">
  import { onMount } from "svelte";
  import type { UnlistenFn } from "@tauri-apps/api/event";
  import { server } from "./lib/store.svelte";
  import { t } from "./lib/i18n.svelte";
  import { onServerStateChanged } from "./lib/ipc";

  let unlisten: UnlistenFn | undefined;
  let disposed = false;

  onMount(() => {
    void server.refresh().catch((error) => (server.error = String(error)));

    // listen() returns a Promise<UnlistenFn>. Calling it before the Promise
    // resolves leaks the listener; handle the mount/unmount race explicitly.
    void onServerStateChanged((status) => {
      server.status = status;
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  const address = $derived(`${server.status.host}:${server.status.port}`);
</script>

{#if server.view === "main"}
  <!-- ServerView: status, Start/Stop, endpoint, encoding, host/port -->
{:else}
  <!-- SettingsView: autostart, on-launch, on-close, updates, language, firewall, About -->
{/if}
```

Events to define between Rust and the frontend:

| Event | Payload | Purpose |
|---|---|---|
| `server:state-changed` | `ServerStatus` | status/address/clients update after Start/Stop |
| `server:error` | `{ message: string }` | readable async error for the view |
| `update:available` | `{ version: string, url: string }` | update banner |
| `app:second-instance` | none | optional UI refresh/focus hint |

Use `$derived` for display values and `$state` for mutable store fields. Do not use Svelte 4 `$:` / stores plus runes in the same module unless intentional.

### Platform-specific settings rows

Linux must not show Repair Firewall or Sound Panel. Add a small Rust command:

```rust
#[tauri::command]
fn platform() -> String {
    if cfg!(windows) { "windows".into() } else { "linux".into() }
}
```

and call `invoke<string>("platform")` once in the store. Linux rows show an explanation link (for example to PipeWire/WirePlumber docs) instead of the Windows actions.

## 7. Packaging: exact commands, paths, and dependencies

All commands run in the GUI directory (the directory containing `package.json` and `src-tauri/`), after `npm ci`.

### Windows portable exe

```bat
npm ci
npm run build
npx tauri build --no-bundle
```

Or, with `bundle.targets: []` in the base config, plain `npx tauri build` also produces just the exe (verified on Linux with an empty targets array). `--no-bundle` is explicit and verified.

Output (with a Cargo workspace): `<workspace-root>\target\release\AudioShareServer.exe`.
Without a workspace: `src-tauri\target\release\AudioShareServer.exe`.
`tauri build --debug --no-bundle` was verified without a workspace to produce `src-tauri/target/debug/AudioShareServer`. After adding a workspace root, `tauri build --debug` with `targets: []` produced `<workspace>/target/debug/AudioShareServer`; the target directory follows Cargo's workspace root.

Do not use plain `cargo build --release` for the portable exe unless you also enable `--features audio-share-server-gui/custom-protocol` after `npm run build`; otherwise the binary loads `devUrl` (`http://localhost:1420`) and is not shippable.

### Linux deb + AppImage

```bash
npm ci
npm run build
npx tauri build --bundles deb,appimage
```

With a workspace, outputs are:

```text
<workspace-root>/target/release/bundle/deb/AudioShareServer_0.4.0_amd64.deb
<workspace-root>/target/release/bundle/appimage/AudioShareServer_0.4.0_amd64.AppImage
```

Without a workspace, replace `<workspace-root>/target` with `src-tauri/target`.

Observed release artifacts:

- deb: 5.23 MiB, `Package: audio-share-server`, `Version: 0.4.0`, `Architecture: amd64`, installs `/usr/bin/AudioShareServer`, `/usr/share/applications/AudioShareServer.desktop`, and hicolor icons. `dpkg -i`, running the installed binary headlessly, and `dpkg -r` all succeeded.
- AppImage: 80.11 MiB, ELF x86-64 static-pie, desktop file `Exec=AudioShareServer`, `StartupWMClass=AudioShareServer`, `Icon=AudioShareServer`. It bundled `libwebkit2gtk-4.1.so.0`, `libgtk-3.so.0`, `libayatana-appindicator3.so.1`, and GStreamer core libraries in this probe, but still needs a compatible glibc (built on 24.04) and a running PipeWire server for capture.

Deb dependency behavior:

- Tauri's default runtime deps (observed with the tray feature): `libwebkit2gtk-4.1-0, libgtk-3-0, libayatana-appindicator3-1`.
- Ubuntu 24.04 package reality: `libgtk-3-0` is `Provides:` by `libgtk-3-0t64`; `libpipewire-0.3-0` is `Provides:` by `libpipewire-0.3-0t64`; `libfuse2` is `Provides:` by `libfuse2t64`. Declaring the non-t64 names is correct and works across 22.04/24.04.
- Add only `libpipewire-0.3-0` for the audio capture runtime; Tauri appends defaults. Set `bundle.publisher` or `[package] authors` to avoid the default `Maintainer: github`, and set `bundle.shortDescription`/`longDescription` to avoid `(none)` in the description.

### AppImage constraints

- Direct execution requires FUSE 2: on Ubuntu 24.04 install `libfuse2t64` (provides `libfuse2`). Verified direct execution once FUSE2 was present.
- Without FUSE2, run with `APPIMAGE_EXTRACT_AND_RUN=1` or `./X.AppImage --appimage-extract` then `squashfs-root/AppRun`. Verified with `APPIMAGE_EXTRACT_AND_RUN=1`.
- Build on the oldest supported distro. Ubuntu 24.04 produces a glibc 2.39 baseline; the target is Ubuntu 24.04+, so this is fine, but the AppImage will not run on 22.04.
- AppImage bundling downloads `linuxdeploy` and its plugin from GitHub at build time. In this environment the plugin download failed and Tauri fell back to an older built-in version; the build still succeeded. CI needs network access or a primed `~/.cache/tauri` cache.
- Symbol stripping with linuxdeploy can fail on newer distros (`Strip call failed`). Tauri 2.12.5 built successfully here without intervention. If CI hits it, set `NO_STRIP=true`.
- AppImage autostart uses the `APPIMAGE` env var; moving the file breaks the XDG entry. Capture also needs the system PipeWire daemon; an AppImage does not bundle a running sound server.

### Runtime dependencies for a clean Ubuntu 24.04 desktop

Deb declares them automatically; document them for AppImage/portable users:

```text
libwebkit2gtk-4.1-0
libgtk-3-0t64        (Provides libgtk-3-0)
libayatana-appindicator3-1
libpipewire-0.3-0t64 (Provides libpipewire-0.3-0)
libfuse2t64          (AppImage direct execution only; Provides libfuse2)
```

Plus a running PipeWire session for audio capture. Dev/build packages:

```text
build-essential curl wget file pkg-config libssl-dev
libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev
libayatana-appindicator3-dev librsvg2-dev libxdo-dev patchelf
libpipewire-0.3-dev libspa-0.2-dev
```

### Config file location problem on Linux

The spec says `config.ini` stays next to the executable. On Windows portable exe that is writable. On Linux deb the executable is `/usr/bin/AudioShareServer` (root-owned), and inside an AppImage the mount is read-only. Writing `config.ini` next to the exe will fail. Recommendation to resolve before ticket 09: keep exe-dir `config.ini` reading for Windows compatibility, but on Linux write to `$XDG_CONFIG_HOME/audio-share-server/config.ini` (default `~/.config/audio-share-server/config.ini`) and migrate/import an exe-dir file if present. Decision needed; flagged as a spec conflict, not silently changed.

### CI sketch

Windows job (`windows-latest`):

```yaml
- uses: actions/checkout@v4
- uses: actions/setup-node@v4
  with:
    node-version: 22
    cache: npm
    cache-dependency-path: server-rs/gui/package-lock.json
- uses: dtolnay/rust-toolchain@stable
- uses: Swatinem/rust-cache@v2
  with:
    workspaces: server-rs -> target
- run: npm ci
  working-directory: server-rs/gui
- run: npx tauri build --no-bundle
  working-directory: server-rs/gui
- uses: actions/upload-artifact@v4
  with:
    name: AudioShareServer-windows
    path: server-rs/target/release/AudioShareServer.exe
```

Linux job (`ubuntu-24.04`):

```yaml
- uses: actions/checkout@v4
- uses: actions/setup-node@v4
  with:
    node-version: 22
- uses: dtolnay/rust-toolchain@stable
- uses: Swatinem/rust-cache@v2
  with:
    workspaces: server-rs -> target
- run: |
    sudo apt-get update
    sudo apt-get install -y build-essential curl wget file pkg-config libssl-dev \
      libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
      libayatana-appindicator3-dev librsvg2-dev libxdo-dev patchelf \
      libpipewire-0.3-dev libspa-0.2-dev libfuse2t64
- run: npm ci
  working-directory: server-rs/gui
- run: npx tauri build --bundles deb,appimage
  working-directory: server-rs/gui
  env:
    NO_STRIP: "true"   # only needed if linuxdeploy strip fails
- uses: actions/upload-artifact@v4
  with:
    name: AudioShareServer-linux
    path: |
      server-rs/target/release/bundle/deb/*.deb
      server-rs/target/release/bundle/appimage/*.AppImage
```

`tauri-apps/tauri-action@v1` is the current action generation and can replace the npm/tauri steps, but for the Windows portable exe you still need to upload the bare `.exe` manually. The plain CLI steps above are simpler and were verified locally.

## 8. WebView2 on Windows

### What Tauri already does

Tauri v2 checks the runtime before creating the first webview. The implementation in `tauri-runtime-wry-2.12.3/src/lib.rs:4678` is:

```rust
if !context.webview_runtime_installed {
    #[cfg(all(not(debug_assertions), windows))]
    dialog::error(
      r#"Could not find the WebView2 Runtime.
Make sure it is installed or download it from <A href="https://developer.microsoft.com/en-us/microsoft-edge/webview2">...</A>"#,
    );
    // ...
    return Err(Error::WebviewRuntimeNotInstalled);
}
```

- `webview_runtime_installed` is `wry::webview_version().is_ok()` (`tauri-runtime-wry-2.12.3/src/lib.rs:2783`).
- Wry's Windows implementation calls `GetAvailableCoreWebView2BrowserVersionString` (`wry-0.57.1/src/webview2/mod.rs:2021`).
- In a release Windows build, Tauri shows a native error dialog with the download URL and then returns `WebviewRuntimeNotInstalled`; the final `.expect("error while running ...")` panics, but with `windows_subsystem = "windows"` there is no console to show it. In debug builds the dialog is skipped and the app just returns the error.
- Therefore the spec requirement "detect WebView2, show a clear prompt and download link, do not crash" is already met in release builds by Tauri, though the crash-path panic is ugly. A pre-start check gives a cleaner exit and lets you use a Yes/No "open download page" flow.

### Pre-start registry check (recommended)

Microsoft's documented detection locations and GUID (`https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution#detect-if-a-webview2-runtime-is-already-installed`):

```text
HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}   value: pv
HKCU\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}                value: pv
```

Treat missing, empty, or `0.0.0.0` as not installed; a version greater than `0.0.0.0` means installed. On 32-bit Windows the HKLM path omits `WOW6432Node`; the target is x86_64, so the paths above apply.

Probe implementation (`src-tauri/src/webview2_check.rs`, Windows-only; compiled with `cargo check --target x86_64-pc-windows-msvc`, not run on Windows):

```rust
use std::ffi::c_void;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_ICONERROR, MB_YESNO, SW_SHOWNORMAL,
};
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};

const WEBVIEW2_GUID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
const DOWNLOAD_URL: &str = "https://developer.microsoft.com/en-us/microsoft-edge/webview2/";

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn pv_at(hive: winreg::HKEY, path: &str) -> Option<String> {
    let key = winreg::RegKey::predef(hive)
        .open_subkey_with_flags(path, KEY_READ)
        .ok()?;
    let version: String = key.get_value("pv").ok()?;
    (!version.is_empty() && version != "0.0.0.0").then_some(version)
}

pub fn installed_version() -> Option<String> {
    if let Some(folder) = std::env::var_os("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER") {
        // Fixed-version runtime: no registry entry is written.
        if std::path::Path::new(&folder).join("msedgewebview2.exe").exists() {
            return Some("fixed-version".to_string());
        }
    }
    let machine = format!(r"SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_GUID}");
    let user = format!(r"Software\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_GUID}");
    pv_at(HKEY_LOCAL_MACHINE, &machine).or_else(|| pv_at(HKEY_CURRENT_USER, &user))
}

pub fn preflight() -> bool {
    if installed_version().is_some() {
        return true;
    }
    let text = wide("Audio Share Server needs the Microsoft Edge WebView2 Runtime.\n\n\
        Click Yes to open the download page. Install the Evergreen Runtime, then start again.");
    let title = wide("Audio Share Server");
    let operation = wide("open");
    let url = wide(DOWNLOAD_URL);

    // SAFETY: valid null-terminated UTF-16 buffers; null HWND is allowed.
    unsafe {
        let answer = MessageBoxW(
            std::ptr::null_mut::<c_void>(),
            text.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONERROR,
        );
        if answer == IDYES {
            ShellExecuteW(
                std::ptr::null_mut::<c_void>(),
                operation.as_ptr(),
                url.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            );
        }
    }
    false
}
```

Call it before Tauri starts:

```rust
fn main() {
    #[cfg(windows)]
    if !webview2_check::preflight() {
        std::process::exit(1);
    }

    tauri::Builder::default()
        // ...
}
```

### Portable exe specifics

- `bundle.windows.webviewInstallMode` only configures MSI/NSIS installers. With `--no-bundle` or `targets: []`, it does nothing. Do not expect it to install WebView2 for a portable exe.
- With the MSVC target (`x86_64-pc-windows-msvc`, what `windows-latest` uses), `webview2-com-sys 0.39.1` statically links `WebView2LoaderStatic.lib` (`src/lib.rs:14-22`). The portable exe does not need `WebView2Loader.dll` next to it. With a GNU target it links `WebView2Loader.dll` instead, so stick to MSVC.
- Windows 11 includes the Evergreen runtime; most Windows 10 machines have it, but Windows 10 is not guaranteed. Keep the check.
- The portable exe still needs the .NET-free WebView2 runtime; the static loader only loads it.

## 9. Headless smoke testing on Ubuntu under Xvfb

Verified script (run from the workspace root after a debug build):

```bash
xvfb-run -a -s "-screen 0 1280x800x24" dbus-run-session -- bash -c '
  export WEBKIT_DISABLE_COMPOSITING_MODE=1
  export WEBKIT_DISABLE_DMABUF_RENDERER=1
  export LIBGL_ALWAYS_SOFTWARE=1
  export GDK_BACKEND=x11
  timeout 30 ./target/debug/AudioShareServer --smoke-test
'
```

The `--smoke-test` branch from section 4 logs and exits 0 after 4 seconds. Verified output from the probe:

```text
SMOKE visible=Ok(true) size=Ok(PhysicalSize { width: 420, height: 640 }) title=Ok("Audio Share Server")
```

Exit code was 0. An external X11 query also found a top-level window named `Audio Share Server` at 420x640. If `xdotool` is installed, `xdotool search --name "Audio Share Server"` is the shortest external check; `xwininfo -root -tree | grep "Audio Share Server"` also works. This machine did not have those tools, so the probe used a ctypes `libX11` query (`/home/w0n9/.claude/jobs/d82859a4/tmp/tauri-probe/x11-windows.py`).

Environment notes:

- `dbus-run-session` is recommended: the tray (StatusNotifierItem), notification plugin, and single-instance plugin all use the session bus. A bare Xvfb session may still start (this machine had an ambient session bus), but CI should provide one.
- `WEBKIT_DISABLE_COMPOSITING_MODE=1` and `WEBKIT_DISABLE_DMABUF_RENDERER=1` avoid WebKitGTK DMABUF/blank-window problems on virtual displays. `LIBGL_ALWAYS_SOFTWARE=1` avoids GPU access errors. `GDK_BACKEND=x11` prevents Wayland attempts.
- Harmless noise under Xvfb: `libEGL warning: failed to open /dev/dri/...: Permission denied` and `error: fuse init failed: Can't mount path /run/user/1000/doc`. The app still created its window and exited 0.
- A tray panel is not required to assert startup. Do not assert tray icon visibility in Xvfb; assert process start, `is_visible()`, window size, and exit 0.
- `tauri dev` under Xvfb was not tested. It should work with the same environment plus `npm run dev`; `beforeDevCommand` starts Vite on port 1420. Prefer the built binary for CI smoke tests because it does not depend on a dev server.
- Deb smoke test: `sudo dpkg -i ...deb`, run `/usr/bin/AudioShareServer --smoke-test` under the same Xvfb wrapper, then `sudo dpkg -r audio-share-server`. Verified.
- AppImage smoke test: direct execution requires FUSE2 (`libfuse2t64`); without it use `APPIMAGE_EXTRACT_AND_RUN=1 ./AudioShareServer_0.4.0_amd64.AppImage --smoke-test`. Both paths were verified.

## 10. Gotcha checklist

1. `tauri-build::build()` must be in `build.rs`; without it capabilities/config/icons are not generated.
2. Single-instance plugin must be the first plugin registered; no npm package, no capability permission.
3. `frontendDist` is only embedded with `tauri/custom-protocol`. Bare `cargo build --release` produces a dev binary pointing at `devUrl`; `tauri build` passes the feature itself (verified verbose command). If you force `custom-protocol` without `npm run build`, compilation panics on the missing `dist/`.
4. With a Cargo workspace, artifact paths are `<workspace>/target/...`, not `src-tauri/target/...`.
5. Linux tray needs `libayatana-appindicator3-1` at runtime and `libayatana-appindicator3-dev` at build time (Tauri's crate is still named `libappindicator`).
6. Linux Tauri does not deliver tray click events. Use the right-click menu; do not rely on left-click-to-toggle.
7. `show_menu_on_left_click(false)` is required on Windows for left-click-to-toggle; otherwise the menu opens on left click.
8. `app.exit(0)` for tray Exit; `window.close()` will hit `CloseRequested` and be turned into a hide by close-to-tray.
9. `api.prevent_close()` plus `hide()`; `minimize()` is not "minimize to tray" (it leaves a taskbar entry).
10. Async commands run off the UI thread, sync commands run on it. Use `spawn_blocking` for WASAPI/PipeWire. Do not hold a std mutex guard across `.await`.
11. `listen()` returns `Promise<UnlistenFn>`; always clean up with an unmount-safe pattern.
12. Capabilities are auto-loaded from `src-tauri/capabilities/*.json` if `app.security.capabilities` is unset. `core:default` includes `core:event:default` (so `listen` works). App-defined commands are allowed to all windows by default. Plugin JS calls need `opener:default`, `notification:default`, `autostart:default`.
13. `$schema: "../gen/schemas/desktop-schema.json"` only exists after a build; add `src-tauri/gen/` to `.gitignore`.
14. `csp: null` is the zero-risk v0.4.0 setting. If enabling CSP, include `connect-src ipc: http://ipc.localhost` and test Vite HMR with `devCsp`.
15. `bundle.windows.webviewInstallMode` is installer-only; it has no effect on the portable exe.
16. `webview2-com-sys` statically links the loader for MSVC but requires `WebView2Loader.dll` for GNU targets. Build Windows with MSVC.
17. Autostart path/args are written unquoted by auto-launch 0.6.0 on Windows and Linux. Avoid spaces in install paths or write the startup entry yourself.
18. Autostart on AppImage depends on the `APPIMAGE` env var; moving the AppImage breaks it. `--hide` avoids showing the window at login, but `visible: true` may flash.
19. Notification toasts are not fully supported for portable/uninstalled Windows apps. Use an in-app update banner as the primary UX.
20. AppImage built on Ubuntu 24.04 has a glibc 2.39 baseline. Direct execution needs FUSE2 (`libfuse2t64`); otherwise use `APPIMAGE_EXTRACT_AND_RUN=1`. Build-time linuxdeploy downloads need network; `NO_STRIP=true` if stripping fails.
21. `config.ini` next to the executable is writable on Windows portable but not on Linux deb (`/usr/bin`) or AppImage (read-only mount). Resolve with an XDG config path on Linux before implementing ticket 09.
22. Deb `depends` appends to Tauri defaults; declare only `libpipewire-0.3-0` to avoid duplicates. Ubuntu 24.04's t64 packages provide the non-t64 names.
23. `typescript@7.0.2` breaks `svelte-check@4.7.8` peer resolution. Pin `6.0.3`.
24. Runes in `.svelte.ts` files only; a plain `store.ts` will not compile `$state`.
25. Use `tauri::Emitter` and `tauri::Manager` traits; `emit`, `get_webview_window`, `manage`, and `state` are trait methods, not inherent methods.
26. `tauri::generate_handler!` may be called only once; list all commands in one place.
27. `productName` feeds `PackageInfo.name`, so the autostart Run value/desktop filename is `AudioShareServer`, and the deb package name becomes `audio-share-server`. Keep productName free of spaces and path-hostile characters.
28. Set `bundle.publisher`/Cargo authors and descriptions; otherwise deb shows `Maintainer: github` and `(none)`.

## 11. Source citations

Tauri docs:

- System tray: https://v2.tauri.app/learn/system-tray/
- Single instance: https://v2.tauri.app/plugin/single-instance/
- Autostart: https://v2.tauri.app/plugin/autostart/
- Opener: https://v2.tauri.app/plugin/opener/
- Notification: https://v2.tauri.app/plugin/notification/
- Capabilities: https://v2.tauri.app/security/capabilities/
- Calling Rust: https://v2.tauri.app/develop/calling-rust/
- Config reference: https://v2.tauri.app/reference/config/
- Platform config: https://v2.tauri.app/develop/configuration-files/
- CSP: https://v2.tauri.app/security/csp/
- Windows installer / WebView2: https://v2.tauri.app/distribute/windows-installer/
- Debian: https://v2.tauri.app/distribute/debian/
- AppImage: https://v2.tauri.app/distribute/appimage/
- Linux prerequisites: https://v2.tauri.app/start/prerequisites/
- Linux graphics: https://v2.tauri.app/develop/debug/linux-graphics/
- WebDriver CI (Xvfb): https://v2.tauri.app/develop/tests/webdriver/ci/
- tauri-action: https://github.com/tauri-apps/tauri-action

Microsoft:

- WebView2 distribution and detection: https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution
- WebView2 download page: https://developer.microsoft.com/en-us/microsoft-edge/webview2/

Local crate sources consulted (mirror path abbreviated as `$REG` = `~/.cargo/registry/src/mirrors.ustc.edu.cn-38d0e5eb5da2abae`):

- `$REG/tauri-runtime-wry-2.12.3/src/lib.rs:2783` and `:4678-4695` (runtime detection, missing-runtime dialog/error)
- `$REG/wry-0.57.1/src/webview2/mod.rs:2021` (`GetAvailableCoreWebView2BrowserVersionString`)
- `$REG/webview2-com-sys-0.39.1/src/lib.rs:14-22` (static loader for MSVC)
- `$REG/auto-launch-0.6.0/src/windows.rs`, `src/linux.rs`, `src/lib.rs` (Run key, XDG desktop, unquoted paths, Dynamic mode, XDG default)
- `$REG/tauri-codegen-2.7.4/src/context.rs:182-198` (frontendDist panic) and `:250-263` (PackageInfo.name = productName)
- `$REG/tauri-build-2.7.4/src/lib.rs:594-610,714` (`dev` cfg / `DEP_TAURI_DEV`)
- `$REG/tauri-2.12.5/build.rs:250-257` (`dev = !custom-protocol`)
- `$REG/tauri-utils-2.10.2/src/config.rs` (window/bundle/security/webview defaults)
- `$REG/tauri-plugin-autostart-2.7.0/src/lib.rs` (ManagerExt and init)
- `$REG/tauri-plugin-notification-2.8.0/src/desktop.rs:500-520` (Windows AppUserModelID behavior)

Prototype A:

- `git show prototype/server-gui:prototype/server-gui/index.html` (variant A markup and CSS tokens)
- `git show prototype/server-gui:prototype/server-gui/README.md` (variant A won; tray-first single window)
