# Wire Protocol harness

Status: implemented by ticket 01. The harness is the automated integration seam
for the Rust Server rewrite: it drives the frozen v0.3.4 Wire Protocol, records
golden fixtures from the old `as-cmd`, and lets the new Server Core be compared
against that baseline without an Android device or audio hardware.

## Architecture

```
crates/audio-share-core
  protocol::format    hand-written prost AudioFormat + Encoding (no protoc)
  protocol::framing   Command, GET_FORMAT / START_PLAY / HEARTBEAT / UDP bind
  protocol::mtu       1492 - 20 - 8 = 1464, aligned down to block_align

tools/protocol-harness
  record       launch an external server, capture control + UDP into fixtures
  replay       validate fixtures offline, or drive a live server
  serve-fake   v0.3.4-compatible fake server with synthetic PCM
  synthesize   write the Windows v0.3.4-semantics fixture sets
  probe        drive a live server and report format/framing/heartbeats/tone
  fixture      manifest schema, deterministic writer/reader, path checks
  process      free-port choice, {host}/{port}/{bind} expansion, child cleanup
```

`audio-share-core` owns all Wire Protocol encoding, so `as-cmd`, the Server GUI,
and the harness share one implementation. `protocol-harness` is a binary crate;
it exists for verification, not as a Server Core API.

### Frozen v0.3.4 observations encoded by the harness

- Control integers are little-endian `u32`; PCM is little-endian.
- `GET_FORMAT = 1`, `START_PLAY = 2`, `HEARTBEAT = 3`.
- `GET_FORMAT` reply: `cmd u32 LE`, `size u32 LE`, protobuf `AudioFormat`.
- `START_PLAY` reply: `cmd u32 LE`, `id i32 LE`; the Android Client binds UDP by
  sending that `id` as `i32 LE`.
- The Server sends `HEARTBEAT` every 3 seconds; a session with no client
  heartbeat for 5 seconds is closed.
- UDP segmentation: `1492 - 20 - 8 = 1464` bytes, then
  `max -= max % block_align`. A `block_align` of 0 or greater than 1464 produces
  no segments instead of panicking or emitting an oversized datagram.
- `AudioFormat` has only `encoding`, `channels`, and `sample_rate`. Encoding
  values are `INVALID=0`, `FLOAT=1`, `8BIT=2`, `16BIT=3`, `24BIT=4`, `32BIT=5`.

## Fixture schema

Committed fixtures live under `fixtures/protocol/v0.3.4/`; see
`fixtures/protocol/v0.3.4/README.md` for the file layout, manifest fields, and
provenance. Every manifest is deterministic JSON: no timestamps, no absolute
paths, and no run-specific identifiers such as the chosen port. Recorded PCM is
a snapshot of real capture, so its bytes change when re-recorded; the framing
and segment structure do not.

## Commands

Build first:

```bash
cargo build -p protocol-harness
```

### Replay committed fixtures (canonical parity entry point)

This is the one documented command that replays the committed fixtures:

```bash
cargo run -p protocol-harness -- replay --fixture fixtures/protocol/v0.3.4/linux/f32-48k-stereo
```

Without `--connect` or `--server-cmd`, replay is offline: it verifies the
manifest, control hashes, protobuf bytes, heartbeat bytes, UDP bind id, MTU
values, and that PCM segments rebuild the stream and stay block-aligned.

### Drive a live server

```bash
# Already running server
cargo run -p protocol-harness -- replay \
  --fixture fixtures/protocol/v0.3.4/linux/f32-48k-stereo \
  --connect 127.0.0.1:65530

# Or launch a server; {host}, {port}, {bind} are substituted
cargo run -p protocol-harness -- replay \
  --fixture fixtures/protocol/v0.3.4/linux/f32-48k-stereo \
  --server-cmd /path/to/as-cmd \
  --server-arg=--bind=127.0.0.1:{port} \
  --server-arg=--encoding=f32 \
  --server-arg=--channels=2 \
  --server-arg=--sample-rate=48000
```

Live replay requires GET_FORMAT bytes to match the fixture exactly, START_PLAY to
return a positive id, heartbeats to be the frozen command, and every UDP
datagram to be at most the aligned MTU payload and a multiple of `block_align`.
PCM content is not byte-compared: the live Endpoint/tone may differ from the
recording. The command exits non-zero on any mismatch and reports
`format_bytes_match=true` on success.

### Record from the old `as-cmd`

```bash
AS_CMD_034=/path/to/as-cmd-0.3.4-linux scripts/record-fixtures.sh
```

The script uses `scripts/pw-test-session.sh`, which creates a temporary
`XDG_RUNTIME_DIR`, starts PipeWire and WirePlumber, creates a `TestSink`
`Audio/Sink` null sink, and loops a generated sine WAV (`scripts/gen-tone-wav.py`)
into it before recording. It exports `PW_TEST_ENDPOINT_SERIAL`, which the record
command passes to `as-cmd -e` so endpoint selection does not depend on the
host's default-sink priority. All child processes and the temp directory are
cleaned up on exit and on error; no sudo is used.

`record` itself enforces a total timeout, responds to server heartbeats, kills
the launched server on every exit path, and validates each captured datagram
against the advertised `AudioFormat` before writing the fixture set.

### Probe a live server (acceptance/soak evidence)

```bash
cargo run -p protocol-harness -- probe \
  --connect 127.0.0.1:65530 --duration-secs 4 --tone-hz 440 \
  --json-out /tmp/probe.json
```

`probe` performs the v0.3.4 client flow (`GET_FORMAT`, `START_PLAY`, UDP id
bind), replies to heartbeats, and prints one JSON report. It records the
format the server actually negotiated (never the requested one), datagram
counts and min/max/mean sizes, `block_align` and 1464-byte MTU compliance,
heartbeat cadence and misses, PCM byte counts, datagram-arrival gap buckets as
a glitch proxy, and a Goertzel scan for the 440 Hz tone (300-600 Hz, with a
1000 Hz control). Useful flags:

- `--datagrams N` stops a single session after N datagrams (`0` = duration).
- `--reconnect-every-secs N` disconnects/reconnects inside one probe run and
  reports fresh `START_PLAY` ids.
- `--no-heartbeat-reply --expect-server-close` observes the ~6 s timeout and
  the TCP close delay.
- `--ready-file PATH` creates a handshake file after START_PLAY/UDP bind.
- `--tone-hz 0` disables tone analysis (silence/hotplug checks).

`scripts/acceptance-matrix.sh` uses this command for the encoding/channel/rate
matrix, multi-client, reconnect, heartbeat, server-stop, and A/B runs;
`scripts/soak.sh` uses it for long-running clients. Both scripts write raw
reports plus TSV/JSON summaries under their output directory.

### Fake v0.3.4 Server

```bash
cargo run -p protocol-harness -- serve-fake \
  --bind 127.0.0.1:65530 \
  --fixture fixtures/protocol/v0.3.4/linux/f32-48k-stereo
```

The fake Server implements `GET_FORMAT`, `START_PLAY`, 3-second heartbeats with
a 5-second timeout, UDP id binding, and MTU segmentation, with deterministic
synthetic PCM. `--duration-secs` bounds its lifetime; test-only overrides for
heartbeat/timeout/chunk intervals keep the smoke test fast. Without `--fixture`
it advertises `--encoding`, `--channels`, and `--sample-rate` (default f32, 2,
48000).

### Synthesize Windows fixtures

```bash
cargo run -p protocol-harness -- synthesize --out fixtures/protocol/v0.3.4/windows
```

This writes `f32-48k-stereo`, `s16-44k1-stereo`, `s16-48k-mono`, and
`s24-48k-stereo` with provenance `synthesized-v0.3.4-semantics`. They are not
recorded on Windows and must never be described as such.

## CI smoke path

`.github/workflows/server-rewrite.yml` runs on `ubuntu-24.04` and
`windows-latest`:

1. `cargo fmt --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace` (includes the core protocol and fixture tests)
4. `cargo test -p protocol-harness --test harness_smoke` — launches
   `serve-fake` with a synthesized fixture, waits for TCP, and runs live
   `replay --connect`; no audio devices are required.
5. Offline replay of the committed Linux fixture.

The Ubuntu job installs `libpipewire-0.3-dev`, `libclang-dev` (the safe
`pipewire` crate builds its `pipewire-sys` bindings with bindgen), and
`pkg-config`; the Windows job never touches an audio device.

## Comparing a new Server with the old `as-cmd`

On one machine and one audio source:

1. Record or reuse a fixture whose `AudioFormat` matches the Capture Format
   under test.
2. Run live replay against the old `as-cmd` with `--server-cmd` and confirm
   `format_bytes_match=true`.
3. Run the same command against the new Server executable. The same fixture
   must pass: identical GET_FORMAT bytes, positive START_PLAY id, 3-second
   heartbeat cadence, 5-second timeout, and MTU-aligned UDP datagrams.
4. Record a new fixture from each Server and compare manifests structurally;
   PCM content is expected to differ for live audio.

The released v0.3.4 Android Client remains the final acceptance client; this
harness narrows regressions before that manual gate.
