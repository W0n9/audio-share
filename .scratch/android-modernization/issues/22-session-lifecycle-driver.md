# 22: SessionLifecycleDriver 与 Transfer Format 清理

**What to build:** 把会话时序与所有权抽成 JVM 可测的 driver，端口化 native、网络、调度、媒体与 outcome；行为不变。同时删除 Kotlin 侧不可达的 Transfer Format 分支，Rust 闸门不动。

**Blocked by:** 21。

**Status:** resolved

端口：NativeSession、NetworkMonitor、Scheduler、MediaController、OutcomeSink。

- [x] 创建/连接失败、connect 被拒 → bounded backoff；复用现有 reconnect policy。
- [x] 握手期 network available 不 rebind；rebind 成功继续；rebind 失败 teardown + backoff。
- [x] STOP 与 create/重连交错时旧代际任务丢弃，不泄漏 native handle；媒体先停、再销毁 native。
- [x] JVM 时序测试覆盖上述路径；`EMULATOR` 覆盖真实网络回调与 rebind；真机断网重连留给证据票。
- [x] 删除 Kotlin AudioSink 中不可达的 U8/S32 分支，集中 supported-format 表；Rust 闸门与 Wire Protocol 不变。
- [x] `ClientSession` 退化为 Android adapter；行为与设备用例不回归。

## Comments

2026-11-17 — 完成并合入集成分支（merge `4406cf2`，feature `73ba250`，已同步 ticket 17 tip）。
`SessionLifecycleDriver` + 五个端口落地，`ClientSession` 成为 Android adapter；
媒体先停、再 stop/destroy native 的顺序被测试锁定；Kotlin 侧 U8/S32 死分支删除，
Transfer Format 表以 Rust 闸门为权威。JVM 190 tests/variant PASS、cargo 154 PASS。
证据：`evidence/ticket-22/`。模拟器真实网络回调/rebind 与真机项 NOT RUN（08/27 承接）。
