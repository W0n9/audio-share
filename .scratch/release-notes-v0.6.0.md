# Audio Share 0.6.0 — release notes draft

Status as recorded on 2026-11-22: **local annotated tag only** (`v0.6.0`), not pushed,
no GitHub Release, and no release artifacts built for this version. See the maintainer's
[narrowed stable gate](wire-protocol-v2/roadmap.md#5-发布与迁移-gateexperimental--stable).

The tag points to `675f290`; `ac9482a` is the earlier code and device-evidence baseline.
The tagged source still has `VERSION=0.5.0-beta`, and its v2 Android app reports
`0.2.0-beta` (`versionCode 2`). The review branch bumps the product to `0.6.0` and the
Android app to `0.6.0` (`versionCode 3`), and the release workflow now produces a
separate APK per ABI (`arm64-v8a`, `x86_64`). Neither set of artifacts is a v0.6.0
release artifact until the final commit is built and accepted.

## What this release is

Single-Client PCM over Wire Protocol v2: encrypted, authenticated transport (M4), plus the
M5 jitter buffer, concealment and clock-drift correction that were verified on device.
Capability boundary for this stable: **one Client**, no multi-Client isolation (M6) and no
full M7 acceptance.

## Changes since 0.5.0-beta

- M5-03/04/05/06: bounded jitter buffer (60 ms target / 200 ms hard limit), deadline
  late-drop and concealment, 10 s drift measurement, frame intervention, timestamp
  re-anchoring, server-side drop accounting, §11.2 host matrix and parser fuzz.
- M5-09: drift step and burst-trim investigation — adjudicated PASS under the spec §8 /
  §11.2 criteria on device (long-run median/mean within ±100 ppm, no catastrophic
  outliers, full coverage, bounded correction and buffer).
- M5-11: playback clock source now `AudioTrack.getTimestamp()` interpolation with a
  32-bit head fallback; the "accepted bytes caused the noise" hypothesis was **not
  confirmed** and is recorded as such.
- M5-12: drift correction no longer saturates on measurement noise. Control now runs on a
  180 s residual baseline with 20 s damping (the 10 s estimate remains diagnostic); the
  closed loop is reconstructed as `residual + mean(applied)` (the old controller only
  applied half of a real drift); the integer-Hz executor is clamped inside the §8 ±100 ppm
  bound (48 kHz: ±4 Hz = ±83.3 ppm, previously +100 ppm rounded to +104.2 ppm). New
  diagnostic field `residual_ppm=` on `playback.drift`.
- Server: media pacing, `media.stats`, `media.drops`; v2 session log events.

## Verified

- Host `PASS`: [native tests](wire-protocol-v2/evidence/m5-12/host-drift-saturation.md)
  260 each for default and `test-support`, Clippy, fmt;
  [JVM 278](wire-protocol-v2/map.md#decisions-so-far).
- M5-12 [Windows + Android device `PASS`](wire-protocol-v2/evidence/m5-12/RESULT-WINDOWS.md)
  (debug APK `4120cdd`, Server `431997b`, 48 kHz): at the actuator ceiling **0/180**
  in the post-warm-up judgement window (**17/33** during warm-up, reported separately).
  `residual_ppm` median 1 / mean 0.92; max absolute correction 5 ppm. Raw 10 s snapshots
  ranged −225…+361 ppm; separately, 170/180 diagnostic intervals crossed ±100 ppm.
  Buffer 0–20 ms; Server pending frames and drops both 0 in the judgement window.
- M5-09 Windows + Android device: `PASS` under the
  [adjudicated long-window drift, coverage, and bounds criteria](wire-protocol-v2/evidence/m5-09/windows-retest.md).
  The [48 kHz run](wire-protocol-v2/evidence/m5-11/RESULT-WINDOWS-48K.md) still failed the
  earlier per-window criterion.

## Not verified — WAIVED / NOT RUN, never PASS

- M5-01 end-to-end audible latency baseline: `WAIVED`.
- M5-07 60-minute soak and `p95 ≤ 80 ms / p99 ≤ 120 ms`: `WAIVED`.
- §11.2 real-network impairment rows (loss, reorder, duplicate, jitter, congestion, MTU,
  reconnect): `NOT RUN` — host matrix only.
- M5-12 listening A/B and on-device ppm injection: `NOT RUN`.
- M5-10 concentrated re-anchor burst: not reproduced since the M5-09 fix; `needs-triage`.
- API 26 real device: `WAIVED` (build-level compliance only).
- Dual-device concurrency, 60-minute server soak, Windows long run: `NOT RUN`.
- M6 multi-Client isolation and M7 full acceptance, v1 regression, migration guide and
  rollback path: not part of this release.

## Upgrade notes

- v1 is unchanged and still experimental-scoped: v2 is used first, and **v2 never falls
  back to v1** on failure.
- Upgrading from an older v2 beta build with `install -r` may fail with error 205
  (ticket 34, `WAIVED`): uninstall the old v2 beta before installing.
- Release assets (signed APK, Server packages) for 0.6.0 have not been produced; the tested
  debug APK is `9a039a04…b25b8fdf` (arm64) and is not a release artifact.
