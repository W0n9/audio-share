# M5 supplement — sub-floor buffer + ring backlog (2026-11-19)

Build: `experiment/m5-audiotrack-buffer`, added two adb-only knobs on top of
`track_buffer_ms` / `low_latency`:
- `debug.audioshare.track_frames <N>` → calls `AudioTrack.setBufferSizeInFrames(N)` AFTER
  build+play. This lowers the **effective** buffer below `getMinBufferSize`'s floor
  (the 4-arg `getMinBufferSize` overload is @hide and unusable from Kotlin).
- `RingBacklog` logcat tag → logs Kotlin ring depth (`bufferMs`, `bufferedFrames`) every
  report tick (100 ms).

## Finding 1: ring backlog is NOT the hidden latency

Default tier (track_buffer_ms=100, plain path), 60 s of `RingBacklog`:
**bufferMs stayed at 0–20 ms** (mostly 10 ms, occasionally 0), underruns=0 throughout.
See `ring-backlog/ring-backlog.txt`.

→ The reviewer hypothesis "latency sits in the 256 KiB Kotlin ring" is **disproven** on this
device. The ring is not accumulating a startup backlog. Remaining latency is upstream
(Windows WASAPI 1 s buffer / 3 ms poll, Server send-ahead ≤100 ms) and in AudioTrack.

## Finding 2: setBufferSizeInFrames breaks the 42 ms floor

| Prop `track_frames` | open log actual | dumpsys FrmCnt | Latency | Underruns (early) |
|---|---|---|---|---|
| 1024 | 1024 fr / 21 ms (capacity 2052) | 1024r | 88–110 ms | 77824 at 4 s, then 0 at 6 s |
| 512 | 512 fr / 10 ms (capacity 4800) | 512r | 74.56 ms | 118272 at 4 s |

- Effective buffer CAN be set below `getMinBufferSize`. 21 ms and 10 ms both accepted.
- Early underrun counters are huge, but MIUI's counter is known-unreliable; the 1024 tier
  settled to Underruns=0 in the next sample. **Subjective listening is required.**
- Current listening tier: 1024 fr / 21 ms, `bufferMs` 15–41 ms, Underruns=0, Latency 88.68 ms.

## User listening (2026-11-19)

- **1024 fr / 21 ms tier: FAIL — "延迟高了，有爆音" (higher latency, audible crackling).**
  Hard-forcing the effective buffer below the platform floor is **not viable** on this
  chain. Despite `audio_flinger.Underruns` later reading 0, the ear heard artifacts — another
  data point that the MIUI counter is not trustworthy. Tier stopped immediately and props
  restored to default.
- **512 fr / 10 ms tier: NOT Run for listening** (only dumpsys captured; not played long
  enough for a judgment — and given the 21 ms result, unlikely to help).
- **`low_latency=1` + 20 ms (fast path, 42 ms buffer, Latency 58–80 ms): NOT RUN** — app was
  stopped before listening. If retried, must be judged by ear (counter showed +11808/2min,
  FrmRdy 1796→749).

## Conclusion for "can we go smaller?"

No — at least not by shrinking the AudioTrack buffer. 21 ms produced crackling and *higher*
perceived latency (consistent with underrun/concealment restarting playback). This confirms
there is no latency win available at the playback buffer; the earlier 128→42 ms reduction
was the safe floor. **The perceived delay lives upstream** (Windows WASAPI 1 s buffer /
3 ms poll, Server send-ahead ≤100 ms) — which is where L1 should focus
(`../../l1-latency-measurement.md`).

All changes remain on the experimental branch, uncommitted, unpushed. Device props restored
(`track_frames=0`, `track_buffer_ms=100`, `low_latency=0`), app stopped.
