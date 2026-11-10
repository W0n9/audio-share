# Linux capture (PipeWire)

Status: implemented by ticket 06. Verified headlessly against PipeWire 1.0.5 +
WirePlumber on Ubuntu 24.04; ticket 15 added the full Wire Protocol matrix, a
12-minute release soak, and an 8-hour release soak (`acceptance-matrix.md`,
`soak.md`). Real hardware and session-manager variants remain for later
acceptance.

`audio-share-core::capture::linux::PipeWireCapture` implements the core
`CaptureBackend` trait with the safe `pipewire` crate (0.10.1, feature
`v1_0_0`). `capture::platform_backend()` selects it on Linux; `as-cmd` uses that
backend by default and keeps `--fake-capture` for the deterministic synthetic
source used by tests.

## Endpoints and default selection

- Enumerates registry nodes with `media.class = "Audio/Sink"`.
- Endpoint `id` is `object.serial` (the Wire Protocol endpoint id and the
  `target.object` value), **not** the registry/global object id.
- Display name is `node.nick`, else `node.description`, else `node.name`, else
  the serial.
- Default endpoint: the sink with the highest numeric `priority.session`
  strictly greater than `-1`; the first sink wins ties. If no sink exceeds
  `-1`, there is no default and defaulted start fails with a readable error.
  This replicates v0.3.4 rather than "pick the highest even if negative".
- A requested id is validated against the live registry before capture starts.
  PipeWire silently falls back to another node for unknown `target.object`
  values, so `start` returns `InvalidConfig` for an unknown serial instead of
  capturing the wrong device.

## Capture session

- One dedicated PipeWire thread per session creates and drops every
  `MainLoop`/`Context`/`Core`/`Registry`/`Stream` object. The thread first does
  a registry sync round-trip so target validation and stream creation share one
  connection, then runs the stream until stopped.
- Stream properties: `media.type=Audio`, `media.category=Capture`,
  `media.role=Music`, `target.object=<object.serial>`,
  `stream.capture.sink=true`, `AUTOCONNECT | MAP_BUFFERS | RT_PROCESS`.
- `start` waits up to 5 s for format negotiation and returns `CaptureSession`
  only after the actual rate/channels/format are known, so `GET_FORMAT` never
  races. A timeout explains that a session manager (WirePlumber) or the target
  sink is missing.
- Stop: `CaptureSession::stop` sets the core stop signal; a watcher thread sees
  it and sends a `pipewire::channel` stop message; the loop thread disconnects
  the stream, quits the main loop, drops all PipeWire objects, and joins the
  forwarder and watchdog. Repeated start/stop creates fresh threads and is
  idempotent; stop during connect also works.
- Realtime safety: the `process` callback only reads the buffer, stores
  atomics, and copies PCM into a lock-free SPSC ring (`ringbuf`, 4 MiB). A
  forwarder thread drains the ring with `try_send` on the bounded core
  `FrameSink`; a full sink drops whole aligned chunks and accounts the bytes,
  and a closed sink ends the session. The RT callback never locks, allocates,
  blocks, or logs.
- Target loss: a registry `global_remove` watch ends the session as soon as the
  target node disappears. Because `StreamState` does not report this, a
  watchdog also ends the session when callbacks stop for 5 s after audio had
  been flowing. Both paths drop the frame sender so the server sees the capture
  source end; the reason is reported on stderr by the backend.

## Encoding mapping

| `EncodingPreference` | Requested SPA format | Protocol `Encoding` | Bytes/sample |
|---|---|---|---|
| `Default` | `F32LE` | `PcmFloat` | 4 |
| `F32` | `F32LE` | `PcmFloat` | 4 |
| `S8` | `U8` | `Pcm8Bit` | 1 (unsigned) |
| `S16` | `S16LE` | `Pcm16Bit` | 2 |
| `S24` | `S24LE` | `Pcm24Bit` | 3 (packed) |
| `S32` | `S32LE` | `Pcm32Bit` | 4 |

`S24LE` is packed 24-bit, not the 32-bit `S24_32LE` container; `S24_32LE` is
rejected. `block_align` is derived from the **negotiated** format and channels
(`bytes_per_sample × channels`), not from the requested encoding. Requested
channels/rate are hints passed to PipeWire (`0` = let the graph decide);
PipeWire may resample/mix, and the negotiated values are what the Wire Protocol
advertises. There is no application-level resampling.

## Headless verification recipe

`scripts/pw-test-session.sh` starts a disposable PipeWire + WirePlumber session
with a `TestSink` null sink and optionally loops a tone into it. The ignored
integration test drives the same test binary inside that session:

```bash
# 440 Hz stereo 48 kHz s16 WAV
python3 scripts/gen-tone-wav.py /tmp/tone.wav --duration 30 --amplitude 0.25

cargo test -p audio-share-core --test capture_linux -- --ignored --nocapture
```

The test covers:

- `TestSink` enumeration by display name and `object.serial` id, default
  selection by `priority.session`, and invalid-target rejection;
- default/F32, s8 (U8), s16, packed s24, and s32 monitor capture with a 440 Hz
  Goertzel tone check, block alignment, and non-empty PCM;
- 10 start/stop cycles, including stops immediately after start and while
  streaming, plus switching between two sinks and back;
- destroying the target `Audio/Sink` mid-stream and asserting the session
  ends within the global_remove/watchdog window.

CI without a PipeWire daemon still passes because the test is `#[ignore]`d; it
also skips with a message when `pipewire`/`dbus-run-session` are absent.

Manual equivalent with `as-cmd` and the protocol harness:

```bash
cargo build -p as-cmd -p protocol-harness
python3 scripts/gen-tone-wav.py /tmp/tone.wav --duration 30 --amplitude 0.25
scripts/pw-test-session.sh --tone-wav /tmp/tone.wav -- \
  target/debug/protocol-harness record \
  --server-cmd target/debug/as-cmd \
  --server-arg='--bind=127.0.0.1:{port}' \
  --out /tmp/capture-new --heartbeats 1 --datagrams 700 --timeout-secs 25
```

`record` reports `format: Some(PcmFloat) channels=2 sample_rate=48000
block_align=8`, and its UDP segments stay within the 1464-byte aligned MTU
payload.

## Caveats

- Only the `support.null-audio-sink` virtual sink was exercised here; real ALSA
  and USB sinks, suspend/resume, and hotplug variants are not covered.
- The captures recorded on this page run for seconds; long-duration behavior is
  covered by the release soaks in `soak.md`.
- A sink whose graph is idle may never call `process`; the watchdog treats a
  stream that never produced a callback as idle rather than lost. Target loss
  is still detected by `global_remove`.
- The old `as-cmd` 0.3.4 s8 path is broken (it requests U8 but only maps signed
  S8, then exits); the rewrite maps negotiated U8 to `Pcm8Bit`. See
  `ab-linux.md`.
