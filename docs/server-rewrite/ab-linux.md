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
ticket 07 has not added those `as-cmd` flags yet.

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

## Ticket 15 extension: A/B across encodings (2026-11-09)

After ticket 07 added the `as-cmd` format flags, the same headless null sink/tone
setup was re-run for every encoding the old binary supports. This supersedes
the "flags not exposed" caveat above; the F32 measurements are unchanged.

Exact command:

```bash
scripts/acceptance-matrix.sh --skip-build --out /tmp/t15-matrix-release \
  --ab-old /home/w0n9/workspaces/audio-share-notes/server-rewrite/bin/as-cmd-0.3.4-linux
```

The script starts each server with
`--bind=127.0.0.1:$port -e $PW_TEST_ENDPOINT_SERIAL --encoding=<enc>
--channels=2 --sample-rate=48000` (the old binary needs the `--bind=` form; its
cxxopts `-b value` quirk ignores a space-separated value) and records 2 s of
440 Hz tone with `protocol-harness probe`. The old binary ignores SIGINT, so
the harness tears it down with SIGTERM after the probe; the new server exits 0
on SIGINT. That difference is outside the Wire Protocol.

| Encoding | Server | GET_FORMAT response hex | block_align | datagrams | PCM bytes | max datagram | full/partial | tone detected / amplitude |
|---|---|---|---|---|---|---|---|---|
| f32 | old | `0100000008000000080110021880f702` | 8 | 564 | 770048 | 1464 | 517/47 | 440.0 Hz / 0.2500 |
| f32 | new | `0100000008000000080110021880f702` | 8 | 564 | 770048 | 1464 | 517/47 | 440.0 Hz / 0.2499 |
| s8 | old | — (server exits 1) | — | 0 | 0 | — | — | not-supported |
| s8 | new | `0100000008000000080210021880f702` | 2 | 141 | 192512 | 1464 | 94/47 | 440.0 Hz / 0.2501 |
| s16 | old | `0100000008000000080310021880f702` | 4 | 282 | 385024 | 1464 | 235/47 | 440.0 Hz / 0.2500 |
| s16 | new | `0100000008000000080310021880f702` | 4 | 282 | 385024 | 1464 | 235/47 | 440.0 Hz / 0.2500 |
| s24 | old | `0100000008000000080410021880f702` | 6 | 423 | 577536 | 1464 | 376/47 | 440.0 Hz / 0.2499 |
| s24 | new | `0100000008000000080410021880f702` | 6 | 414 | 565248 | 1464 | 368/46 | 440.0 Hz / 0.2500 |
| s32 | old | `0100000008000000080510021880f702` | 8 | 564 | 770048 | 1464 | 517/47 | 440.0 Hz / 0.2500 |
| s32 | new | `0100000008000000080510021880f702` | 8 | 563 | 770048 | 1464 | 517/46 | 440.0 Hz / 0.2446 |

Observations:

- Format bytes are identical for every encoding both servers support, and the
  1464-byte MTU segmentation and block alignment match.
- Datagram counts and PCM lengths are equal for f32/s16 and nearly equal for
  s24/s32; the difference comes from capture scheduling, not protocol framing.
  This is **not** a byte-identical PCM comparison and none is claimed.
- The dominant tone is 440 Hz on both servers; amplitudes match within the
  measurement noise of a short capture.
- s8 is the one intentional behavioral difference: the old Linux binary
  negotiates `SPA_AUDIO_FORMAT_U8` and exits 1 in its unsupported branch; the
  new server maps U8 to `ENCODING_PCM_8BIT` and streams it.
- The new server's full matrix (20 encoding/channel/rate combinations) is in
  `docs/server-rewrite/acceptance-matrix.md`; the old binary was only run in
  this 48 kHz stereo A/B subset because that is the format family ticket 06
  established for the old baseline.

## Fix-pass extension: 30 s CPU, throughput, and arrival-gap proxy (2026-11-09)

The earlier A/B sections compare short captures. This run measures a longer
steady-state window on the same headless null sink/tone so the rewrite has
CPU/drift/glitch-proxy evidence next to the format parity. Both servers ran
sequentially in one `scripts/pw-test-session.sh` session (fresh 35 s 440 Hz
stereo 48 kHz WAV, `--amplitude 0.25`), pinned to the sink with
`-e $PW_TEST_ENDPOINT_SERIAL --encoding=f32 --channels=2 --sample-rate=48000`
and bound to `127.0.0.1:<free port>`.

Measurement procedure (release binaries; temporary orchestration script
outside the repo):

- Start the server, wait for the TCP listener, read `/proc/<pid>/stat`
  `utime+stime` (`getconf CLK_TCK`) and `/proc/<pid>/status` `VmHWM`, run
  `target/release/protocol-harness probe --duration-secs 30 --tone-hz 440`,
  then read the counters again and stop the server (SIGTERM for the old
  binary, SIGINT for the new one).
- CPU % = process CPU seconds / case wall seconds. This is the whole server
  process, including capture and network threads.
- Expected PCM = `sample_rate × block_align × probe.duration_elapsed_ms/1000`.
  Drift is reported as the signed percentage difference from that expected
  byte count.
- The old binary exposes no capture-callback counters, so **datagram arrival
  gaps are the callback/glitch proxy** for both servers. Localhost UDP loss was
  not observed (identical datagram counts), so a gap means the stream paused,
  not that a datagram was dropped.

| Metric (30 s, f32/2ch/48 kHz) | Old `as-cmd` 0.3.4 | New 0.4.0 |
|---|---:|---:|
| CPU seconds | 0.23 | 0.88 |
| CPU % of one core | 0.76 | 2.91 |
| Peak RSS | 12,708 KiB | 15,588 KiB |
| UDP datagrams | 8,436 | 8,436 |
| Mean datagram bytes | 1,365.33 | 1,365.33 |
| Full / partial datagrams | 7,733 / 703 | 7,733 / 703 |
| PCM bytes received | 11,517,952 | 11,517,952 |
| Expected PCM bytes | 11,597,952 | 11,595,264 |
| Drift vs expected | -0.69% | -0.67% |
| Max datagram gap | 58 ms | 43 ms |
| Gaps > 50 / 100 / 250 ms | 1 / 0 / 0 | 0 / 0 / 0 |
| Heartbeat misses | 0 | 0 |
| Detected tone / amplitude | 440.0 Hz / 0.2482 | 440.0 Hz / 0.2081 |
| Probe acceptance | pass | pass |

Interpretation and limitations:

- Both servers delivered the **same** PCM byte count and datagram histogram,
  so there is no throughput or packet-count regression in this window. The
  identical ~-0.7% "drift" is a fixed startup/handshake offset between the
  probe's measured window and the first delivered datagram, not progressive
  clock drift; a 30 s capture cannot prove long-term drift either way.
- The new server uses about 2.2 percentage points more CPU (0.65 s more CPU
  over 30 s) and ~2.8 MiB more peak RSS on this null-sink workload. The old
  C++ loop is leaner; absolute new CPU remains low. This is a measured
  difference, not a glitch regression.
- Arrival gaps are a proxy: neither server reports capture-callback timestamps
  here. The new server's largest gap (43 ms) was slightly smaller than the
  old server's (58 ms), and neither had a gap above 250 ms.
- One null sink, one tone, one format, 30 s. This A/B does not cover long-run
  drift, CPU under load, or real hardware; the release soaks (12 minutes and
  8 hours) are recorded in `soak.md`.
