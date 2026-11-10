# Real v0.3.4 Android APK acceptance (ticket 15)

Status: **EXECUTED on one Android 12 phone on 2026-11-10** (Xiaomi/Redmi
M2007J22C, Android 12 / API 31, arm64; MIUI V14.0.6.0.SJECNXM
`Redmi/cannon/cannon:12/SP1A.210812.016/V14.0.6.0.SJECNXM:user/release-keys`;
released v0.3.4 APK, SHA-256
`77cc93d32a3e6b6117b9c202ff5be69082ea5711087c23e6877cef5ac4a471b4`). The
10 executable stereo combinations played through the real client; the 10 mono
combinations are **N/A on the Windows Realtek endpoint** because the driver
rejects mono at `Initialize` (the old v0.3.4 server also cannot capture mono on
this endpoint). Two Android clients were not available; multi-client was
exercised as one phone plus a scripted `protocol-harness probe`. The
**60-minute playback PASSED** on the fixed build (12 five-minute probes, one
automatic reconnect, 0 app crashes).

The phone reached the Windows host through the host's Mobile Hotspot (`5277`,
PC `192.168.137.1`, phone `192.168.137.212`); the host firewall rules for the
test executables allowed TCP control and UDP PCM.

The Wire Protocol is frozen at v0.3.4 and the scripted matrix
(`docs/server-rewrite/acceptance-matrix.md`) already proves the server side of
the contract. The remaining gate is the real Android Client: it must play
audio
without audible artifacts or parsing misalignment, and its reconnect/heartbeat
behavior must match v0.3.4 when driven by this server.

## Required inputs

- The released **v0.3.4 APK** (`io.github.mkckr0.audio_share_app`), with its
  SHA-256 recorded. Do not substitute a locally rebuilt Android Client.
- Android phones: at least one API 31+ (Android 12) for the 24/32-bit
  cases; note the model, Android version, and build number.
- A host on the same LAN running the candidate Server (`0.4.0`) and the
  headless or real audio source described below.
- APK on Android 12+: `ENCODING_PCM_24BIT_PACKED` and
  `ENCODING_PCM_32BIT` exist. On API < 31 those encodings map to
  `ENCODING_INVALID` by Android Client design (parity contract §4.10), so
  s24/s32 playback failure on older Android is **expected Android Client
  behavior**, not a server defect; record it as not-applicable for that phone.

## Server preparation

For each requested combination, start the candidate server with a known
endpoint and format. Example (`$SERIAL` is the target sink's `object.serial`;
on a real desktop use the local output Endpoint):

```bash
target/release/as-cmd --bind=<LAN-IP>:65530 -e "$SERIAL" \
  --encoding=s16 --channels=2 --sample-rate=48000
```

Record the server's `GET_FORMAT` reply hex and the actual negotiated format
from `target/release/as-cmd` stdout/server.log. Never assume the request was
honored. A 440 Hz tone (or music with clear pitch) must be playing into that
sink for the whole test.

## Matrix (must be completed on the phone)

Requested combinations: encodings `f32, s8, s16, s24, s32` × channels
`1, 2` × sample rates `44100, 48000` = 20. API < 31 phones only run the
encodings the Android Client supports and must mark s24/s32 N/A (with the
Android Client API level in the evidence).

For each combination record:

| Field | Evidence |
|---|---|
| Requested format | server command line |
| Actual negotiated format | `GET_FORMAT` hex + decoded encoding/channels/rate |
| Client plays | 60 s continuous audible tone/music; screen recording or timestamped notes |
| No parsing misalignment | no pitch shift, stutter, noise bursts, or Android Client error toast during the 60 s |
| Android Client format display | screenshot (the Android Client shows the format it received) |
| Server-side datagrams | `protocol-harness probe` JSON recorded during the same window |
| Result | pass / fail / N/A with reason |

A combination passes only if the phone plays cleanly and the server-side probe
reports `alignment_ok=true`, `max_datagram_ok=true`, a non-empty PCM stream,
and the expected tone. If the Android Client refuses a format (AudioTrack
creation fails), record the Android Client logcat line and mark the
combination failed for that phone/API level.

## Multi-client, reconnect, heartbeat, long playback

- **Multiple Android Clients**: two or more phones (or one phone plus a second
  phone/emulator with the same APK) connect to one server and play the same
  sink simultaneously for at least 10 minutes. Each must stay aligned; record
  server logs and at least one probe JSON showing all clients bound.
- **Reconnect**: stop the server (Ctrl-C/SIGINT), wait 5 s, start it again.
  The phone must show its ~3 s countdown and reconnect automatically; audio
  resumes with a fresh `START_PLAY` id. Record logcat and server log timestamps.
- **Heartbeat timeout**: with audio playing on the phone, disable its Wi-Fi
  (airplane mode or firewall drop) so the Android Client cannot answer heartbeats.
  The server must close that session ~6 s after the last Android Client
  activity
  (`heartbeat-timeout` scenario in the scripted matrix shows the server-side
  contract). Re-enable networking and confirm the Android Client reconnects.
  Also confirm an active Android Client is *not* closed while heartbeats
  continue.
- **Long playback / UDP alignment**: play for at least 60 minutes at
  48 kHz stereo s16. Verify no progressive pitch drift, no noise bursts, no
  Android Client crash. Capture server log and (if possible) `adb logcat` for
  the Android Client. This is the "no UDP parsing misalignment after long
  playback" item in the GUI/Android Client checklist; the 8-hour server soak
  is separate.

## Evidence package

Collect under one directory (attach to the release gate):

1. APK SHA-256 and version string; phone model/Android API level.
2. Server binary version (`as-cmd -v`) and build commit.
3. `GET_FORMAT` hex for every combination plus the requested command line.
4. Screenshots of the Android Client format display and, for failures, the
   error toast
   and logcat line.
5. Screen recording or timestamped pass/fail notes for the 60 s playback.
6. `protocol-harness probe --json-out` reports from the same server run.
7. Server `server.log` / stdout and `adb logcat` for reconnect, heartbeat, and
   the 60-minute run.
8. A completed copy of the matrix table above with pass/fail, signed by the
   human tester.

## Executed matrix (2026-11-10)

Phone: Xiaomi M2007J22C, Android 12 / API 31, arm64. APK: released v0.3.4
(`versionCode 3004`, SHA-256 `77cc93d3…a471b4`). Host: Windows 11
Enterprise 10.0.26300, Mobile Hotspot `5277` (`192.168.137.1`). Tone: 440 Hz
from the GUI WebView2 `AudioContext`. Server: the new CI `as-cmd.exe`
(run 42766244106, `ebdd468`) for the 44.1 kHz and 60-minute cases, and the
locally built fixed GUI for the 48 kHz encoding matrix (same source/fixes).
Evidence: `audio-share-artifacts/android/`.

### 48 kHz stereo (GUI, requested = actual)

| Encoding | `GET_FORMAT` hex | Phone AudioTrack format | Server probe |
|---|---|---|---|
| f32 | `0100000008000000080110021880f702` | 5 (`PCM_FLOAT`), 48 kHz stereo | ok, 440 Hz, aligned |
| s8 | `0100000008000000080210021880f702` | 2 (`PCM_8_BIT`), 48 kHz stereo | ok, 601 datagrams × 960 B, 440 Hz |
| s16 | `0100000008000000080310021880f702` | 1 (`PCM_16_BIT`), 48 kHz stereo | ok, 440 Hz, aligned |
| s24 | `0100000008000000080410021880f702` | 6 (`PCM_24_BIT_PACKED`), 48 kHz stereo | ok, 1416/1464 B, 440 Hz |
| s32 | `0100000008000000080510021880f702` | 3 (`PCM_32_BIT`), 48 kHz stereo | ok, 440 Hz, aligned |

Each case showed the pause button and “已开始” on the phone, and `dumpsys
media.audio_flinger` reported an active track for the app PID with the format
in the table. The phone auto-reconnected between rows (new AudioTrack id each
time).

### 44.1 kHz requested (as-cmd, closest match)

All five requests negotiated **f32 / 2 ch / 48000 Hz** (`S_FALSE` closest
match; the Realtek shared-mode mix format). The phone played f32 48 kHz
(`PCM_FLOAT`) and the probes passed; this is the documented closest-match
behavior, not a silent resample. `GET_FORMAT` hex was the f32 value above for
every row.

### Mono (N/A on this endpoint)

`--channels 1` exits 1 before listening with the readable
`AUDCLNT_E_UNSUPPORTED_FORMAT` error (`mono-48000-stderr.txt`); the phone
cannot connect. The released v0.3.4 `as-cmd --channels 1` also cannot capture
on this Realtek endpoint (it hangs and returns a zero-length format). The 10
mono combinations are recorded as N/A for this Windows host; a Linux
PipeWire sink or a different Windows driver is required to run them.

### Multi-client, reconnect, heartbeat, long playback

- **Multi-client**: the phone played while `protocol-harness probe` connected
  concurrently (a second Wire Protocol client); every matrix probe ran with
  the phone playing. Two Android clients were not available, so the
  two-Android-client item remains partial.
- **Reconnect**: the server was stopped/started between every matrix row; the
  phone reconnected automatically and created a new AudioTrack each time.
  With Wi-Fi disabled for 10 s the app showed “Network is unreachable,
  会在2秒内重试”; re-enabling Wi-Fi rejoined the hotspot and playback resumed
  (AudioTrack id changed from 125 to 126; screenshots
  `heartbeat-wifi-off.png` / `heartbeat-wifi-on.png`).
- **Heartbeat timeout**: a Windows `protocol-harness probe --no-heartbeat-reply
  --expect-server-close` against the RC `as-cmd` reported `server_closed: true`
  at **6037 ms** while the phone session stayed active; the scripted Linux
  matrix also closes at 6001 ms. The phone dropped its dead session while
  Wi-Fi was off and reconnected after the network returned; no server crash.
- **60-minute playback**: **PASS.** The server started at 14:42:50; 12 probes
  ran at five-minute intervals (14:46:09–15:43:04) plus a final probe at
  15:45. Every probe: 1,994–2,000 datagrams per 6 s, 440 Hz present, max gap
  27 ms, 0 heartbeat misses, server alive. The phone's AudioTrack was active at
  every sample; it auto-reconnected once around 15:10 (track 129 → 131) and
  stayed on track 131 for the remaining ~31 minutes. No app FATAL/exception,
  no broadcast-drop warning; wlan0 received 682,263,771 bytes during the run.
  At 15:45 the phone still showed 193 K/s and “已开始”. Evidence:
  `audio-share-artifacts/android/soak-60m-fixed/` (`soak.csv`,
  `soak-summary.json`, `probe-01..12.json`, `soak-final-probe.json`,
  `final-screenshot.png`, `soak-final-60m.png`, `final-logcat.txt`).

### Fixed during the real-client run: UDP broadcast stall

The first 60-minute attempt (pre-fix `as-cmd`) stalled after about five
minutes: the phone still showed “已开始” but received 0.1 K/s, the monitor's
second probe got **0 datagrams** while `GET_FORMAT`/`START_PLAY`/heartbeats
still worked, and a separate `wasapi-spike capture` could still capture
144,000 frames in 3 s. Root cause: `broadcast_loop` awaited
`udp.send_to(segment, peer)`. When a Wi-Fi peer stopped draining (power save,
screen/doze, link stall), the UDP socket send buffer filled; the awaited send
blocked the broadcast loop, the bounded frame channel filled, and the capture
thread blocked in `blocking_send`. The control plane stayed alive, so the
client showed “playing” with no audio.

Fix: the broadcast loop uses `try_send_to` and drops a datagram on
`WouldBlock` (one warning per session), so a slow peer cannot stall capture or
other clients. Post-fix validation: with the phone's Wi-Fi disabled for 60 s
and a deliberately non-reading UDP peer connected, the PC probe still received
1,198 datagrams per 6 s with 440 Hz present and a 16 ms max gap. The fixed
60-minute phone soak is the regression test; the pre-fix stall is retained in
`soak-60m/soak.csv` and `soak-check-1435.png`.

### Evidence package

`audio-share-artifacts/android/` contains the probe JSONs
(`{enc}-48k-probe.json`, `{enc}-44100-probe.json`), AudioFlinger lines
(`*-audioflinger.txt`), the phone screenshots (`phone-playing.png`,
`s16-48k-home.png`, `heartbeat-wifi-off.png`), the mono stderr, the 60-minute
`soak-60m-fixed/` evidence, and the final CI artifact smoke
`ci-final-smoke/` (6 probes over ~11 minutes, 1,997–2,000 datagrams/6 s,
440 Hz, 0 heartbeat misses, 0 app crashes).

## Relationship to the release gate

The single-phone matrix, reconnect, heartbeat, and 60-minute evidence above now
exists on one Android 12 phone. A two-Android-client run and the mono
combinations (N/A on this Realtek host) are still open; the ticket-15 "real
APK" box stays unchecked until the two-client item is recorded (the mono
combinations require a different host/driver). The scripted matrix, server
soak, and GUI checklist are necessary but not sufficient: only the real v0.3.4
APK can prove the "无感播放" (seamless playback) requirement.
