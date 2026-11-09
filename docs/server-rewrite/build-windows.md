# Windows build and packaging (ticket 13)

Status: **packaging code, CI, and documentation implemented on the Linux
development host.** The Windows exe was **not built or executed** here: there
is no Windows linker or Windows runtime. `cargo check`/`clippy` for
`x86_64-pc-windows-msvc` pass, the WebView2 decision logic is unit-tested on
Linux, and the artifact packaging script is self-tested on Linux. Real Windows
acceptance is tracked in the checklist below.

The Windows release is a **portable `AudioShareServer.exe`** with no installer.
The GUI targets Windows 10+ and requires the Microsoft Edge WebView2 Runtime,
which is detected before Tauri starts.

## Artifacts

| Artifact | Source | Contents |
|---|---|---|
| `AudioShareServer.exe` | `gui/` → `npx tauri build --no-bundle` | Portable Server GUI (Tauri), icon embedded |
| `AudioShareServer.exe.sha256` | packaging script | `sha256sum`-format checksum |
| `audio-share-server-cmd-windows.zip` | `cargo build --release -p as-cmd` + packaging script | Top-level `audio-share-server-cmd/` containing `as-cmd.exe`, `LICENSE`, `README.md` |
| `audio-share-server-cmd-windows.zip.sha256` | packaging script | `sha256sum`-format checksum |

The archive keeps the v0.3.4 name and internal layout (`as-cmd.exe` inside
`audio-share-server-cmd/`). Artifact names do not contain the version; version
`0.4.0` is embedded in the binaries. Release wiring is ticket 16.

`tauri.conf.json` keeps:

- `productName: AudioShareServer`
- `mainBinaryName: AudioShareServer`
- `version: 0.4.0`
- `bundle.targets: []` (no MSI/NSIS installer)
- `bundle.icon` including `icons/icon.ico` (embedded in the exe)

`bundle.windows.webviewInstallMode` remains in the config for reference, but it
only affects MSI/NSIS bundles. It does nothing for the portable exe.

## Prerequisites

- Windows 10 or 11, x64.
- Rust stable 1.90 or newer (workspace MSRV) with the default Rust Windows target
  (`rustup default stable-x86_64-pc-windows-msvc`).
- Visual Studio 2022 Build Tools with the **Desktop development with C++**
  workload (the linker and Windows SDK used by the default Rust Windows
  target). The GNU target is not supported: `webview2-com-sys` statically
  links `WebView2LoaderStatic.lib` only for the default target; GNU builds
  would require `WebView2Loader.dll` next to the exe.
- Node.js 22 and npm.
- Microsoft Edge WebView2 Runtime (Evergreen). Windows 11 includes it; most
  Windows 10 machines have it, but it is not guaranteed.

## Build steps

Run from the repository root unless noted. The Cargo workspace puts build
outputs under `<repo>\target\release`, **not** `gui\src-tauri\target\release`.

```bash
cd gui
npm ci
npm run build
npx tauri build --no-bundle
cd ..
cargo build --release -p as-cmd

# Package the flat artifact directory (defaults to <repo>/dist-windows).
bash scripts/package-windows-artifacts.test.sh
bash scripts/package-windows-artifacts.sh
```

Expected outputs:

```text
<repo>/target/release/AudioShareServer.exe
<repo>/target/release/as-cmd.exe
<repo>/dist-windows/AudioShareServer.exe
<repo>/dist-windows/AudioShareServer.exe.sha256
<repo>/dist-windows/audio-share-server-cmd-windows.zip
<repo>/dist-windows/audio-share-server-cmd-windows.zip.sha256
```

Packaging script options (paths are resolved against `--root` when relative):

```text
scripts/package-windows-artifacts.sh [--root DIR] [--target-dir DIR] [--out-dir DIR]
```

Do not use a bare `cargo build --release` for the GUI exe: without
`tauri/custom-protocol` it points at the dev server (`http://localhost:1420`)
instead of the embedded frontend. `npx tauri build` enables the feature itself.
With `bundle.targets: []`, plain `npx tauri build` also produces only the exe,
but `--no-bundle` is explicit.

Verify a downloaded artifact in PowerShell:

```powershell
(Get-FileHash .\AudioShareServer.exe -Algorithm SHA256).Hash.ToLower()
Get-Content .\AudioShareServer.exe.sha256
```

or in Git Bash: `sha256sum -c AudioShareServer.exe.sha256`.

## WebView2 detection and preflight

The preflight runs in `main()` before `tauri::Builder` is created
(`gui/src-tauri/src/webview2_check.rs`). Tauri's own late check can end in an
`expect()` panic; this path exits cleanly instead.

Detection order:

1. `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` (fixed-version runtime): installed when
   `msedgewebview2.exe` exists in that folder. No registry entry is written for
   a fixed-version runtime.
2. `HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`
   value `pv` (machine-wide, 32-bit registry view used by x64 apps).
3. `HKCU\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`
   value `pv` (per-user install).

`pv` that is missing, empty/blank, or `0.0.0.0` means **not installed**. Any
other value is accepted. The pure decision function (`decide`) is unit-tested
with injected registry and environment values on every host.

When the runtime is missing, the app shows a native `MessageBoxW` titled
“Audio Share Server” explaining that the WebView2 Runtime is required:

- **Yes** opens
  <https://developer.microsoft.com/en-us/microsoft-edge/webview2/> in the
  default browser.
- **Yes or No** then exits the process with status **0** (no Tauri error, no
  panic). The user installs the Evergreen Runtime and starts the app again.

Windows 11 is expected to pass without a prompt. Windows 10 must be tested both
with the runtime present and absent (see checklist).

## CI

`.github/workflows/server-rewrite.yml` job **Windows portable packaging**
(`windows-packaging`, `windows-latest`) runs on pushes to
`integration/server-rewrite` and `ticket/**`, on pull requests touching
`gui/**`, `crates/**`, `scripts/**`, the workflow, and on manual dispatch:

1. Rust stable (Windows target), Node 22, npm cache, Rust target cache.
2. `npm ci`, `npm run build`, `npx tauri build --no-bundle`.
3. `cargo build --release -p as-cmd`.
4. Asserts `<workspace>/target/release/AudioShareServer.exe` and
   `as-cmd.exe` exist (workspace target path guard).
5. Runs `scripts/package-windows-artifacts.test.sh`.
6. Runs `scripts/package-windows-artifacts.sh`.
7. Uploads GitHub Actions artifacts:
   - `AudioShareServer-windows`: `AudioShareServer.exe` + `.sha256`
   - `audio-share-server-cmd-windows`: the zip + `.sha256`

The parallel `Rust (windows-latest)` job keeps running fmt, clippy, and the
workspace tests (including the WebView2 decision tests) on a native Windows
host.

## Manual acceptance checklist (Windows 10 and Windows 11)

Run each item on **both** Windows 10 x64 and Windows 11 x64. Record OS build,
artifact SHA-256, and pass/fail. Do not mark the ticket resolved without these
results.

### Artifact and launch

- [ ] CI `windows-packaging` job is green and its artifacts download.
- [ ] `AudioShareServer.exe.sha256` matches the downloaded exe
      (`sha256sum -c` or `Get-FileHash`).
- [ ] No installer/MSI/NSIS artifact exists; the exe runs directly from a
      normal folder (and from a path containing spaces).
- [ ] Launching the exe opens the 420×640 window (no console flash); the tray
      icon appears.
- [ ] `AudioShareServer.exe` is the process image name (Task Manager → Details).

### WebView2

- [ ] **Runtime present:** app starts normally on a machine with the Evergreen
      Runtime (Windows 11 default).
- [ ] **Runtime missing:** on a clean Windows 10 VM without WebView2, launch
      shows the native Audio Share Server dialog with the download link.
- [ ] Clicking **Yes** opens the Microsoft WebView2 page in the default
      browser; the app then exits with code 0.
- [ ] Clicking **No** exits with code 0. No Tauri panic dialog, no
      “WebView2 runtime not installed” crash text.
- [ ] After installing the Evergreen Runtime, launching the same exe succeeds.
- [ ] (If available) a per-user install (HKCU `pv`) and a fixed-version folder
      set via `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` are both detected.

### Server GUI behavior

- [ ] Start shows Running and a listen address; Stop returns to Stopped.
- [ ] Settings lists real Windows audio endpoints; encoding/host/port selectable.
- [ ] `config.ini` is created next to the exe; an existing v0.3.4 config is
      migrated (language, host, port, endpoint, encoding, app settings).
- [ ] Tray menu Show / Start / Stop / Exit works; left-click toggles the window;
      close button follows `WhenClose` (minimize to tray or exit).
- [ ] Single instance: a second launch focuses the existing window.
- [ ] Autostart writes/removes the HKCU Run entry (`"<exe>" /hide`) and survives
      a reboot.
- [ ] Repair Firewall prompts for UAC and creates both inbound rules; a phone
      can connect with the firewall enabled. Cancelling UAC shows a readable error.
- [ ] Sound Panel opens `control.exe /name Microsoft.Sound`.
- [ ] Update check reports No Update or finds a newer release; disabling
      auto-update stops the periodic check; About shows 0.4.0, homepage, license.
- [ ] en-US and zh-CN render after restart; system-locale default works.
- [ ] A v0.3.4 Android Client connects and plays audio (protocol acceptance).

### as-cmd archive

- [ ] Extract `audio-share-server-cmd-windows.zip`; it contains
      `audio-share-server-cmd/as-cmd.exe`, `LICENSE`, `README.md`.
- [ ] Zip SHA-256 matches the `.sha256` file.
- [ ] `as-cmd.exe --version` prints the expected name/version/homepage.
- [ ] `as-cmd.exe --list-encoding` and `as-cmd.exe -l` work; `-b` serves audio
      to the Android Client.

## Known limitations and residual verification

- Not built or executed on Windows from this Linux host; `cargo
  check/clippy --target x86_64-pc-windows-msvc` only type-checks the code (the
  two `GNU compiler is not supported for this target` warnings come from
  `tauri-build` resource embedding and disappear in a native Windows build).
- No code signing: SmartScreen may warn on first run; this matches v0.3.4 and
  the 0.4.0 scope.
- No installer and no Tauri updater: users replace the portable exe manually.
- Toast notifications are not fully supported for uninstalled portable apps;
  the in-app update banner is the primary UX.
- Real WebView2 detection, the missing-runtime dialog, and Windows 10/11 GUI
  acceptance remain **manual Windows verification** for ticket 13.
