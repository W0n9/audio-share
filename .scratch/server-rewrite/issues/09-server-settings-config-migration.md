# 09: Server settings + config migration

**What to build:** Server GUI 提供 Host、Port、Endpoint、Encoding 与 Reset；读取并迁移现有 `config.ini`；设置持久化并驱动 core。改设置后用 v0.3.4 Android Client 验证生效。

**Blocked by:** 08 (Tauri tray-first shell + Start/Stop)

**Status:** resolved

- [x] Host、Port、Endpoint、Encoding 可编辑并持久化。
- [x] 现有 `config.ini` 的键被读取并迁移，不丢设置。
- [x] configVersion 不匹配时给出明确迁移结果，不静默丢弃用户配置。
- [x] Reset 恢复默认并写回。
- [ ] 修改设置后启动 Server，v0.3.4 Android Client 按新设置连接。
- [x] 配置写入原子化。

## Comments

2026-11-09 — branch `ticket/09-settings-config`, implementation commit `fb61a1f`.
Base/integration tip: `9f712de`; final `git merge integration/server-rewrite`
was already up to date (no merge commit required).

Implementation:
- `audio-share-core::config`: Win32 INI semantics (case-insensitive
  sections/keys, trimming, quote stripping); unknown sections/keys/comments and
  key spelling survive rewrites; UTF-16LE BOM and UTF-8/BOM reads; stable UTF-8
  writes without BOM. Missing/1 -> migrate all recognized keys, write v2, report
  `Migrated{from}`; 2 -> load; >2 -> read recognized keys, keep `config.ini.bak`,
  do not rewrite, report `FutureVersion`; corrupt -> back up to `config.ini.bak`,
  defaults, report `Recovered`; invalid port/encoding/host -> defaults + report;
  unknown endpoint -> `default` + report. `[App] Running` is written 1/0 on
  successful start/stop. Writes are atomic: same-directory temp + fsync + rename
  (Windows `MoveFileExW(MOVEFILE_REPLACE_EXISTING|MOVEFILE_WRITE_THROUGH)`).
- `paths`: reports an existing exe-dir `config.ini` when the resolved target is
  XDG; the loader migrates it into the writable location and reports it.
- GUI: startup config load and real endpoint enumeration run on a dedicated
  thread before Tauri init; commands `get_settings`, `save_settings` (validate +
  endpoint check + atomic write; refused unless stopped), `reset_settings`, and
  `list_endpoints` (`spawn_blocking` on the real backend). Main/Settings views
  enable Host/Port/Endpoint/Encoding with immediate persist on change/blur,
  disable them while Starting/Running/Stopping, show readable save errors, and
  display a dismissible migration/recovery banner. `WhenClose` is applied to the
  close handler (0 = stop core and exit, 1 = hide); `WhenAppStart`/language are
  loaded and displayed for ticket 10.

Verification (Ubuntu 24.04, Xvfb 1280x800, private PipeWire 1.0.5 + null
`Audio/Sink`):
- `cargo fmt --all -- --check` pass.
- `cargo clippy --workspace --all-targets -- -D warnings` pass.
- `cargo test --workspace` 206 passed, 0 failed (20 config integration tests, 32
  GUI lib tests).
- `cd gui && npm run check` 0 errors / 0 warnings; `npm run build` pass.
- `cargo check --target x86_64-pc-windows-msvc -p audio-share-core
  -p audio-share-server-gui` pass (only the two known tauri-build GNU-compiler
  warnings).
- `npx tauri build --debug --no-bundle` produced `target/debug/AudioShareServer`.
- Headless GUI with `AUDIO_SHARE_CONFIG` pointing at a v1 file: migrated to v2,
  unknown `[Legacy] Keep=me` preserved, banner showed migration + the invalid
  `65611` port (greater than 65535) reset to 65530
  (`docs/server-rewrite/img/ticket09-migration-banner.png`).
- Main-view edit to `0.0.0.0`/`65512` persisted immediately; after quit/restart
  the UI and file still held the new values
  (`docs/server-rewrite/img/ticket09-persistence-running.png`,
  `ticket09-settings-edited.png`).
- Start with persisted `0.0.0.0:65512`/f32: listener on `0.0.0.0:65512`;
  `protocol-harness replay --fixture fixtures/protocol/v0.3.4/linux/f32-48k-stereo
  --connect 127.0.0.1:65512 --heartbeats 1 --datagrams 16` ->
  `replay: ok (live)`, `format_bytes_match=true`, 846 datagrams / 1,155,072
  bytes. Stop removed the listener; `Running=1`/`0` written on start/stop.
- Reset clicked in the Settings view with fake capture: host restored to the
  default address, port 65530, encoding 0, while application settings
  (AutoRun=1, WhenAppStart=1, language=zh-CN) were preserved
  (`docs/server-rewrite/img/ticket09-settings.png`).
- Real `WM_DELETE_WINDOW` client message: `WhenClose=1` hid the window while the
  process and server stayed alive; `WhenClose=0` stopped the core, released the
  listener, wrote `Running=0`, and exited 0.
- Config path in the manual runs was the `AUDIO_SHARE_CONFIG` override. The full
  resolution order (`AUDIO_SHARE_CONFIG` -> writable exe dir -> XDG) and the
  read-only exe-dir legacy migration are covered by tests.

Residual: the real v0.3.4 Android APK was not available, so that checkbox stays
open; scripted Wire Protocol coverage satisfies the automated seam. Real APK
acceptance remains ticket 15.
