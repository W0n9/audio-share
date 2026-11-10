# WASAPI loopback spike (ticket 02)

Status: implemented and **runtime-validated on Windows 11 on 2026-11-10**. The
measurement matrix below was executed on a native Windows host with a Realtek
render endpoint and a 440 Hz WebView2 `AudioContext` tone source. The
original Linux-host status (compile-only, NOT EXECUTED) is retained at the end
of this page for traceability.

## Purpose

Decide whether `wasapi = "0.27.0"` can back the production Windows capture
backend (ticket 05) before writing it, and produce a small standalone tool to
measure shared-mode loopback behavior on a real Windows host:

- enumerate active render endpoints (IMMDevice ID + friendly name, default
  marked);
- capture the default or selected endpoint to raw PCM in shared-mode loopback
  with a 1-second buffer, polling at the device period;
- request `default`, `f32`, `s8`, `s16`, `s24`, `s32`, and fall back to the
  closest match when `IsFormatSupported` returns `S_FALSE`;
- report packet/poll interval min/avg/max/p99, CPU time, silence,
  discontinuity, timestamp-error and index-gap counts, frames, and bytes;
- churn start/stop with a 1-second buffer and report failures and handle count.

The tool lives at `tools/wasapi-spike` and is a workspace member. The pure
format/statistics logic is platform-neutral and unit-tested on Linux; the
Windows capture path is behind `#[cfg(windows)]`. On non-Windows hosts the
binary prints `wasapi-spike: Windows only` and exits 1, so workspace builds
work everywhere.

## Commands

Linux stub and pure tests:

```bash
cargo build -p wasapi-spike
cargo test -p wasapi-spike          # 26 pure-logic tests
./target/debug/wasapi-spike list    # stdout/stderr: "Windows only"; exit 1
```

Supported cross-target validation (no Windows linker required):

```bash
cargo check  --target x86_64-pc-windows-msvc -p wasapi-spike
cargo clippy --target x86_64-pc-windows-msvc -p wasapi-spike --all-targets -- -D warnings
```

Windows runtime matrix (executed on Windows 11 on 2026-11-10; measured results
are below; the command list is retained as the reproduction procedure):

```text
wasapi-spike list
wasapi-spike capture --endpoint default --encoding default \
    --channels 0 --sample-rate 0 --seconds 60 --out cap-default.pcm
wasapi-spike capture --encoding f32|s8|s16|s24|s32 ...
wasapi-spike capture --sample-rate 44100 ...        # closest-match fallback
wasapi-spike capture --channels 1 ...               # closest-match fallback
wasapi-spike capture --endpoint <bogus-id> ...      # readable error, exit 1
wasapi-spike measure --seconds 600 --cycles 10
wasapi-spike start-stop --iterations 200 --milliseconds 100
```

`capture` prints the mix format, requested format, `S_OK`/`S_FALSE` support
result, negotiated `AudioFormat`, `block_align`, actual `GetBufferSize()`
frames and milliseconds, and device periods before capturing. `measure`
additionally prints interval summaries, CPU as a percentage of one core, and
glitch counters, then runs start/stop cycles. `start-stop` reports iteration
failures and process handle counts before/after.

## API findings (verified in wasapi 0.27.0 source; compile-checked here)

- Loopback is expressible: activate `IAudioClient` from a render `IMMDevice`,
  then `initialize_client(&format, Direction::Capture,
  StreamMode::PollingShared { autoconvert: false, buffer_duration_hns:
  10_000_000 }, None)`. The crate sets `AUDCLNT_STREAMFLAGS_LOOPBACK` only for
  `(Render, Capture, Shared)`. There is no public flag parameter.
- A capture endpoint passed with `Direction::Capture` would silently create a
  normal microphone stream. The tool validates `device.get_direction() ==
  Render` (the crate queries `IMMEndpoint::GetDataFlow`, so this works for
  `get_device(id)` too) and refuses otherwise.
- `IAudioClient::Initialize` is called with period 0 internally in shared mode;
  the 1-second request (`10_000_000` hns) is a minimum. `GetBufferSize()` is
  logged rather than assumed.
- `AudioClient::is_supported(&fmt, Shared)` returns `Ok(None)` for `S_OK`,
  `Ok(Some(closest))` for `S_FALSE` + closest match, and `Err` for
  `AUDCLNT_E_UNSUPPORTED_FORMAT`. No raw HRESULT handling is needed. The
  advertised `AudioFormat` is always built from the negotiated format, never
  from the requested one.
- `WaveFormat::new(24, 24, Int, ..)` is packed 24-bit (`nBlockAlign = 3 *
  channels`). A 24-valid-bit sample in a 32-bit container maps to `PCM_32BIT`.
  The tool rejects any negotiated `nBlockAlign` that disagrees with the mapped
  encoding instead of advertising a misaligned frame size.
- `read_from_device` performs `GetBuffer`/`ReleaseBuffer` and zero-fills silent
  packets. `BufferInfo` exposes `silent`, `data_discontinuity`,
  `timestamp_error`, the device index, and the QPC timestamp.
- Fresh enumerator/device/client per session (as in the old C++ server) avoids
  `AUDCLNT_E_ALREADY_INITIALIZED` and stale pre-stop audio.
- `windows = "0.62.2"` is a direct dependency (pinned to the version wasapi
  re-exports) for `AUDCLNT_E_UNSUPPORTED_FORMAT`, `GetProcessTimes`, and
  `GetProcessHandleCount`. Do not bump the windows minor independently.

## Validation performed on this host

| Command | Result |
|---|---|
| `cargo build -p wasapi-spike` (Linux) | pass; binary is the stub |
| `cargo test -p wasapi-spike` | 26 passed (encoding map, block_align, requested format, closest-match, advertised-format consistency, stats, percentiles, CPU) |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo check --target x86_64-pc-windows-msvc -p wasapi-spike` | pass |
| `cargo clippy --target x86_64-pc-windows-msvc -p wasapi-spike --all-targets -- -D warnings` | pass |
| `cargo check --workspace --all-targets --target x86_64-pc-windows-msvc` | pass |

## Measured results (Windows 11, 2026-11-10)

Host: Windows 11 Enterprise 10.0.26300, x64; Rust 1.99.1 (MSVC); wasapi
0.27.0; Realtek(R) Audio render endpoint
`{0.0.0.00000000}.{e235fde3-4eb3-49cd-b203-765b5468d5d3}`; WebView2
157.0.4341.42. The tone source was a 440 Hz `AudioContext` oscillator running
in the GUI WebView2 page (the Realtek endpoint was muted; loopback still
captured the pre-mute stream). `wasapi-spike` release build; PCM analysed with
Goertzel/RMS (`analyze-pcm.mjs`). Raw captures are under
`audio-share-artifacts/spike/ticket02/`.

| Check | Command | Result |
|---|---|---|
| Render endpoint enumeration + default marking | `list` | PASS — one active render endpoint at run time, marked `*`; an earlier run also showed the display-audio endpoint, which later reported `DeviceState=4` (not present) |
| Default endpoint loopback to raw PCM | `capture --seconds 5 --out default.pcm` | PASS — 240,000 frames / 1,920,000 bytes; f32 2 ch 48 kHz block_align 8; 440 Hz amplitude 0.13758 (−17.23 dB); RMS −20.24 dBFS; both channels equal |
| f32 / s8 / s16 / s24 / s32 requests | `capture --encoding ... --seconds 5` | PASS — all `S_OK`; 1,920,000 / 480,000 / 961,920 / 1,440,000 / 1,920,000 bytes; every capture block-aligned; tone ≈0.13758; RMS ≈−20.24 dBFS; no timestamp errors; one start discontinuity |
| Closest-match fallback (44.1 kHz on 48 kHz) | `capture --encoding s16 --sample-rate 44100` | PASS — `S_FALSE (closest match used)`; negotiated f32 2 ch 48 kHz block_align 8; capture ran and was aligned |
| Mono at the mix rate | `capture --channels 1` | LIMITATION — `IsFormatSupported` returned `S_OK`, but `Initialize` returned `AUDCLNT_E_UNSUPPORTED_FORMAT` (0x88890008). After the error-mapping fix the tool exits 1 with a readable message. The old v0.3.4 `as-cmd --channels 1` also cannot capture on this endpoint (it hangs after “server started”; the client receives a zero-length format). This is a driver/endpoint limitation, not a regression |
| Unsupported request error readability | invalid `--endpoint` | PASS — exit 1, `open endpoint {bogus}: Windows returned an error: 参数错误。 (0x80070057)` (OS-localized) |
| 1-second buffer frames/ms | `capture` header `buffer:` line | PASS — 48,000 frames (1000.0 ms at 48 kHz); negotiated loopback packets up to 480 frames (10 ms) |
| Packet/poll interval min/avg/max/p99 | `measure --seconds 600` | PASS — packet 0.001 / 10.000 / 27.611 / 13.206 ms (n=59,997); poll 3.010 / 3.393 / 20.347 / 3.933 ms (n=176,850) |
| CPU time and one-core percentage | `measure` | PASS — 4.312 s CPU over 600.034 s wall = 0.72 % of one core |
| Silence/discontinuity/timestamp-error/index-gap counts | `measure` | PASS — silent=0, data_discontinuity=4, timestamp_error=0, index gaps 1200 frames across 3 packets; 36 polls slower than 2× min period; effective rate 47,998.4 frames/s. The 3 index-gap events are 0.3/min, below the 1 packet/min NO-GO threshold; the discontinuities coincided with hiding/showing the GUI window during the run |
| 10 start/stop cycles, failures and handles | `measure --cycles 10` | PASS — 10/10 cycles, process survived; handles 162 before / 162 after |
| 200 start/stop cycles | `start-stop --iterations 200` | PASS — 200/200 succeeded, 0 failed, process survived; handles 107 → 164 on two independent runs (one-time caches, identical both times; the 600 s run showed 162/162, so there is no per-cycle growth) |

Endpoint switching while stopped/running was **not executed** because only one
render endpoint was active during this session; the display-audio endpoint was
present in an earlier enumeration and later reported not-present, but the
default capture endpoint never changed. The `Initialize` error mapping was
added after the first mono run (see the production implementation note below).

Runtime procedure and acceptance thresholds are in the exploration notes
(`server-rewrite/wasapi.md` §7.4–7.5, local notes repo): run the encoding
matrix with a continuous known tone, compare negotiated formats to the old
`as-cmd`, check for frame gaps over a 10-minute steady run, and treat an
unexplained loss of >= 1 packet/minute as NO-GO pending root cause.

## Recommendation

**GO with `wasapi = "0.27.0"`** for the production Windows backend, pending the
runtime matrix above.

Rationale (API coverage, not runtime measurements): the crate expresses
polling shared-mode loopback, the 1-second buffer, closest-match negotiation,
silent-packet zero filling, packet flags/device position, device invalidation
errors, and device enumeration/format APIs required by ticket 05. The WASAPI
capture calls themselves require no unsafe code (the only `unsafe` blocks are
for process CPU/handle metrics). The `windows-rs` fallback remains viable
and was compile-checked in the exploration notes, but requires roughly 2–3x the
unsafe/boilerplate code and is not implemented here.

Switch to the `windows-rs` fallback if a Windows run shows: the crate cannot
express the required loopback/format combination; a reproducible bug in
`is_supported` or `read_from_device`; or the crate becomes unmaintained before
cutover. The executed matrix above showed none of these; packed s24 delivered
the requested representation and every requested encoding was accepted.

## Production implementation (ticket 05)

The spike logic was lifted into Server Core, and the spike crate remains
unchanged as a standalone measurement tool:

- `crates/audio-share-core/src/capture/windows/backend.rs`:
  `WasapiCapture: CaptureBackend` (`#[cfg(windows)]`), with per-operation MTA
  threads, shared-mode loopback using `StreamMode::PollingShared {
  autoconvert: false, buffer_duration_hns: 10_000_000 }`, render-direction
  validation, closest-match negotiation, and a `Start()`-before-format-
  publication handshake.
- `crates/audio-share-core/src/capture/windows/format.rs`: platform-neutral
  encoding/block-align/request/closest-match/advertised-format rules, unit-tested
  on Linux.
- `capture::platform_backend()` selects WASAPI on Windows and
  `SyntheticCapture` elsewhere; `as-cmd` uses it by default and keeps
  `--fake-capture`.
- `scripts/ab-compare-windows.ps1` drives `protocol-harness record` against the
  old and new servers and compares format bytes, PCM levels, packet sizes, and
  optional `wasapi-spike measure` counters.

The Windows run exposed one unreadable path: `IsFormatSupported` can return
`S_OK` for a format that shared-mode loopback rejects at `Initialize` (mono at
the mix rate produced raw `0x88890008`). Both the production backend and the
spike now map that HRESULT to a readable “endpoint rejected the negotiated
loopback format … try another encoding, channel count, or sample rate” error
(unit-tested in `backend.rs`).

See `docs/server-rewrite/capture-windows.md` for the behavior contract,
validation results, and the manual Windows checklist.

## Residual Windows-only steps

Executed on 2026-11-10 (Windows 11, Realtek): the full encoding matrix, the
10-minute steady-state, the 200-cycle start/stop run, and the A/B against the
old `as-cmd` (see `capture-windows.md`). Remaining work:

- Windows 10 was not available; only Windows 11 Enterprise 10.0.26300 was
  executed.
- No exclusive-capable render endpoint was available, so only shared-mode
  loopback was measured.
- Endpoint switching was not executed because only one render endpoint was
  active in this session.
- Event-driven loopback was deliberately not implemented: the old server polls
  and event support requires Windows 10 1703+. Compare only if polling fails.
