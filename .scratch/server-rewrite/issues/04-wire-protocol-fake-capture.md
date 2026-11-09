# 04: Wire Protocol slice with fake capture + `as-cmd -b`

**What to build:** 新 Server 用 synthetic capture source 完成 Wire Protocol 全链路：TCP `GET_FORMAT`、`START_PLAY`、3 秒 heartbeat、5 秒超时、UDP id 绑定、裸 PCM 发送、MTU 1492 分片与 `block_align` 对齐；`as-cmd -b` 能启动和停止。harness 独立验证，不依赖真实音频设备。

**Blocked by:** 01 (New Rust workspace + Wire Protocol harness + v0.3.4 golden fixtures)

**Status:** resolved

- [x] `GET_FORMAT` 返回与 v0.3.4 兼容的 `AudioFormat`。
- [x] `START_PLAY` 返回 id；客户端发送 id 后收到 UDP PCM。
- [x] heartbeat 每 3 秒发送；5 秒无活动关闭会话。
- [x] MTU 分片与 `block_align` 对齐符合 v0.3.4；多字节整数显式 little-endian。
- [x] `as-cmd -b host:port` 可启动和停止，harness 全链路通过。
- [x] 无真实音频设备时测试仍可运行。

## Comments

2026-11-09 — branch `ticket/04-wire-protocol` (worktree `audio-share-wt/04`).

Implemented:

- `audio-share-core::capture`: `CaptureConfig`, `EncodingPreference`, `Endpoint`,
  `CaptureBackend`, `CaptureSession`, `FrameSink`, and deterministic
  `SyntheticCapture` (`new` generator + `with_frames` exact-frame test
  constructor). Capture runs on a joined thread; stop/drop are idempotent.
- `audio-share-core::server`: capture starts before TCP/UDP bind; `ServerConfig`
  (port 0 supported), `Server::start`, `ServerHandle::{local_addr, format, stop}`;
  `GET_FORMAT`/`START_PLAY` framing, per-server play ids from 1, heartbeat on the
  configured interval with the timeout checked on the same tick, UDP id binding
  (last bind wins, unknown ids ignored), MTU segmentation via ticket 01's helper,
  broadcast to all bound peers, cancellable bounded stop that closes sessions and
  joins tasks. Format is an immutable `Arc<AudioFormat>`.
- `as-cmd -b/--bind [host][:port]`: starts the server with synthetic
  F32/stereo/48 kHz capture, runs until Ctrl-C, then stops and exits 0; readable
  errors exit 1. Default port 65530.
- `protocol-harness` is now also a library so `as-cmd`'s integration test drives
  the same replay path as the CLI.

Commands and results:

- `cargo fmt --check` — pass.
- `cargo clippy --workspace --all-targets -- -D warnings` — pass.
- `cargo test --workspace` — 58 passed, 0 failed (as-cmd 2 unit + 1 harness
  replay integration; core 4 unit + 5 capture + 17 protocol + 15 server;
  harness 11 unit + 2 CLI + 1 smoke; 0 doctests).
- `cargo check --workspace --all-targets --target x86_64-pc-windows-msvc` — pass.
- Explicit harness replay:
  `cargo run -p protocol-harness -- replay --fixture fixtures/protocol/v0.3.4/linux/f32-48k-stereo --server-cmd target/debug/as-cmd --server-arg=--bind={bind}`
  → `replay: ok (live)`, `format_bytes_match=true`, `start_play_id: 1`,
  `heartbeats: 1`, 1698 datagrams (aligned payload 1464).

Caveats (no protocol deviations):

- Play ids start at 1 and increase per server instance. v0.3.4 used a process-wide
  static, so ids would continue across stop/start inside one process; the client
  contract only requires `id > 0`, and one server per process is the normal case.
- Bare `-b` default-host selection uses the documented RFC 1918 ranges
  (10/8, 172.16/12, 192.168/16) and falls back to the first non-loopback IPv4.
  The old exact mask comparison also matched a few non-private ranges; strict CLI
  parity/decisions belong to ticket 07.
- Windows runtime is unavailable on this Linux host; Windows validation is the
  `cargo check --target x86_64-pc-windows-msvc` above. Real WASAPI/PipeWire
  capture, full CLI flags, config, and GUI are later tickets.
