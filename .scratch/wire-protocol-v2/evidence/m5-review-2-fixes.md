# M5 second code-review fix pass — evidence

Branch: `fix/m5-review-2` (based on `integration/m5` @ `e960b70`)
Head: `823b928`
Worktree: `<repo>`

Merge: `git merge --no-ff integration/m5` reported "Already up to date"
(no merge commit), because `integration/m5` (`e960b70`) is already an
ancestor of the fix commit.

Every item is FIXED with a regression test unless explicitly listed under
NOT RUN. Evidence is host/JVM only; no device claims.

## Finding-by-finding

### 1. Deadline wedge (regression) — FIXED (PASS)
- `native/src/jitter.rs`: the playout anchor is now `(session now_ms, capture
  position at the end of the newest accepted frame)`. `submit_inner` classifies
  lateness against the PRE-update anchor, then refreshes the live anchor on
  every live arrival (late/reordered included) with
  `position = max(anchor_end, arrival_end)` and `session = now`. Replay
  (`submit_replay`) never moves the anchor. A stalled capture clock plus one
  late datagram therefore recovers instead of wedging.
- Tests (native): `a_late_datagram_refreshes_the_deadline_anchor` (unit) and
  `a_late_datagram_does_not_wedge_the_deadline_anchor` (session seam). Both
  assert one late drop followed by accepted datagrams and no repeated late
  drops.
- Existing deadline tests (`an_arrival_after_a_sender_pause_is_late_by_deadline`,
  `a_resumed_sender_with_advanced_capture_clock_is_not_late`,
  `a_timely_reorder_still_plays_under_the_deadline_rule`,
  `a_far_late_arrival_is_late_dropped_and_not_network_loss`) stay green.

### 2. Underrun false positives — FIXED (PASS)
- `MediaPlayer.kt`: the fill gate is now
  `platform underrunCount advanced AND hasPlayed AND available < one frame`.
  The platform count is consumed each iteration, so one platform event causes
  at most one fill; `recordPlayback` continues to report the platform
  `underruns` field separately (no double count). Playback also prebuffers to
  the jitter target before the first write (`readyForPlayback`), so startup
  emptiness is not counted.
- JVM tests: `underrun_fill_is_gated_on_a_platform_underrun_delta`,
  `playback_prebuffers_to_the_jitter_target_before_starting`, plus the existing
  `an_underrun_feeds_one_playback_period_of_silence_and_reports_it`,
  `an_underrun_the_sink_refuses_is_not_counted`.
- Native test: `reporting_a_platform_underrun_is_not_a_concealment_by_itself`
  (the §9 platform field is separate; only an actual fill counts). This one
  pins an existing invariant rather than a pre-fix failure; the true RED→GREEN
  regression for this finding is the JVM gate test above.
  `an_underrun_fill_is_counted_as_its_own_intervention` still green.

### 3. Soft-trim hysteresis + accounting — FIXED (PASS)
- `MediaRingBuffer.trim` takes `hysteresisBytes` and trims softly only when
  `backlog > target + hysteresis`; it trims to the frame-aligned target. Hard
  `backlog > hardLimit` remains `ReAnchor`. `MediaPlayer` computes the 20 ms
  band (`SOFT_TRIM_HYSTERESIS_MS`) in bytes from the active geometry.
- Accounting: soft trims increment `soft_trimmed_frames` / `soft_trim_events`
  (position cursor still advanced) and NEVER the §8 concealment counters. Hard
  trims keep `trimmed_frames` / `trim_events` and `ReAnchor`.
  `session.rs` emits `event = playback.trim soft_frames=… soft_events=…
  reanchor_frames=… reanchor_events=…` on the diagnostics path, so the
  counters are log-reachable.
- Code documents the deliberate M5-03 deviation from §8's four-intervention
  list (buffer management, not concealment) in `MediaRingBuffer.kt`,
  `MediaPlayer.kt`, and `media_session.rs`. The ticket file lives untracked in
  the main checkout; per the task's "do not touch the main checkout" rule it
  was not edited (see NOT RUN).
- JVM tests: `soft_trim_waits_for_the_hysteresis_band`,
  `soft_target_trim_is_reported_separately_from_reanchor`,
  `ring_trim_reports_whether_it_was_a_hard_limit_reanchor`,
  `soft_trim_is_frame_aligned_with_an_unaligned_target`.
- Native test: `a_soft_target_trim_is_not_concealment` (unchanged, green).

### 4. Drift window pollution from soft trim — FIXED (PASS)
- `MediaSession::record_soft_trim` now resets the 10 s estimator window and
  clears `drift_ppm`, exactly like the hard re-anchor.
- Native test: `a_soft_target_trim_restarts_the_drift_window`.

### 5. Config-change rebuild — FIXED (PASS)
- `MediaSession::apply_configuration` rebuilds `JitterBuffer` on an accepted
  generation change with the preserved `JitterConfig`, the NEW
  `sample_rate` and NEW `frame_bytes`, before releasing held datagrams.
- Native test: `a_decode_affecting_config_change_rebuilds_the_jitter_buffer`
  (gen 1 48 kHz stereo F32 → gen 2 24 kHz mono S16; asserts the gap fill
  happens at the new 1440-frame target and the silence bytes are 64 × 2 B,
  not 64 × 8 B). `held_datagrams_released_on_config_change_keep_the_frontier_in_sync`
  and the drift geometry-reset path stay green.
- BEHAVIOUR CHANGE / residual risk: the rebuild happens on every accepted
  generation bump, including one whose geometry is unchanged (per the brief's
  "geometry/generation change"). It drops the superseded generation's pending
  frames, frontier, deadline anchor, and A.8 estimate. That is the intended
  per-generation reset, and the held-release test still passes, but it changes
  pre-config-change queue state.

### 6. `reset_arrival` doc accuracy — FIXED (PASS)
- Doc now states it runs on a timestamp re-base; held-media replay never calls
  `observe_arrival`, so it needs no reset. Behaviour unchanged (tests green).

### 7. Clippy — FIXED (PASS)
- `Arrival` struct removes `too_many_arguments` from `submit_inner`;
  `HeldDatagram`/`HeldMedia` remove `type_complexity` from `holding`; the
  `drain` loop is a `while let`.
- Also fixed the pre-existing `unused variable: case` in
  `native/tests/parser_fuzz.rs` (only surfaced once the lib compiled clean
  under `--all-targets`).
- `cargo clippy --manifest-path …/native/Cargo.toml --all-targets -- -D warnings`
  exits 0.

### 8. Dead code — FIXED (PASS)
- `MediaRingBuffer.trimmedReadPosition` removed; tests call `trim(...)` and read
  `.readPosition`. No references remain (`grep` exit 1).
- Soft-trim accessors are consumed by the `playback.trim` log line, not
  write-only.

## Test / tool results (all run in this worktree)

- `cargo test -p v2-transport`: PASS — 289 passed, 0 failed, 0 ignored.
- `cargo test -p audio-share-core`: PASS — 118 passed, 0 failed, 1 ignored.
- `cargo test --manifest-path android-client-v2/native/Cargo.toml` (default):
  PASS — 227 passed, 0 failed (147 lib + 80 integration across 8 targets).
- Same with `--features test-support`: PASS — 227 passed, 0 failed.
- `./gradlew testReleaseUnitTest -PnoNative=true …`: PASS — 268 tests,
  0 failures, 0 errors, 0 skipped (32 classes).
- `cargo clippy … --all-targets -- -D warnings`: PASS — exit 0.
- `cargo fmt --manifest-path …/native/Cargo.toml --all -- --check`: PASS —
  exit 0.

## NOT RUN / not done

- Real Android device / dual-device / 60-minute soak: NOT RUN (no device).
- Real-device audibility of soft trim / underrun gating / drift correction:
  NOT RUN.
- Ticket edit for the deliberate M5-03 soft-trim deviation: NOT DONE in the
  ticket file — it is an untracked file in the main checkout, which this
  worktree is forbidden to touch. The deviation is documented in the three
  code sites named above and here.
- Protocol changes / normative default changes: none made (target 60 ms,
  hard limit 200 ms unchanged; no proto fields added).
