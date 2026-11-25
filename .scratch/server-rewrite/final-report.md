# Server rewrite — final report outline (update as tickets land)

## Delivered
- Integration branch `integration/server-rewrite` (not pushed) based on `refactor` c7b31cd.
- Rust workspace: `crates/audio-share-core`, `crates/as-cmd`, `tools/protocol-harness`, `tools/wasapi-spike`, `tools/pipewire-spike`, `gui/`.
- Frozen v0.3.4 Wire Protocol, real WASAPI + PipeWire capture, Tauri v2 GUI, config migration, lifecycle/i18n/update, packaging/CI.

## Tickets
- Resolved: 01, 03, 04, 06, 07, 08, 09, 10, 12, 14.
- ready-for-human (code/docs complete; hardware/manual gate open): 02, 05, 11, 13, 15, 16.
- Final integration: `integration/server-rewrite` @ 3e8074e (+ ticket 15 follow-up notes 280d0da to merge), 271 workspace tests passed, 2 ignored headless tests.

## Code review
- Two-axis review since `refactor`.
- Standards: tracker status vocabulary, glossary drift, duplicated encoding/status mappings, server data clump, dead exports, shell duplication.
- Spec: GUI server.log missing (fixed), UDP oversized when block_align>1464 (fixed), START_PLAY ids per-server vs process-wide (fixed), startup update check (changed to 3h-first), A/B metrics (extended), hotplug auto-recovery (documented known limitation; operator Stop/Start only), rust-ini unused (hand-rolled parser retained and documented).
- All code fixes re-run through fmt/clippy/test + GUI/Windows checks.

## Verified locally
- `cargo fmt --check`, clippy `-D warnings`, `cargo test --workspace` (final count TBD).
- Windows cross-target `cargo check/clippy`.
- Protocol harness offline + live replay against new `as-cmd`.
- Headless PipeWire real capture, tone verification, start/stop cycles, A/B vs old as-cmd.
- Acceptance matrix: 20/20 encoding/channel/rate combinations and 6/6 scripted scenarios (multi-client, reconnect, heartbeat timeout ~6s, stop closes) pass on release binaries.
- 12-minute soak with 2 clients/reconnects: no crash (final results in docs/server-rewrite/soak.md).
- Tauri GUI under Xvfb (`--smoke-test`), real webview clicks, close-to-tray, single-instance, `--hide`; deb install/run/uninstall; AppImage FUSE/extract; pristine Ubuntu 24.04 container dependency resolution.
- i18n en-US/zh-CN 114/114 key parity; autostart quoted entries; update-check mock/real behavior.
- Windows/WebView2/firewall code compile-checked; packaging CI defined and actionlint-clean.

## Not verified (human gates)
- Windows runtime: WASAPI measurements/A-B, UAC firewall repair, WebView2 missing dialog, Windows 10/11 GUI acceptance, Windows portable exe build (CI only).
- Real v0.3.4 Android APK acceptance (5 encodings, channels, rates, multi-client, reconnect).
- 8-hour soak, device hotplug/sleep/resume.
- Linux tray right-click under a real desktop environment (Xvfb has no SNI watcher).
- GitHub Actions run of the new workflow (validated locally only).

## Residual tickets
- 02 ready-for-human (Windows measurements).
- 05 ready-for-human (Windows runtime/A-B).
- 11 ready-for-human (UAC/firewall runtime).
- 13 ready-for-human (Windows exe/CI).
- 15 ready-for-human (real APK + 8h + desktop GUI).
- 16 status TBD (cutover code/docs vs release smoke gate).

## Commands
- Build/test: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- CLI: `cargo run -p as-cmd -- -b 127.0.0.1:65530`
- GUI dev/build: `cd gui && npm ci && npm run build && npx tauri build`
- Linux packages: `cd gui && npx tauri build --bundles deb,appimage`
- Headless PipeWire: `scripts/pw-test-session.sh --tone-wav tone.wav -- <cmd>`
- Harness: `cargo run -p protocol-harness -- replay --fixture fixtures/protocol/v0.3.4/linux/f32-48k-stereo`
