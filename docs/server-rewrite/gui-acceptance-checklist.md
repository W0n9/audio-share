# Server GUI manual acceptance checklist (v0.4.0)

Status for ticket 15: **partial**. Items marked `[x]` were observed or
automated on this host (Ubuntu 24.04.5, Xvfb 1280x800, a private
`dbus-run-session`, a throwaway StatusNotifierWatcher, and a private
PipeWire/WirePlumber null sink). `[~]` means partially observed: the exported
behavior was exercised, but the real desktop/panel interaction was not. `[ ]`
requires a real desktop, Windows, or an Android phone and remains a release
gate. Evidence is in `docs/server-rewrite/linux.md` (ticket 12),
`.scratch/server-rewrite/issues/10-lifecycle-i18n-about-update.md` (ticket 10),
and the screenshots referenced there.

Use on Windows 10/11 and Ubuntu 24.04 (X11 and Wayland if available). Record
pass/fail and evidence (screenshot/log line). Items marked [env] require a real
desktop session.

## Launch and single instance
- [x] First launch opens one window with status Stopped and a listening address.
  (Xvfb screenshots `ticket12-stopped-linux.png`, `ticket12-running-linux.png`.)
- [x] Launching a second instance does not create a second server; the existing
  window is shown/focused. (Ticket 10: second launch exited 0 in ~0 s and
  re-showed the hidden existing window.)
- [x] `--hide` (autostart path) starts with no visible window, tray icon present.
  (Ticket 12: `SMOKE visible=Ok(false)`; SNI item still registered.)
- [x] Closing the window with WhenClose=Exit exits the process and tray icon.
  (Ticket 10: listener released, `Running=0`, process exited.)
- [x] Closing the window with WhenClose=Minimize keeps the process/server
  running; tray Show restores it. (Ticket 10.)

## Tray
- [~] Tray icon appears (Windows notification area; Linux SNI/appindicator).
  Xvfb: a minimal `org.kde.StatusNotifierWatcher` observed
  `RegisterStatusNotifierItem`, title `AudioShareServer`, status `Active`;
  no real panel exists to display it.
- [ ] Windows: left click toggles window; right click opens menu. [env]
- [~] Linux: right click opens menu (left click is not a dependency; document
  observed behavior). The exported `com.canonical.dbusmenu` layout
  `Show` / `Start` / `Stop` / `Exit` was read and its `Event("clicked")`
  handlers were activated over DBus; a real panel right-click was not performed.
- [x] Menu items Show, Start/Stop (label reflects state), Exit work.
  (Handlers exercised over DBus; Start opened `127.0.0.1:65050` and flipped
  enabled states, Stop closed it.)
- [~] Start/Stop from tray updates the window state and address without opening
  the window. State flip observed; "window not opened" is normal for a menu
  activation but was not re-checked on a panel.
- [ ] Closing the app removes the tray icon. [env]

## Main view (prototype A)
- [x] Status pill shows Stopped/Running and updates live. (Xvfb screenshots.)
- [x] Listening address shows host:port, updates after Start. (Ticket 12.)
- [x] Start/Stop is the primary action and does not freeze the UI. (Tauri
  commands are async; Xvfb clicks returned immediately.)
- [~] Endpoint and Encoding selectors list real values; default is marked.
  Endpoint list against the null sink was observed at Start; the encoding list
  is the frozen six-entry list. A real device list needs a desktop.
- [ ] Host and Port are editable; invalid host/port starts fail with a readable
  error, not a crash. [env]
- [ ] Changing settings while stopped persists; changing while running is
  disabled or applied after restart. (Config persistence is unit-tested; the
  GUI click path was not re-run here.) [env]
- [ ] Start failure (bad endpoint, port in use) shows a readable message and
  returns to Stopped. [env]
- [x] Settings view opens/closes without losing state. (Ticket 12 Xvfb
  settings screenshots.)

## Settings and config
- [ ] Host, Port, Endpoint, Encoding persist across restart. [env]
- [ ] Reset restores defaults and writes config.ini. [env]
- [x] Existing v0.3.4 config.ini (`[Network] host/port`, `[Capture]
  endpoint/encoding`, `[App] Running/language/configVersion`,
  `[App Settings] ...`) is read without losing values. (Automated config tests;
  a v1 Chinese config showed the localized migration notice under Xvfb.)
- [x] configVersion mismatch: migration result is visible/logged; user values
  are not silently dropped. (Ticket 10: version 0 -> 2, unknown key kept.)
- [x] Corrupt config.ini: backup is created, defaults load, readable warning
  shown/logged. (Automated config recovery tests.)
- [x] Atomic write: kill the app during save; config.ini is either old or new,
  never truncated. (Automated temp-file + rename/ReplaceFile tests.)
- [x] `[App] Running` reflects actual state after start/stop and is used by
  WhenAppStart=Keep last state. (Ticket 10 launch-matrix checks with `ss`.)

## Lifecycle
- [x] Autostart off removes the Windows Run entry / XDG autostart file.
  (Linux XDG path automated and observed; Windows registry path compile-only.)
- [ ] Autostart on creates it and launches hidden after login (reboot/re-login
  required). [env]
- [x] WhenAppStart=Do nothing does not start the server. (Ticket 10.)
- [x] WhenAppStart=Start server starts it after launch. (Ticket 10.)
- [x] WhenAppStart=Keep last state starts only if the previous session was
  running. (Ticket 10.)
- [~] Single instance works when started from autostart and from the Start
  menu. Single instance was verified on a fresh bus; the login-session
  autostart path is a real-desktop check.

## i18n
- [x] en-US shows all labels in English. (Ticket 10 screenshots.)
- [x] zh-CN shows all labels in Simplified Chinese; no missing-key placeholders.
  (Ticket 10/12 screenshots; en-US/zh-CN key parity is unit-tested.)
- [x] Language=default follows system locale. (Ticket 10 `LANG` checks.)
- [x] Language change persists and applies on next startup (not live, per spec).
  (Ticket 10 restart hint.)
- [~] Tray menu, About, update prompts, and error messages are localized.
  Tray labels/About/settings observed; Windows toast text and real desktop
  error dialogs need a real desktop.

## About and updates
- [x] About shows product name, version 0.4.0, homepage link, and Apache-2.0
  license. (Ticket 12 screenshot.)
- [x] Homepage link opens the default browser. (Fake `xdg-open` received the
  expected URL.)
- [x] Manual Check for Update reports No Update when current, or shows the new
  version and opens the release page when newer. (Ticket 10 local mock
  `v9.9.9` and same-version checks.)
- [x] AutoUpdate off: no background checks/prompts. (Ticket 10.)
- [~] AutoUpdate on: checks at startup and every 3 hours; notification/balloon
  appears; clicking opens the release page. Startup check and banner observed;
  the 3-hour interval was not observed in real time, and the native toast
  click is not wired by the Tauri notification plugin (documented).
- [x] Network failure during update check yields a readable error only for
  manual checks; background failures are silent/logged. (Ticket 10 HTTP
  500/malformed JSON checks; the real GitHub API returned 403 on this IP.)

## Windows-only
- [ ] Repair Firewall prompts for elevation, creates TCP and UDP inbound rules,
  and the Android client can connect. [env: Windows]
- [ ] Cancelling UAC or a netsh failure shows a clear error. [env: Windows]
- [ ] Sound Panel opens `control.exe /name Microsoft.Sound`. [env: Windows]
- [x] Repair Firewall and Sound Panel are hidden on Linux; Linux shows
  documentation links instead. (Ticket 12: Linux settings section and fixed
  help URLs; Windows rows do not render on Linux.)

## Linux-only
- [x] Package installs (deb) and AppImage runs after chmod +x. (Ticket 14,
  merged at `b456c10`: `dpkg -i` installs `/usr/bin/AudioShareServer`; the
  AppImage runs directly with FUSE 2 and with `APPIMAGE_EXTRACT_AND_RUN=1`;
  Start/Stop/Quit verified on the packaged binary under Xvfb. Not re-run by
  ticket 15.)
- [x] Missing WebKitGTK/PipeWire produces a readable error or documented
  dependency message. (Ticket 14 clean-container dependency resolution plus
  `build-linux.md`; GUI startup failure path not re-executed here.)
- [x] XDG autostart file is created at `~/.config/autostart/`. (Ticket 12.)
- [~] Tray right-click menu works under the target desktop environment.
  Exported SNI/dbusmenu item observed under Xvfb; a real GNOME (AppIndicator
  extension), KDE, or XFCE panel click is still required.
- [~] PipeWire sinks are listed; default follows priority.session; monitor
  audio plays on the phone. Sink listing/default were verified against the
  headless null sink; playback on the phone requires the real APK (see
  `android-v034-acceptance.md`).

## Client-facing regression (with v0.3.4 APK)
All items below are **NOT EXECUTED**; no Android device is attached.
See `docs/server-rewrite/android-v034-acceptance.md` for the exact matrix and
evidence requirements.
- [ ] Client connects, shows the format, and plays audio.
- [ ] All 5 encodings play; mono and stereo; 44.1 and 48 kHz.
- [ ] Multiple clients play simultaneously.
- [ ] Client reconnect works after server stop/start.
- [ ] Heartbeat timeout closes dead sessions within ~5 s; active sessions stay
  alive. (Server side is covered by the scripted matrix: close at ~6 s.)
- [ ] No UDP parsing misalignment after long playback.
