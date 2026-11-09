# Linux A/B: old `as-cmd` 0.3.4 vs new Server (ticket 06)

Date: 2026-11-09. Environment: Ubuntu 24.04.5 x86_64, libpipewire 1.0.5,
WirePlumber 0.4.17, headless `scripts/pw-test-session.sh` session, one
`TestSink` null `Audio/Sink` (`priority.session=1000`), 440 Hz stereo 48 kHz
s16 tone (`scripts/gen-tone-wav.py --duration 30 --amplitude 0.25`) looped into
the sink.

- Old binary: `/home/w0n9/workspaces/audio-share-notes/server-rewrite/bin/as-cmd-0.3.4-linux`
  (release build, local packaging patches described in `environment.md`).
- New binary: `target/debug/as-cmd` (ticket 06, default PipeWire capture).
- Recorder: `target/debug/protocol-harness record` (v0.3.4 Wire Protocol;
  same fixture writer used by the committed recordings).

Both servers were recorded in the same headless session, in sequence, with the
tone playing, 700 UDP datagrams and one heartbeat each. The old server was
pinned with `-e $PW_TEST_ENDPOINT_SERIAL --encoding=f32 --channels=2
--sample-rate=48000`; the new server uses its default endpoint/format because
ticket 07 has not added those CLI flags yet.

## Commands

```bash
python3 scripts/gen-tone-wav.py /tmp/ticket06-ab/tone.wav --duration 30 --amplitude 0.25

scripts/pw-test-session.sh --tone-wav /tmp/ticket06-ab/tone.wav -- bash -c '
  "$HARNESS" record --server-cmd "$OLD" --server-arg="--bind=127.0.0.1:{port}" \
    --server-arg="-e" --server-arg="$PW_TEST_ENDPOINT_SERIAL" \
    --server-arg="--encoding=f32" --server-arg="--channels=2" \
    --server-arg="--sample-rate=48000" --out /tmp/ticket06-ab/old \
    --heartbeats 1 --datagrams 700 --timeout-secs 25 --server-label as-cmd-0.3.4-linux
  "$HARNESS" record --server-cmd "$NEW" --server-arg="--bind=127.0.0.1:{port}" \
    --out /tmp/ticket06-ab/new --heartbeats 1 --datagrams 700 --timeout-secs 25 \
    --server-label as-cmd-ticket06-linux
'
```

Analysis: Goertzel scan over channel 0 of the recorded F32 stream (coarse 10 Hz
plus 1 Hz refinement, 300–600 Hz) using a temporary script outside the repo.

## Results

| Metric | Old `as-cmd` 0.3.4 | New (`ticket/06-linux-capture`) |
|---|---|---|
| `GET_FORMAT` response hex | `0100000008000000080110021880f702` | same |
| `AudioFormat` protobuf hex | `080110021880f702` | same |
| Encoding / channels / rate | `ENCODING_PCM_FLOAT` / 2 / 48000 | same |
| `block_align` | 8 | 8 |
| `START_PLAY` id | 1 | 1 |
| Heartbeats captured | 1 | 1 |
| UDP datagrams | 700 | 700 |
| Datagram bytes min / max / mean | 280 / 1464 / 1365.897 | same |
| Full / partial datagrams | 642 / 58 | 642 / 58 |
| PCM bytes | 956,128 | 956,128 |
| PCM frames / seconds | 119,516 / 2.4899 | same |
| Dominant tone | 440.0 Hz, amplitude 0.2456 | same |
| Goertzel 1000 Hz | 0.0001 | 0.0001 |

All format bytes match exactly, the datagram size distribution is identical,
and the captured tone frequency/level match. PCM length is equal because both
runs recorded the same number of datagrams and the segmentation is
deterministic; this is **not** a byte-identical PCM comparison. Capture
scheduling and buffer sizes can vary between runs and formats, as the ticket-03
spike measured, so PCM hashes/bytes are expected to differ on re-recording.

## Old s8 defect (fixed by the rewrite)

The old Linux path requests `SPA_AUDIO_FORMAT_U8` for `--encoding=s8` but its
`param_changed` switch only maps `SPA_AUDIO_FORMAT_S8`, so negotiation with U8
falls into the unsupported branch and the process exits:

```text
[info] audio_info.info.raw.format: 258
[info] the capture format is not supported
old-s8-exit=1
```

258 is `SPA_AUDIO_FORMAT_U8`. The new backend maps negotiated U8 to
`Encoding::Pcm8Bit`; the headless integration test captures the 440 Hz tone at
s8 with `block_align=2` (stereo U8), verifying the fix. This is an intentional
behavior difference, not a protocol change.

## Caveats

- One null sink, one tone, one audio format pair (F32) for the A/B; s16/s24/s32
  parity is verified through the core backend integration test, not against the
  old binary.
- The new `as-cmd` does not yet expose `--endpoint/--encoding/--channels/
  --sample-rate` (ticket 07); this A/B used defaults, which negotiated the same
  F32/stereo/48 kHz as the pinned old run.
- Latency/jitter/drift over long runs and CPU under load were not re-measured
  here; the ticket-03 spike covers short-run CPU and callback spacing.
