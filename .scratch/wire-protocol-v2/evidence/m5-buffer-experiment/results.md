# M5 AudioTrack buffer experiment — results (objective)

Date: 2026-11-18 · Device: Redmi Note 9 (M2007J22C / cannon), MIUI, Android 12 / API 31
LAN: phone `<REDACTED>` ↔ server `<REDACTED>:65531`, RTT 2–9 ms
Source: `experiment/m5-audiotrack-buffer`, release APK signed with the official beta key,
installed via `adb install -r` (pairing + Keystore preserved; no re-pair needed).

## Method

`AudioSink.open` reads `debug.audioshare.track_buffer_ms` (adb-only, default 100 ms unchanged)
and logs requested bytes, `getMinBufferSize`, and actual `bufferSizeInFrames`.
Each tier: setprop → `am force-stop` → cold `am start` → capture open log → steady
playback → `dumpsys media.audio_flinger` before/after. Screen awake, same server and audio.
**Actual tier durations differed**: baseline ≈76 s, 20 ms ≈70 s (both cut short when the
harness misflagged the background collector), 30 ms and 40 ms = full 120 s. Tiers ran
sequentially on changing music content, so underrun Δ values are NOT directly comparable
across tiers.

## Raw open-log evidence

| Tier | getMinBufferSize (bytes / ms) | requestedMs → bytes | **Actual frames / ms** |
|---|---|---|---|
| 100ms-baseline | 16416 / 42.8 | 100 → 38400 | **6150 fr / 128 ms** |
| 20ms | 16416 / 42.8 | 20 → 16416 (clamped) | **2052 fr / 42 ms** |
| 30ms | 24624 / 64.2 | 30 → 24624 (clamped) | **3078 fr / 64 ms** |
| 40ms | 24624 / 64.2 | 40 → 24624 (clamped) | **3078 fr / 64 ms** |

Notable: `getMinBufferSize` returned 16416 B on the first two cold starts and 24624 B on the
later two. Cause not investigated (possible output-route or track-flag difference). Whatever
the cause, all sub-floor requests clamped to the returned minimum.

## audio_flinger track rows (active track, before → after)

Columns: FrmCnt / FrmRdy / Underruns / Latency(ms)

| Tier | FrmCnt | FrmRdy before→after | **Underruns before→after (Δ)** | Latency before→after |
|---|---|---|---|---|
| 100ms-baseline | 6150 | 3200 → 2528 | **0 → 0 (Δ0)** | 213.4 → 198.8 |
| 20ms | 2052 | 2052 → 2052 (full) | **0 → 0 (Δ0)** | 126.7 → 125.9 |
| 30ms | 3078 | 2464 → 1222 | **0 → 1286 (Δ1286)** | 135.8 → 109.3 |
| 40ms | 3078 | 2688 → 499 | **0 → 1779 (Δ1779)** | 140.5 → 72.2 |

Counter-intuitive: the *larger* 64 ms buffer tiers (30/40) accrued underruns while the
smaller 42 ms tier (20) had none. Two readings:
1. MIUI's `underrunCount` is unreliable (already flagged in m5-proposal §3 Q3) — note that a
   force-stop teardown produced 2432/34768/50240 underruns on old tracks instantly.
2. Something else (server pacing / the MIUI screenshot loop seen every ~4 s in logcat)
   affected the later tiers. Not a clean A/B on buffer size alone.

**Therefore the underrun numbers are reference-only. Subjective listening is the primary
judgment and is reported by the user below.**

## Subjective listening (user, 2026-11-18)

User report, verbatim: **"都连续，但都感觉有一点延迟，希望延迟尽可能小"**
(all four tiers continuous with no clicks/dropouts; all still feel slightly laggy; want
latency as low as possible).

| Tier | User verdict |
|---|---|
| 100ms-baseline | continuous, slight perceptible delay |
| 20ms (42 ms actual) | continuous, slight perceptible delay |
| 30ms (64 ms actual) | continuous, slight perceptible delay |
| 40ms (64 ms actual) | continuous, slight perceptible delay |

**Interpretation (preliminary, single device / screen-on / 1–2 min per tier / not blind)**:
all tiers subjectively clean → the 128 ms platform buffer is **not required** for click-free
playback on this device. Enough to **preliminarily support** m5-proposal L2, not to prove it.
Supported objectively: baseline Underruns=0 and 20 ms tier Underruns=0, both with normal FrmRdy.

## What the result does NOT show

- **The user could not hear a difference between 128 ms and 42 ms** — all four tiers were
  reported as "continuous but slightly laggy". This *weakens* m5-proposal's central premise
  that the AudioTrack buffer is the dominant source of audible delay. Do not infer that
  end-to-end latency dropped by the 75–87 ms that `audio_flinger.Latency` fell: that column
  is not an end-to-end audible-latency metric.
- **Latency may have moved upstream rather than disappeared.** In the 20 ms tier `FrmRdy`
  stayed at 2052 (full), i.e. writes were blocking at the platform; the backlog may simply
  sit in the native ring or Server send-ahead queue now. Server §9 `buffer_current_ms` was
  not collected — **that check is `NOT RUN`**, and it is required before claiming a net win.
- End-to-end audible latency was never measured (m5-proposal L1). Upstream contributors
  include Windows WASAPI capture buffering and Server send-ahead (up to ~100 ms).

## Decision (m5-proposal §4)

- **L2: preliminarily supported, not proven** — 42/64 ms tiers were subjectively clean on
  one device for 1–2 min; no clicks. Larger buffer is not necessary for click-free playback
  here, but the audible-latency benefit was not demonstrated.
- **Run L1 next** — measure end-to-end audible latency (reference signal → device → loopback
  or timestamp method) before changing any production default. Candidate levers to measure:
  `PERFORMANCE_MODE_LOW_LATENCY` (API 26+), Server send-ahead / ring target, Windows capture
  buffer.
- **Q1 (60/200 ms values)**: NOT decided — no audible-latency measurement; the AudioTrack
  platform floor is not the jitter-buffer target. Leave unchanged.
- **Q2 (128 ms = M4 limitation or M5 scope)**: a 42 ms buffer is reachable and clean here,
  but the user's "still slightly laggy" across all tiers means changing the production
  default needs L1 data + longer soak + maintainer adjudication.
- **Q3 (MIUI underruns trustworthy?)**: **NO** — 30/40 ms tiers recorded +1286/+1779 with
  clean subjective playback, and teardown alone produced 2432–50240 underruns instantly.
  Confirms m5-proposal §3 Q3; do not use as an acceptance metric on this device.

## Supplement: PERFORMANCE_MODE_LOW_LATENCY (2026-11-19)

Added `debug.audioshare.low_latency=1` knob (sets `AudioTrack.setPerformanceMode`);
compiled into an experimental release APK, installed with `adb install -r`.

| Tier | open log | Track flags | FrmCnt | FrmRdy before→after | Underruns Δ | Latency before→after |
|---|---|---|---|---|---|---|
| lowlatency + 20ms | minBuffer=16416, lowLatency=true, actual 2052 fr/42ms | **F2 / f (fast path)** | 2052 | 1796 → 749 | **+11808** | **79.7 → 57.7 ms** |

- `PERFORMANCE_MODE_LOW_LATENCY` **did take effect**: track moved to the fast path (`F2`/`f`
  flags) and `audio_flinger.Latency` fell from ~126 ms (plain 20 ms tier) to **~58–80 ms**.
  Buffer frame count stayed at 2052/42 ms — the floor is unchanged; the gain is the path.
- Underruns +11808 in 2 min, **but MIUI's counter was already proven unreliable**; only
  user listening can decide if this tier is usable. FrmRdy dropped 1796→749, so the fast
  path is running with less headroom — plausible cause of real glitches if any.
- This is still NOT an end-to-end audible-latency measurement. See
  `../../l1-latency-measurement.md` for the L1 plan.

Evidence tier: real device (M2007J22C / MIUI / API 31), LAN RTT 2–9 ms. NOT EMULATOR.
Underrun figures are reference-only. `60 min soak`, `p95/p99`, audible-latency baseline
remain `NOT RUN`.
