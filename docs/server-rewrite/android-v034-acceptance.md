# Real v0.3.4 Android APK acceptance (ticket 15)

Status: **NOT EXECUTED**. No Android phone or emulator with the released
v0.3.4 APK is available on this host. This page defines the exact matrix,
procedure, and evidence a human must produce; nothing here is a claim that the
client passed.

The Wire Protocol is frozen at v0.3.4 and the scripted matrix
(`docs/server-rewrite/acceptance-matrix.md`) already proves the server side of
the contract. The remaining gate is the real client: it must play audio
without audible artifacts or parsing misalignment, and its reconnect/heartbeat
behavior must match v0.3.4 when driven by this server.

## Required inputs

- The released **v0.3.4 APK** (`io.github.mkckr0.audio_share_app`), with its
  SHA-256 recorded. Do not substitute a locally rebuilt client.
- Android device(s): at least one API 31+ (Android 12) for the 24/32-bit
  cases; note the model, Android version, and build number.
- A host on the same LAN running the candidate Server (`0.4.0`) and the
  headless or real audio source described below.
- APK on Android 12+: `ENCODING_PCM_24BIT_PACKED` and
  `ENCODING_PCM_32BIT` exist. On API < 31 those encodings map to
  `ENCODING_INVALID` by client design (parity contract §4.10), so
  s24/s32 playback failure on older Android is **expected client behavior**,
  not a server defect; record it as not-applicable for that device.

## Server preparation

For each requested combination, start the candidate server with a known
endpoint and format. Example (`$SERIAL` is the target sink's `object.serial`;
on a real desktop use the local output device):

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
`1, 2` × sample rates `44100, 48000` = 20. API < 31 devices only run the
encodings the client supports and must mark s24/s32 N/A (with the device API
level in the evidence).

For each combination record:

| Field | Evidence |
|---|---|
| Requested format | server command line |
| Actual negotiated format | `GET_FORMAT` hex + decoded encoding/channels/rate |
| Client plays | 60 s continuous audible tone/music; screen recording or timestamped notes |
| No parsing misalignment | no pitch shift, stutter, noise bursts, or client error toast during the 60 s |
| Client format display | screenshot (the client shows the format it received) |
| Server-side datagrams | `protocol-harness probe` JSON recorded during the same window |
| Result | pass / fail / N/A with reason |

A combination passes only if the phone plays cleanly and the server-side probe
reports `alignment_ok=true`, `max_datagram_ok=true`, a non-empty PCM stream,
and the expected tone. If the client refuses a format (AudioTrack creation
fails), record the client logcat line and mark the combination failed for that
device/API level.

## Multi-client, reconnect, heartbeat, long playback

- **Multiple clients**: two or more phones (or one phone plus a second
  device/emulator with the same APK) connect to one server and play the same
  sink simultaneously for at least 10 minutes. Each must stay aligned; record
  server logs and at least one probe JSON showing all clients bound.
- **Reconnect**: stop the server (Ctrl-C/SIGINT), wait 5 s, start it again.
  The phone must show its ~3 s countdown and reconnect automatically; audio
  resumes with a fresh `START_PLAY` id. Record logcat and server log timestamps.
- **Heartbeat timeout**: with audio playing on the phone, disable its Wi-Fi
  (airplane mode or firewall drop) so the client cannot answer heartbeats.
  The server must close that session ~6 s after the last client activity
  (`heartbeat-timeout` scenario in the scripted matrix shows the server-side
  contract). Re-enable networking and confirm the client reconnects. Also
  confirm an active client is *not* closed while heartbeats continue.
- **Long playback / UDP alignment**: play for at least 60 minutes at
  48 kHz stereo s16. Verify no progressive pitch drift, no noise bursts, no
  client crash. Capture server log and (if possible) `adb logcat` for the
  client. This is the "no UDP parsing misalignment after long playback" item
  in the GUI/client checklist; the 8-hour server soak is separate.

## Evidence package

Collect under one directory (attach to the release gate):

1. APK SHA-256 and version string; device model/Android API level.
2. Server binary version (`as-cmd -v`) and build commit.
3. `GET_FORMAT` hex for every combination plus the requested command line.
4. Screenshots of the client format display and, for failures, the error toast
   and logcat line.
5. Screen recording or timestamped pass/fail notes for the 60 s playback.
6. `protocol-harness probe --json-out` reports from the same server run.
7. Server `server.log` / stdout and `adb logcat` for reconnect, heartbeat, and
   the 60-minute run.
8. A completed copy of the matrix table above with pass/fail, signed by the
   human tester.

## Relationship to the release gate

This page is a residual release gate for ticket 15 and must not be checked off
until the evidence above exists. The scripted matrix, short soak, and partial
GUI checklist are necessary but not sufficient: only the real v0.3.4 APK can
prove the "无感播放" (seamless playback) requirement.
