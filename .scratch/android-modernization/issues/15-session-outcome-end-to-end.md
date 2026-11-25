# 15: Session outcome 端到端

**What to build:** 一次 Client Session 的结局由一个 Kotlin 深模块独占；UI 只看到 `SessionOutcome`，不再解释 native 状态字符串。重连期间留在 SESSION，Ended 只在原位呈现。

**Blocked by:** 13。

**Status:** resolved

类型形状（来自工单 01）：

```text
SessionOutcome = Live(LivePhase) | Reconnecting(attempt, lastCloseCode) | Ended(UserStop | Surfaced(code))
LivePhase      = CONNECTING | AUTHENTICATING | READY | PLAYING
```

- [x] `ClientSession` 按关闭码与调用点分类；Stop 归因来自调用点（User / Superseded / Shutdown / PairingCancel）。
- [x] Service 持有唯一可重放 store；注册监听同步重放；每个 Client Session 恰好一个 Ended；旧代际事件被丢弃。
- [x] 删除 `sessionPhase`、`connectRequested`、`stopping`、`expectedStop`、Service 侧 last error code；native 状态字符串不越过 Service seam。
- [x] reducer 使用单一 `outcome` 字段；Live/Reconnecting 留 SESSION，Ended 也在 SESSION 原位显示。
- [x] JVM 测试覆盖分类表、store 不变量、重连不换屏、两种 Ended 的视觉差异。
- [x] 不改 Wire Protocol / `stats_json`；native 表示法清理作为独立小步骤，行为不变。

## Comments

2026-11-17 — 完成并合入集成分支（merge `ca0186a`，feature `971a66c`）。
JVM 136 tests PASS、androidTest 编译 PASS、native `cargo test` 154 PASS；
`SessionStateRelay` / `SessionCommandExecutor` 及其测试已删除，配对交换不进入 outcome store。
证据：`evidence/ticket-15/report.md`。真机/`EMULATOR` 项按工单 08/27 统一执行。
