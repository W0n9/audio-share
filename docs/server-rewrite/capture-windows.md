# Windows real capture path (ticket 05)

Status: **implemented and runtime-validated on Windows 11 on 2026-11-10.**
Real WASAPI loopback audio reaches the frozen wire protocol, all five
encodings A/B against the released v0.3.4 `as-cmd.exe`, and 200 start/stop
cycles plus a 10-minute steady-state run completed without a crash. The only
open hardware cases are mono capture on this Realtek endpoint (the driver
rejects it at `Initialize` with a now-readable error), endpoint switching (only
one render endpoint was active), and Windows 10. The original Linux-host
compile-only status is retained in the ticket/commit history.

## What landed

| Piece | Location |
|---|---|
| `WasapiCapture: CaptureBackend` (`#[cfg(windows)]`) | `crates/audio-share-core/src/capture/windows/backend.rs` |
| Pure format/negotiation rules (all hosts) | `crates/audio-share-core/src/capture/windows/format.rs` |
| Platform selection | `crates/audio-share-core/src/capture/mod.rs` (`platform_backend`) |
| `as-cmd` default + escape hatch | `crates/as-cmd/src/main.rs` (`--fake-capture`) |
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
`--channels`, `--sample-rate`) remain ticket 07's `as-cmd`-parity work; the
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

## Windows runtime results (2026-11-10)

Host: Windows 11 Enterprise 10.0.26300 x64; Rust 1.99.1 MSVC; Realtek(R) Audio
render endpoint `{0.0.0.00000000}.{e235fde3-4eb3-49cd-b203-765b5468d5d3}`;
WebView2 157.0.4341.42; a 440 Hz `AudioContext` oscillator supplied the tone.
Evidence: `audio-share-artifacts/ab/*`, `audio-share-artifacts/probe/`,
`audio-share-artifacts/captures/`, `audio-share-artifacts/spike/ticket02/`.

- Default endpoint: f32 2 ch 48 kHz, block_align 8; `GET_FORMAT` reply hex
  `0100000008000000080110021880f702`; real 440 Hz PCM at RMS ≈ −20.24 dBFS.
- Encoding matrix (requested → negotiated): f32/default → f32, s8 → s8,
  s16 → s16, s24 → s24 (packed, block_align 6), s32 → s32; every capture was
  block-aligned and every datagram was at most 1464 bytes. s8 callbacks were
  960-byte packets (below the MTU, so no segmentation).
- 44.1 kHz request: `S_FALSE` closest match to the f32 48 kHz mix format; the
  stream runs normally. This is the documented closest-match behavior, not a
  silent resample.
- `--channels 1`: `IsFormatSupported` returns `S_OK`, but shared-mode loopback
  `Initialize` rejects it with `AUDCLNT_E_UNSUPPORTED_FORMAT` (0x88890008).
  The backend now maps that HRESULT to a readable “endpoint rejected the
  negotiated loopback format … try another encoding, channel count, or sample
  rate” error. The released v0.3.4 `as-cmd --channels 1` also fails to capture
  on this endpoint (it hangs after starting the listener and the client sees a
  zero-length format), so this is a driver limitation, not a regression.
- GUI probe via `protocol-harness probe --connect 127.0.0.1:65531
  --duration-secs 10` (s16): 1,998 datagrams, `alignment_ok=true`,
  `max_datagram_ok=true`, 3 heartbeats at 3000 ms with 0 misses, 440 Hz tone
  detected at amplitude 0.1375, no datagram gap over 50 ms (max 16 ms).
- Repeated start/stop: `wasapi-spike start-stop --iterations 200` 200/200 and
  the 10-minute `measure --cycles 10` 10/10; the process survived and handle
  counts were stable (162/162 in the 10-minute run; 107→164 on two identical
  200-cycle runs, a one-time cache rather than per-cycle growth).
- No shared-`AudioFormat` race was observed: the format is published only after
  `Start()` succeeds and is immutable for the session; the capture thread and
  network task exchange owned buffers through the bounded frame channel.

### A/B against the released v0.3.4 `as-cmd.exe`

Same endpoint, same 440 Hz tone, 64 datagrams and 3 heartbeats per server;
`scripts/ab-compare-windows.ps1` in `audio-share-artifacts/ab/`.

| Format | `GET_FORMAT` bytes | Negotiated AudioFormat | PCM RMS old / new | Max datagram | Result |
|---|---|---|---|---|---|
| default | identical | f32 / 2 ch / 48000 | −20.30 / −20.24 dBFS | 1464 | PASS |
| f32 | identical | f32 / 2 ch / 48000 | −20.24 / −20.24 dBFS | 1464 | PASS |
| s8 | identical | s8 / 2 ch / 48000 | −20.23 / −20.23 dBFS | 960 | PASS |
| s16 | identical | s16 / 2 ch / 48000 | −20.24 / −20.24 dBFS | 1464 | PASS |
| s24 | identical | s24 / 2 ch / 48000 | −20.23 / −20.23 dBFS | 1464 | PASS |
| s32 | identical | s32 / 2 ch / 48000 | −20.24 / −20.24 dBFS | 1464 | PASS |

All six A/B cases passed: exact `GET_FORMAT` bytes, identical datagram size
distributions (min/max/full-size counts), and RMS/peak deltas ≤ 0.06 dB. PCM
bytes are not byte-compared because the recordings are sequential.

## Manual Windows verification procedure

The procedure below was executed on Windows 11 on 2026-11-10 except step 7
(unplug/disable the selected endpoint), which remains a release-gate check.
Results are in “Windows runtime results” above; the list is retained as the
reproduction procedure.

1. Build on Windows with Rust 1.90 or newer (the workspace MSRV; `wasapi 0.27`
   needs at least 1.85):
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
   s24 (packed), and s32. Ticket 07 exposes encoding selection; the matrix was
   executed on Windows 11 (see above).

## A/B procedure (executed on Windows 11)

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
    -NewServerArgs @("--encoding", "f32", "--channels", "2", "--sample-rate", "48000")
```

Pass the same capture selection to `-NewServerArgs` (ticket 07 exposes those
flags). PCM bytes are not byte-compared because the recordings are sequential;
the script fails on a format mismatch, an MTU violation, or a level difference
outside tolerance. The six executed Windows cases (default, f32, s8, s16, s24,
s32) all passed; see the results table above.

## Residual risks / follow-ups

- Windows 10, endpoint switching, and unplug/disable of the active endpoint
  remain unexecuted; only one render endpoint was active in this session.
- The tone matrix confirmed that this endpoint delivers packed s24 and the
  integer encodings as requested. Other drivers may silently return the mix
  representation; re-run the matrix per endpoint in that case.
- Mono at the mix rate is rejected by this Realtek driver at `Initialize`; the
  backend now reports a readable error. The old v0.3.4 server also cannot
  capture mono on this endpoint.
- A mid-stream invalidation stops capture and reports to stderr; there is no
  supervisor/restart loop in Server Core yet (GUI lifecycle work, ticket 10).
- Event-driven loopback is intentionally not used: the old server polls, and
  event-driven loopback requires Windows 10 1703+.
