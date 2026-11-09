# 01: New Rust workspace + Wire Protocol harness + v0.3.4 golden fixtures

**What to build:** 一个新的 Rust workspace，能构建出 core、CLI、GUI 三部分骨架；同时提供一个 scripted Wire Protocol client，能驱动旧 `as-cmd`，记录 TCP 控制 transcript 与 UDP/PCM golden fixtures。完成后，后续所有 ticket 都能用同一套 harness 对比新 Server 与 v0.3.4 基线。

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] `cargo build` 与 `cargo test` 在 workspace 上通过，且不依赖 vcpkg、MSVC 或 MFC。
- [ ] scripted client 能启动旧 `as-cmd`，完成 `GET_FORMAT`、`START_PLAY`、heartbeat 与 UDP id 绑定。
- [ ] 生成并提交 Windows 与 Linux 的 golden fixtures：控制消息、帧结构、MTU 分片、`AudioFormat`。
- [ ] 提供一条命令重放 fixtures，作为后续 parity 测试入口。
- [ ] 最小 CI 任务在 `windows-latest` 与 `ubuntu-24.04` 上构建并运行 harness。
