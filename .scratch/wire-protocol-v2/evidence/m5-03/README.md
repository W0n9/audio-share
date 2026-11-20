# M5-03 implementer evidence — 2026-11-19

Branch: `ticket/m5-03-jitter-buffer` (worktree `agent-a62d94b9f822c92d4`), base `integration/m5` @ 8c35411.

Commits: implementation `491d159`, merge of integration/m5 `43e2c15` (clean, ort; picked up M5-05 server pacing commits, no conflicts, no server files touched by me), held-datagram frontier fix `bb08808`. All test results below were re-run on the final tip `bb08808` and pass.

## Test commands run (worktree paths)

The task's literal host command uses the main-checkout manifest path
(`<repo>/...`), which does NOT see this worktree's
uncommitted changes; I ran the same command pointed at the worktree manifest
so it exercises the implementation. (Path-only difference; identical Cargo
target/feature set.)

1. `cargo test --manifest-path <repo>/android-client-v2/native/Cargo.toml`
   - PASS — 169 tests total across 6 targets (140 lib + 10 jitter_buffer_contract + 2 media_control_closing + 5 media_control_contract + 12 media_session_contract + 0 doc). 0 failed, 0 ignored.
2. `cargo test --manifest-path <repo>/android-client-v2/native/Cargo.toml --features test-support`
   - PASS — 169 tests, 0 failed, 0 ignored.
3. `ANDROID_HOME="$HOME/Android/Sdk" ./gradlew testReleaseUnitTest -PnoNative=true`
   (from worktree `android-client-v2`; Gradle needed
   `-Porg.gradle.java.installations.paths=<home>/.cache/audio-share-refactor-worktrees/toolchain/jdk-21.0.13+9`
   because the test JVM toolchain is JDK 21 and the default `java` is 17)
   - BUILD SUCCESSFUL — 254 tests, 0 failures, 0 skipped (JUnit XML aggregate).

## RED → GREEN slices

- RED: `jitter_buffer_contract.rs` failed to compile (missing `JitterConfig`,
  `ConcealmentKind`, `LateDropped`, counter methods, real diagnostic fields).
- RED: `MediaPlaybackTest.kt` failed to compile (missing `trimmedReadPosition`,
  `applyReadPosition`, `recordTrim`).
- GREEN after implementation: all host + JVM tests above pass.

## What is implemented

- Native `jitter.rs`: bounded reorder buffer keyed by RFC3550 extended sequence;
  payload duration measured by wire `timestamp` (capture-clock position, NOT
  assumed == capture frame — re-bases on contiguous timestamp jumps to absorb
  the M5-05 server-drop offset without inserting silence). D12 target 60 ms /
  hard limit 200 ms as named configurable `JitterConfig` (defaults pinned by
  test). Late arrivals past the frontier → `LateDropped` (separate from network
  `lost`). Gap fill = all-zero PCM in active format. RFC3550 A.8 jitter at
  published rate.
- `media_session.rs`: real values in `diagnostics()` for late drops,
  concealments, concealed_frames (inserted − deleted), jitter_microseconds;
  `record_concealment(kind, delta)` public hook for M5-04 (Gap/Underrun/Drift/
  ReAnchor all share the §9 counter); `record_trim()` kept separate.
- Kotlin: `MediaRingBuffer.trimmedReadPosition` (drop oldest to target,
  frame-aligned); `MediaPlayer` lap path replaced with trim/re-anchor +
  `recordTrim` JNI; `BUFFER_TARGET_MS` now 60.

## NOT RUN

- Real-device audible-click listening test: NOT RUN (no device available). No
  PASS claimed.
- Device/emulator Gradle device tests: NOT RUN (no device/AVD in this context).
