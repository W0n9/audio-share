# Server GUI manual acceptance checklist (v0.4.0)

Use on Windows 10/11 and Ubuntu 24.04 (X11 and Wayland if available). Record pass/fail and evidence (screenshot/log line). Items marked [env] require a real desktop session.

## Launch and single instance
- [ ] First launch opens one window with status Stopped and a listening address.
- [ ] Launching a second instance does not create a second server; the existing window is shown/focused.
- [ ] `--hide` (autostart path) starts with no visible window, tray icon present.
- [ ] Closing the window with WhenClose=Exit exits the process and tray icon.
- [ ] Closing the window with WhenClose=Minimize keeps the process/server running; tray Show restores it.

## Tray
- [ ] Tray icon appears (Windows notification area; Linux SNI/appindicator).
- [ ] Windows: left click toggles window; right click opens menu.
- [ ] Linux: right click opens menu (left click is not a dependency; document observed behavior).
- [ ] Menu items Show, Start/Stop (label reflects state), Exit work.
- [ ] Start/Stop from tray updates the window state and address without opening the window.
- [ ] Closing the app removes the tray icon.

## Main view (prototype A)
- [ ] Status pill shows Stopped/Running and updates live.
- [ ] Listening address shows host:port, updates after Start.
- [ ] Start/Stop is the primary action and does not freeze the UI (window remains responsive).
- [ ] Endpoint and Encoding selectors list real values; default is marked.
- [ ] Host and Port are editable; invalid host/port starts fail with a readable error, not a crash.
- [ ] Changing settings while stopped persists; changing while running is disabled or applied after restart.
- [ ] Start failure (bad endpoint, port in use) shows a readable message and returns to Stopped.
- [ ] Settings view opens/closes without losing state.

## Settings and config
- [ ] Host, Port, Endpoint, Encoding persist across restart.
- [ ] Reset restores defaults and writes config.ini.
- [ ] Existing v0.3.4 config.ini (`[Network] host/port`, `[Capture] endpoint/encoding`, `[App] Running/language/configVersion`, `[App Settings] ...`) is read without losing values.
- [ ] configVersion mismatch: migration result is visible/logged; user values are not silently dropped.
- [ ] Corrupt config.ini: backup is created, defaults load, readable warning shown/logged.
- [ ] Atomic write: kill the app during save; config.ini is either old or new, never truncated.
- [ ] `[App] Running` reflects actual state after start/stop and is used by WhenAppStart=Keep last state.

## Lifecycle
- [ ] Autostart off removes the Windows Run entry / XDG autostart file.
- [ ] Autostart on creates it and launches hidden after login (reboot/re-login required).
- [ ] WhenAppStart=Do nothing does not start the server.
- [ ] WhenAppStart=Start server starts it after launch.
- [ ] WhenAppStart=Keep last state starts only if the previous session was running.
- [ ] Single instance works when started from autostart and from the Start menu.

## i18n
- [ ] en-US shows all labels in English.
- [ ] zh-CN shows all labels in Simplified Chinese; no missing-key placeholders.
- [ ] Language=default follows system locale.
- [ ] Language change persists and applies on next startup (not live, per spec).
- [ ] Tray menu, About, update prompts, and error messages are localized.

## About and updates
- [ ] About shows product name, version 0.4.0, homepage link, and Apache-2.0 license.
- [ ] Homepage link opens the default browser.
- [ ] Manual Check for Update reports No Update when current, or shows the new version and opens the release page when newer.
- [ ] AutoUpdate off: no background checks/prompts.
- [ ] AutoUpdate on: checks at startup and every 3 hours; notification/balloon appears; clicking opens the release page.
- [ ] Network failure during update check yields a readable error only for manual checks; background failures are silent/logged.

## Windows-only
- [ ] Repair Firewall prompts for elevation, creates TCP and UDP inbound rules, and the Android client can connect.
- [ ] Cancelling UAC or a netsh failure shows a clear error.
- [ ] Sound Panel opens `control.exe /name Microsoft.Sound`.
- [ ] Repair Firewall and Sound Panel are hidden on Linux; Linux shows documentation links instead.

## Linux-only
- [ ] Package installs (deb) and AppImage runs after chmod +x.
- [ ] Missing WebKitGTK/PipeWire produces a readable error or documented dependency message.
- [ ] XDG autostart file is created at `~/.config/autostart/`.
- [ ] Tray right-click menu works under the target desktop environment.
- [ ] PipeWire sinks are listed; default follows priority.session; monitor audio plays on the phone.

## Client-facing regression (with v0.3.4 APK)
- [ ] Client connects, shows the format, and plays audio.
- [ ] All 5 encodings play; mono and stereo; 44.1 and 48 kHz.
- [ ] Multiple clients play simultaneously.
- [ ] Client reconnect works after server stop/start.
- [ ] Heartbeat timeout closes dead sessions within ~5 s; active sessions stay alive.
- [ ] No UDP parsing misalignment after long playback.
