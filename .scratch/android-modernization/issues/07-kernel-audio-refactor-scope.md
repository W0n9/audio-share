# 07: 内核/音频内部重构范围

Labels: wayfinder:grilling
Type: grilling
Status: resolved
Blocked by: 01

Preconditions from [05](05-di-and-modularization.md#answer)：Service 只依赖 `SessionEngineFactory`；
`ClientSession` 是默认实现。本票可决定内部拆分、`Scheduler` 与 `MediaPlayer` 构造方式，
但不得扩大 Service 的依赖面，也不得要求引入 DI 框架。

## Question

在「不改 Wire Protocol、不改播放语义、不碰 M5」的前提下，决定内核 / 音频轴进入本图的重构项。架构审查给出的候选：

- **候选 05**：会话驱动的可测性——stats loop、network callback、`tryRebind` 的时序与所有权无法在 JVM 构造；纯策略函数被覆盖，真实调用点没有。
- **候选 06**：Transfer Format 能力与 ring 契约的单一来源——Rust 闸门 / Rust 映射 / Kotlin sink / S24 注释四处陈述；Kotlin 的 U8 / S32 分支在生产路径不可达。
- **候选 07**：Rust `session.rs` 的六种变更理由与 `jni.rs` 的零抽象——连接生命周期、手写 JSON 投影、媒体环交接、网络迁移、统计格式化、JNI 编组混在一起；错误塌缩为 `0` / `""` / `-1`。

逐项决定：进入本图、延后，还是排除；给出风险、顺序与不做的理由。特别注意与 01 的重叠：会话驱动的重连 / 关闭部分可能已经被 01 吸收，不要重复开票。

## Answer

**结论（用户逐条确认）：候选 05 窄范围进入；候选 06 仅做 Kotlin 侧清理；
候选 07 排除出本图；01 的二阶段 native 表示法仍归 01；driver 先有 JVM 测试再改
`ClientSession`。**

### 候选 05：进入本图（窄范围）

把 `ClientSession` 的**编排**抽成 JVM 可测的 `SessionLifecycleDriver`；`ClientSession`
退化为持有 Android 对象的 adapter。行为不变，不改 Wire Protocol、不改播放语义。

端口（全部由 adapter/fake 实现）：

| 端口 | 职责 |
|---|---|
| `NativeSession` | create/connect/stop/destroy/getState/getStats/rebind/getPairingState/getCredentialSummary |
| `NetworkMonitor` | 注册/注销网络回调，投递 available/lost 事件 |
| `Scheduler` | `postDelayed` 与循环 sleep，测试用虚拟时间 |
| `MediaController` | attach/start/stop；`MediaPlayer` 由 adapter 构造（05 的前置条件允许） |
| `OutcomeSink` | 01 定义的代际 sink，driver 只发布 Live/Reconnecting/Ended |

driver 负责的时序与所有权：

- 创建失败 / connect 被拒 → backoff 重试（复用 `ReconnectPolicy`）。
- 握手期 network available → 不 rebind（`SessionStartupPolicy` 判定保持不变）。
- rebind 成功 → 继续；失败 → teardown + backoff fallback。
- STOP 与 create/重连交错 → 旧代际任务丢弃，不泄漏 native handle。
- 停止顺序：先停 `MediaController`，再 stop/destroy native。

不重复开票（与 01 的边界）：

- `SessionOutcomePolicy` 与 `ReconnectPolicy` 仍是唯一判断来源；driver 只消费，
  不重新分类关闭码、不复制可重连集合。
- `OutcomeSink` / store 由 01 拥有；07 不新增 outcome 类型。
- 01 的 native 第二阶段（`record_close` 顺序、`stop()` 记 701）仍归 01，07 不重做。

测试（先写后改）：

- JVM：创建失败→backoff、connect 拒绝→backoff、rebind 成功、rebind 失败→teardown、
  握手期网络事件、STOP 与 create 交错、媒体停止顺序、旧代际事件丢弃。
- `EMULATOR`：真实 `ConnectivityManager` 回调与 `fd` 交接。
- 真机：beta 关键路径（锁屏、断网重连）记 `PASS` / `NOT RUN`。

风险：并发所有权改动是本次重构中风险最高的一项；必须先让 driver 测试绿，再让
`ClientSession` 切换到 driver，不能反过来。

### 候选 06：进入本图（仅 Kotlin 侧清理）

- 删除 Kotlin `AudioSink.open` 中不可达的 U8（2）与 S32（5）分支
  （`MediaPlayer.kt:38-44`）：Rust 闸门只放行 F32LE(1)/S16LE(3)。
- 在 Kotlin 侧集中一张 supported-format 表，注明 Rust 闸门
  （`native/src/control.rs:1112`）是权威来源；保留 S24 注释（`MediaPlayer.kt:43`）并
  说明其与闸门的关系。
- **不修改 Rust 闸门、不扩大接受面、不改 Wire Protocol**；跨语言单一来源用
  “权威注释 + 现有 control.rs 拒绝测试”表达，不追求共享代码。

### 候选 07：排除出本图

- `native/src/session.rs`（1,601 行）与 `jni.rs`（532 行）承担连接生命周期、JSON 投影、
  媒体环、网络迁移、统计格式化、JNI 编组六类变更理由；全量分解风险最高，且对
  UI/架构轴没有杠杆。
- JNI 错误塌缩为 `0` / `""` / `-1` 影响的是可诊断性，而诊断导出已在地图 Out of scope。
- 拆分会同时触碰连接生命周期与 JNI 契约，需要大范围真机验证，收益不足以进入本图。
- 记录到 map 的 Out of scope，附理由与链接；本图不为其开工单。

### 顺序与门禁

1. 01（Kotlin outcome + 二阶段 native 表示法）。
2. 07/05：先补 driver JVM 测试 → 再抽端口 → 再让 `ClientSession` 切换。
3. 07/06：随手删除死分支与注释，与 05 同一轴收尾。
4. 之后进入 beta 冻结/合并流程（02）。

门禁：driver JVM 测试全绿是改 `ClientSession` 的前置条件；`EMULATOR` 覆盖真实网络
回调与 rebind；真机只覆盖 beta 关键路径并显式记录 `PASS` / `NOT RUN`。

### 对后续工单的约束

- 08：验证矩阵加入 driver 的 JVM 时序测试、`EMULATOR` 的 rebind/网络回调场景。
- 10：会话状态展示只依赖 01 的 `SessionOutcome`，不直接依赖 driver 或 native。
- 12：`MediaPlaybackTest`、`SessionStateRelayTest`（由 01 替换）与新增 driver 测试的
  归属纳入资产处置。

### 证据

本票为决策票，未改动代码，故无 PASS/FAIL；事实来源为上列 `file:line`。
未建分支、未 commit。
