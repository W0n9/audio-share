# 10: Lifecycle + i18n + About + update check

**What to build:** 开机启动（Windows 用户启动项 / XDG autostart）、启动行为（不操作 / 启动服务器 / 保持上次状态）、关闭行为（退出 / 最小化到托盘）、单实例、Hide 与 Exit；en-US 与 zh-CN 持久化并在启动时应用；About 显示版本、主页与许可证；每 3 小时查询 GitHub releases/latest，提示并打开 release 页面。

**Blocked by:** 09 (Server settings + config migration)

**Status:** resolved

- [ ] 开机启动在 Windows 与 Linux 都生效。（Linux XDG 已运行验证；Windows 仅 `cargo check/clippy --target x86_64-pc-windows-msvc`，运行时待 ticket 15）
- [x] 启动行为与关闭行为按设置执行。
- [x] 单实例：重复启动只显示已有窗口。
- [x] en-US 与 zh-CN 切换持久化，启动时应用。
- [x] About 显示版本、主页与许可证。
- [x] 更新检查每 3 小时一次且可关闭；有新版本时提示并打开 release 页面。

## Comments

2026-11-09 — branch `ticket/10-lifecycle-i18n`, implementation commit `f79d5c9`.
Base/integration tip at implementation: `1785d70`.

Implemented:
- `gui/src-tauri/src/autostart.rs`: direct entries (no `auto-launch`), always
  quoted. Windows HKCU Run value `AudioShareServer` = `"<exe>" /hide`; Linux
  `$XDG_CONFIG_HOME/autostart/AudioShareServer.desktop` (default
  `~/.config/autostart/`) with `Exec="<exe>" --hide`, `TryExec=<exe>`,
  `Type=Application`, `Terminal=false`, `X-GNOME-Autostart-enabled=true`, using
  `$APPIMAGE` when set. Startup reconciles the entry to `AutoRun`; failures
  surface as a localized notice. Removed the unused autostart plugin.
- `WhenAppStart` (0/1/2 incl. persisted `Running`), `WhenClose` (0 exit with
  graceful core stop, 1 tray), tray Exit/Quit always stopping the core, and
  `--hide`/`-hide`/`/hide` hidden launches that do not steal focus.
- `gui/src-tauri/src/i18n.rs`, `strings.rs`, `gui/src/locales/zh-CN.json`:
  key-complete en-US/zh-CN resources embedded for tray/notifications, system
  locale resolution (`zh*` -> zh-CN else en-US), language applied before mount,
  restart-to-apply hint. Config migration/recovery notices are structured
  (`{code, params}`) and rendered through `notice.<code>` keys.
- About modal: product name, 0.4.0, homepage via opener, Apache License 2.0,
  copyright, localized.
- `gui/src-tauri/src/update.rs`: GitHub `releases/latest` GET with
  User-Agent `AudioShareServer/0.4.0` and 10 s timeout; v0.3.4 tag comparison
  (leading `v`, exactly 3 components, `stoi`-style numeric prefixes,
  remote > local). Newer releases store state, emit `update:available`, show
  an in-app banner plus native notification with the raw tag; banner opens
  `html_url`. Manual checks report Up To Date or a readable error; automatic
  checks run at startup and every 3 h while `AutoUpdate` and only log failures.
  `AUDIO_SHARE_UPDATE_URL` is a replay-test seam; no Tauri updater/download.

Commands/results (Ubuntu 24.04, Xvfb 1280x800, private PipeWire 1.0.5):
- `cargo fmt --all -- --check` pass; `cargo clippy --workspace --all-targets --
  -D warnings` pass; `cargo test --workspace` pass (63 GUI lib tests incl.
  update compare/parse, autostart quoting/writes, language resolution, launch
  mapping, plus 21 core config tests).
- `cargo check`/`clippy --target x86_64-pc-windows-msvc -p
  audio-share-server-gui -p audio-share-core` pass (only the known tauri-build
  GNU-compiler warnings; Windows TLS uses schannel because rustls/ring needs
  an MSVC C toolchain for the cross-check).
- `npm run check` 0 errors/0 warnings; `npm run build`;
  `npx tauri build --debug --no-bundle` -> `target/debug/AudioShareServer`.
- Language: `zh-CN` config -> Chinese UI; UI change to `en-US` persisted and a
  restart showed English with the restart hint; `default` + `LANG=zh_CN.UTF-8`
  -> Chinese, `LANG=en_US.UTF-8` -> English. A v1 Chinese config showed the
  localized migration notice (version 0 -> 2, unknown `[Legacy] Keep` kept).
- Autostart: UI toggle on wrote the exact quoted XDG entry above and
  `AutoRun=1`; off removed it and wrote `AutoRun=0`; startup reconciliation
  removed a seeded stale entry with `AutoRun=0` and created it with `AutoRun=1`.
- Launch behavior (fake capture, checked with `ss`): 0 + `Running=1` -> no
  listener; 1 -> listener; 2 + `Running=1` -> listener; 2 + `Running=0` -> no
  listener.
- Close behavior: `WhenClose=1` hid the window, kept the process alive and the
  listener open, `Running=1`; `WhenClose=0` stopped the core, released the
  listener, wrote `Running=0`, exited. Settings Hide hid the window while the
  process stayed alive; a second normal launch exited 0 and re-showed it.
- Single instance on a fresh `dbus-run-session` bus: hidden first instance has
  no visible window; a second `--hide` launch exited 0 in 0 s without showing a
  window; a normal second launch exited 0 in 0 s and showed the existing window.
  (The ambient session bus lacks `org.a11y.Bus`, which made every launch take
  ~30 s; the recommended `dbus-run-session` path is fast.)
- Update checks against a local mock: newer `v9.9.9` -> banner + settings
  message + native notification call; banner link invoked the opener with
  `http://127.0.0.1:8765/release/v9.9.9`; same `v0.4.0` -> "You are running the
  latest version."; HTTP 500 -> readable error; malformed JSON -> readable
  error; request log showed `User-Agent: AudioShareServer/0.4.0`. With
  `AutoUpdate=1`, the startup check issued the request and produced the banner
  without user action. The real GitHub API returned HTTP 403 (shared-IP rate
  limit) and the UI/log showed the readable message; TLS and the request path
  worked.
- Headless PipeWire with a null `Audio/Sink` and `WhenAppStart=1`: server
  reached Running and listened on 127.0.0.1:65050.

Screenshots (Xvfb, committed): `docs/server-rewrite/img/ticket10-language-zh.png`,
`ticket10-language-en.png`, `ticket10-language-restart-hint.png`,
`ticket10-about-zh.png`, `ticket10-config-notice-zh.png`,
`ticket10-update-banner.png`, `ticket10-pipewire-running.png`. The update
banner screenshot uses the replay mock's `v9.9.9`.

Residual / not claimed:
- Windows runtime login autostart, tray appearance/interaction, and notification
  appearance are not executable here; compile/clippy only. Referred to ticket 15.
- Linux tray menu interaction needs a StatusNotifier panel; Xvfb has none, so
  Show/Start/Stop/Exit were validated through the shared handlers and the
  Settings Hide/Quit paths, not by clicking a tray menu.
- The Tauri notification plugin exposes no notification-click callback; the
  in-app banner click opens `html_url`, while the native notification carries
  the raw tag as its body. Ticket 15 should check the Windows toast click UX.
- The 3-hour interval was not observed in real time; startup checks and the
  interval loop were verified, and toggling `AutoUpdate` is re-read by the loop.
