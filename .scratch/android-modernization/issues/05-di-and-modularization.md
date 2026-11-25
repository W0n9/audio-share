# 05: 依赖注入与模块化粒度

Labels: wayfinder:grilling
Type: grilling
Status: resolved
Blocked by: 01

## Question

在「Session outcome」深模块确定后，决定应用架构轴的两件事：

1. **DI**：Hilt、Koin，还是手写依赖传递？以 JVM 可测性与现有 Service / Activity 生命周期为准；不得引入抬高 minSdk 26 的依赖。
2. **模块化粒度**：保持单 app module，还是拆 feature modules？如果拆，边界按什么划（UI / session / native adapter / pairing），以及这样做能带来什么 leverage，而不只是目录变化。

同时决定测试 adapter 的形状：哪些依赖需要 fake，哪些通过真实 seam 测试。产出是决策与模块边界，不是实现。

## Answer

**结论（用户逐条确认）：手写组合根；保持单 `:app` module；按四轴增量建包；Service 依赖
`SessionEngineFactory`；测试替身手写为主；只收拢影响决策的环境依赖。**

### 1. DI：手写组合根，不引入框架

- 现有依赖图很小，且已经是构造器注入：`SessionController`（Binder 接口，
  `ControlSessionService.kt:25`）、`ClientSession.Listener`、`MediaPlayer.NativeMedia`、
  `AudioSink`、`CredentialStore`、`ClientSigner`、`UiPreferences`、
  `SessionStateRelay.Source`。
- **两个 composition root**：
  - `ControlSessionService`：构造 `SharedPreferencesCredentialStore`、`SessionStateRelay`、
    `PairingStateRelay`、`SessionEngine`（现状 `ClientSession`，
    `ControlSessionService.kt:323`）。
  - `MainActivity`：持有 `AppState`、`AppReducer`、`UiPreferences`、
    `SessionCommandExecutor`，通过 Binder 使用 `SessionController`。
- 不引入 Hilt：需要 KSP/注解处理，当前缓存里没有 Hilt/KSP，且会与工单 03 的
  AGP 9.5 / Kotlin 2.4 升级叠加兼容风险。不引入 Koin：运行时查找换不来编译期检查，
  对这个规模的图没有收益。不新增 `javax.inject` 注解。
- 规则：除两个 composition root 外，任何类不得在体内 `new` 有状态协作者；
  平台对象（Context、Handler、Looper）由根或 platform adapter 构造后注入。

### 2. 模块粒度：保持单 `:app`，不拆 Gradle module

- 规模与耦合不支持拆分：20 个 main 文件、6,386 行（main+test）；cargo/NDK 构建绑在
  `app/build.gradle.kts`；`ForegroundServiceContractTest.kt:15` 直接读 `src/main/...`
  相对路径；beta 并行分支会放大文件移动冲突。
- `:core` 纯 JVM 模块留作将来出现第二消费端时的选项；feature modules 与
  「大规模多模块重划」已在地图 Out of scope。
- **包结构（增量迁移，不做一次性全量搬包）**：
  - `dev.audioshare.v2.ui`：`AppState`、`AppReducer`、`UiEvent`/`AppCommand`、
    `SessionCommandExecutor`、`AudioShareApp`、`AudioShareTheme`、`QrScanner`。
  - `dev.audioshare.v2.session`：`SessionEngine`/`Factory`、`ClientSession`、
    `SessionStateRelay`、`MediaPlayer`、`MediaRingBuffer`、`ReconnectPolicy`、
    `SessionStartupPolicy`、`SessionConfiguration`、`CloseCode`、`PairingStart`。
  - `dev.audioshare.v2.pairing`：`Pairing`、`PairingPayload`、`SasCode`、
    `PairingStateRelay`、`ClientSigner`、`CredentialStore`、`PairingErrorGuidance`、
    `QrPayloadDecoder`。
  - `dev.audioshare.v2.jni`：Kotlin 侧适配器，目前是 `NativeMediaBridge`
    （从 `ClientSession.kt` 移入），将来只放不参与 JNI 符号名的适配代码。
  - **`ClientNative` 固定留在 `dev.audioshare.v2`，永不搬包**：Rust 侧导出的
    `Java_dev_audioshare_v2_ClientNative_*` 符号名依赖当前 package/class 名
    （`ClientNative.kt:9-10`），搬包等于改 native 导出，超出本票范围。也不要使用
    `native` 作为包名：它是 Java 关键字，`androidTest` 的 `.java` 文件无法引用。
  - 根包（platform）：`MainActivity`、`ControlSessionService`、`ServiceLifecycle`、
    `UiPreferences`、`AudioTrackSink`、`ClientNative`。
- 迁移规则：新文件直接进目标包；既有文件只在**本次重构必须触碰时**搬，单次 commit
  不混入无关文件移动。
- **可见性限制（更正）**：Kotlin 的 `internal` 只按 Gradle module 生效；在单 module 内
  它**不能**在编译期阻止跨包访问。实际约束来自：每个包只暴露一个门面接口、实现类不加
  `public`、代码审查；若日后漂移严重，再加一个源码扫描式架构测试或重新评估拆 module。
  不要在本轮声称 `internal` 已提供包级强制。

### 3. Service 与会话实现之间的 seam

```kotlin
interface SessionEngine {
    fun start(configuration: SessionConfiguration)
    fun stop()
    fun confirmPairing(): Boolean

    interface Listener {           // 即现有 ClientSession.Listener
        fun onStats(json: JSONObject)
        fun onStateChange(state: String)
        fun onLog(line: String)
        fun onPairingState(json: String) {}
        fun onCredential(json: String) {}
    }
}

interface SessionEngineFactory {
    fun create(listener: SessionEngine.Listener): SessionEngine
}
```

- `ClientSession` 成为默认实现；Service 只依赖工厂，配对编排（QR 解析、身份创建、
  `SessionConfiguration`、凭据持久化）留在 Service。
- 接口只保留 Service 当前真正调用的三个成员：`start`（`ControlSessionService.kt:337`）、
  `stop`（:371）、`confirmPairing`（:160），外加 `Listener`。`stateSnapshot` 来自
  `SessionStateRelay`，`sessionToken` 来自平台 `MediaSession`（:444），都不属于本接口。
  01/07 若需要新成员，必须由实际调用点证明后再加，不预先铺开。
- 与 01 对齐：01 用 `OutcomeSink` 取代 `Listener.onStateChange`；最终
  `SessionEngineFactory.create` 接受 `OutcomeSink` + 配对/日志回调，不改变本票的
  「Service 只依赖工厂」决策。配对阶段是否发布 outcome 由 11 决定（不发布）。
- `MediaPlayer` 仍由 `ClientSession` 内部创建（`ClientSession.kt:194`）；是否把它提升为
  注入依赖由工单 07 决定，05 不预设。

### 4. 测试替身策略

- 手写 fake 为主；mockk 只留给无法手写的第三方/平台边缘。
- 固定替身：`FakeSessionController`（已有）、`FakeSessionEngine` + `FakeSessionEngineFactory`
  （新增，用于 Service 编排）、`FakeAudioSink`（已有）、`InMemoryCredentialStore`、
  `FakeUiPreferences`、`FakeSigner`。
- 不 fake 的纯策略（用真实实现 + golden vectors）：`AppReducer`、`ReconnectPolicy`、
  `SessionStartupPolicy`、`ErrorGuidanceBook`、`PairingPayload`、`SasCode`、
  `CredentialCodec`、协议编解码。
- 测试层级：reducer/策略/编排 = JVM；Compose 屏 = Robolectric；Service 生命周期与
  真实 Binder = instrumentation（`androidTest`）；JNI/音频真实路径 = `EMULATOR`/真机。

### 5. 环境依赖

- `Random`：已按参数注入（`ReconnectPolicy.backoffDelayMs(attempt, random)`），保持。
- `Clock`：只在工单 01/07 需要确定性重连/outcome 时序时引入，05 不提前加。
- `Context`：只允许出现在 platform adapter 的构造器（stores、`AudioTrackSink`、
  Service、`ClientSession` 的 ConnectivityManager 部分），不进入纯策略、reducer、ui 层。
- `Handler`/`Looper`：platform 层持有，接口只暴露回调；是否抽 `Scheduler` 留给 07。

### 对后续工单的约束

- 06：导航只依赖 `ui` 包的 `AppScreen`/`AppState`，不得直接触碰 `session`/`native`。
- 07：`SessionEngine` 接口是既定 seam；内部 stats loop、network callback、`tryRebind`
  的拆分与 `Scheduler`/`MediaPlayer` 构造方式由 07 决定，但不得扩大 Service 的依赖面。
- 08：验证矩阵需覆盖 factory 注入下的 Service 编排测试，以及现有 fake 的复用。
- 03：本决策不引入注解处理，工具链升级路径不变。
- 12：现有测试替身的保留/重写纳入其资产处置清单。

### 证据

本票为决策票，未改动任何代码，故无 PASS/FAIL；事实来源为上列 `file:line` 与构建/缓存核对。
未建分支、未 commit。
