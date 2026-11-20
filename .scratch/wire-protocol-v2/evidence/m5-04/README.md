# M5-04 implementer evidence — 2026-11-19

Branch: `ticket/m5-04-clock-drift` (worktree `agent-a4cee74fb41c77391`).
Base: `integration/m5` @ `43d7fae` (M5-03 + M5-05). Implementation commits
`0a19a06` ("feat(m5-04): clock drift measurement and correction") and
`356a38a` ("fix(m5-04): gate silence insertion on a settled buffer", the
review-driven insert-storm guard).
Merge of `integration/m5` after the commit: **no-op** — the tip was already
the branch base (`git merge --no-ff` reported "already up to date"), so there
is no merge commit and `git merge-base --is-ancestor integration/m5 HEAD` is OK.

Ticket claim: the `.scratch/.../m5-04-clock-drift-correction.md` status edit was
**blocked by worktree isolation** (`Edit` refuses the shared-checkout path and
`.scratch/` is outside the ticket's file ownership). Status left as
`ready-for-agent`; the merger should claim/close it.

## Seams tested (pre-agreed by the ticket)

1. `native/src/drift.rs` — pure `DriftEstimator` / `DriftController`
   (clock-injected; no sleeping).
2. `MediaSession` public API — `record_playback_at`, `drift_ppm`,
   `drift_correction_ppm`, `take_drift_intervention`,
   `record_drift_intervention`, `record_trim`, `submitted_position`,
   `diagnostics`.
3. Kotlin pure helpers — `playbackRateHz`, `MediaPlayer.applyDriftIntervention`
   / `applyDriftControl` against a fake `AudioSink` and fake `NativeMedia`.

## Commands and exact results

All commands were run from the **worktree** (the main-checkout manifest does not
see worktree changes; same path-only difference M5-03 recorded).

1. `cargo test --manifest-path <repo>/android-client-v2/native/Cargo.toml`
   - **PASS** — 195 tests, 0 failed, 0 ignored, across 7 targets:
     142 lib + 17 `clock_drift_contract` + 7 `clock_drift_session_contract`
     + 10 `jitter_buffer_contract` + 2 `media_control_closing`
     + 5 `media_control_contract` + 12 `media_session_contract`, 0 doc-tests.
2. Same manifest `--features test-support`
   - **PASS** — 195 tests, 0 failed, 0 ignored (same target breakdown).
3. From worktree `android-client-v2`:
   `ANDROID_HOME="$HOME/Android/Sdk" ./gradlew testReleaseUnitTest -PnoNative=true "-Porg.gradle.java.installations.paths=<home>/.cache/audio-share-refactor-worktrees/toolchain/jdk-21.0.13+9"`
   - **PASS** — BUILD SUCCESSFUL; JUnit XML aggregate: **259 tests, 0
     failures, 0 skipped** across 32 result files.
4. `rustfmt --edition 2021 --check` on the new/modified Rust files listed below
   — clean (the pre-existing unformatted M5-03 closures in `jitter.rs` were left
   untouched to keep the diff scoped).
5. Source sets not exercised by the JVM unit run:
   `./gradlew compileReleaseAndroidTestKotlin compileReleaseUnitTestKotlin -PnoNative=true -PtestSupport=true …`
   (same ANDROID_HOME/JDK paths) — **PASS**, BUILD SUCCESSFUL.
   A repo-wide grep of `app/src` found no other `MediaPlayer.NativeMedia` or
   `AudioSink` implementations; the three new `NativeMedia` members also have
   default bodies so future test doubles compile without drift support.

## RED → GREEN slices

- RED: `tests/clock_drift_contract.rs` failed to compile (no `drift` module /
  `DriftEstimator` / `DriftController`).
- RED: `tests/clock_drift_session_contract.rs` failed to compile
  (`record_playback_at`, `drift_ppm`, `drift_correction_ppm`,
  `take_drift_intervention`, `record_drift_intervention` missing).
- RED: `MediaPlaybackTest.kt` failed to compile (`playbackRateHz`,
  `applyDriftIntervention`, `applyDriftControl`, new `NativeMedia` members).
- GREEN: every suite above passes.
- One M5-03 test was updated by design: `record_trim` now counts `ReAnchor`
  concealment (renamed `a_hard_limit_re_anchor_trim_is_reanchor_concealment`).

## What is implemented

- **Measurement (§8, 10 s rolling window).** `DriftEstimator` compares the
  capture clock (latest wire `timestamp` + frames) with the playback clock
  (`submitted_position`, anchor-relative). Estimate = least-squares slope of
  `capture − playback` vs injected session time, converted to ppm at the
  published rate. No estimate until the window spans a full 10 s. Resets on:
  timestamp re-base (M5-05 server-drop offset), playback underrun increase,
  hard-limit re-anchor, and applied drift frame interventions — so the
  corrector never feeds on its own discontinuities.
- **Correction (§8, ≤ ±100 ppm, smooth).** `DriftController` clamps the
  estimate to ±100 ppm and slews by at most 10 ppm per ~100 ms report (about
  1 s to full range), then returns the applied ppm. Application path:
  Kotlin `AudioTrack.setPlaybackRate(playbackRateHz(sampleRate, ppm))`.
  At 48 kHz 1 Hz ≈ 20.8 ppm, so the effective steps are 0/±21/±42/…; the
  residual (≤ ~10 ppm) is absorbed by the buffer. Documented in code.
- **Sustained deviation (§8, > 20 ms for > 5 s).** The controller tracks the
  out-of-deadband run; strictly after 5 s it queues one bounded whole-frame
  adjustment (excess over 20 ms, capped at 10 ms). Sign: too much buffered →
  delete, too little → insert. Native does not own the read cursor, so Kotlin
  takes the adjustment through JNI, applies it at the playout point (sink
  silence for insert, reader advance for delete), and acknowledges only the
  frames actually applied. `record_drift_intervention` performs the
  `record_concealment(ConcealmentKind::Drift, ±frames)` accounting. The 5 s
  timer re-arms after each intervention.
  - **Insert-storm guard (review finding).** The platform unit run does not
    exercise a device, where the application ring can legitimately sit well
    below the 60 ms target because the AudioTrack's own buffer holds the rest
    (M5-02 evidence: ring p50 ≈ 10 ms). Without a guard, "> 20 ms low" would be
    permanently true and insert silence every 5 s — an audible dropout cadence.
    Insertion therefore requires (a) the buffer to have been inside the
    deadband at least once since reset and (b) a real 10 s drift estimate.
    Deletion only reduces latency and stays available. Host tests pin
    `a_permanently_shallow_buffer_never_inserts_silence` and
    `insertions_require_a_drift_estimate`; the device run must still watch for
    periodic inserts (NOT RUN here).
- **Hard-limit re-anchor (§8, > 200 ms).** The M5-03 Kotlin trim already drops
  oldest to target; M5-04 changes native `record_trim` to also count
  `ConcealmentKind::ReAnchor` with `−frames` and to advance the
  submitted/playout position by the deleted frames (`position_adjust_frames`).
  The position is `anchor + platform-accepted frames + re-anchor/deleted frames
  − inserted drift silence`, so latency registers as skipped media rather than
  accumulating. The trim diagnostics (`trimmed_frames`, `trim_events`) are kept.
- **Diagnostics for L1.** `PlaybackDiagnostics` is **unchanged** (no proto
  edit): §9 has no drift field and adding one is a protocol change that needs
  maintainer adjudication (noted in code). The estimate is exposed through
  `MediaSession::drift_ppm` → JNI `ClientNative.driftPpm` and a periodic
  `playback.drift drift_ppm=… correction_ppm=… buffer_current_ms=…` log line in
  `session.rs::write_diagnostics`. `drift_correction_ppm` is exposed the same
  way for the player.
- **Counters.** `cumulative/interval_concealments` and signed
  `concealed_frames` now include ReAnchor deletions and acknowledged Drift
  insert/delete; the other §9 fields are untouched.

## Files changed (all inside ticket ownership)

- Native: `native/src/drift.rs` (new), `native/src/lib.rs`,
  `native/src/jitter.rs` (SubmitResult `rebased` signal), `native/src/media_session.rs`,
  `native/src/session.rs`, `native/src/jni.rs`,
  `native/tests/clock_drift_contract.rs` (new),
  `native/tests/clock_drift_session_contract.rs` (new),
  `native/tests/jitter_buffer_contract.rs` (updated M5-03 assertion).
- Kotlin: `app/src/main/java/dev/audioshare/v2/MediaPlayer.kt`,
  `ClientNative.kt`, `ClientSession.kt`,
  `app/src/test/java/dev/audioshare/v2/MediaPlaybackTest.kt`.
- No changes to `crates/**`, `tools/**`, `protos/**`, `docs/**`, `.scratch/**`.

## NOT RUN

- **Real-device ±100 ppm drift injection and audible validation: NOT RUN** (no
  device/emulator in this context). No PASS claimed; all evidence above is
  HOST/JVM only. Per evidence policy, host stubs and JVM unit tests are not
  device/interop PASS.
- Device/emulator Gradle device tests: **NOT RUN** (no AVD/device).
- `AudioTrack.setPlaybackRate` acceptance by real audio routes: **NOT RUN**
  (the code logs and retains the previous rate if the platform rejects it).
- Device/emulator Gradle test execution (only Kotlin compilation of the
  `androidTest`/`testSupport` source sets was verified): **NOT RUN**.

## Device-run watch items (no code change; for the merger / device pass)

- **Estimator starvation.** Every underrun increase resets the 10 s drift
  window, and M5-02 device logs showed underruns rising. A device that
  underruns every few seconds may never accumulate a full window, so neither
  rate correction nor the sustained-deviation path engages. The device run
  should count how often the `playback.drift` log line appears.
- **Residual drift.** `setPlaybackRate`'s 1 Hz granularity leaves up to
  ~10 ppm uncorrected at 48 kHz. Only sustained frame deletes (or inserts,
  after a deadband pass) absorb that residue.
- **Insert storms.** Confirm the deadband/estimate guard holds in practice:
  look for periodic Drift inserts in the logs during otherwise healthy
  playback.

## Handoff notes for M5-06 (impairment matrix + parser fuzz)

- Deterministic media injection seam: `MediaSession::on_datagram(now_ms, bytes)`
  and `record_playback_at(now_ms, PlaybackReport)` take an injected session
  clock; datagrams are built with `MediaHeader::write_be`
  (`native/tests/media_session_contract.rs:27-45`,
  `native/tests/clock_drift_contract.rs` helpers).
- Pure drift components (`DriftEstimator`, `DriftController`) accept arbitrary
  synthetic capture/playback positions and time, so matrix rows can assert
  exact ppm/concealment behaviour without audio hardware.
- New signal for matrix assertions: `jitter::SubmitResult::rebased` marks the
  M5-05 server-drop timestamp offset; `MediaSession` resets the drift window on
  it. Fuzz/matrix tests can drive contiguous-sequence timestamp jumps and
  observe `drift_ppm() == None` plus unchanged `cumulative_lost`.
- The hard-limit/reader path is `MediaRingBuffer.trimmedReadPosition` +
  `MediaPlayer.applyReadPosition` (JVM-testable, no native needed).
- §9 diagnostics remain the authoritative playback-evidence channel; drift is
  deliberately out-of-protocol, so matrix reports must read the log/JNI
  accessor rather than expecting a proto field.
