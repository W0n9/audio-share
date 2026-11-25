# 12: 现有 UI 资产与测试的处置：截图、Compose 测试与源码断言

Labels: wayfinder:grilling
Type: grilling
Status: resolved
Blocked by: 04, 10

Preconditions from [10](10-ui-implementation-scope.md#answer)：文案迁入 `strings.xml` +
`values-zh-rCN`；错误指引按 code 映射资源；SETTINGS 为只读页；a11y 断言（逐位 SAS、
liveRegion、48dp、200% 字体）纳入测试资产处置。
注意 `ForegroundServiceContractTest` 等直接读源码并断言 Kotlin 字面量的用例：
通知渠道名/正文资源化后这些断言会失效，需一并处置。
Preconditions from [11](11-pairing-outcome.md#answer)：`awaitCredential` 轮询删除、
`CredentialStored` 不再触发 `ConnectStored`、`PairingCompleted` 一次性交接的测试
纳入保留/重写清单。

## Question

方向 A 选定后，决定 `android-client-v2` 现有 UI 资产哪些保留、哪些重写、哪些删除，
使重构后的验证与 beta 证据边界清楚（沿用 `docs/agents/evidence-policy.md`）：

1. **Compose 测试**：`app/src/test/.../AudioShareAppTest.kt` 的断言依赖 `testTag` 和
   当前布局文案；方向 A 改版后哪些用例原样保留、哪些必须重写、哪些应删除。纯 reducer
   测试（`AppStateReducerTest`）是否只需补 `SessionOutcome` 的场景。
2. **截图与手工验收证据**：beta-spec 与 `.scratch/wire-protocol-v2/evidence/` 中引用的
   UI 截图/步骤，哪些必须按方向 A 重拍，哪些作为 beta 历史证据冻结、不继承（参照
   「Beta 冻结点与重构分支的合并机制」）。
3. **证据边界**：每类检查落在 JVM/Robolectric、`EMULATOR` 还是真机，并给出方向 A 的
   最小验收矩阵。

a11y 与 i18n 的决策在工单 10，本票不重复。

产出是资产处置清单与验收场景，不是实现。

## Answer

**结论（用户逐条确认）：按文件处置现有测试；源码扫描只保留结构/seam 不变量；旧截图全部冻结；
新 UI 测试用 Robolectric 语义 + `EMULATOR` 截图，不引入 golden；a11y 用 JVM semantics +
`EMULATOR` 手工；验收矩阵按 JVM / `EMULATOR` / 真机分层。**

### 1. 逐文件处置

| 处置 | 文件 | 说明 |
|---|---|---|
| **保留** | `PairingPayloadTest`（10）、`QrPayloadDecoderTest`（2）、`ControlFixtureReplayTest`（9）、`MediaFixtureReplayTest`（9）、`MediaPlaybackTest`（9）、`CredentialStoreTest`（8）、`DerSignatureTest`（5）、`ReconnectPolicyTest`（7）、`SessionStartupPolicyTest`（3） | 纯逻辑/协议/播放，断言不改；只跟随 API 重命名 |
| **重写** | `AudioShareAppTest`（9） | 改为方向 A 五屏 + `strings.xml`/语义断言；按屏拆分，删除硬编码文案与旧布局 tag |
| **重写** | `AppStateReducerTest`（21） | 改为 `SessionOutcome` + 06 导航表 + 11 交接；删除 `sessionPhase`/`connectRequested` 场景 |
| **重写** | `ErrorGuidanceTest`（6） | 保留 `retryable`/`requiresRepairing`/`automaticRetry` 断言；标题/指引文案改由资源 key 测试 |
| **适配** | `PairingConfirmGateTest`（3） | 继续覆盖 32 的「QR 自动确认恰好一次」；SAS 不自动确认 |
| **删除/替换** | `SessionCommandExecutorTest`（4） | 01 删除 `SessionCommandExecutor` |
| **删除/替换** | `SessionStateRelayTest`（10） | 01 以 outcome store 取代 relay；由 store/policy 测试替换 |

### 2. 源码扫描式契约测试

- **保留**：manifest 权限、`foregroundServiceType="mediaPlayback"`、无 `RECEIVE_BOOT_COMPLETED`/
  boot receiver、`applicationId = dev.audioshare.v2`、版本号契约；以及「UI 只依赖
  `SessionController`，不直接调 JNI/QUIC/Keystore」的 seam 断言。
- **替换为行为测试**：对 `session?.stop()`、`MediaPlayer(NativeMediaBridge)`、类形状、
  通知字符串等实现字面量的断言；改为 driver/fake engine 的行为断言，或在 `EMULATOR`
  上验证通知与生命周期。
- 结论：源码扫描只用于「不能从 JVM 观察的结构事实」，不再用来证明实现形状。

### 3. 截图与 beta 证据

- `ticket-21`、`ticket-24`、`ticket-25`（93 张真机截图）、`ticket-32` 的截图与记录：
  **全部冻结为「重构前证据」**，不重拍、不改写、不搬移；不作为 post-refactor `PASS`。
- beta 文档与 beta-spec 不追溯更新。
- post-refactor 新截图写入 `evidence/ticket-08-refactor/emulator/`：五屏 × 关键状态 ×
  Light/Dark + zh-CN + 200% 字体；真机不新增截图基线。

### 4. UI 测试分层

- **JVM/Robolectric**：五屏 must-have/空态各一条 Compose 测试；08 的 17 条中可在 JVM
  断言的部分；`strings.xml` en/zh key 对齐；a11y semantics。
- **`EMULATOR`**：真实渲染截图、zh-CN、200% 字体、旋转/进程死亡、API 26 smoke。
- **真机**：只跑 08 的四项关键路径；不做像素对比。
- **不引入**截图 golden 基础设施（Roborazzi/Robolectric screenshot 对比等）。

### 5. a11y 测试深度

- JVM semantics 断言：icon-only 按钮 `contentDescription`、SAS 逐位朗读、`liveRegion`、
  触控目标 ≥48dp、焦点顺序。
- `EMULATOR` 手工：TalkBack 实际播报、200% 字体、zh-CN、RTL、对比度观感。
- 不引入第三方 a11y 扫描依赖。

### 6. 方向 A 的最小验收矩阵

| 层级 | 内容 |
|---|---|
| **JVM** | 五屏状态/空态；outcome→屏幕；store 不变量；11 交接；资源 key 对齐；a11y semantics |
| **`EMULATOR`** | 签名 APK 安装；五屏关键状态 Light/Dark 截图；zh-CN；200% 字体；旋转/进程死亡；API 26 smoke |
| **真机** | 08 的四项：QR+SAS 配对、5–10 分钟可听播放、锁屏 2–5 分钟、断网恢复 + Stop |

### 7. 测试归属（谁写/改）

- 01：`SessionOutcomePolicyTest`、outcome store 测试；删除 `SessionStateRelayTest`、
  `SessionCommandExecutorTest`。
- 06：reducer 导航与恢复场景（并入 `AppStateReducerTest` 重写）。
- 07：`SessionLifecycleDriverTest`（JVM 时序）。
- 10：五屏 Compose 测试、资源 key 对齐、a11y semantics。
- 11：`PairingCompleted` 交接、配对期间不发 outcome、恢复规则。
- 12：只定义处置，不实现测试。

### 证据

本票为决策票，未改动代码，故无 PASS/FAIL；事实来源为当前测试文件与证据目录
（HEAD `cb2faf1`）。未建分支、未 commit。
