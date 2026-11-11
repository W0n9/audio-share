# Server GUI modernization acceptance (Variant C)

Status for the Server GUI modernization (tickets 01-04): **partial — all
automated checks plus headless Linux Xvfb evidence recorded on 2026-11-11;
real-desktop, Windows, and phone items remain release gates and are marked
`[ ]`**. This file extends, and must be read together with, the v0.4.0
baseline in [`gui-acceptance-checklist.md`](gui-acceptance-checklist.md): the
baseline remains authoritative for every native behavior the modernization
did not touch.

Design source: the local `prototype/server-gui-modernization` branch, commit
`604f0d5` ("prototype(gui): compare modern Server GUI variants"). The user
selected **Variant C ("focused minimal")** with the restrained Stop danger
style. The production rewrite does not lift prototype code; it re-implements
the selected direction on `integration/server-gui-modernization`
(tickets 01-03; `git log --oneline main..integration/server-gui-modernization`).

Markers, matching the baseline checklist: `[x]` executed with evidence,
`[~]` partially observed, `[ ]` not executed — release gate. Items are tagged
(agent) when they were executed by the ticket agent and (human) when a real
desktop pass is still required.

Hosts/evidence for this file:

- Ubuntu 24.04.5 (ticket agent's Linux machine), Xvfb 1280x800 (no window
  manager, no settings daemon), a private `dbus-run-session`, and a private
  PipeWire/WirePlumber null sink via `scripts/pw-test-session.sh`.
  WebKitGTK 2.52.6, GTK 3.24.41, libayatana-appindicator 0.5.93, Rust 1.98.1,
  Node 22.18.0. Build: `npm run build` + `cargo build -p
  audio-share-server-gui --features audio-share-server-gui/custom-protocol`
  (debug, embedded dist), driven with `xdotool` clicks/keys and ImageMagick
  `import` screenshots, per the headless recipe in [`linux.md`](linux.md).
- Windows: **not executed** — no Windows host was available to this ticket.
  Every Windows row below is `[ ]` and stays a release gate.

Automated results (this ticket branch, all green):

- `npm run check`: svelte-check 4.7.8, 99 files, 0 errors, 0 warnings.
- `npm run build`: vite 8.3.6, target es2021, 138 modules, single bundle
  (`dist/assets/index-*.js` + one CSS file), no prototype/variant chunks.
- `cargo test --workspace`: all suites pass — `audio_share_core` unit 35,
  `config` 24, `paths` 8, `capture_synthetic` 5, `protocol_wire` 17,
  `server_wire` 17; `audio_share_server_gui` 93 (includes the en-US/zh-CN
  key-parity and controller behavior tests); `as-cmd` 5+12, `protocol_harness`
  11+2+1+3+5 (harness smoke, CLI, probes), `replay_fixture` 1,
  `server_process` 1, spikes 17+26.
- Scope evidence (`git diff --stat main..integration/server-gui-modernization`):
  only `crates/audio-share-core/src/config.rs` (+ its tests, for `[App] theme`
  persistence), `gui/src-tauri/src/{controller,model}.rs` (application
  settings plumbing), and the `gui/` frontend. No Wire Protocol, capture, or
  lifecycle files changed; `gui/src-tauri/tauri.conf.json` (window
  `420×640`, min `380×560`) is unchanged.

## Main window (Variant C)

- [x] Header carries only the brand and the Settings entry; the server state
  is not repeated in the header. (agent, headless:
  `img/gui-mod-04-main-light.png`, `img/gui-mod-04-main-light-zh.png`.)
- [x] Exactly one status display: static status dot + status title +
  listening address chip. Stopped shows "Server stopped" and host:port.
  (agent, headless, same screenshots; live region scoped to the status
  content, `ServerView.svelte`.)
- [x] Address chip shows the configured host:port while stopped and the real
  bound address after Start. (agent, headless: the running chip
  `192.168.123.24:65530` in `img/gui-mod-04-main-running.png` matches the
  "server running on 192.168.123.24:65530" line in the app log.)
- [x] Start is the primary circular action in Stopped; the action label sits
  under the circle. (agent, headless: `img/gui-mod-04-main-light.png`.)
- [x] Stop keeps the restrained danger style while Running (soft danger
  background, danger icon/text — not a solid saturated red block).
  (agent, headless: `img/gui-mod-04-main-running.png`.)
- [x] Endpoint and Encoding are quick settings, visible without expansion;
  "Default" is marked. (agent, headless: main screenshots; with the null
  sink the Endpoint list resolves the real enumeration path — a real desktop
  sink list is still covered by the baseline `[~]` row.)
- [x] Server fields are locked while Running and editable while Stopped.
  (agent, headless: `img/gui-mod-04-main-running.png` shows disabled quick
  settings; behavioral lock is additionally covered by the `server_gui` Rust
  tests and the Windows 11 baseline rows.)
- [x] Network configuration is a native disclosure, collapsed by default, and
  expands to Host and Port only. (agent, headless:
  `img/gui-mod-04-main-light.png` collapsed,
  `img/gui-mod-04-main-network-expanded.png` expanded.)
- [x] Reset lives in the main view under the Server configuration and is
  worded as resetting all Server fields (Host, Port, Endpoint, Encoding)
  while application settings are kept. (agent, headless: main screenshots;
  the reset behavior itself is covered by the Windows 11 baseline row and
  the `config`/`server_gui` tests.)
- [x] Footer keeps the state hint and Quit. (agent, headless: main
  screenshots.)

## Async states (Starting / Stopping)

- [ ] Starting transition observed (pending dot, disabled action) and settles
  into Running. The server binds in ~20 ms headlessly, faster than any
  screenshot round-trip; burst captures (6 frames at ~0.15 s spacing) caught
  only Stopped and Running frames. The transition is also a new design with
  no visual reference (ticket 02 leftover). (human, real desktop.)
- [ ] Stopping transition observed on a real desktop. (human; same reason.)
- [x] Repeated activation is blocked: the action button is disabled while a
  command is in flight (`disabled={server.busy || server.saving ||
  transitioning}` in `ServerView.svelte`; controller state machine tested in
  the `server_gui` Rust tests). (agent, code + tests.)

## Errors and banners

- [x] Invalid input produces a readable error alert and the layout does not
  collapse. (agent, headless: config with `host = no-such-host.invalid`,
  Start clicked → `img/gui-mod-04-main-error.png` shows the danger-styled
  alert "cannot resolve address … Name or service not known", the window
  back in Stopped; errors render as `role="alert"` paragraphs under the
  hero.)
- [x] Start failure returns the window to Stopped (controller test "a failed
  start publishes a readable error and recovers" in the `server_gui` suite).
  (agent, automated.)
- [x] Config migration notice and config-recovery warning banners render in
  the new layout. (agent, headless: a v0.3.4-style file without
  `configVersion` → `img/gui-mod-04-banner-migration.png` shows both the
  "Configuration migrated from version 0 to version 2" notice and a recovery
  warning for an invalid legacy encoding value, with the dismiss action —
  this also covers the user story that migration and recovery warnings stay
  visible.)
- [ ] Update banner and manual check-for-update inside the new layout on a
  real desktop (ticket 10's mock-server evidence covers the update logic;
  the banner moved into the rewritten shell — re-check visually). (human)

## Settings (grouped disclosures)

- [x] Settings no longer repeats Host/Port/Endpoint/Encoding; the main view
  owns Server configuration. (agent, headless:
  `img/gui-mod-04-settings-light.png`.)
- [x] Single scrolling page with grouped disclosure sections: Appearance
  (always expanded), Startup & window, Updates, System, About.
  (agent, headless: settings screenshots + expanded groups in
  `img/gui-mod-04-settings-system-updates.png` (Updates + System +
  About all expanded) and `img/gui-mod-04-settings-about.png`.)
- [x] Appearance offers Theme System / Light / Dark as a segmented control
  with `aria-pressed` state, and Language with the "applies on next launch"
  hint. (agent, headless: settings screenshots; language-restart hint
  behavior is unit-covered and its visual position shows in
  `img/gui-mod-04-settings-light-zh.png` — the hint text itself was not
  triggered headlessly. (human) click-through.)
- [x] Application settings stay editable while the Server is Running
  (`applicationLocked` only covers in-flight saves, `store.svelte.ts`; MFC
  parity preserved). (agent, code + `server_gui` tests.)
- [x] About group shows product name, version 0.4.0, homepage link, license
  and copyright. (agent, headless: `img/gui-mod-04-settings-about.png` —
  "Audio Share Server / Version 0.4.0 · Apache-2.0 / Home page / Copyright".)

## Themes (System / Light / Dark)

- [x] Light and Dark both render every surface, text, control, semantic
  color, and focus ring from the shared token set. (agent, headless:
  main and Settings in both themes — `img/gui-mod-04-main-light.png`,
  `img/gui-mod-04-main-dark.png`, `img/gui-mod-04-settings-light.png`,
  `img/gui-mod-04-settings-dark.png`.)
- [x] Theme applies before the shell mounts, so there is no wrong-theme
  flash on launch (`main.ts` applies the persisted theme before mounting,
  `theme.ts`; the CSS `prefers-color-scheme` fallback covers only the
  pre-script moment). (agent, code + restart screenshot
  `img/gui-mod-04-main-dark-restart.png`; the perceived "no flash" check is
  ultimately (human) on a real desktop.)
- [x] Manual Light/Dark applies immediately from Settings without a restart.
  (agent, headless: light → click Dark in Settings → dark Settings and main
  without relaunch.)
- [x] Theme choice persists across restart. (agent, headless: Dark chosen in
  the UI, app quit, relaunched with the same profile → dark without any
  click, `img/gui-mod-04-main-dark-restart.png`; config round-trip covered by
  the `config` tests `theme_values_round_trip_through_the_ini_key` and
  `unknown_theme_values_fall_back_to_system_and_are_reported`.)
- [x] Missing or unknown `[App] theme` values fall back to System without
  touching other fields; legacy configs without a theme key migrate
  losslessly. (agent, automated: `config` tests — default, case-insensitive
  and quoted parsing, unknown value fallback, v0.3.4 migration writes
  `theme=system`.)
- [~] System theme tracks the OS color scheme live. (agent, headless
  limitation: with System selected on a fresh profile the window renders
  light, matching the session's absent XSETTINGS provider, and
  `matchMedia("(prefers-color-scheme: dark)")` wiring is covered by
  `theme.ts`; but no settings daemon exists under Xvfb, so a live OS
  light→dark switch was **not** observed. (human) on GNOME/KDE via
  `gsettings set org.gnome.desktop.interface color-scheme prefer-dark`.)
- [ ] System theme switch also re-checks both themes' readability after the
  switch (no stale surfaces). (human; depends on the row above.)
- [x] The native tray menu, Server Core, and Wire Protocol do not observe the
  theme (by design, spec; tray code is untouched — see scope evidence).
  (agent, code scope.)

## Window sizes

- [x] Default window is `420×640`. (agent, headless: `--smoke-test` under
  Xvfb reported `SMOKE visible=Ok(true) size=Ok(PhysicalSize { width: 420,
  height: 640 }) title=Ok("Audio Share Server")`.)
- [x] Minimum size `380×560` is accepted and every control stays reachable
  and readable, in both themes. (agent, headless: `xdotool windowsize` → GTK
  geometry `380x560`, `img/gui-mod-04-main-min-380.png` +
  `img/gui-mod-04-settings-min-380.png` (light),
  `img/gui-mod-04-main-min-380-dark.png` +
  `img/gui-mod-04-settings-min-380-dark.png` (dark); Reset text wraps but
  stays visible.)
- [x] Settings is scrollable at `380×560`. (agent, headless: mouse-wheel
  scroll over the settings page at the minimum size —
  `img/gui-mod-04-settings-min-380-scrolled.png` shows the page scrolled
  with Appearance moved off the top and every group reachable; the
  scrollbar is visible in `img/gui-mod-04-settings-min-380.png`.)
- [x] The window remains resizable between the minimum and larger sizes.
  (agent, headless: 420→380 resize applied via the toolkit geometry.)

## Keyboard focus

- [~] Tab reaches the header Settings entry and main controls with a visible
  focus ring; screenshots capture focus stops. (agent, headless:
  `img/gui-mod-04-focus-tab1.png`, `img/gui-mod-04-focus-tab4.png`.) A full
  keyboard pass (quick settings, disclosure summaries, segmented control,
  About links, dialogs) on a real keyboard is (human).
- [x] Focus indicators are token-driven and visible in both themes; the
  status live region covers only the status content, so keyboard/assistive
  users are not re-notified by unrelated updates. (agent, code:
  `styles.css` focus ring tokens, `ServerView.svelte` aria-live scoping,
  svelte-check a11y pass with 0 warnings.)

## Reduced motion

- [ ] With the system set to reduce motion, non-essential transitions and
  animations are disabled while color/opacity feedback stays. The CSS block
  exists (`@media (prefers-reduced-motion: reduce)` in `styles.css`, which
  sets `transition: none !important; animation: none !important`), but
  observing it requires a desktop session setting — not available under
  Xvfb. (human, real desktop.)

## Tray (real desktop)

- [ ] Tray icon, Show/Start/Stop/Exit menu, left-click toggle (Windows), and
  close-to-tray with the modernized build. The tray implementation was not
  touched by tickets 01-03 (frontend-only change; see scope evidence), so
  the baseline evidence (Xvfb SNI export in `linux.md`; Windows 11 panel
  interaction in the baseline checklist) still describes the same native
  code — but the modernized build itself has not been tray-clicked on a real
  panel. (human, Windows + Linux desktop.)

## Platform maintenance entries

- [x] Linux: the System group explains the Windows-only actions and offers
  the desktop-sound-settings and PipeWire documentation links (fixed URLs,
  opened via the `open_help` allow-list). (agent, headless:
  `img/gui-mod-04-settings-system-updates.png` shows the "Linux audio"
  section with the "Windows-only features" explanation and both "Open"
  rows; URL mapping is unit-tested in `help.rs`.)
- [ ] Windows: Repair Firewall and Sound Panel rows render only on Windows
  and still work in the new System group. (human, Windows desktop — the
  platform gating code path (`server.platform === "windows"`) is shared with
  the Linux branch verified above.)

## i18n

- [x] en-US and zh-CN resource files have identical key sets with no missing
  translations (119 keys each) and missing keys fall back to the key name.
  (agent, automated: `strings.rs` locale tests in the 93-test `server_gui`
  suite.)
- [x] en-US renders all labels in English in the new layout. (agent,
  headless: `img/gui-mod-04-main-light.png`, settings screenshots.)
- [x] zh-CN renders all labels in Simplified Chinese in the new layout.
  (agent, headless: `img/gui-mod-04-main-light-zh.png`,
  `img/gui-mod-04-settings-light-zh.png`.)
- [~] zh-CN legibility of the small muted text (the 10 px address chip and
  quick-setting labels are the tightest cases; ticket 02 leftover). The
  screenshots allow inspection and read acceptably at 100% zoom, but the
  final call belongs to a (human) reviewer on a real display.
- [x] Language=default follows the system locale. (agent, headless: the same
  build rendered zh-CN under `LANG=zh_CN.utf8` and en-US under
  `LANG=en_US.UTF-8`.)
- [x] Language change applies on the next launch with a restart hint
  (behavior preserved by `setLanguage` + restart hint; baseline ticket 10
  evidence covers the flow, the hint position is visible in the Settings
  screenshots). (agent, code + headless partial.)

## About and updates

- [x] About content (product name, version, homepage, copyright) in the new
  About disclosure. (agent, headless: `img/gui-mod-04-settings-about.png`.)
- [x] Homepage link opens through the opener allow-list. (agent, automated:
  `help.rs`/opener mapping unchanged and unit-tested; click-through covered
  by the baseline fake-`xdg-open` evidence.)
- [ ] Manual/auto update check inside the new Settings layout on a real
  desktop (logic unchanged and unit-tested; `update.rs` untouched — see
  scope evidence). (human)

## Lifecycle regression

- [x] Launch, single instance, `--hide`, WhenAppStart matrix, close-to-tray /
  close-to-exit, autostart, and config persistence behaviors are unchanged.
  (agent, automated: the `server_gui` Rust suite covers controller and
  options behavior (93 tests green); `git diff` shows no lifecycle files
  changed; the baseline checklist's executed rows remain the desktop
  evidence for the same native code.)
- [x] The modernized build passes the headless smoke path end to end
  (window exists, correct title/size, exits cleanly). (agent, headless:
  `--smoke-test` output above.)
- [ ] Single-instance/tray interaction of the modernized build on a real
  desktop session. (human; combines the two rows above.)

## Windows-only

- [ ] Every Windows row of the baseline checklist (launch/tray/left-click,
  close behaviors, WhenAppStart, autostart registry, WASAPI Start/Stop,
  firewall repair, Sound Panel, WebView2 check) re-run on Windows 11 for the
  modernized build, plus Windows 10. Not executed — no Windows host.
  (human, release gate.)

## Client-facing regression (v0.3.4 APK)

- [ ] Real-phone regression re-run against the modernized build. Not
  executed here. The protocol surface is unchanged: Wire Protocol files have
  no diff (`git diff --stat main..integration/server-gui-modernization`
  touches `config.rs` only in `audio-share-core`), and `protocol_wire` (17),
  `server_wire` (17), `replay_fixture` (1), harness smoke/CLI/probe suites,
  and the golden fixture replay are all green. The 2026-11-10 phone matrix in
  [`android-v034-acceptance.md`](android-v034-acceptance.md) therefore still
  describes the protocol; a final phone pass against the shipped build
  remains a release gate. (human, Android phone.)

## Prototype cleanup

- [x] The design source is preserved: branch
  `prototype/server-gui-modernization` exists and its tip is `604f0d5`
  ("prototype(gui): compare modern Server GUI variants"); the branch was not
  modified by tickets 01-04. (agent, `git cat-file -t 604f0d5` → commit,
  `git branch --contains 604f0d5`.)
- [x] The formal tree contains no prototype files, variant switcher, or
  `?variant=` entry: `git ls-files | grep -iE "prototype|variant"` returns
  nothing on the integration tip; `git grep -iE "variant=|variantSwitcher"`
  over `gui/` returns nothing; the only "Variant" hits are comments citing
  the design source (`Icon.svelte:4`, `styles.css:4`, `styles.css:224`);
  `git grep "import.meta.env.DEV"` over `gui/` finds no dev-gated prototype
  entry. (agent.)
- [x] The production build contains no prototype code: `npm run build`
  emits a single JS/CSS pair with no variant chunks. (agent, headless.)

## Known risks / leftovers

- Start glow and Stop hover use `color-mix()` (`styles.css:301`, `styles.css:321`).
  Very old WebKitGTK builds that predate `color-mix()` support silently drop
  those two declarations (ticket 02 leftover; no layout breakage — the base
  circle background still renders). The acceptance host's WebKitGTK 2.52.6
  renders them fine.
- zh-CN 10 px muted text legibility — see the i18n section. (human)
- Starting/Stopping transitions are a new design without a pre-existing
  visual reference (ticket 02 leftover) — confirm they feel right on a real
  desktop. (human)
- System-theme live switching, reduced motion, real-panel tray interaction,
  Windows, and the phone regression remain release gates as marked above.
