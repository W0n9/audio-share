# 05: Windows real capture path

**What to build:** 把 Windows WASAPI loopback 接入 core，替换 synthetic source；选定 render Endpoint 后，新 Server 通过 Wire Protocol 发送真实系统音频。与旧 `as-cmd` 在同一台机器、同一音频源、同一 Capture Format 下做 A/B。

**Blocked by:** 02 (Windows WASAPI loopback spike), 04 (Wire Protocol slice with fake capture + `as-cmd -b`)

**Status:** ready-for-human

- [ ] Windows 上启动 Server 后，scripted client 收到真实系统音频。
- [ ] 默认 Capture Format 来自 Endpoint mix format；encoding、channels、sample rate 请求遵循 closest-match 回退。
- [ ] 与旧 `as-cmd` 的 `AudioFormat` 和 PCM 输出对比无明显差异。
- [ ] 反复 start/stop 与切换 Endpoint 不崩溃。
- [ ] 采集线程与网络线程共享的 `AudioFormat` 无数据竞争。

## Implementation checklist (Linux-verifiable work)

- [x] `audio-share-core::capture::windows::WasapiCapture: CaptureBackend` behind `#[cfg(windows)]`; no COM handles stored in the backend, all COM work on per-operation MTA threads.
- [x] Capture thread negotiates the format and calls `Start()` before the `CaptureSession` is returned; advertised `AudioFormat` is built from the negotiated format and rejects inconsistent `block_align`.
- [x] Requested encoding/channels/sample-rate copied from the mix format, `IsFormatSupported` closest match used, unsupported formats return a readable `Error::Capture`; packed s24 is 3 bytes per sample.
- [x] Stop is `AtomicBool` + condvar wake + single join; repeated start/stop and endpoint switches create fresh enumerator/device/client each time.
- [x] Pure format logic covered by unit tests on Linux (19 tests).
- [x] `capture::platform_backend()` added (Windows => WASAPI, other platforms => synthetic fallback); `as-cmd` uses it by default and keeps `--fake-capture`.
- [x] `cargo fmt`, Linux clippy/tests, and Windows-target check/clippy pass.
- [x] `docs/server-rewrite/capture-windows.md` and `scripts/ab-compare-windows.ps1` added.
- [ ] Windows runtime behaviour, encoding matrix, start/stop churn, and A/B with the old `as-cmd` — NOT EXECUTED (requires a Windows host).

## Comments

2026-11-09 — branch `ticket/05-windows-capture`.

Implemented the WASAPI shared-mode loopback backend in Server Core and wired it
through `capture::platform_backend()` / `as-cmd`. The ticket-02 spike crate was
left unchanged; its pure format logic was lifted and adapted into
`crates/audio-share-core/src/capture/windows/format.rs`, with the COM path in
`backend.rs`. Windows-only dependencies are target-gated (`wasapi 0.27.0`,
`windows 0.62.2`); the Linux build and tests do not require them.

Commands run on the Linux host (all pass):

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace` (19 new pure-format tests; every existing suite green)
- `cargo check --workspace --all-targets --target x86_64-pc-windows-msvc`
- `cargo clippy --target x86_64-pc-windows-msvc -p audio-share-core --all-targets -- -D warnings`

What this proves: format mapping/request/closest-match/block-align logic is
tested, and the Windows code typechecks and lints. What it does not prove:
anything requiring a Windows audio runtime (see residuals). No Windows
measurements or A/B results are claimed.

Residual steps for a Windows host (all NOT EXECUTED):

1. Start the new `as-cmd.exe` with a continuous tone playing and drive it with
   `protocol-harness record`/`replay`; confirm real system audio, identical
   `GET_FORMAT` bytes, 3-second heartbeats, and MTU-aligned datagrams.
2. Confirm the default format equals the endpoint mix format and that
   f32/s8/s16/s24/s32 plus channel/rate requests follow `S_FALSE` closest-match.
   Encoding selection flags arrive with ticket 07, so run this matrix once they
   exist (or via a temporary harness behind `CaptureConfig`).
3. Verify packed s24 acceptance and whether loopback really delivers the
   requested non-mix representation (the WASAPI docs say loopback data may be
   in the device format).
4. Run `wasapi-spike measure --seconds 600 --cycles 10` and
   `start-stop --iterations 200 --milliseconds 100`; record interval, CPU,
   silence/discontinuity/index-gap counters, handle counts, and confirm no
   crash or `AUDCLNT_E_ALREADY_INITIALIZED`.
5. Switch endpoints while stopped and while running; unplug/disable the active
   endpoint and confirm the process survives with a readable invalidation
   error. Automatic restart after invalidation is supervisor work not in this
   ticket.
6. Run `scripts/ab-compare-windows.ps1` against the old v0.3.4 `as-cmd.exe` on
   the same machine, source, and format; compare AudioFormat, PCM levels,
   datagram sizes, and (optionally) `wasapi-spike measure` glitch counters.
7. Record the results in `docs/server-rewrite/capture-windows.md` and close the
   remaining runtime checkboxes only after they are actually observed.
