# 21: SessionEngineFactory seam

**What to build:** Service 不再直接构造 `ClientSession`，而是依赖 `SessionEngineFactory`；这样 Service 的配对交接与 Connect/Stop 编排可用 fake engine 在 JVM 测试，`ClientSession` 保持默认实现。

**Blocked by:** 15。

**Status:** resolved

- [x] Service 只依赖工厂与最小 engine 接口（start、stop、confirm pairing + 回调/outcome sink）；不扩大依赖面。
- [x] 工厂接受 outcome sink 与配对/日志回调；`ClientSession` 为默认实现。
- [x] 配对编排（QR 解析、身份创建、SessionConfiguration、凭据持久化）仍留在 Service。
- [x] fake engine 覆盖 Connect/Stop、配对交接、旧代际拒绝；不引入 DI 框架。
- [x] 现有行为与设备用例不回归；接口不为未来需求预留成员。

## Comments

2026-11-17 — 完成并合入集成分支（merge `1b73e61`，feature `98d0857`）。
`SessionEngine` / `SessionEngineFactory` 落地，`ClientSession` 为默认实现；
`internal SessionOrchestrator` 使 Service 编排可在 JVM 用 fake engine 驱动。
JVM 147 tests/variant PASS，androidTest Java 编译 PASS。证据：`evidence/ticket-21/`。
设备项未跑（由工单 08/27 统一执行）。
