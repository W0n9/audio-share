# 06: 导航模型与进程死亡后的状态恢复

Labels: wayfinder:grilling
Type: grilling
Status: resolved
Blocked by: 01, 04

Preconditions from [04](04-modern-ui-direction.md#answer) / [05](05-di-and-modularization.md#answer)：
视觉方向 A；导航只依赖 `ui` 包的 `AppScreen`/`AppState`，不得直接触碰 `session`/`native`。

## Question

决定 UI 的状态与导航基础：

1. **导航**：保留现有 `AppScreen` 枚举状态机，还是引入 Navigation Compose？以配对、会话、设置、关于四类屏幕为范围，说明哪种更贴合「Session outcome」的 interface。
2. **状态恢复**：进程死亡 / Activity 重建后恢复到哪一屏；哪些状态必须恢复（配对快照、当前会话、地址、错误、播放状态），哪些必须重新建立（native session、凭据轮询）。
3. **与 01 / 04 的配合**：outcome 如何驱动导航；UI 原型中的失败 / 重连 / 结束三态如何映射到具体目的地。

产出是导航模型、恢复清单与验收场景，不是实现。

## Answer

**结论（用户逐条确认）：保留 `AppScreen` 枚举、不引入 Navigation Compose；新增 SETTINGS
目的地；outcome 不再自动跳 HOME；配置变更只存最小 UI bundle 并从 Service 回放；进程死亡
落 HOME 且配对不续接；Back 只做导航，Activity finish 仍发 `ACTION_STOP`。**

### 1. 导航机制

- 保留 `AppScreen`（`AppState.kt:4`）+ `AppReducer` 作为唯一导航规则所在地；不引入
  Navigation Compose。单 Activity、目的地少、导航由 outcome 驱动而非 URL/返回栈；
  Navigation Compose 会引入第二份状态与独立 saved-state 语义，与 01 的可重放 store 重复。
- 目的地扩为五个：`HOME`、`PAIRING`、`SESSION`、`ABOUT`、`SETTINGS`。
- 不新增导航依赖；不新增 route/deeplink。

### 2. outcome → 屏幕（替换现有自动跳转）

现行 `onNativeState`（`AppState.kt:392`）在 `CLOSED` 时回 HOME，`onSessionError`
（`AppState.kt:410`）也回 HOME；两者都改为：

| 事件 / outcome | 目的地 |
|---|---|
| Connect / `PairingCompleted`（Service 交接后） | SESSION |
| Live / Reconnecting / Ended（UserStop 或 Surfaced） | 留在 SESSION |
| Pairing FAILED / pairing error | 留在 PAIRING |
| Surfaced session error | 留在 SESSION，显示错误卡，不自动回 HOME |
| Retry（`requiresRepairing`） | PAIRING |
| ShowHome / ShowAbout / ShowSettings | 显式用户动作才离开 |

- Ended 的两种原因只在 SESSION 内用文案/图标区分，不产生不同目的地。
- ABOUT / SETTINGS 是用户主动打开的页面；会话状态变化不得把用户从这两个页面拉走。
- `connect` 仍负责 HOME → SESSION；配对完成后的自动进入由 11 的 `PairingCompleted` 驱动，
  `CredentialStored` 只更新列表，不再发 `ConnectStored`。

### 3. SETTINGS 目的地

- 新增 `AppScreen.SETTINGS`；入口在顶栏（方向 A）。
- 行项目、开关与空态由工单 10 决定；本票只定「它是可导航目的地」。若 10 最终找不到
  内容，删除该枚举值即可回退，不构成架构负担。

### 4. 状态恢复

**配置变更（Activity 重建，Service 存活）**

- `onSaveInstanceState` 只存最小 UI bundle：`screen`、`address`、`sasInput`。
- 一律从 Service 回放：`outcomeSnapshot()`、`pairingSnapshot()`、`credentials()`；
  错误由 `Ended(Surfaced(code))` 经 `ErrorGuidanceBook` 映射，不再有 `lastErrorGuidance()`。
- 配对完成后的导航由 11 的 `PairingCompleted` 推送驱动；`CredentialStored` 只更新 Server 列表。
- 不保存 `scanActive`（不自动开相机）；不保存 `error`、`storedServers`、
  `sessionPhase`、`pairing` 等会话派生状态。
- `AppState` 保持普通 data class，不改成 Parcelable/Serializable；`pastedLink` 继续用
  `rememberSaveable`（`AudioShareApp.kt:161`）。
- auto-connect 在配置变更时不触发（`hadSavedState`，`MainActivity.kt:62,125`）。

**进程死亡（Service 与 native session 一并消失）**

- 恢复的 `screen` 只是提示：首次绑定后若 `outcomeSnapshot() == null || Ended` 且
  `pairingSnapshot() == null`，把 SESSION / PAIRING 降级为 HOME。
  > 11 补充：若 `pairingSnapshot() == null` 但 outcome 为 `Live`/`Reconnecting`
  > （配对交接已发生、Activity 错过 `PairingCompleted`），`PAIRING` 落 **SESSION**，不是 HOME。
- 配对中断不续接：native 配对上下文已丢失，回到 HOME 重新发起；不弹伪装成可恢复的
  半完成配对。
- 不把 `SessionOutcome` 落盘。进程死亡后系统重建 Activity 时 `savedInstanceState != null`，
  现有 `AutoConnectPolicy` 会因 `savedStatePresent=true` 抑制 auto-connect
  （`MainActivity.kt:62`、`AppState.kt` 的 `AutoConnectPolicy`）：**恢复的进程落在 HOME，
  不自动连接**，由用户点 Connect。冷启动（没有 saved bundle）仍按现有策略 auto-connect。
  本票沿用该策略，不修改。
- Service 已经持有会话时，绑定只回放，不重复连接。

**必须 / 不得恢复**

| 类别 | 内容 |
|---|---|
| 配置变更必须恢复 | screen、address、SAS 输入；会话/配对/凭据/错误从 Service 回放 |
| 进程死亡必须重建 | native session、配对上下文、通知；配对成功交接由 Service 推送 `PairingCompleted`，不靠轮询 |
| 永远不恢复 | 相机扫描、瞬时 error、`sessionPhase`、outcome 的磁盘副本 |

### 5. 返回键

| 当前屏 | Back 行为 |
|---|---|
| PAIRING | 发 `AppCommand.CancelPairing`，然后 HOME |
| SETTINGS / ABOUT | HOME |
| SESSION | HOME；会话继续，不 Stop |
| HOME | 系统默认 finish |

- `onDestroy` 继续在 `isFinishing && !isChangingConfigurations` 时发
  `ControlSessionService.ACTION_STOP`（`MainActivity.kt:101-108`）；现有
  `ForegroundServiceContractTest.kt:34-37` 与 instrumentation 依赖该契约，不在本票改动。
- 结果需要明确：在 SESSION 连按两次 Back（SESSION→HOME→finish）会停止会话。这是
  「Back 只导航 + finish 停会话」两条规则叠加的自然结果，不是 bug；10/08 不应把它当
  回归处理。
- Back 只承担导航；停止播放仍只由应用内 Stop、通知 Stop 或离开应用触发。

### 对后续工单的约束

- 10：SETTINGS 的内容与空态；每个目的地的 must-have 状态需与本票的转移表一致。
- 08：恢复验收至少覆盖「旋转/重建」「进程死亡」「Service 已有会话时重绑」三类场景，
  并明确 JVM / `EMULATOR` / 真机层级。
- 12：`AudioShareAppTest` 中与导航/自动跳 HOME 相关的断言、以及 lifecycle
  instrumentation 的适用范围纳入资产处置。
- 01：Ended/Reconnecting 留 SESSION 的语义由本票映射；reducer 不再拥有自己的结束跳转。

### 证据

本票为决策票，未改动代码，故无 PASS/FAIL；事实来源为上列 `file:line` 与现有测试断言。
未建分支、未 commit。
