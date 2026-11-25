# 11: 配对终局是否复用 Session outcome 骨架

Labels: wayfinder:grilling
Type: grilling
Status: resolved
Blocked by: 01

## Question

[01](01-session-outcome-module.md#answer) 已定：`SessionOutcome` 只描述一次会话从 Connect 到 Ended，配对结局不并入，`PairingStateRelay` 与 `PairingUi` 原样保留。本票决定配对终局与 outcome 模块的关系：

1. 配对终局是否需要同样的「单一归属 + 可重放」深模块，还是现有 `PairingStateRelay` + `PairingUi` 已足够？
2. 若需要，`PairingOutcome` 与 `SessionOutcome` 共享什么机制（generation / sink / replay / 终止一次），各自独占什么语义？
3. 配对成功到自动连接的交接：`CredentialStored` → `ConnectStored` 目前由 reducer 驱动，配对终局模块是否应拥有这次交接？
4. 配对进行中的会话 `Live(AUTHENTICATING)` 与 `PairingUi` 是两个来源（`ControlSessionService.kt:329-331`）。如何避免出现第二个「谁决定屏幕」的所有者？
5. 测试策略：`PairingPayloadTest`、配对 fixture replay、reducer 的 pairing 分支哪些保留、哪些迁移？

产出是决策与 interface 草图，不是实现代码。

## Answer

**结论（用户逐条确认）：不新建 PairingOutcome；`PairingStateRelay` + `PairingUi` 就是配对
终局的唯一所有者；与 `SessionOutcome` 只共享不变量、不共享实现；配对成功交接改由 Service
拥有并推送一次性 `PairingCompleted`，删除 UI 轮询；配对交换不进入 session outcome store。**

### 1. 配对终局的所有者

- 保留 `PairingStateRelay` + `PairingUi`，不引入 `PairingOutcome` sealed 类型，也不抽
  session/pairing 共用 store。
- 工单 30 已补齐 `beginSession()/Source` 所有权；`PairingSnapshot` 继续承载 live 数据
  （mode、phase、SAS、fingerprint、displayName、errorCode、clientId），`PairingUi` 仍是
  reducer 的纯投影。
- 不变量（作为接口契约与测试断言）：
  1. 同一时刻只有一个 `Source` 拥有发布权；
  2. `setListener` 注册即同步重放当前快照；
  3. 新交换开始时旧 source 失效并清空可见快照；
  4. 终局（COMPLETE / FAILED）后旧 source 的迟到事件一律丢弃；
  5. 配对终局不是 `SessionOutcome` 的一部分（延续 01）。

### 2. 与 SessionOutcome 共享什么

- **只共享不变量，不共享代码**：source ownership、注册重放、新交换清旧态、终局后拒绝旧事件。
- 01 删除 session relay 后，泛型 store 只剩 pairing 一个用户，没有 leverage；不抽
  `ReplayableStore<T>`，也不让 pairing 产生 `SessionOutcome` 风格的类型。

### 3. 配对成功 → 自动连接的交接

- **Service 拥有交接**：`persistCredential()` 成功后，Service 立即启动新的认证会话
  （新 generation），并向 UI 推送一次 `PairingCompleted(record)`。
- UI 收到后只做导航（PAIRING → SESSION）与 Server 列表更新；**不再发 `ConnectStored`**。
- **删除 `MainActivity.awaitCredential()` 与 100 ms / 5 s 轮询**，消除 COMPLETE 与
  持久化之间的竞态。
- 持久化失败：保持 COMPLETE 快照，记录日志，不自动连接；用户可返回 HOME 重试。
- **实现约束**：`persistCredential()` 运行在 pairing watcher 线程的 `onCredential` 回调里；
  交接必须通过 Service action 或主线程 handler **异步派发**，不得在回调内同步调用
  `startSession` / `teardownSession`（会在旧会话自己的线程上 `stop()` 并 `join` MediaPlayer，
  有死锁与迟到事件风险）。
- QR 一步配对（32）与 SAS 显式 Confirm 的行为不变；本票不重写。

### 4. 单一来源：配对交换与 session outcome

- 配对交换**不进入 session outcome store**：Service 只把配对会话的事件送到 pairing relay。
- 交接后新建的认证会话才接入 outcome store（01 的 sink/generation）。
- 结果：PAIRING 阶段只有 `PairingUi` 一个来源，SESSION 阶段只有 `SessionOutcome` 一个来源；
  屏幕归属只由 reducer 持有，不再出现「谁决定屏幕」的第二个所有者。
- 恢复补充：若恢复出的 `screen == PAIRING`、`pairingSnapshot == null` 且 outcome 为
  `Live`/`Reconnecting`（交接已发生但 Activity 错过了推送），落 **SESSION**，不是 HOME。
  这与 06 的「无活会话才降级 HOME」一致。

### 5. 接口草图

```kotlin
class PairingStateRelay {                       // 唯一配对终局所有者
    fun snapshot(): PairingSnapshot?
    fun setListener(listener: ((PairingSnapshot?) -> Unit)?)   // 注册即重放
    fun beginSession(): Source                  // 新交换；旧 source 失效并清空
}

interface SessionController {
    // 一次性推送；每个 exchange 至多一次，不重放
    fun setPairingCompletedListener(listener: ((CredentialRecord) -> Unit)?)
    // 其余方法不变
}
```

- `PairingCompleted` 一次性语义：Activity 重建时依靠 outcome（Live/Reconnecting）+ 恢复规则
  落 SESSION，而不是重放旧完成事件（避免用户主动回 HOME 后被再次拉走）。
- 若实现时发现 `CredentialRecord` 过大，可只推 `serverSpkiSha256Hex` + `StoredServer`，
  但不得回退到轮询。

### 6. 测试策略

保留：

- `PairingPayloadTest`、配对 fixture replay、`SasCode`、`ErrorGuidance`；
- 32 的 QR 自动确认恰好一次 reducer 测试；
- 30 的 relay source ownership 测试。

新增：

- Service 交接测试（fake engine）：持久化成功 → 启动新会话 → 推送一次
  `PairingCompleted`；持久化失败 → 不启动、不推送；
- 配对期间不发布 session outcome 的测试；
- 恢复规则：`screen=PAIRING` + 无 pairing 快照 + `Live` → SESSION。

删除/迁移：

- `MainActivity.awaitCredential` 轮询及其测试；
- `CredentialStored` 触发 `ConnectStored` 的 reducer 分支改为「只更新列表，不发起连接」。
- 现有测试资产的最终去留归 12。

### 对后续工单的约束

- 05：`SessionEngine` 接口不变；`setPairingCompletedListener` 位于 Binder 层的
  `SessionController`，由 Service 实现，不扩大 `SessionEngine`。
- 06：恢复规则补充见第 4 节；PAIRING 无活会话仍落 HOME。
- 07：driver 不在配对阶段发布 outcome；配对交接后的新会话才使用 `OutcomeSink`。
- 12：`awaitCredential`、`CredentialStored` 分支与 pairing relay 测试纳入资产处置。

### 证据

本票为决策票，未改动代码，故无 PASS/FAIL；事实来源为工单 30/32 的最新代码
（`Pairing.kt` 的 `PairingStateRelay`、`AppState.kt` 的 `autoConfirmSent`、
`MainActivity.kt:188-210` 的轮询）。
未建分支、未 commit。
