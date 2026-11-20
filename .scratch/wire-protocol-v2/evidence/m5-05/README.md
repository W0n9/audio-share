# M5-05 implementer evidence (host only)

Ticket: `.scratch/wire-protocol-v2/issues/m5-05-upstream-pacing-tuning.md`
Branch: `ticket/m5-05-pacing` (worktree `.claude/worktrees/agent-a45d1092ef2dd24ee`)
Base: `integration/m5` @ `8c35411` (BASE_OK verified before work)

## What was implemented

1. **Timestamp re-anchor (spec §7.2).** `MediaPump::PendingChunk` now carries
   `start_frame`; `take_sendable` advances it as bytes drain and calls
   `packetizer.seek(next_sequence, chunk_start)` before each `packetize`. The
   header `timestamp` is therefore the capture-clock position of the first
   payload frame even after a pre-packetize drop; `sequence` stays +1 per
   datagram sent. Fix lives in the pump (not `media.rs`) because `start_frame`
   is lost before `MediaPacketizer::packetize` sees the bytes; `media.rs`
   callers in `tools/` / `tests/media_packetizer.rs` are not owned by this
   ticket and must keep their signature. `debug_assert!(chunk_start >=
   next_timestamp)` guards against backwards clocks. No protocol change.
2. **Send-ahead guard test (spec §7.4).** Characterization test verifies the
   pacer never releases more than one 4800-frame (100 ms @ 48 kHz) window up
   front plus 480 frames per 10 ms elapsed, and still drains a full 500 ms
   burst in 50 ticks. Thresholds are literals from the spec, not recomputed
   from pacer internals. This is a guard test that passed immediately — it is
   NOT claimed as a RED→GREEN cycle. `MAX_PENDING_MS` (500 ms) is unchanged and
   is backlog latency, not the §7.4 send-ahead window.
3. **Server-side drop instrumentation.** `DropReporter` (pure, injected
   `std::time::Instant`) emits a `DropReport` only when the 5 s diagnostics
   cadence elapsed AND a counter changed. `session_log::media_drops()` emits
   `tracing::info!(event = %"media.drops", session_id, instance,
   pump_dropped_chunks, queue_dropped_newest, pending_frames, ...)` — GUI
   tracing_bridge forwards it to `server.log`; no GUI or as-cmd logging change.
   Wired in `driver.rs` media loop after `queue.enqueue`, using the existing
   `now` passed to `take_sendable`. `MediaTask::spawn` gained `session_id` and
   `stream_instance_id` parameters (both available at the call site). Only
   public counters; no key material/audio (asserted by a log test).

## TDD loop (RED → GREEN, vertical slices)

- Slice 1 RED: `cargo test -p v2-transport a_dropped_chunk_jumps_the_wire_timestamp_not_the_sequence`
  failed with `left: 23552, right: 24576` — a 1024-frame offset exactly equal
  to the dropped chunk (first payload frame at 24576, stale accumulated clock
  at 23552). GREEN after pump re-anchor: `1 passed; 0 failed`.
- Slice 2 (guard): `send_ahead_never_exceeds_window_plus_rate_times_elapsed`
  passed after correcting the test's offered backlog to the pump's real
  500 ms bound (24000 frames); `1 passed; 0 failed`. Characterization, not RED.
- Slice 3 RED: `drop_reporter_fires_only_on_change_at_the_diagnostics_cadence`
  failed to compile (`cannot find DropReporter in super`). GREEN after adding
  the reporter: `1 passed; 0 failed`.

## Commands run and exact results (host)

- `cargo test -p v2-transport` — ALL PASS. Summary by binary:
  - lib: 79 passed, 0 failed
  - auth_primitives 9; auth_store 21; golden_vectors 3; media_packetizer 9;
    media_queue 9; pairing_live 8; pairing_primitives 21;
    pairing_state_machine 16; redaction 5; registry 2; session_driver 12;
    session_log 5; state_machine 36; state_machine_playing 14;
    state_machine_replay 19; store_admin 8; doc-tests 0 — all `0 failed`.
- `cargo test -p audio-share-core` — ALL PASS:
  - lib 38; capture_linux 0 (1 ignored); capture_media_tap 9;
    capture_synthetic 5; config 24; paths 8; protocol_wire 17;
    server_wire 17; doc-tests 0 — all `0 failed`.
- `cargo test -p as-cmd` (extra, advisor-suggested) — ALL PASS:
  - main 9; cli 20; cli_pipewire 0 (1 ignored); replay_fixture 1;
    server_process 1; v2_media 2; v2_pairing 5; v2_server 5;
    lib 0; doc-tests 0 — all `0 failed`.
- `cargo build -p v2-transport` — no warnings, no errors.
- `cargo fmt -p v2-transport -- --check` — initially FAILED (reflow in
  driver.rs/media_pump.rs/session_log.rs test); applied `cargo fmt -p
  v2-transport`, re-check clean. Committed as follow-up `e6f48e3`.
- `cargo clippy -p v2-transport --all-targets -- -D warnings` — PASS, no
  warnings. (CI runs `cargo clippy --workspace --all-targets -- -D warnings`;
  the focused `-p` form avoids the GUI/webkit deps unrelated to this change.)
- Post-fmt `cargo test -p v2-transport` re-run — counts identical to above.

All three required per-crate suites pass. Workspace-wide `--workspace` was not
run: codemap notes it pulls in `gui/src-tauri`, which needs webkit/GTK dev libs
unrelated to this server-side change.

## Files changed (all worktree, explicit owned paths)

- `crates/v2-transport/src/media_pump.rs` — PendingChunk.start_frame, re-anchor
  in take_sendable, DropReporter/DropReport + unit tests.
- `crates/v2-transport/src/session_log.rs` — `media_drops` event function.
- `crates/v2-transport/src/driver.rs` — spawn signature, reporter wiring.
- `crates/v2-transport/tests/session_log.rs` — `media.drops` log event test.

NOT touched: `media.rs` packetize signature (tools/ callers), GUI, protos, docs,
`backend.rs`, capture thread, sample rate/channels/format defaults, protocol.

## NOT RUN / unavailable on integration/m5

- **WASAPI per-callback timing breakdown** (`AUDIO_SHARE_CAPTURE_TIMING`):
  NOT RUN — L1 timing hooks live only on experiment branch
  `experiment/m5-audiotrack-buffer` @ `f073e07`; maintainer ruling: keep there,
  do not promote. No env-gated timing added to backend.rs/driver.rs.
- **Pacer backlog / network latency split** (`AUDIO_SHARE_L1_TIMING`):
  NOT RUN — same experiment-branch instrumentation, not merged. In-memory drop
  counters now log, but no per-stage latency numbers are available.
- **Real-device latency retest (M5-01 baseline A/B)**: NOT RUN — no device;
  host-only implementation. No real-device claim made.
- **MediaTap.dropped_chunks logging**: NOT COVERED — the counter lives in
  `audio-share-core::capture::media` and is not reachable from v2-transport; only
  the v2-transport `MediaPump`/`MediaSendQueue` counters are logged.
- **Experiment-branch instrumentation promotion**: NOT RUN by design (ruling).

## Known gaps (not fixed in this ticket)

- Fix location differs from the ticket text: the re-anchor is in
  `MediaPump::take_sendable`, not `MediaPacketizer::packetize()`. Deliberate —
  `start_frame` is not visible to the packetizer and it has `tools/` callers
  this ticket does not own. Merger should know this was intentional.
- The last drop count can go unlogged: `media.drops` fires only when the 5 s
  cadence is due AND a counter changed; drops in the final sub-5 s window
  before the media task ends are never emitted (no exit flush). Accepted gap;
  no extra code added.
- The L1 `capture_frame − submitted_position` metric will NOT flatten after
  this fix: the client's `submitted_position` = `initial_timestamp` + submitted
  bytes and ignores wire timestamps, so it keeps growing under server-side
  drops. This fix makes timestamps honest; it does not change the L1 metric.
  M5-03/M5-04 need a timestamp-gap check (next ts > last ts + frames) to detect
  server-side loss, because sequence stays contiguous across those drops.
- Slice 2 send-ahead test is a characterization guard that never went RED; do
  not describe it as a RED→GREEN cycle.

## Handoff note for M5-03 (jitter buffer)

Wire `timestamp` may now jump forward on server-side pre-packetize drops
(`MediaPump.dropped_chunks`) and on upstream `MediaTap` full-subscriber drops:
`MediaTap::publish` advances `stream.next_frame` to `chunk.end_frame` BEFORE the
subscriber `try_send`, so a dropped-for-subscriber chunk still consumes capture
frame positions. The next delivered chunk therefore starts at a higher
`start_frame`, which the re-anchor now reflects as an honest timestamp gap.
Sequence stays contiguous (no wire gap on server-side drops), so the client's
`cumulative_lost` will NOT see these — M5-03/M5-04 must use timestamp gaps, not
sequence gaps, to detect such losses. Network loss still shows as sequence gaps.

## Process notes

- Ticket status claim via Edit on the main-checkout absolute path was REFUSED by
  the worktree isolation guard; status left as `ready-for-agent` for the merger.
- No `git add -A`; only explicit owned paths staged. No Co-Authored-By trailer.
