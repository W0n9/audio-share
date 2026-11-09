# 01: New Rust workspace + Wire Protocol harness + v0.3.4 golden fixtures

**What to build:** 一个新的 Rust workspace，能构建出 core、CLI、GUI 三部分骨架；同时提供一个 scripted Wire Protocol client，能驱动旧 `as-cmd`，记录 TCP 控制 transcript 与 UDP/PCM golden fixtures。完成后，后续所有 ticket 都能用同一套 harness 对比新 Server 与 v0.3.4 基线。

**Blocked by:** None (can start immediately)

**Status:** resolved

- [x] `cargo build` 与 `cargo test` 在 workspace 上通过，且不依赖 vcpkg、MSVC 或 MFC。
- [x] scripted client 能启动旧 `as-cmd`，完成 `GET_FORMAT`、`START_PLAY`、heartbeat 与 UDP id 绑定。
- [x] 生成并提交 Windows 与 Linux 的 golden fixtures：控制消息、帧结构、MTU 分片、`AudioFormat`。
- [x] 提供一条命令重放 fixtures，作为后续 parity 测试入口。
- [x] 最小 CI 任务在 `windows-latest` 与 `ubuntu-24.04` 上构建并运行 harness。

## Comments

### 2026-11-09 — resolved on `ticket/01-workspace-harness`

Commits: `26069c7` (workspace + core protocol + harness), `32e1324` (golden
fixtures + recording scripts), `2028499` (CI workflow + harness docs). This
ticket file is updated in a follow-up commit.

Implementation:

- Rust workspace with resolver 2 and edition 2021: `crates/audio-share-core`,
  `crates/as-cmd` (skeleton), `tools/protocol-harness`. Builds with plain
  `cargo`; no vcpkg, MSVC, MFC, or `protoc`.
- Core has hand-written prost types (`AudioFormat`, `Encoding`), v0.3.4
  framing, heartbeat constants (3 s / 5 s), and MTU segmentation
  (`1492 - 20 - 8 = 1464`, aligned down to `block_align`, safe for 0 and
  >1464). Unit/integration tests cover exact LE and protobuf bytes,
  round-trips, and block_align 1/2/3/4/6/8/32 plus odd sizes.
- Harness commands: `record`, `replay` (offline and live `--connect` /
  `--server-cmd`), `serve-fake`, `synthesize`. Record has a total timeout and
  always kills its child server. Fake server implements GET_FORMAT,
  START_PLAY, heartbeats, timeout, UDP bind, and MTU segmentation.
- Reproducible recording: `scripts/pw-test-session.sh` (disposable headless
  PipeWire + WirePlumber + `TestSink` null sink, tone loop, trap cleanup),
  `scripts/gen-tone-wav.py`, `scripts/record-fixtures.sh`.
- CI: `.github/workflows/server-rewrite.yml` on `ubuntu-24.04` and
  `windows-latest` runs fmt, clippy with `-D warnings`, workspace tests, the
  hardware-free smoke test, and offline replay of a committed fixture.
  Existing workflows untouched.

Exact commands run locally (Ubuntu 24.04, Rust 1.98.1):

```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
cargo check --workspace --all-targets --target x86_64-pc-windows-msvc
AS_CMD_034=/home/w0n9/workspaces/audio-share-notes/server-rewrite/bin/as-cmd-0.3.4-linux scripts/record-fixtures.sh
cargo run -p protocol-harness -- replay --fixture fixtures/protocol/v0.3.4/linux/f32-48k-stereo
cargo run -p protocol-harness -- replay --fixture <fixture> --server-cmd <old-as-cmd> --server-arg=--bind=127.0.0.1:{port} ...
```

Results: fmt clean; clippy clean; `cargo test --workspace` passed 31 tests
(17 core protocol, 11 harness unit, 2 CLI integration, 1 serve-fake live smoke).
The real old-`as-cmd` record produced non-silent PCM and the live replay against
old `as-cmd` reported `format_bytes_match=true`, one heartbeat, and MTU-aligned
UDP. The `x86_64-pc-windows-msvc` check passed locally.

Fixtures (under `fixtures/protocol/v0.3.4/`):

- `linux/f32-48k-stereo` and `linux/s16-44k1-stereo`:
  `"provenance": "recorded-v0.3.4-linux"`. Recorded from the released
  `as-cmd` 0.3.4 Linux binary in a headless PipeWire `TestSink` null sink with a
  generated sine tone. Real control bytes, real UDP/PCM snapshot, exact
  AudioFormat bytes.
- `windows/f32-48k-stereo`, `windows/s16-44k1-stereo`,
  `windows/s16-48k-mono`, `windows/s24-48k-stereo`:
  `"provenance": "synthesized-v0.3.4-semantics"`. These are synthesized from
  the frozen v0.3.4 encoding and were **not** recorded on Windows.

Residual caveats:

- The ticket file contains five checkboxes (not six); all five are checked.
- Windows fixtures are synthesized, not hardware-recorded; they pin protocol
  encoding only.
- The Windows CI job was validated locally with `cargo check --target
  x86_64-pc-windows-msvc`, not by an actual GitHub Actions run.
- Linux s8 capture was not recorded: the null-sink session rejected U8 with
  "the capture format is not supported"; s8 stays covered by synthesized
  encoding tests.
- Recorded PCM is a live snapshot; re-recording changes PCM bytes but not the
  manifest structure or framing.

### Merge verification

Merged to `integration/server-rewrite` in merge commit `4c291d2` (no
conflicts). Merge-gate checks rerun on the integration branch:

```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                         # 31 passed
cargo run -p protocol-harness -- replay --fixture fixtures/protocol/v0.3.4/linux/f32-48k-stereo
cargo check --workspace --all-targets --target x86_64-pc-windows-msvc
```

All exited 0. The fixture replay reported `replay: ok (offline)`, PCM float,
2 channels, 48 kHz, 32 datagrams / 44,480 bytes, MTU payload 1464.
