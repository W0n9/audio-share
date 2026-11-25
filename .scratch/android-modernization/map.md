# Android Client 现代化 — map

Labels: wayfinder:map
Status: complete

Spec: [spec.md](spec.md)

## Destination

产出一份**决策完整的 `android-client-v2` 现代化方案**：UI/交互、应用架构、内核/音频、构建/工具链四轴各有可验证的验收项；beta 并行边界、分支与合并机制、证据标准、推进顺序全部落定；可直接交给 `/to-spec` → `/to-tickets` → `/implement`。

## Notes

- 对象：只现代化 `android-client-v2`。v1 `android-app` 保持冻结；「合并成单一 app」是本图之外的独立决策。
- 领域与决策：`GLOSSARY.md`、`docs/adr/0004-wire-protocol-v2-transport.md`。架构语言用 `/codebase-design` 的 module / interface / depth / seam / adapter / leverage / locality。
- 硬约束：不改 Wire Protocol 行为；不改 beta-spec 验收范围；minSdk 26 不得抬高；v1 fixtures 与冻结的 v0.3.4 APK 不动。
- beta 并行：独立 worktree + 分支；默认从当前 `integration/wire-protocol-v2-beta` 切出，beta 验收证据冻结后再合入；精确机制由「Beta 冻结点与重构分支的合并机制」决定。
- 证据：沿用 `docs/agents/evidence-policy.md`。JVM / `EMULATOR` 为主，真机只覆盖 beta 关键路径（锁屏、断网重连、配对）。
- 顺序：架构先行。`Session outcome`（架构审查候选 01 + 04 合并）是第一张 grilling 票；之后其余架构 seam → UI → 构建。
- 新功能：不预先排除；每张轴票在决议时明确是否带入新功能，且任何新功能不得改变 beta 验收范围。
- 架构审查报告（临时文件，可能已失效）：`/tmp/architecture-review-20261116-171652.html`；候选 01 + 04 的完整证据摘要已写入「Session outcome」工单正文，解析不依赖该 HTML。
- 工单类型：`grilling` / `prototype` 为 HITL，`research` / `task` 为 AFK；`wayfinder:` 标签是唯一标签，不使用 triage 标签。

## 推进顺序

1. [Beta 冻结点与重构分支的合并机制](issues/02-beta-freeze-and-merge-point.md#answer)：worktree / 分支 / 冻结锚点准备。
2. [Session outcome：关闭归属与重连语义的深模块](issues/01-session-outcome-module.md#answer)。
3. [构建升级落地、CI 与 baseline profile 范围](issues/09-build-ci-baseline-profile.md#answer) 阶段 0（16 KB + CI SDK）并行。
4. [依赖注入与模块化粒度](issues/05-di-and-modularization.md#answer) → [内核/音频内部重构范围](issues/07-kernel-audio-refactor-scope.md#answer)（driver 先写测试再改 `ClientSession`）。
5. [导航模型与进程死亡后的状态恢复](issues/06-navigation-and-state-restoration.md#answer) + [配对终局是否复用 Session outcome 骨架](issues/11-pairing-outcome.md#answer)。
6. [UI 实现范围：屏幕清单、M3 组件与 a11y/i18n](issues/10-ui-implementation-scope.md#answer)。
7. [构建升级落地、CI 与 baseline profile 范围](issues/09-build-ci-baseline-profile.md#answer) 阶段 1–3。
8. [现有 UI 资产与测试的处置](issues/12-existing-ui-assets-disposition.md#answer) 的清单随各票执行；25 GO + 冻结后跑 [重构验证与 beta 证据保全](issues/08-refactor-verification-and-beta-evidence.md#answer)，通过后合入。

## Decisions so far

- [Beta 冻结点与重构分支的合并机制](issues/02-beta-freeze-and-merge-point.md#answer)：原定从 `d72b050` 切 `refactor/android-modernization`（分支点已因 beta 推进到 `cb2faf1` 改为冻结 tip，见该票 Comments）；冻结 = 25 go + 本地 annotated `v0.5.0-beta` tag；beta 只收阻塞修复并单向 merge forward；`refactor/android-*` → 集成分支 → beta（`--no-ff`，禁 squash）；证据绑定 SHA + 产物哈希、不继承也不失效；每轴独立 worktree 与 `CARGO_TARGET_DIR`，禁改版本号。
- [Session outcome：关闭归属与重连语义的深模块](issues/01-session-outcome-module.md#answer)：`ClientSession` 分类、Service 持有唯一可重放 outcome store；`SessionOutcome = Live(LivePhase) | Reconnecting(attempt, lastCode) | Ended(UserStop | Surfaced(code))`；Stop 归因由调用点决定；删除 `sessionPhase`/`connectRequested`/`stopping`/`expectedStop`/`lastErrorCode`；重连期间停留 SESSION；native 表示法清理为第二阶段（不改协议）；配对终局另开 [11](issues/11-pairing-outcome.md)。Ended 的导航已由 06 覆盖为留 SESSION。
- [构建工具链升级目标与顺序](issues/03-toolchain-upgrade-targets.md#answer)：推荐 AGP 9.5.1 / Gradle 9.7.1 / Kotlin 2.4.21 / compileSdk 37 / targetSdk 36 / Compose BOM 2026.09.00；16 KB ELF 对齐与 targetSdk 36 是两个独立发布阻塞；不升 targetSdk 37（`ACCESS_LOCAL_NETWORK`）。完整矩阵见 [research/toolchain-upgrade.md](research/toolchain-upgrade.md)。
- [现代 UI 方向：三版可对比原型](issues/04-modern-ui-direction.md#answer)：选定 A「状态优先」——每屏一个 hero + 一个主操作，失败/重连/结束原位呈现，Settings/About 走顶栏、不设 tab bar；原型与全矩阵截图在 [prototypes/ui-direction](prototypes/ui-direction/README.md)，不带入新功能。
- [依赖注入与模块化粒度](issues/05-di-and-modularization.md#answer)：手写组合根（`ControlSessionService` + `MainActivity`），不引入 Hilt/Koin/注解处理；保持单 `:app`，按 ui/session/pairing/jni 增量建包（`ClientNative` 固定留根包以免破坏 JNI 符号）；Service 只依赖 `SessionEngineFactory`（`start`/`stop`/`confirmPairing` + Listener），`ClientSession` 为默认实现；测试替身手写为主，`Random`/`Clock` 注入、`Context` 限 platform adapter；单 module 内 `internal` 不提供包级强制，靠门面接口 + 审查。
- [导航模型与进程死亡后的状态恢复](issues/06-navigation-and-state-restoration.md#answer)：保留 `AppScreen` 枚举（新增 SETTINGS），不引入 Navigation Compose；Ended/Reconnecting 留 SESSION、失败留 PAIRING、surfaced error 不自动回 HOME；配置变更只存 screen/address/sasInput 并从 Service 回放；进程死亡落 HOME、配对不续接、outcome 不落盘；Back 只导航，Activity finish 仍发 `ACTION_STOP`。
- [内核/音频内部重构范围](issues/07-kernel-audio-refactor-scope.md#answer)：候选 05 窄范围进入——抽 `SessionLifecycleDriver` + 端口（NativeSession / NetworkMonitor / Scheduler / MediaController / OutcomeSink），先写 driver JVM 测试再改 `ClientSession`；候选 06 仅删 Kotlin 死分支、Rust 闸门不动；候选 07（Rust `session.rs`/`jni.rs`）排除；01 的二阶段 native 表示法仍归 01。
- [重构验证与 beta 证据保全](issues/08-refactor-verification-and-beta-evidence.md#answer)：JVM 覆盖 outcome/store/driver/UI/恢复，`EMULATOR` 覆盖签名安装、配对、播放/锁屏/旋转/进程死亡/rebind/API 26，真机只跑四项关键路径；25 的 Windows/协议/v1/30 分钟证据保留为重构前证据、不继承；证据绑定精确 SHA + APK/Server 哈希写入 `evidence/ticket-08-refactor/manifest.tsv`；25 GO 并冻结后采集，缺项或 `FAIL` 不合入。
- [构建升级落地、CI 与 baseline profile 范围](issues/09-build-ci-baseline-profile.md#answer)：四阶段渐进（16 KB/CI SDK → AGP 8.13.2 桥接 → AGP 9.5.1 + built-in Kotlin/KGP 2.4.21 → compileSdk 37/targetSdk 36/Compose BOM），每阶段固定兼容性断言；构建 JDK 17、测试 JDK 21，不采用 JDK 25；新增 `android-ci` 只跑 JVM/构建/ELF/签名检查，不跑模拟器/真机；Baseline Profile / Macrobenchmark 排除；每阶段 `--no-ff` merge，失败 revert 到桥接阶段。
- [UI 实现范围：屏幕清单、M3 组件与 a11y/i18n](issues/10-ui-implementation-scope.md#answer)：按方向 A 的五屏 must-have/空态；静态 M3 跟随系统、无动态取色与用户主题设置；SETTINGS 只读信息页；文案资源化 en + zh-CN、错误指引按 code 映射资源；a11y 十项（逐位 SAS、liveRegion、48dp、200% 字体等）；暂停、动态色、多 Server 管理等明确延后。
- [配对终局是否复用 Session outcome 骨架](issues/11-pairing-outcome.md#answer)：不建 `PairingOutcome`，`PairingStateRelay` + `PairingUi` 是唯一所有者（30 已有 source ownership/replay）；只与 `SessionOutcome` 共享不变量、不共享实现；配对成功交接改由 Service 持久化后启动会话并推送一次性 `PairingCompleted`，删除 5 秒轮询；配对交换不进入 outcome store；恢复时 `Live` + PAIRING 无快照落 SESSION。
- [现有 UI 资产与测试的处置](issues/12-existing-ui-assets-disposition.md#answer)：保留纯逻辑/协议/播放测试；重写 `AudioShareAppTest`、`AppStateReducerTest`、`ErrorGuidanceTest`；删除 `SessionCommandExecutorTest`、`SessionStateRelayTest`；源码扫描只保留结构与 seam 不变量；旧截图冻结为重构前证据；新 UI 用 Robolectric 语义 + `EMULATOR` 截图，不引入 golden；验收矩阵按 JVM/`EMULATOR`/真机分层。

## Not yet specified

_None._ 四轴决议均未带入新功能（04 / 07 / 09 / 10）；延后项列在
[UI 实现范围](issues/10-ui-implementation-scope.md#answer) 的第 6 节。

## Out of scope

- v1 现代化、v1 功能迁移、合并成单一 app。
- Wire Protocol 行为变更（版本协商、传输、帧、错误码）。
- M5 抖动 / 漂移 / PLC / FEC、多 Client、诊断导出。
- F-Droid、应用内自动更新、开机启动、mDNS / STUN / TURN / relay。
- AAudio / Oboe / 低延迟音频引擎替换；大规模多模块重划（未选的「激进」范围）。
- Rust `native/src/session.rs` / `jni.rs` 全量重构与 JNI 错误模型改造：无 UI/架构杠杆，诊断导出已排除，风险最高；结论见 [内核/音频内部重构范围](issues/07-kernel-audio-refactor-scope.md#answer)。
- Baseline Profile / Macrobenchmark：需要专用 benchmark 构建与设备，对 beta 验收无贡献，留到 stable gate 之后；结论见 [构建升级落地、CI 与 baseline profile 范围](issues/09-build-ci-baseline-profile.md#answer)。
- v1 独立禁用开关、stable-gate 项。
