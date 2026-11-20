# M5 integration review — fix pass

Branch: `fix/m5-review` (based on `integration/m5` @ `733f02b`)
Worktree: `<repo>`

Every item below is either FIXED with a regression test or explicitly deferred.
Test evidence is host/JVM only; real-device/real-network items are `NOT RUN`.

## Findings

### 1. Drift estimator never produced an estimate — FIXED
- `android-client-v2/native/src/drift.rs` `observe()` now keeps ONE sample at or
  before the 10 s cutoff (`while samples[1].now_ms <= cutoff`) instead of
  evicting every sample `< cutoff`, which left the retained span strictly under
  the window under jitter.
- Test: `estimator_reports_within_a_bounded_number_of_jittered_updates` — a
  100 ms ± a few ms cadence produces `Some(~100 ppm)` within ≤112 updates.
  (RED before: `ppm()` stayed `None`.)

### 2. `late_dropped` is deadline-based — FIXED
- `android-client-v2/native/src/jitter.rs`: `JitterBuffer::submit` takes the
  session clock. A capture-clock→session-clock `playout_anchor` is established
  at the first datagram and refreshed on each accepted datagram; a datagram
  whose freshest frame (`timestamp + frames`) is more than `target_ms` behind
  the anchor is `late` even when the sequence frontier never advances. The
  frontier check is kept, so timely reorders still play.
- Held media replay uses `submit_replay` (original arrival instant, but does not
  move the live anchor).
- Tests: `an_arrival_after_a_sender_pause_is_late_by_deadline`,
  `a_resumed_sender_with_advanced_capture_clock_is_not_late`,
  `a_late_arrival_after_a_sender_pause_is_late_dropped` (session-level, counted
  and never written), `a_timely_reorder_still_plays_under_the_deadline_rule`.

### 3. A.8 jitter corrupted by held release / timestamp re-base — FIXED
- `media_session.rs` only calls `observe_arrival` for live datagrams with the
  real `now_ms`; released held media passes `observe_arrival = false`.
- `jitter.rs` `reset_arrival()` clears the A.8 window; called on
  `SubmitResult::rebased`.
- Tests: `releasing_held_media_does_not_inflate_the_jitter_estimate`,
  `a_timestamp_rebase_does_not_inflate_the_jitter_estimate`,
  `a_rebase_resets_the_arrival_estimate`.

### 4. Underrun fill counted — FIXED
- Kotlin `MediaPlayer.fillUnderrun`: when the reader has no data for one
  playback period while playing (gated on having played once), it feeds 10 ms
  of silence and reports the accepted frames through
  `recordUnderrun` → `MediaSession::record_underrun` →
  `record_concealment(Underrun, frames)`. Inserted silence is offset out of
  `submitted_position`; the platform `underruns` field is untouched (no double
  counting).
- JNI: `Java_dev_audioshare_v2_ClientNative_recordUnderrun`; wrapper in
  `session.rs`; Kotlin externals/bridge.
- Tests (JVM): `an_underrun_feeds_one_playback_period_of_silence_and_reports_it`,
  `an_underrun_the_sink_refuses_is_not_counted`.
  Tests (native): `an_underrun_fill_is_counted_as_its_own_intervention`.

### 5. Ring trim at the TARGET — FIXED
- `MediaRingBuffer.trim` now returns `TrimResult(readPosition, trimmedFrames,
  reanchor)`: backlog above `targetBytes` is soft-trimmed to the target
  (counted separately, never concealment); above `hardLimitBytes` the same jump
  is flagged `reanchor` and reported as `ReAnchor` concealment.
- `MediaPlayer.applyReadPosition` routes soft trims to `recordSoftTrim`
  (`MediaSession::record_soft_trim`, new `soft_trimmed_frames/events`, no
  concealment counters, position cursor still advanced) and hard trims to the
  existing `recordTrim` (`ReAnchor`).
- Tests (JVM): `soft_target_trim_is_reported_separately_from_reanchor`,
  `ring_trim_reports_whether_it_was_a_hard_limit_reanchor`,
  `soft_trim_is_frame_aligned_with_an_unaligned_target`, plus the existing
  hard-limit and wrap tests.
  Tests (native): `a_soft_target_trim_is_not_concealment`.

### 6. Kotlin duplicated 60/200 constants — FIXED
- Native accessors `jitterTargetMs` / `jitterHardLimitMs` (JNI + `session.rs`),
  backed by `MediaSession::jitter_config()` → `JitterBuffer::config()`.
- `MediaPlayer.resolveJitterBounds` uses native values per sink and falls back
  to `FALLBACK_TARGET_MS`/`FALLBACK_HARD_LIMIT_MS` only when native reports no
  session.
- Tests: `jitter_bounds_use_native_values_with_normative_fallbacks`,
  `jitter_bounds_are_exposed_from_the_active_configuration`.

### 7. Impairment row 6 never transmitted — FIXED
- `crates/v2-transport/tests/impairment_matrix_server.rs` carries the 375 B/tick
  link budget over ticks until a whole datagram fits, and now asserts actual
  transmitted bytes: budget conservation (`sent + carried == earned`), ≥95 % of
  the 300 kbit/s budget actually transmitted, bounded 50-datagram link queue and
  256-datagram send queue, and unchanged full-rate recovery.
- Dead code `let bytes = front.len()...; let _ = bytes;` removed.

### 8. rustfmt — FIXED
- `cargo fmt --all` on the native workspace; `cargo fmt --all -- --check` and
  root `cargo fmt --check -p v2-transport` both clean.

### 9. Orphan comment / dead code — FIXED
- Removed the orphaned doc comment in `native/tests/parser_fuzz.rs`; removed the
  dead `bytes` binding in the row-6 test; removed `capture_at(...) + 0` in
  `clock_drift_session_contract.rs`.

### 10. Stale "reserved for M5-04" comment — FIXED
- `ConcealmentKind` doc now states M5-03 emits Gap/Underrun and M5-04 emits
  Drift/ReAnchor.

### 11. `record_concealment` discarded `kind` — FIXED
- Per-kind `ConcealmentCounts {gap, underrun, drift, reanchor}` tracked and
  exposed; `session.rs` emits `event = playback.concealments gap=… underrun=…
  drift=… reanchor=… total=…` on the diagnostics path when any exist.
- Test: `concealment_kinds_are_kept_as_a_breakdown`.

### 12. Test helper dedupe — PARTIAL (best effort)
- `android-client-v2/native/tests/common/mod.rs` now owns splitmix64 `Rng` and
  the env-seeded helper; `impairment_matrix.rs` and `parser_fuzz.rs` use it.
  Per-file `config`/`session`/`datagram` fixtures were left in place: their
  signatures differ and moving them would have churned every new test file for
  no behavioural gain. No suite risk taken.

## Explicitly deferred / NOT RUN
- M5-05 WASAPI/pending/pacer/network-latency attribution and tuning: NOT
  implemented; the L1 instrumentation lives only on
  `experiment/m5-audiotrack-buffer` and the maintainer ruled it stays there. Not
  merged or cherry-picked.
- A drift field in §9 `PlaybackDiagnostics`: adding one is a protocol change.
  Deferred; `driftPpm` JNI accessor + `playback.drift` log remain the
  non-protocol channel.
- Real Android device / real network verification (late drops, target trim
  audibility, 300 kbit/s path): NOT RUN.
- Row 6 is a host application-layer simulation of the byte budget, not a real
  300 kbit/s network path; row 7 DPLPMTUD over a real path and row 10 live
  connection kill are NOT RUN (unchanged from M5-06).

## Test results (all run in this worktree)

- `cargo test -p v2-transport`: **289 passed, 0 failed, 0 ignored**.
- `cargo test -p audio-share-core`: **118 passed, 0 failed, 1 ignored**.
- `cargo test --manifest-path android-client-v2/native/Cargo.toml` (default):
  **222 passed, 0 failed** across 9 targets (146 lib + contracts/fuzz).
- `cargo test --manifest-path android-client-v2/native/Cargo.toml --features
  test-support`: **222 passed, 0 failed** (same counts).
- `./gradlew testReleaseUnitTest -PnoNative=true ...`: **265 tests, 0 failures,
  0 errors, 0 skipped** (32 classes).
