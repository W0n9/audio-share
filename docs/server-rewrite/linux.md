# Linux support and Windows equivalents

Status: implemented by ticket 12. Verified on Ubuntu 24.04.5 (x86_64) under
Xvfb + a private `dbus-run-session` + a private PipeWire 1.0.5/WirePlumber
session with a null `Audio/Sink`. Real desktop panel interaction and real
audio hardware remain manual checks (ticket 15).

The Server GUI is the same Tauri application on Windows and Linux. This page
covers the Linux-specific runtime, startup entry, tray limitation, the
replacement for the Windows-only maintenance actions, and the headless
verification recipe.

## Runtime dependencies

| Package | Why |
|---|---|
| `libwebkit2gtk-4.1-0` | Tauri webview (WebKitGTK 4.1). This is the Linux equivalent of WebView2 and is not bundled. |
| `libgtk-3-0` (Ubuntu 24.04 `libgtk-3-0t64` provides it) | Tauri/GTK shell. |
| `libsoup-3.0-0` | WebKitGTK network stack. |
| `libayatana-appindicator3-1` | StatusNotifierItem tray icon and right-click menu. |
| `libpipewire-0.3-0` | Audio capture through the PipeWire client library. |
| PipeWire daemon + a session manager (WirePlumber) | Provides the `Audio/Sink` endpoints the Server enumerates and captures. |
| `libfuse2` (Ubuntu 24.04 `libfuse2t64`) | Only for running the AppImage directly. Without FUSE use `APPIMAGE_EXTRACT_AND_RUN=1`. |
| `xdg-utils` (`xdg-open`) | Opened by the Tauri opener plugin for About/update/help links. Normal desktop installs already have it. |

The deb declares the Tauri defaults (`libwebkit2gtk-4.1-0`, `libgtk-3-0`,
`libayatana-appindicator3-1`) plus `libpipewire-0.3-0`; see ticket 14 for
packaging. PipeWire and the session manager are not packagable dependencies of
the app: the user's desktop session must provide them.

Build dependencies: the Tauri list in `tauri.md` §7
(`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libsoup-3.0-dev`,
`libayatana-appindicator3-dev`, `librsvg2-dev`, `libxdo-dev`, `patchelf`,
`libpipewire-0.3-dev`, `libspa-0.2-dev`) plus Rust 1.90+ and Node 22.

## Configuration and log location

Linux packages cannot write next to `/usr/bin/AudioShareServer` or inside a
read-only AppImage mount. Resolution order is:

1. `AUDIO_SHARE_CONFIG` (absolute config file path), if set.
2. `config.ini` next to the executable when that directory is writable
   (development builds, portable installs).
3. `$XDG_CONFIG_HOME/audio-share-server/config.ini`, default
   `~/.config/audio-share-server/config.ini`.

`server.log` lives in the same directory as the resolved `config.ini`. A
legacy executable-directory config is migrated when the fallback is used
(ticket 09).

## XDG autostart

`[App Settings] AutoRun=1` writes, and `AutoRun=0` removes,
`$XDG_CONFIG_HOME/autostart/AudioShareServer.desktop`; when
`XDG_CONFIG_HOME` is unset the path is
`~/.config/autostart/AudioShareServer.desktop`. The file is written directly
(not through `auto-launch`) so the executable is always quoted:

```ini
[Desktop Entry]
Type=Application
Name=Audio Share Server
Exec="/home/user/Audio Share/AudioShareServer" --hide
TryExec=/home/user/Audio Share/AudioShareServer
Terminal=false
X-GNOME-Autostart-enabled=true
```

- The quoted `Exec=... --hide` form is required for paths with spaces; the
  old `auto-launch` crate joined paths and arguments unquoted.
- Inside an AppImage, `$APPIMAGE` is used as the executable; moving or renaming
  the AppImage breaks the entry (re-enable AutoRun after moving it).
- `--hide` (and `-hide`/`/hide`) starts the GUI hidden in the tray; the window
  is not flashed at login.
- `AutoRun` is authoritative at every launch: startup creates/rewrites the
  entry when enabled and removes it when disabled. Failures are reported as a
  localized notice instead of being silent.

Verified under Xvfb + `dbus-run-session`:

- toggle on in Settings created the file with the exact content above and
  `AutoRun=1`; toggle off removed it and wrote `AutoRun=0`;
- with `XDG_CONFIG_HOME` unset, startup created
  `~/.config/autostart/AudioShareServer.desktop`;
- a stale entry was removed with `AutoRun=0` and rewritten with `AutoRun=1`;
- the `--hide --smoke-test` launch reported `SMOKE visible=Ok(false)`.

## Tray: what works, and the Xvfb limitation

Tauri's Linux tray uses a StatusNotifierItem (via libayatana-appindicator).
The application offers the same menu as Windows: **Show**, **Start**,
**Stop** (dynamic enabled state), separator, **Exit**.

- A StatusNotifierWatcher/panel must be present to display the icon. GNOME
  needs the "AppIndicator and KStatusNotifierItem Support" extension; KDE,
  XFCE, and similar panels provide a watcher themselves.
- Tauri does not deliver tray mouse events on Linux. Left-click-to-toggle is
  Windows-only; **the right-click menu is the Linux tray control**, which is
  also the documented user story (spec #36).
- Xvfb has no panel and therefore no watcher. For verification, a throwaway
  minimal `org.kde.StatusNotifierWatcher` was started on the private session
  bus. It observed:
  - `RegisterStatusNotifierItem('/org/ayatana/NotificationItem/tray_icon_tray_app_main_tray')`;
  - item title `AudioShareServer`, status `Active`, and menu object
    `/org/ayatana/NotificationItem/tray_icon_tray_app_main_tray/Menu`;
  - `com.canonical.dbusmenu` layout `Show` / `Start` / `Stop` (disabled while
    stopped) / `Exit`.
- Activating those exported menu items over DBus (a `com.canonical.dbusmenu`
  `Event("clicked")`, not a real panel right-click) exercised the same Tauri
  `on_menu_event` handlers: Start opened `127.0.0.1:65050` and flipped the
  item state to Start disabled / Stop enabled; Stop closed the listener and
  flipped it back.
- A real right-click on GNOME/KDE is still a manual check because Xvfb cannot
  display or interact with a StatusNotifier panel. Do not record a panel click
  as performed.

## Windows-only actions and their Linux equivalents

The Settings view shows the Windows-only rows only when
`platform == "windows"`. On Linux, a `LinuxSettings` component replaces them:

| Windows action | Linux behavior |
|---|---|
| Repair Firewall | No GUI action. Linux has no Windows Firewall; if a host firewall (nftables/ufw/firewalld) is in use, allow inbound TCP and UDP on the Server port (default 65530). The Settings view explains that the action is Windows-only. |
| Sound Panel (`control.exe /name Microsoft.Sound`) | No GUI action. Use the desktop sound settings to choose the output Endpoint (GNOME: Settings → Sound; KDE: System Settings → Audio) and PipeWire/WirePlumber to manage Endpoints. |

The Linux section shows a clear "Windows-only features" explanation and two
opener-backed documentation links:

- **Desktop sound settings**: https://help.gnome.org/users/gnome-help/stable/sound-volume.html
- **PipeWire documentation**: https://docs.pipewire.org/

The frontend sends only a topic id (`desktopSound` or `pipewire`) to the
`open_help` Tauri command. Rust maps it to a fixed URL and opens it through
`tauri-plugin-opener`; the webview cannot ask the command to open arbitrary
URLs. The mapping is unit-tested in `gui/src-tauri/src/help.rs`.

### Where audio is configured on Linux

- The app lists PipeWire nodes with `media.class = "Audio/Sink"`; a sink's
  `object.serial` is the Endpoint ID used by the Wire Protocol, and the
  default Endpoint is the sink with the highest `priority.session`.
- Inspect the graph with `wpctl status`, `pw-dump`, or `pw-cli ls Node`.
  `wpctl set-default <id>` changes the session manager's default sink.
- Selecting a different output in the desktop sound settings changes the
  system default; the app's "Default (system)" Endpoint follows the
  `priority.session` ordering used by the capture backend (see
  `capture-linux.md`).
- A missing PipeWire daemon or session manager is reported as a localized
  startup notice; the app still opens so settings/help remain reachable.

## Update check and About

Both use the same code path as Windows:

- `GET https://api.github.com/repos/mkckr0/audio-share/releases/latest` with
  `User-Agent: AudioShareServer/0.4.0`; compare `tag_name` to `v0.4.0`; show an
  in-app banner plus a native notification and open `html_url` through the
  opener. Automatic checks run every 3 hours while
  `AutoUpdate` is enabled; manual checks report "up to date" or a readable
  error. `AUDIO_SHARE_UPDATE_URL` is the replay-test seam.
- About shows the build version (`0.4.0`), homepage
  `https://github.com/mkckr0/audio-share`, and Apache License 2.0; the
  homepage button uses the opener plugin.

Verified with a local mock returning `v9.9.9`: the settings view showed
"A new version is available: v9.9.9", the in-app banner appeared, and the
homepage button passed `https://github.com/mkckr0/audio-share` to the system
opener. The real GitHub API path is the same code; on this shared IP it
returned a readable HTTP 403 rate-limit error (ticket 10).

## No Windows-only API on Linux

- Autostart (`winreg`) is in `#[cfg(windows)]` code with a target-specific
  dependency; the Linux branch uses XDG files.
- WASAPI lives in `crates/audio-share-core/src/capture/windows/` behind
  `#[cfg(windows)]`; Linux builds use the PipeWire backend.
- `cargo tree -p audio-share-server-gui --target x86_64-unknown-linux-gnu`
  and the same command for `audio-share-core` contain no `winreg`,
  `windows-sys`, or `wasapi` entries.
- Linux `cargo check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  and `cargo test --workspace` pass. The Windows-only maintenance commands are
  still type-checked with
  `cargo check --target x86_64-pc-windows-msvc -p audio-share-server-gui -p audio-share-core`.

## Headless verification recipe

```bash
# Build the GUI (npm build runs first; no bundle)
cd gui && npm ci && npx tauri build --debug --no-bundle
cd ..

# Xvfb + private session bus + private PipeWire/WirePlumber null sink
xvfb-run -a -s "-screen 0 1280x800x24" scripts/pw-test-session.sh -- \
  bash -c '
    export WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1
    export LIBGL_ALWAYS_SOFTWARE=1 GDK_BACKEND=x11
    export HOME=/tmp/t12/home XDG_CONFIG_HOME=/tmp/t12/home/.config
    export AUDIO_SHARE_CONFIG=/tmp/t12/config.ini
    target/debug/AudioShareServer
  '
```

`scripts/pw-test-session.sh` creates a `TestSink` null `Audio/Sink` and cleans
up PipeWire afterwards. The probing used `xdotool` for clicks and ImageMagick
`import` for screenshots; no panel exists, so tray interaction is limited to
the DBus observations described above.

Screenshots (all taken in this environment):

| File | Content |
|---|---|
| `img/ticket12-settings-linux.png` | Linux Settings section: Windows-only explanation, desktop sound settings and PipeWire links, About. |
| `img/ticket12-settings-linux-zh.png` | The same section in Simplified Chinese. |
| `img/ticket12-about-linux.png` | About modal with version 0.4.0, homepage, Apache-2.0. |
| `img/ticket12-update-linux.png` | Update banner from the mock `v9.9.9` release. |
| `img/ticket12-running-linux.png` | Server running on `127.0.0.1:65050` with the PipeWire null sink. |
| `img/ticket12-stopped-linux.png` | Server stopped again. |

## Residual manual checks

- Right-click the tray icon on a real GNOME (with AppIndicator extension),
  KDE, or XFCE desktop and use Show/Start/Stop/Exit. Xvfb can only prove the
  exported menu and its handlers, not the panel interaction.
- Ticket 15 executed the scripted Wire Protocol matrix and the 12-minute and
  8-hour release soaks; real audio hardware, suspend/resume, hardware hotplug,
  and real tray panel interaction remain (`acceptance-matrix.md`, `soak.md`,
  `hotplug-sleep.md`, `gui-acceptance-checklist.md`).
- Wayland sessions were not exercised; the headless runs used
  `GDK_BACKEND=x11` under Xvfb.
