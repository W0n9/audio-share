# Code review fixes — integration/server-rewrite

Branch: create `fix/code-review` from `integration/server-rewrite`. One implementer, one worktree. Follow `tdd` for code changes. Run the full verification set at the end and report honestly.

## P0 — spec/behaviour fixes

1. **GUI `server.log` is never written.** Spec requires `server.log` in this round; MFC wrote it. The GUI currently only uses `eprintln!`. Add a small logging module used by the GUI:
   - Resolve the path with `audio_share_core::paths::paths().log`; create parent dir if needed; open with truncate at app startup.
   - Write timestamped `[info]`/`[error]` lines for: app start (version, config path), config migration/recovery notices, autostart reconciliation failures, server Starting/Running(address)/Stopping/Stopped, start/stop errors, update check outcomes (automatic failures at least).
   - Keep stderr output for developer visibility (write both).
   - Unit-test creation/truncation and line writing with a temp path. Add a GUI code path test if feasible without a display.
   - Update README if it claims GUI logging (it already does) and `build-linux.md` if it says the GUI does not create a log.

2. **Oversized UDP datagrams when `block_align > 1464`.** `server/mod.rs` broadcast_loop currently sends the whole frame; docs promise no oversized datagrams. Change it to send nothing for that format (log once/rate-limited) instead of oversized datagrams, matching `docs/protocol.md` and `segment_audio` semantics. Add an integration test with a SyntheticCapture whose block_align > 1464: client binds, asserts no datagrams and the server stays alive / stop works.

3. **START_PLAY ids must be process-wide.** v0.3.4 used a function-static counter; the rewrite allocates per `Server`. Move the counter to a process-wide `static AtomicI32` in `server/mod.rs` so GUI Stop/Start and multiple Server instances in one process continue monotonically (still >0). Update tests that assert a fresh server starts at 1 to assert >0/monotonic; keep fresh-process CLI/acceptance expectations working. Update `docs/protocol.md` to state process-wide ids and not reset per session/instance. Verify `protocol-harness`/acceptance probe don't assume 1 (fresh as-cmd processes still get 1).

4. **Automatic update check timing.** Spec says follow v0.3.4: first automatic check after 3 hours. The GUI currently checks at startup plus every 3h. Remove the startup automatic check; keep manual Check Now. Update ADR-0003/tests if they claim a startup check. Add a unit test for the scheduling decision if the logic is extracted.

5. **A/B missing callback/drop/drift/CPU evidence.** Extend `docs/server-rewrite/ab-linux.md` with measured old-vs-new numbers on the same null sink/tone: CPU (e.g. `/usr/bin/time -v` or sampled `ps` over a 30s run), PCM throughput/expected bytes (drift), datagram arrival gaps as the callback/glitch proxy (old has no callback counters), and any drops. Do not fabricate; state the proxy limitation. Reuse `scripts/acceptance-matrix.sh`/`protocol-harness probe` if useful.

6. **Hotplug/sleep automatic recovery.** Do NOT attempt a risky supervisor rewrite in this pass. The docs already state only operator Stop/Start is supported. Add an explicit **Known limitation** bullet to README and ADR-0003: capture does not auto-restart after target loss/sleep; the server stays Running with the old format but sends no PCM; use Stop/Start (or restart `as-cmd`). Keep ticket 15 ready-for-human. This is the honest resolution; do not claim auto-recovery.

## P1 — standards / vocabulary / dead code

7. **Tracker status vocabulary.** Add a "Closing work" note to `docs/agents/triage-labels.md`: `Status: resolved` is the local close state for work merged and verified; `Status: ready-for-human` means code complete with a human-only gate remaining. This makes the 10 resolved tickets conform to the repo's documented tracker vocabulary (the five triage labels are pre-work roles).

8. **GLOSSARY vocabulary drift.**
   - Replace "CLI" with "`as-cmd`" or "command-line" where it means the binary/archive: README, ADR-0003, `crates/audio-share-core/src/paths.rs` doc comment, `docs/server-rewrite/build-linux.md` (keep artifact names/quoted code unchanged).
   - In new docs, use "Android Client" instead of "client"/"phone app" and "Endpoint" instead of "device" where the glossary term applies (android-v034-acceptance.md, acceptance-matrix.md).
   - Replace "MFC GUI" with "legacy Windows GUI" in new code comments/docs (maintenance.rs, as-cmd main) except historical research/ADR-0001/0002.

9. **Duplicated encoding mapping.** Make `as-cmd` use `audio_share_core::config::EncodingSetting::from_name(...).preference()` instead of its own `encoding_preference` match (keep parse-time rejection of unknown values and the exact error text/tests). Extract the frontend encoding list to `gui/src/lib/encodings.ts` (or similar) and use it in ServerView and SettingsView. Leave the protocol-harness/fake-server mapping independent (it is a client-side test double) with a one-line comment if needed.

10. **Duplicated status label / unused exports.**
    - Extract the `ServerState`→i18n key helper shared by App.svelte/ServerView (if ServerView duplicates it) into `gui/src/lib/status.ts`.
    - Use the existing `locales` export in SettingsView instead of hardcoded language `<option>`s, or remove the unused export.
    - Remove `getLocale` if unused; remove `Encoding::ALL` if unused or use it in a test; remove the unused `api.getStatus` wrapper and registered `get_status` command if nothing calls it (check tests first).
    - Remove the dangling `architecture.md` reference in `paths.rs`/`tests/paths.rs` (point to `docs/server-rewrite/README.md` or drop it).

11. **Server data clumps.** Bundle the shared server connection context (heartbeat interval/timeout, sessions map, format, shutdown, tasks, writer) into a struct so `accept_loop`/`handle_connection`/`heartbeat_loop` no longer need `#[allow(clippy::too_many_arguments)]`. Keep behavior/tests identical.

12. **`exit_app` duplication.** Reuse `stop_core_if_running` inside `exit_app` instead of reimplementing the timeout/stop logic.

13. **Duplicated 65530 constant.** Define/use `audio_share_core::config::DEFAULT_PORT` (and server/config) in as-cmd; GUI model can use the config default. The TS pre-bootstrap fallback may keep 65530 with a comment, or take it from bootstrap.

14. **Obsolete workflow/trigger/docs hygiene.**
    - Delete `.github/workflows/test.yml` (manual upload/download demo, not a test suite).
    - Add `main` to the `server-rewrite.yml` push branches so CI runs after cutover.
    - Fix `docs/protocol.md`: TCP listener uses a 1024 backlog (code), not the default backlog.
    - Root `Cargo.toml` `rust-version` is 1.75 but the GUI requires 1.90 and wasapi 1.85; set workspace `rust-version = "1.90"` and keep member overrides if desired.

15. **Shell duplication (judgement, do what is safe).** Extract genuinely shared helpers from `scripts/package-{linux,windows}-artifacts.sh` (and their tests) into `scripts/lib.sh`, and/or the PipeWire session/port/tone helpers shared by `acceptance-matrix.sh`/`soak.sh`/`hotplug-recovery.sh`. Run `bash -n` on every script and both packaging self-tests after refactoring. If a refactor risks behaviour, leave the duplication and explain why in the final report rather than breaking a tested script.

## Verification

- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (0 failures).
- `cargo check/clippy --target x86_64-pc-windows-msvc -p audio-share-server-gui -p audio-share-core`.
- `cd gui && npm ci && npm run check && npm run build`; `npx tauri build --debug --no-bundle`; Xvfb smoke; confirm `gui/dist` ignored.
- `bash scripts/package-windows-artifacts.test.sh`, `bash scripts/package-linux-artifacts.test.sh`.
- Clean `git status`.
- Report per item: fixed/skipped + why, commands/results, residual risks.
