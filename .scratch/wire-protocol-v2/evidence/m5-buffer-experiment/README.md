# M5 AudioTrack buffer experiment — real device

Status: `DONE` — 4 tiers collected on real device 2026-11-18; see [results.md](results.md).
L2 only preliminarily supported (42/64 ms tiers subjectively clean on one device, 1–2 min,
not blind); L1 audible-latency measurement is the next step. Config restored to default
(`debug.audioshare.track_buffer_ms=100`, app force-stopped).
Device: Xiaomi M2007J22C "cannon" / MIUI / Android 12 API 31 / arm64, adb over tailscale
LAN: phone `<REDACTED>`, server `<REDACTED>:65531` (measured RTT 2–9 ms)
Build: `experiment/m5-audiotrack-buffer`, release-signed, native rebuilt from current source

## Method (m5-proposal.md §4)

`AudioSink.open` reads `debug.audioshare.track_buffer_ms` via `setprop`; default 100 ms
unchanged. The app logs requested vs **actual** `bufferSizeInFrames`/capacity/actual ms.
Each tier is 2 minutes with the same audio source and volume, screen awake.
**Subjective listening (continuous / clicks / dropouts) is the primary judgment and must be
reported by the user.** MIUI `underruns` is reference-only (known-unreliable: 23,028 in 5 min
with no audible clicks). QUIC ACK / frame arrival is NOT playback evidence.

Actual frame count from the log/dumpsys is the evidence — the requested value may be clamped
by `getMinBufferSize` or rounded up by the platform. If all tiers clamp to the same value,
that is itself the answer.

## Tiers

Order: baseline first, then 20/30/40 ms. Only one APK; tier switched with `setprop`
between sessions (cold restart the app so `AudioSink.open` re-runs).

**Measured 2026-11-18 baseline**: `minBuffer=16416 B = 2052 frames = 42.8 ms` at 48 kHz
f32 stereo. So requests below ~43 ms are clamped to the platform minimum. 20/30/40 ms
requests likely all collapse to the same value. If so, add 50/70 ms tiers to get a real
gradient up to the 100 ms baseline. Record requested AND actual; a clamped tier is itself
a valid finding.

| Tier | setprop | Requested | Actual frames / ms (from log) | FrmRdy steady | underrun Δ | Subjective (user) |
|---|---|---|---|---|---|---|
| 100ms-baseline | (unset / 100) | 100 ms | 6150 fr / 128 ms (latency 224 ms) | 3680 | 0 | — |
| 20ms | `debug.audioshare.track_buffer_ms 20` | 20 ms | — (expect clamp to ~43) | — | — | — |
| 30ms | `... 30` | 30 ms | — (expect clamp to ~43) | — | — | — |
| 40ms | `... 40` | 40 ms | — (expect clamp to ~43) | — | — | — |

## Commands (agent side, via adb over tailscale)

```
adb -s <REDACTED>:5555 shell setprop debug.audioshare.track_buffer_ms 20
adb -s <REDACTED>:5555 shell getprop debug.audioshare.track_buffer_ms
# cold restart with QR-paired credential (pairing stays in app prefs):
adb -s <REDACTED>:5555 shell am force-stop dev.audioshare.v2
adb -s <REDACTED>:5555 shell am start -n dev.audioshare.v2/.MainActivity
./collect.sh 20ms 120
```

For baseline: `adb shell setprop debug.audioshare.track_buffer_ms 100` (or clear it).

## User-side steps (Windows, no Rust needed)

1. Start the Windows portable Server (`AudioShareServer.exe`) — Server LAN IP is
   `<REDACTED>:65531`; capture must be READY.
2. Begin pairing in the GUI (QR). The phone is already paired to this server identity;
   if the experimental APK keeps the same signing + Keystore, no re-pair is needed.
   If re-pairing is required, tell the agent.
3. For each tier: tell the agent you have started playback; at the end of each 2-minute
   tier, report **continuous / clicks / dropouts** and any change in sound.

## Decision rule

- 20–40 ms tiers subjectively clean and no significant underrun growth → **L2 holds**;
  M5 work shifts to scheduler + small buffer.
- Otherwise keep the large buffer; jitter buffer re-valued at the application layer.
- Record as real-device PASS/FAIL (not EMULATOR). Any `NOT RUN` must stay `NOT RUN`.
