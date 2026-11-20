# Ticket 14 — Android PCM playback and PlaybackDiagnostics: evidence

Worktree: `phase-4/single-client-pcm` (main M4 worktree), base `a5de2d9` + ticket 13 merged as a separate branch `m4-server-pcm`.

Run date: 2026-11-14. **Host/JVM only — no device run was performed in this session.**

## RED → GREEN (real assertions)

| Command | Before | After |
|---|---|---|
| `cargo test --manifest-path android-client-v2/native/Cargo.toml --test media_session_contract` | 11 failures: `MediaSession`, `DatagramOutcome`, `PlaybackReport` did not exist | 11 passed (`media-session-red.log` is the compile-time RED evidence; GREEN logged in run) |
| `cargo test --manifest-path android-client-v2/native/Cargo.toml --test media_control_contract` | 5 compile errors (no `media_control`); then 1 behavioural RED: the cadence after a pending send | 5 passed |
| `cargo test .../native --lib` (new JNI media tests) | `media_format_json` empty, `ring_position` -1, `attach_ring` missing | 106 passed |
| `./gradlew testDebugUnitTest` (MediaPlaybackTest) | `ring_refuses_partial_frames` failed: ring returned 8 for a 10-byte request (rounding was in the wrong layer) | 7 passed |
| `ForegroundServiceContractTest` | asserted the literal "media playback is not implemented" (now false) | 4 passed, including the new M4 playback-owner assertions |

## GREEN totals

- `cargo test --manifest-path android-client-v2/native/Cargo.toml` → 106 lib + 5 + 11 = **122 passed**, and the same under `--features test-support`.
- `./gradlew testDebugUnitTest` → **46 JVM tests**, 0 failures (ControlFixture 9 · ForegroundService 4 · MediaFixture 9 · MediaPlayback 7 · ReconnectPolicy 7 · SessionStateRelay 10).
- `./gradlew assembleRelease` → BUILD SUCCESSFUL, default build (`testSupport` NOT enabled), arm64 `.so` 2.4 MB, no fixture identity.
- `cargo clippy --manifest-path android-client-v2/native/Cargo.toml --all-targets` → clean.

## What was implemented (interface level)

- `media_session.rs` — one module owns the per-session media decisions: header
  parse, §7.5 dedup/reorder/gap/malformed accounting, stale-generation drops,
  §7.6 bounded hold (≤32 datagrams / ≤100 ms, 2 s give-up clock), ring
  delivery, §9 diagnostics, and `submitted_position = anchor + accepted bytes/frame`.
- `media_control.rs` — the cadences: subscribe once after the first
  configuration, 5 s diagnostics with at most one pending, re-subscribe after
  a stream end.
- `session.rs` — the control loop now selects on datagrams, the media tick, and
  the control stream; `StreamStarted`/`StreamEnded`/`ConfigurationPublished`
  update the media session; 403/602-style local fatals go through one writer.
- `jni.rs` + `ClientNative.kt` — `attachMediaRing`, `ringPosition`,
  `mediaFormatJson`, `recordPlayback`; the ring is the only PCM path.
- `MediaRingBuffer.kt` / `MediaPlayer.kt` / `AudioSink` — a direct ByteBuffer
  read by a monotonic cursor (lapped → resync), `AudioTrack` sink, and an
  accounted-bytes `playOnce` so only platform-accepted bytes count.
- `ClientSession.kt` / `ControlSessionService.kt` — the playback owner lives
  with the native session (attached before `connect`, stopped before
  `destroy`), the notification no longer claims playback is unimplemented.

## Deliberately not done

- Jitter buffering / playout deadline / late drops / concealment / drift —
  M5. Diagnostics report those counters as zero rather than inventing values;
  `buffer_target_ms` is reported as 0 for the same reason.
- S24LE has no `AudioTrack` encoding on API 26; such a configuration yields no
  sink (the native side still reports the geometry) — M5/M7 decide the
  resampling policy.
- Real `AudioTrack` playback, screen-off behaviour, and 60-minute audio quality
  are **NOT RUN** on hardware in this session. The service device harness from
  ticket 11 has not been extended to media yet; ticket 15 owns that.
