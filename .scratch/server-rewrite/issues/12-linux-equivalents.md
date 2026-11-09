# 12: Linux equivalents

**What to build:** Linux 上验证托盘右键菜单、XDG autostart、更新检查与 About；对 Windows-only 的 Firewall Repair 与 Sound Panel 给出说明链接；在 Ubuntu 24.04 上可用。

**Blocked by:** 10 (Lifecycle + i18n + About + update check)

**Status:** resolved

- [ ] 托盘右键菜单在 Ubuntu 24.04 可用。（实现与 DBus 菜单已验证；真实桌面面板上的右键点击无法在 Xvfb 中执行，见 Comments 的 Residual。）
- [x] XDG autostart 生成并在登录后生效。
- [x] 更新检查与 About 在 Linux 可用。
- [x] Windows-only 功能有清晰说明链接。
- [x] 不依赖 Windows 专属 API。

## Comments

2026-11-09 — branch `ticket/12-linux-equivalents`; implementation `05c9686`,
integration merge `91091d4` (ticket 11 Windows maintenance), docs `17826ff`.
Base at start: `9731e73`.

Implemented:
- `gui/src-tauri/src/help.rs`: fixed Linux help-topic mapping
  (`pipewire` -> https://docs.pipewire.org/, `desktopSound` ->
  https://help.gnome.org/users/gnome-help/stable/sound-volume.html), topic
  parsing rejects arbitrary ids, and URL invariants (https, no query/fragment,
  unique).
- `open_help` Tauri command opens only a mapped topic through
  `tauri-plugin-opener`; the webview never supplies a URL.
- `gui/src/views/LinuxSettings.svelte`: Linux-only rows shown when
  `platform == "linux"` — a clear "Windows-only features" explanation plus
  "Desktop sound settings" and "PipeWire documentation" links. Ticket 11's
  enabled Repair Firewall and Sound Panel rows remain untouched under
  `platform == "windows"`. Locale keys are prefixed `linux.*`; en-US/zh-CN
  files are the union of ticket 11 and ticket 12 keys with parity (114 keys
  each).
- `docs/server-rewrite/linux.md`: dependencies (WebKitGTK 4.1, GTK, appindicator,
  PipeWire, xdg-utils), XDG autostart path/content, tray limitation, where to
  configure audio, Windows-only equivalents, update/About, no-Windows-API
  checks, headless recipe, screenshots, residual manual items.

Commands/results (Ubuntu 24.04.5, Xvfb 1280x800, fresh `dbus-run-session`,
private PipeWire 1.0.5 + WirePlumber null `Audio/Sink`):
- `cargo fmt --all`; `cargo test --workspace` pass (77 GUI tests incl. 4
  help-link tests and the autostart path/content tests; all workspace suites
  0 failed).
- `cargo clippy --workspace --all-targets -- -D warnings` pass.
- `npm run check` 0 errors/0 warnings; `npm run build`;
  `npx tauri build --debug --no-bundle` -> `target/debug/AudioShareServer`.
- `cargo check --target x86_64-pc-windows-msvc -p audio-share-server-gui -p
  audio-share-core` pass after the ticket 11 merge (only the known tauri-build
  GNU-compiler warnings).
- No Windows API on Linux: `cargo tree --target x86_64-unknown-linux-gnu` for
  the GUI and core contains no `winreg`, `windows-sys`, or `wasapi`; the Linux
  check/clippy/test builds compile without them. Autostart `winreg` and the
  WASAPI module are behind `#[cfg(windows)]` / target-specific dependencies.

XDG autostart (UI toggle and startup reconciliation, Xvfb + dbus):
- Toggle on wrote `$XDG_CONFIG_HOME/autostart/AudioShareServer.desktop`:
  `Exec="<exe>" --hide`, `TryExec=<exe>`, `Type=Application`,
  `Terminal=false`, `X-GNOME-Autostart-enabled=true`; `AutoRun=1`.
  Toggle off removed the file and wrote `AutoRun=0`.
- With `XDG_CONFIG_HOME` unset, startup created
  `~/.config/autostart/AudioShareServer.desktop` (default path).
- `AutoRun=0` + stale entry -> startup removed it; `AutoRun=1` + stale content
  -> startup rewrote the quoted entry.
- `--hide --smoke-test` printed `SMOKE visible=Ok(false)` (hidden autostart
  launch, no window flash).

Linux settings, update, About (Xvfb, xdotool + ImageMagick):
- Settings shows the Linux section with the Windows-only explanation and both
  links; ticket 11's Windows rows do not render on Linux.
- Clicking the link rows passed exactly the mapped URLs to a fake `xdg-open`
  (opener plugin): GNOME sound help, then PipeWire docs. Chinese labels render
  on one line.
- Manual update check against a local mock `v9.9.9`: request received, settings
  showed "A new version is available: v9.9.9", in-app banner shown; the
  homepage button passed `https://github.com/mkckr0/audio-share` to the opener.
- About modal: Audio Share Server, version 0.4.0, homepage, Apache License 2.0,
  copyright.
- Start/Stop with the null `Audio/Sink`: UI Start -> `LISTEN 127.0.0.1:65050`,
  `Running=1`; Stop -> listener closed, `Running=0`.

Tray (Xvfb has no StatusNotifier panel):
- A throwaway minimal `org.kde.StatusNotifierWatcher` on the private bus
  observed `RegisterStatusNotifierItem` for
  `/org/ayatana/NotificationItem/tray_icon_tray_app_main_tray`, title
  `AudioShareServer`, status `Active`, and the dbusmenu layout
  `Show` / `Start` / `Stop` (disabled while stopped) / `Exit`.
- Activating those exported items over DBus (`com.canonical.dbusmenu`
  `Event("clicked")`, not a panel right-click) ran the same Tauri
  `on_menu_event` handlers: Start opened `127.0.0.1:65050` and flipped the menu
  to Start disabled / Stop enabled; Stop closed the listener and reverted it.
- No real right-click was performed; a panel interaction needs a real desktop.

Screenshots (Xvfb, committed under `docs/server-rewrite/img/`):
`ticket12-settings-linux.png`, `ticket12-settings-linux-zh.png`,
`ticket12-about-linux.png`, `ticket12-update-linux.png`,
`ticket12-running-linux.png`, `ticket12-stopped-linux.png`.

Residual / not claimed:
- Real GNOME (AppIndicator extension), KDE, or XFCE tray right-click
  interaction was not executed; Xvfb only proves the exported item/menu and the
  menu handlers. Referred to ticket 15.
- Real audio hardware, hotplug, suspend/resume, Wayland sessions, and the
  8-hour soak remain in ticket 15; headless runs used `GDK_BACKEND=x11`.
- The real GitHub API returned HTTP 403 rate-limit on this shared IP; the
  request path/TLS were exercised, and the mock covered newer/up-to-date/error
  handling.
