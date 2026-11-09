# Windows real capture path (ticket 05)

Status: **implemented and compile/logic validated on the Linux development
host.** Windows runtime behaviour, the endpoint/encoding matrix, start/stop
churn, and the A/B against the old `as-cmd` were **NOT EXECUTED** here: this
host has no Windows audio runtime and no Windows linker.

## What landed

| Piece | Location |
|---|---|
| `WasapiCapture: CaptureBackend` (`#[cfg(windows)]`) | `crates/audio-share-core/src/capture/windows/backend.rs` |
| Pure format/negotiation rules (all hosts) | `crates/audio-share-core/src/capture/windows/format.rs` |
| Platform selection | `crates/audio-share-core/src/capture/mod.rs` (`platform_backend`) |
| CLI default + escape hatch | `crates/as-cmd/src/main.rs` (`--fake-capture`) |
| Target-gated dependencies | `crates/audio-share-core/Cargo.toml` (`wasapi 0.27.0`, `windows 0.62.2`) |
| A/B script | `scripts/ab-compare-windows.ps1` |

The `windows` module is always compiled so the pure logic is unit-tested on
Linux; only `backend.rs` and the `WasapiCapture` re-export are
`#[cfg(windows)]`.

## Threading and lifecycle

- `WasapiCapture` is a stateless unit struct. It stores no COM handles, so it
  satisfies `CaptureBackend: Send + Sync + 'static`.
- `endpoints()` / `default_endpoint()` run on a short-lived thread that calls
  `CoInitializeEx(COINIT_MULTITHREADED)`, does the enumeration, sends owned
  strings back, joins, and calls `CoUninitialize`.
- `start()` spawns the capture thread. That thread opens a fresh enumerator,
  render `Device`, `AudioClient`, and `AudioCaptureClient` (the old C++ server's
  per-session construction), negotiates the format, calls
  `IAudioClient::Initialize`, `GetBufferSize`, `GetService`, then `Start()`.
  Only after `Start()` succeeds does it publish the actual `AudioFormat` to the
  waiting `start()` call. The returned `CaptureSession` therefore never
  advertises a format that failed to start.
- The capture loop polls at the device minimum period, drains all packets
  (`GetNextPacketSize` until 0), and forwards each packet to the bounded
  `FrameSink`. `read_from_device` releases the packet and zero-fills silent
  packets. `data_discontinuity` packets are still forwarded (the frozen Wire
  Protocol has no counter for them).
- Stop is an `AtomicBool` checked between device periods, with a condition
  variable to wake the poll wait immediately. `CaptureSession::stop` sets the
  flag and joins the thread exactly once; `Drop` repeats the same path and is a
  no-op after an explicit stop.
- Reusing an `AudioClient` is deliberately avoided: a fresh client per session
  sidesteps `AUDCLNT_E_ALREADY_INITIALIZED` and up to one second of stale
  pre-stop audio after a `Stop`/`Start` resume.

## Format rules

The default capture format is the endpoint mix format (`GetMixFormat`) with no
resampling or rematrixing (`StreamMode::PollingShared { autoconvert: false,
buffer_duration_hns: 10_000_000 }`, i.e. a 1-second buffer).

| `EncodingPreference` | Requested `WAVEFORMATEXTENSIBLE` |
|---|---|
| `Default` | mix subformat, container bits, valid bits, channel mask |
| `F32` | IEEE float, 32 / 32 bits |
| `S8` | PCM, 8 / 8 bits (unsigned, as in WAVE_FORMAT_PCM) |
| `S16` | PCM, 16 / 16 bits |
| `S24` | PCM, 24 / 24 bits, **packed 3 bytes per sample** |
| `S32` | PCM, 32 / 32 bits |

- `channels == 0` keeps the mix channel count; `sample_rate == 0` keeps the mix
  rate. The mix channel mask is kept only when the channel count is unchanged;
  otherwise `None` lets the `wasapi` crate supply a default mask.
- `IAudioClient::IsFormatSupported(Shared)` is called with the request:
  `S_OK` uses the request, `S_FALSE` uses the returned closest match, and
  `AUDCLNT_E_UNSUPPORTED_FORMAT` becomes a readable `Error::Capture` before any
  `Initialize`.
- The advertised `AudioFormat` is built **only from the negotiated format**.
  Encoding maps from the container width: 32-bit float -> `PCM_FLOAT`; 8/16/24/
  32-bit integer -> `PCM_8/16/24/32BIT`. A 24-valid-bit sample in a 32-bit
  container is advertised as `PCM_32BIT`, never as packed 24-bit.
- The advertised `nBlockAlign` must equal `container_bytes * channels`
  (`3 * channels` for packed s24); an inconsistent driver/OS format is rejected
  instead of mis-advertised to the Android Client.
- Silent packets are forwarded as silence. `wasapi` zero-fills them, which is
  correct for float and signed integer PCM; for unsigned 8-bit PCM the backend
  writes `0x80` (the unsigned zero point) instead of `0x00` (full negative
  scale).
- The endpoint must be a render endpoint. A capture endpoint passed with
  `Direction::Capture` would otherwise become a silent microphone stream, so
  the backend checks `Device::get_direction() == Render` and refuses.
- Any `AUDCLNT_E_DEVICE_INVALIDATED`, `AUDCLNT_E_RESOURCES_INVALIDATED`, or
  `AUDCLNT_E_SERVICE_NOT_RUNNING` is reported as a restartable
  "endpoint invalidated or removed" error.

## Platform wiring and `as-cmd`

`capture::platform_backend()` returns `Arc<dyn CaptureBackend>`:

- Windows: `WasapiCapture`.
- Every other platform: the deterministic `SyntheticCapture` (v0.3.4 Linux
  default: f32, stereo, 48 kHz, 1024-frame callbacks every 10 ms). Ticket 06
  replaces only this non-Windows branch with the PipeWire backend.

`as-cmd` uses `platform_backend()` by default and keeps `--fake-capture` to
force the synthetic backend. Capture selection flags (`-e`, `--encoding`,
`--channels`, `--sample-rate`) remain ticket 07's CLI-parity work; the
`CaptureConfig` path behind them is implemented and unit-tested here.

## Validation performed on this host (Linux)

| Command | Result |
|---|---|
| `cargo fmt --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass, including 19 new pure-format tests |
| `cargo check --workspace --all-targets --target x86_64-pc-windows-msvc` | pass |
| `cargo clippy --target x86_64-pc-windows-msvc -p audio-share-core --all-targets -- -D warnings` | pass |

What the Linux tests cover: encoding mapping (including rejection of 64-bit
float and unknown subformats), block alignment (including packed s24 = 3 bytes
per sample), default/explicit request construction, channel-mask preservation,
closest-match selection, advertised-format construction, and rejection of
inconsistent `nBlockAlign`, zero channels, and unrepresentable sample rates.

What they cannot cover: COM, WASAPI negotiation, real device periods, packet
cadence, silent/hotplug behaviour, and actual PCM sample representation.

## Manual Windows verification (NOT EXECUTED)

1. Build on Windows with Rust 1.85 or newer (`wasapi 0.27` requirement):
   `cargo build --workspace --release`.
2. Play a continuous known tone (for example 1 kHz, 48 kHz, stereo) into the
   endpoint under test and keep it playing for all steps.
3. Start the new server against the default endpoint:
   `target\release\as-cmd.exe --bind=127.0.0.1:65530`
   (until ticket 07 lands, the capture format is the endpoint mix format).
4. Drive it with the harness and confirm the advertised format:
   `protocol-harness.exe record --server-cmd target\release\as-cmd.exe
   --server-arg=--bind={bind} --out target\win-smoke --datagrams 64`
   Then run `protocol-harness.exe replay --fixture target\win-smoke` and check
   `format_bytes_match`, a positive `start_play_id`, 3-second heartbeats, and
   MTU-aligned datagrams.
5. Use `wasapi-spike.exe list` to enumerate render endpoints and
   `wasapi-spike.exe measure --seconds 600 --cycles 10` for capture-side
   interval/CPU/glitch counters.
6. Repeat start/stop at least 100 times with `wasapi-spike.exe start-stop
   --iterations 200 --milliseconds 100` and switch endpoints between runs while
   the server is stopped and while it is running.
7. Unplug/disable the selected endpoint during capture. Expected: the capture
   thread stops with an "endpoint invalidated or removed" message; the process
   must survive. Automatic restart on a new endpoint is supervisor work that is
   not part of this ticket.
8. Verify the requested encoding really delivers that sample representation
   (the WASAPI `Initialize` documentation says loopback data may be in the
   device format). Record `S_OK` vs `S_FALSE` + closest match for f32, s8, s16,
   s24 (packed), and s32 once ticket 07 exposes encoding selection.

## A/B procedure (NOT EXECUTED)

`scripts/ab-compare-windows.ps1` records the frozen Wire Protocol from the old
v0.3.4 `as-cmd.exe` and the new server on the same machine while the same tone
plays into the same endpoint, then compares:

- exact `GET_FORMAT` response bytes and AudioFormat (encoding, channels, sample
  rate, block align);
- captured PCM level statistics (RMS/peak dBFS) within a 3 dB tolerance;
- datagram size distributions (min/max/full-size) and MTU alignment;
- optional `wasapi-spike measure` glitch counters for the new capture path.

Example:

```powershell
.\scripts\ab-compare-windows.ps1 `
    -OldServer C:\audio-share-old\as-cmd.exe `
    -NewServer .\target\release\as-cmd.exe `
    -Harness .\target\release\protocol-harness.exe `
    -SpikeExe .\target\release\wasapi-spike.exe `
    -OldServerArgs @("--encoding", "f32", "--channels", "2", "--sample-rate", "48000") `
    -NewServerArgs @()
```

Pass the same capture selection to `-NewServerArgs` once ticket 07 exposes
those flags. PCM bytes are not byte-compared because the recordings are
sequential; the script fails on a format mismatch, an MTU violation, or a level
difference outside tolerance.

## Residual risks / follow-ups

- Windows runtime and A/B are unexecuted (see the checklist above).
- Loopback may deliver the device/mix sample representation even when
  `IsFormatSupported` accepts a non-mix representation; the tone matrix in
  step 8 must confirm packed s24 and the integer encodings.
- A mid-stream invalidation stops capture and reports to stderr; there is no
  supervisor/restart loop in Server Core yet (GUI lifecycle work, ticket 10).
- `as-cmd` does not yet expose endpoint/encoding/channels/sample-rate flags;
  that is ticket 07 and is required before running the full encoding A/B.
- Event-driven loopback is intentionally not used: the old server polls, and
  event-driven loopback requires Windows 10 1703+.
