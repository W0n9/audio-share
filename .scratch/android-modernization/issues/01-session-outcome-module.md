# 01: Session outcome：关闭归属与重连语义的深模块

Labels: wayfinder:grilling
Type: grilling
Status: resolved
Blocked by: None (can start immediately)

## Question

把架构审查候选 01 与 04 合并成一个深模块，解决「一次会话为什么结束、接下来是重连还是终止」没有所有者的问题。

现状证据：

- `ClientSession.kt:332-348` 的 `scheduleReconnect` 先 `source.close()` 发布 `CLOSED`；`AppState.kt:392-421` 把 `CLOSED` 当终态，设 `screen = HOME`、`connectRequested = false`。重连成功后的 `READY` 回不到会话页。
- 关闭归属分散在 `SessionCommandExecutor.expectedStop`（每次 bind 重建）、`AppState.stopping`、`ControlSessionService.lastErrorCode` 三处；Activity 重建后，迟到的 `CLOSED` 会把上一会话的错误码当成新错误。
- 两层 relay（`SessionStateRelay` 内层 / Service 外层）的注释各自合理，但「CLOSED 是终态还是瞬时态」没有归属。

需要决定：

1. **Interface**：UI / Service 消费什么 outcome？候选 `Reconnecting(attempt)` / `Ended(user)` / `Ended(errorCode)`；配对结局是否也走同一个 outcome，还是留给单独的配对票。
2. **Seam 位置**：放在 Service 层还是 `ClientSession` 层；两层 relay 保留哪一层、删哪一层。
3. **关闭归属**：用户 Stop、传输丢失、协议 `Close`、应用主动 teardown 如何分类；`expectedStop` / `stopping` / `lastErrorCode` 如何收编。
4. **跨语言映射**：与 Rust `CloseInfo.code`、Kotlin `null = transport loss` 的表示法如何对齐；不改协议、不改错误码。
5. **测试策略**：哪些现有测试保留 / 删除；在新 interface 上如何注入迟到 `CLOSED`、故障时钟与 rebind。
6. **迁移顺序**：能否先落 Kotlin 深模块、再清理 Rust 表示；还是必须一次完成。

产出是决策、interface 草图与测试策略，不是实现代码。合并 01 + 04 后，若发现配对完成事件也应归入同一模块，记录并另开票，不要在本票里顺手解决。

## Answer

一次会话的「为什么结束、接下来重连还是终止」由一个 Kotlin 深模块独占：`ClientSession` 分类，Service 持有唯一可重放的 outcome store，UI 只消费 `SessionOutcome`。native 状态字符串不再越过 Service seam。

### 模块契约

```kotlin
sealed interface SessionOutcome {
    /** 会话进行中；phase 是 native 状态的语义化投影。 */
    data class Live(val phase: LivePhase) : SessionOutcome
    /** 连接已断，自动重连进行中；lastCloseCode 仅用于诊断，不展示。 */
    data class Reconnecting(val attempt: Int, val lastCloseCode: Int?) : SessionOutcome
    /** 会话终止；每个会话恰好一次。 */
    data class Ended(val reason: EndReason) : SessionOutcome
}

enum class LivePhase { CONNECTING, AUTHENTICATING, READY, PLAYING }

sealed interface EndReason {
    /** 用户主动结束（in-app Stop、通知栏 Stop、离开 App）。 */
    data object UserStop : EndReason
    /** 不允许自动重连的关闭，码交给 UI 映射 ErrorGuidance。 */
    data class Surfaced(val code: Int?) : EndReason
}

interface SessionController {
    fun setOutcomeListener(listener: ((SessionOutcome) -> Unit)?)
    /** null = Service 自启动以来没有会话。注册监听时同步重放。 */
    fun outcomeSnapshot(): SessionOutcome?
    // 其余配对 / 凭据 / 命令方法不变
}

class ClientSession(..., private val sink: OutcomeSink) {
    fun stop(reason: StopReason)      // User / Superseded / Shutdown / PairingCancel
}
```

- Seam 位置：`ClientSession` 持有 lifetime、handle 代际与 `ReconnectPolicy`，因此由它分类；Service 每个用户会话生成一个代际 `OutcomeSink` 注入，并持有唯一 store。`ClientSession.currentState` 那层 `SessionStateRelay` 删除；Service 层 relay 由 outcome store 取代。
- 保持回调风格，不引入 `StateFlow`/coroutines 依赖（留给「依赖注入与模块化粒度」票）。
- `setStateListener` / `stateSnapshot` / `lastErrorGuidance` 从 seam 删除；`SessionCommandExecutor.expectedStop` 删除。

### 归属与分类

| 输入 | Outcome |
|---|---|
| `stop(User)` 原因（in-app / 通知栏 / Activity finishing） | `Ended(UserStop)` |
| `701`（防御性；正常经 `stop` 归因，不经错码路径） | `Ended(UserStop)` |
| `301,302,503,601,602` 或 `close_code == null` | `Reconnecting` |
| `303,101,102,201,202,203,204,205,401,402,403,599` | `Ended(Surfaced(code))` |
| 任何未列出的码 | `Ended(Surfaced(code))`（不自动重连） |

- 复用 `ReconnectPolicy` 的可重连集合，不另造第二份判断。
- `Superseded`（换会话前 teardown）、`Shutdown`（`onDestroy`）、`PairingCancel` 不产生 UI 可见 outcome。
- 归属由**调用点**决定，不由错误码反推：用户 Stop 不再依赖 native 的 701，也不需要 `lastErrorCode`。
- 配对结局不并入；`PairingStateRelay` 与 `PairingUi` 原样保留（另开票，见 11）。

### 语义不变量

1. 每个用户会话恰好一个终止 `Ended`；此后该 Sink 的 `Live`/`Reconnecting` 一律丢弃。
2. 相同的 `Live(phase)` 不重复发布。
3. 新监听者注册时同步重放最新 outcome。
4. 旧会话 Sink 的迟到事件因代际不符被丢弃（替代 `ownsSessionLocked` 的发布职责，handle/lifetime 守卫保留）。

### AppState 与 UI 契约

> 导航部分已被 [06](06-navigation-and-state-restoration.md#answer) 覆盖：`Ended(UserStop)` 与
> `Ended(Surfaced)` 都留在 SESSION（原位错误卡），不再回 HOME；`Reconnecting` 仍留
> SESSION。其余契约不变。

- `AppState` 用单一字段 `outcome: SessionOutcome?`；删除 `sessionPhase`、`connectRequested`、`mapSessionPhase`。`stopping` 改名为 `stopRequested`，仅表示异步 Stop 期间的本地按钮禁用，不进入领域模型。`screen` 仍是独立导航状态。
- `Live(phase)`：SESSION 页显示 phase，不改屏。
- `Reconnecting(attempt, lastCode)`：**停留 SESSION**，状态行 `RECONNECTING (attempt N)`，不弹错误卡，Stop 可用；音频停后自动续播，无需二次 Connect。
- `Ended(UserStop)`：回 HOME，无错误。
- `Ended(Surfaced(code))`：回 HOME（ABOUT 留在 ABOUT），错误卡由 `ErrorGuidanceBook` 生成；`requiresRepairing` 沿用 Retry→PAIRING。
- 配对页豁免：`screen == PAIRING` 时不因 `Ended` 强制回 HOME。
- `Live`/`Reconnecting` 时 `ConnectRequested` 只导航到 SESSION，不下发 `ConnectStored`，避免 teardown 杀掉正在恢复的会话。
- 会话进行中 Back to Home 仍允许；重连不会把用户拽回 SESSION。
- `AutoConnectPolicy` 改吃 `SessionOutcome?`：仅「全新启动 + 有 credential + `outcome == null || Ended`」自动连接；`Live`/`Reconnecting` 一律不自动连接；`Ended(Surfaced)` 且 `requiresRepairing` 不自动连接。

### 跨语言边界与迁移顺序

- 不改 Wire Protocol、错误码或 `stats_json` 形状。`null` 只解释为「native 未记录结局」，可重连。
- 第一阶段（Kotlin）：outcome 类型 + store + `ClientSession` 分类 + Service seam + reducer/UI + 测试；一次性切换，不留字符串兼容层。
- 第二阶段（native 表示法清理，不改线上行为）：`record_close` 先写 `close_info` 再设 `closed`，消除「`CLOSED` + `close_code:null`」竞态；`Session::stop()` 记录 `CloseInfo{701, remote=false}`，让 `null` 只剩「尚未结束」。
- 两个提交在同一重构分支，与 beta 冻结互不干扰。

### 测试策略

- `SessionOutcomePolicyTest`（纯 JVM）：上面分类表全量，含 `701`、`303`、`205`、`401`、未知码、`null`。
- `SessionOutcomeStoreTest`：重放、终止一次、重复 `Live` 去重、旧代际迟到事件丢弃。
- 重写 `AppStateReducerTest`：重连保持 SESSION；重连后 `READY` 无需二次 Connect；`Ended(UserStop)` 静默回 HOME；`Ended(Surfaced)` 出错误卡；`Live` 时 Connect 只导航；配对页不被 `Ended` 打回。
- 删除 `SessionCommandExecutorTest`；`SessionStateRelayTest` 由 store 测试替代；`ReconnectPolicyTest` 保留。
- 本票不要求 instrumentation / 真机；实现验证由 08 承接（本票 `NOT RUN`）。

### 删除清单

`SessionCommandExecutor`（整类）、`expectedStop`、`AppState.sessionPhase`、`connectRequested`、`stopping`（改为 `stopRequested`）、`mapSessionPhase`、`ControlSessionService.lastErrorCode`、`SessionController.lastErrorGuidance`、`ClientSession.currentState` / `stateSnapshot`。

### 遗留

配对终局是否复用本模块骨架 → 已开 [11](11-pairing-outcome.md)，本票不解决。

