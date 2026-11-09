# 08: Tauri tray-first shell + Start/Stop

**What to build:** Tauri v2 Server GUI 采用原型 A：托盘优先单窗口，显示 Server 状态与监听地址；Start/Stop 驱动同一个 core；托盘菜单 Show、Start、Stop、Exit；关闭窗口最小化到托盘。Windows 与 Linux 都能运行。

**Blocked by:** 05 (Windows real capture path), 06 (Linux real capture path)

**Status:** resolved

- [x] 应用启动后可从托盘显示或隐藏窗口。
- [x] Start/Stop 真实启动和停止 Server，状态与地址实时更新。
- [x] 托盘菜单 Show、Start、Stop、Exit 可用；Linux 右键菜单可用。
- [x] 关闭窗口默认最小化到托盘。
- [x] Start/Stop 不阻塞 UI；错误以可读消息呈现。
- [x] 视觉与信息架构遵循原型 A：状态和主操作优先。

## Comments

2026-11-09 — branch `ticket/08-tauri-shell`. Implementation commit `5b49bfd`;
merged `integration/server-rewrite` (ticket 07 tip) in `9d9dc5d`, then re-ran
fmt/clippy/tests, the Windows cross-check, the Xvfb smoke test, the GUI
Start/Stop click path and the live protocol-harness replay (all pass).

Implemented `gui/` (Svelte 5.57.3 + Vite 8.3.6 + TypeScript 6.0.3, pinned) and
`gui/src-tauri` (tauri 2.12.5, tauri-build 2.7.4, single-instance 2.5.5 as the
first plugin, autostart 2.7.0, opener 2.7.2, notification 2.8.0; capabilities
`core:default`, `autostart:default`, `opener:default`, `notification:default`).
Bundle `targets: []` for now (ticket 14 adds Linux bundles). `gui/src-tauri` is a
workspace member. Icons were produced from
`server-mfc/audio-share-server/res/AudioShareServer.ico` with ImageMagick and
`npx tauri icon`. A placeholder `gui/dist/index.html` is committed so cargo builds
work before npm; `npm run build` overwrites it. `gui/README.md` documents that a
bare `cargo build` produces a dev-URL binary while `tauri build` embeds assets.

Controller: `Option<ServerHandle>`, states Stopped/Starting/Running/Stopping,
actual listen address, last readable error, in-memory defaults (selected default
host, port 65530, endpoint `default`, encoding `default`). Commands
`get_bootstrap`, `get_status`, `start_server`, `stop_server`, `hide_window`,
`exit_app`; async start/stop with no lock held across an await, status emitted as
`server:state-changed`. `--fake-capture` selects synthetic capture; normal runs
use `platform_backend()` and `CaptureConfig { endpoint_id, encoding, channels: 0,
sample_rate: 0 }`. Tray Show/Start/Stop/separator/Exit with dynamic enable/disable;
Windows left-click toggles, Linux relies on the right-click menu. Close request
hides to tray (ticket 09 adds the configurable policy). `--hide` starts hidden.
Window 420x640, title "Audio Share Server", prototype A layout and tokens,
English strings in `gui/src/locales/en-US.json` shared with Rust tray labels.

Verified headlessly on Ubuntu 24.04 under Xvfb 1280x800:

- `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets -- -D warnings`;
  `cargo test --workspace` — all pass, including 22 GUI lib tests for controller
  transitions, failure/recovery, settings/encoding mapping, options and strings.
- `cargo check -p audio-share-server-gui`; `cargo check --target x86_64-pc-windows-msvc
  -p audio-share-server-gui` (only the two known tauri-build "GNU compiler is not
  supported for this target" warnings from tauri.md).
- `npm ci`; `npm run check` (0 errors, 0 warnings); `npm run build`;
  `npx tauri build --debug --no-bundle` -> `target/debug/AudioShareServer`.
- `--smoke-test` under `xvfb-run -a -s "-screen 0 1280x800x24" dbus-run-session`:
  `SMOKE visible=Ok(true) size=Ok(PhysicalSize { width: 420, height: 640 })
  title=Ok("Audio Share Server")`, exit 0.
- Real webview click path with xdotool (not just controller tests): with a
  headless PipeWire session and a null `Audio/Sink`, clicking Start changed the
  status to running and bound TCP/UDP on 192.168.123.24:65530;
  `protocol-harness replay --fixture fixtures/protocol/v0.3.4/linux/f32-48k-stereo
  --connect 192.168.123.24:65530 --heartbeats 1 --datagrams 16` passed live
  (`format_bytes_match=true`, START_PLAY id 1, 846 UDP datagrams / 1,155,072 bytes
  aligned), and clicking Stop removed the listener. Fake-capture Start/Stop was
  also click-tested.
- Close-to-tray via a real WM_DELETE_WINDOW client message: window hidden, process
  still alive; a second launch exited 0 and re-showed the window (single-instance).
  `--hide` started with no visible window; a normal second launch showed it.
  Footer Quit called `exit_app` and exited 0.
- Screenshots: `docs/server-rewrite/img/ticket08-main.png`,
  `ticket08-pipewire-running.png`, `ticket08-settings.png`.

Residual manual checks (referenced from ticket 15's checklist): actual tray icon
menu interaction on a real desktop (Show/Start/Stop/Exit; Linux right-click only),
Windows tray left-click toggle and WASAPI runtime, and visual review of the shell
on Windows/Linux desktops. Config persistence, lifecycle/i18n/About/update,
firewall/sound, and packaging (tickets 09/10/11/13/14) are intentionally not
implemented; extension points are documented in `gui/README.md`.
