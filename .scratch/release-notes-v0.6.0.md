# Audio Share 0.6.0 — release notes draft

Status: **local annotated tag only** (`v0.6.0`), 2026-11-22. Not pushed, no GitHub Release,
no release artifacts built for this version. Created under the maintainer's narrowed stable
gate (see `roadmap.md` §5, 2026-11-22 ruling).

Tag target: `ac9482a` on `integration/wire-protocol-v2-beta` (code + device evidence).
The Android app in this build still reports `0.2.0-beta` (`versionCode 2`); `VERSION` /
`versionName` / `versionCode` were **not** changed and no artifact was rebuilt for 0.6.0.

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

- Host: `cargo test` 260 / `--features test-support` 260, clippy, fmt; JVM 278.
- Device (M5-12, APK `4120cdd`, Windows Server `431997b`, 48 kHz): judgement window
  180 snapshots, correction at the actuator ceiling **0/180**, `residual_ppm` median 1 /
  mean 0.92, `correction_ppm` max 5 while the raw 10 s estimate still ranged −225…+361
  (170/180 intervals outside ±100). Buffer 0–20 ms, server pending 0, drops 0.
- Device (M5-09): see `wire-protocol-v2/evidence/m5-12/` and `evidence/m5-11/`.

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
