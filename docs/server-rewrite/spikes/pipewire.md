# Linux PipeWire capture spike

Status: implemented and measured by ticket 03 (2026-11-09). **Go** for ticket 06.

Tool: `tools/pipewire-spike` (workspace member, Linux-only behavior; a clear
error stub is compiled on other platforms so the workspace builds on Windows).
Headless driver: `scripts/pw-test-session.sh`.

## API decision: safe `pipewire` crate 0.10.1

Use `pipewire = "0.10.1"` with feature `v1_0_0` (the Ubuntu 24.04 baseline is
libpipewire 1.0.5). It covers every ticket-03 requirement:

- `Audio/Sink` enumeration with `object.serial` and `priority.session`.
- Monitor capture through `target.object`, `media.category=Capture`, and
  `stream.capture.sink=true`, with `AUTOCONNECT` + `MAP_BUFFERS` + `RT_PROCESS`.
- F32LE, U8 (`s8`), S16LE, packed S24LE, and S32LE negotiation.
- Exact rate/channel negotiation, read back from `param_changed`.
- Clean stop from the control thread through `pipewire::channel`.

`pipewire-sys` is **not** needed. The explorer's fallback compiled and ran, but
the safe crate meets the requirements with far less `unsafe`: no manual vtable
dispatch, listener hooks, pod building, or buffer re-queueing.

Build dependencies: `pkg-config`, `libpipewire-0.3-dev`, and `libclang-dev`
(the safe crate depends on `pipewire-sys`, whose build script runs bindgen).
CI installs all three on `ubuntu-24.04`. The Linux target dependency is declared
under `[target.'cfg(target_os = "linux")'.dependencies]`, so Windows never
builds PipeWire.

Design notes:
- Pure logic (encoding/block_align mapping, default selection, statistics,
  Goertzel tone analysis) lives in `src/lib.rs` and is unit-tested on any host.
- One dedicated PipeWire thread per capture session; objects are created and
  dropped inside it.
- PCM leaves the RT `process` callback through a lock-free SPSC ring
  (`ringbuf`); callback timestamps/sizes use a second ring. No locks,
  allocations, or I/O in `process`.
- Stop is sent with `pipewire::channel::Sender`; the loop disconnects the
  stream and quits.
- Targets are validated against the registry before connecting, because an
  invalid `target.object` silently falls back to another node.

## Commands

```bash
cargo build -p pipewire-spike

# Disposable headless session: private XDG_RUNTIME_DIR, pipewire + wireplumber,
# TestSink null sink with priority.session=1000, optional looping tone.
python3 scripts/gen-tone-wav.py /tmp/tone.wav --frequency 440 --duration 30

scripts/pw-test-session.sh --tone-wav /tmp/tone.wav -- \
  ./target/release/pipewire-spike list

scripts/pw-test-session.sh --tone-wav /tmp/tone.wav -- \
  ./target/release/pipewire-spike capture --endpoint default --encoding f32 \
    --channels 2 --sample-rate 48000 --seconds 3 --out /tmp/f32.pcm

scripts/pw-test-session.sh --tone-wav /tmp/tone.wav -- \
  ./target/release/pipewire-spike measure --endpoint default --encoding f32 \
    --channels 2 --sample-rate 48000 --seconds 10
```

The driver exports `PW_TEST_ENDPOINT_SERIAL` and `PW_TEST_SINK_NAME`, so an
explicit `--endpoint "$PW_TEST_ENDPOINT_SERIAL"` can be used inside a session.
`object.serial` is allocated dynamically; do not hard-code it across sessions.

Automated real-audio check (ignored by default; starts real daemons):

```bash
cargo test -p pipewire-spike --test headless_pipewire -- --ignored --nocapture
```

It runs list, F32LE/S16LE/S24LE captures with a 440 Hz tone, verifies the tone
with Goertzel, and runs measure with repeated start/stop in one headless
session. Last local run: **1 passed, 10.11 s**.

## Measured results (this host)

Environment: Ubuntu 24.04.5, Rust 1.98.1, libpipewire 1.0.5, wireplumber
0.4.17, `support.null-audio-sink` named TestSink (`priority.session=1000`),
440 Hz stereo tone played with `pw-play`.

### list

```
* serial=37 priority.session=1000 name=TestSink nick=- description=TestSink display="TestSink"
total: 1; default: serial=37 (TestSink)
```

The default follows the old C++ rule exactly: highest `priority.session`
strictly greater than -1, first wins ties; otherwise there is no default.

### Capture (debug build, tone verification done on the written PCM)

| request | negotiated | block_align | bytes | frames | duration | Goertzel 440 | Goertzel 1000 |
|---|---|---|---|---|---|---|---|
| f32, 48 kHz, 2 ch, 3 s | `F32LE` 48000/2 | 8 | 1,146,880 | 143,360 | 2.99 s | 0.2464 | 0.0001 |
| s16, 48 kHz, 2 ch, 2 s | `S16LE` 48000/2 | 4 | 385,024 | 96,256 | 2.01 s | 0.2446 | 0.00001 |
| s24, 48 kHz, 2 ch, 2 s | `S24LE` 48000/2 | 6 | 577,536 | 96,256 | 2.01 s | 0.2446 | 0.00004 |

S24 is packed 3 bytes/sample (frames = bytes/6), not a 32-bit container. The
captured tone amplitude matches the 0.25 source amplitude; the off-frequency
leakage is negligible, proving real monitor audio rather than silence/noise.

### measure (release build, F32LE 48 kHz stereo, 10 s + 10 cycles)

```
callbacks=234 bytes=3833856 frames=479232
callback_interval_ms: min=42.410 mean=42.666 p50=42.666 p95=42.787 max=42.892
quantum_frames: min=2048 mean=2048.0 max=2048
glitches: zero_size=0 null_buffer=0 corrupted=0 discont=0 empty=0 ring_overflow_bytes=0
cpu: user=0.040s sys=0.042s total=0.082s wall=10.030s = 0.82% CPU
cycles: 10/10 ok (0.25 s each, 6 callbacks / 98,304 bytes per cycle)
```

Debug build for comparison: 235 callbacks, 3,850,240 bytes, callback mean
42.666 ms, 0.329 s CPU over 10.025 s = 3.28%, and `discont=1` was observed once
at session startup (release run: 0). 10/10 start/stop cycles also passed in the
debug build. Max RSS was not sampled by this tool; the explorer measured
~12 MB for an equivalent debug/release spike.

## Go / No-Go for ticket 06

**Go.** The safe crate satisfies all ticket-03 requirements on real headless
PipeWire: enumeration/default selection, monitor capture from a chosen sink,
F32LE + non-F32 formats, negotiated `block_align`, measurable CPU, and
crash-free repeated start/stop.

Carry into ticket 06:
- One PipeWire thread per session, lock-free ring to tokio, `pipewire::channel`
  for stop (all verified here).
- Derive `block_align` from the negotiated format and channels; never assume a
  fixed quantum (2048 frames here, but the graph chooses it and resampling can
  produce variable chunk sizes divisible by `block_align`).
- Validate the selected target against the registry (invalid targets fall back
  silently) and watch `global_remove`/data liveness: the stream stays
  `Streaming` for at least 2.5 s after its target sink disappears.
- `s8` maps to `U8`; `s24` is packed 3-byte `S24LE`.
- Keep `pipewire-sys` documented as a fallback only.

## Caveats

- Only the `support.null-audio-sink` virtual sink was exercised; real ALSA/USB
  device monitor ports, device names, and priority values are untested.
- Measurements are from this host (debug CPU 3.28%, release 0.82%); no 8-hour
  soak, suspend/resume, or hotplug stability run is claimed here.
- The Windows stub is compile-checked with
  `cargo check/clippy --target x86_64-pc-windows-msvc`; there is no Windows
  runtime in this environment.
