# WASAPI loopback spike (ticket 02)

Status: implemented and compile-validated; **runtime measurements were NOT
executed**. This host is Ubuntu 24.04 (Linux); WASAPI has no runtime here.

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

Windows runtime matrix (NOT EXECUTED on this Linux host):

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

## Measured results

**NOT EXECUTED.** No row below was run: this host has no Windows runtime and no
Windows linker. No numbers are claimed or inferred.

| Check | Command | Result |
|---|---|---|
| Render endpoint enumeration + default marking | `list` | NOT EXECUTED (Windows host required) |
| Default endpoint loopback to raw PCM | `capture --seconds 60 --out ...` | NOT EXECUTED |
| f32 / s8 / s16 / s24 / s32 requests | `capture --encoding ...` | NOT EXECUTED |
| Closest-match fallback (44.1 kHz on 48 kHz, mono on stereo) | `capture --sample-rate 44100`, `--channels 1` | NOT EXECUTED |
| Unsupported request error readability | invalid `--endpoint`/format | NOT EXECUTED |
| 1-second buffer frames/ms | `capture` header `buffer:` line | NOT EXECUTED |
| Packet/poll interval min/avg/max/p99 | `measure --seconds 600` | NOT EXECUTED |
| CPU time and one-core percentage | `measure` | NOT EXECUTED |
| Silence/discontinuity/timestamp-error/index-gap counts | `measure` | NOT EXECUTED |
| 10 start/stop cycles, failures and handles | `measure`, `start-stop --iterations 200` | NOT EXECUTED |

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
cutover. Also treat packed-s24 negotiation or non-mix sample representations as
unverified until the tone matrix is run.

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

See `docs/server-rewrite/capture-windows.md` for the behavior contract,
validation results, and the manual Windows checklist. Everything in the
"Measured results" and "Residual Windows-only steps" sections above is still
**NOT EXECUTED**; the production backend is compile-checked and covered by
Linux pure-logic tests only.

## Residual Windows-only steps

- Execute the full measurement matrix on Windows 10+ with at least one shared
  and one exclusive-capable render endpoint.
- Verify each requested encoding actually delivers the requested sample
  representation (the `Initialize` docs suggest loopback may use the device
  format); record `S_OK` vs `S_FALSE` + closest match.
- Verify packed s24 acceptance and fallback behavior.
- Run the 10-minute steady-state and A/B against old `as-cmd` on the same host,
  tone, and buffer; fill the measured-results table with real values.
- Run `start-stop --iterations 200` and confirm no crash, no
  `AUDCLNT_E_ALREADY_INITIALIZED`, and stable handle count.
- Event-driven loopback was deliberately not implemented: the old server polls
  and event support requires Windows 10 1703+. Compare only if polling fails.
