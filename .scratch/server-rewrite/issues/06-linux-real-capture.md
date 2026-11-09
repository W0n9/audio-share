# 06: Linux real capture path

**What to build:** 把 PipeWire capture 接入 core；选定 Audio/Sink 后，新 Server 通过 Wire Protocol 发送真实音频；验证 monitor 与非 F32 路径；与旧 `as-cmd` A/B。

**Blocked by:** 03 (Linux PipeWire capture spike), 04 (Wire Protocol slice with fake capture + `as-cmd -b`)

**Status:** resolved

- [x] 枚举 Audio/Sink，默认按 `priority.session` 选择。
- [x] 选定 sink 后，scripted client 收到真实 monitor 音频。
- [x] F32LE 为默认；s8 映射到 U8，s16、s24、s32 映射正确。
- [x] 与旧 `as-cmd` 的 `AudioFormat` 和 PCM 输出对比无明显差异。
- [x] 反复 start/stop 与切换 sink 不崩溃。

## Comments

2026-11-09 — branch `ticket/06-linux-capture` (worktree `audio-share-wt/06`).

Implemented:

- `audio-share-core::capture::linux::PipeWireCapture` (Linux-only) using the
  safe `pipewire` 0.10.1 crate. One dedicated PipeWire thread per session owns
  all loop/context/core/registry/stream objects; `start` validates the
  `object.serial` target against the live registry, connects a monitor capture
  stream (`media.category=Capture`, `media.role=Music`, `target.object`,
  AUTOCONNECT/MAP_BUFFERS/RT_PROCESS), waits up to 5 s for negotiation, and
  returns the negotiated `AudioFormat`.
- Enumeration uses `media.class=Audio/Sink`, display name
  `node.nick → node.description → node.name → serial`, default = highest
  `priority.session` strictly > -1, first wins ties, none otherwise.
- Encoding map: Default/F32→F32LE, s8→U8, s16→S16LE, s24→packed S24LE (3
  bytes), s32→S32LE; protocol encoding and `block_align` are derived from the
  negotiated format, never from the request.
- RT safety: `process` only copies into a 4 MiB lock-free SPSC ring
  (`ringbuf`) and stores atomics; a forwarder thread drains it into the core
  bounded `FrameSink` with `try_send` (full sink drops whole aligned chunks and
  accounts bytes), so the RT callback never blocks/locks/allocates.
- Stop is private to `capture/linux` (the shared `CaptureSession` is
  unchanged): a watcher thread waits on the core `StopSignal` and sends the
  `pipewire::channel` stop; the loop thread disconnects, quits, drops objects,
  and joins the forwarder/watchdog.
- Target loss: registry `global_remove` plus a 5 s callback-liveness watchdog
  end the session and drop the frame sender; stream errors also end it.
- `capture::platform_backend()` selects PipeWireCapture on Linux and
  SyntheticCapture elsewhere; `as-cmd` uses it by default and gained
  `--fake-capture` (the ticket-04 fixture test now passes this flag).

Commands and results (Ubuntu 24.04.5, libpipewire 1.0.5, WirePlumber 0.4.17):

- `cargo test -p audio-share-core --lib capture::linux` — 5 passed (encoding
  map, negotiated block alignment including packed s24, priority/default
  selection, target validation, display-name fallback).
- `cargo test -p audio-share-core --test capture_linux -- --ignored --nocapture`
  — 1 passed in ~17 s against a disposable headless session with `TestSink`
  (1000), `TestSinkLow` (500), `TestSinkGone` (100) and a looped 440 Hz tone.
  Covered: enumeration/`object.serial`, default selection, invalid-target
  rejection, F32/s8/s16/s24/s32 tone capture (Goertzel 440 Hz, block-aligned),
  10 start/stop cycles (including stop while connecting/streaming), sink
  switch-and-back, and destroying the target mid-stream (session ended within
  the global_remove/watchdog window).
- `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings` —
  clean.
- `cargo test --workspace` — all non-ignored tests pass (core 9 unit + 5
  synthetic + 17 protocol + 15 server + 1 ignored Linux capture; as-cmd 2 + 1
  harness replay; spike/harness suites unchanged).
- `cargo check --workspace --all-targets --target x86_64-pc-windows-msvc` —
  pass (PipeWire code is `cfg(target_os = "linux")`; Windows uses the synthetic
  branch until ticket 05 lands).
- A/B vs old `as-cmd` 0.3.4 on the same null sink/tone (protocol-harness record,
  700 datagrams each): identical `AudioFormat`/`GET_FORMAT` bytes
  (`080110021880f702`), identical datagram size distribution (642×1464 + 58
  partial, mean 1365.897), 956,128 PCM bytes / 119,516 frames both, dominant
  tone 440.0 Hz at amplitude 0.2456 both. Not byte-identical PCM; see
  `docs/server-rewrite/ab-linux.md`.
- Old `as-cmd --encoding=s8` exits 1 ("the capture format is not supported")
  because it only maps S8 while requesting U8; the new backend maps negotiated
  U8 to `Pcm8Bit` and the integration test captures s8 successfully.

Docs: `docs/server-rewrite/capture-linux.md` (behavior and headless recipe),
`docs/server-rewrite/ab-linux.md` (A/B commands/results).

Caveats:

- Only the headless `support.null-audio-sink` was available; real ALSA/USB
  sinks, suspend/resume, and hotplug variants are not verified here.
- Short captures only; the 8-hour soak and real v0.3.4 APK acceptance remain in
  ticket 15.
- The watchdog deliberately does not treat "never produced a callback" as
  target loss (an idle graph may legitimately be silent); target loss is caught
  by `global_remove`.
- Windows runtime capture is ticket 05; this host only type-checks it.
