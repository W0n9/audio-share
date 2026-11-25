# Android 客户端 UI 是否应改为 Tauri 以统一 UI 技术栈

调研日期：2026-11-24  
代码基线：分支 `integration/v0.6.3-gui-network`，提交 `9a86841907bc8a1ce49d8c72c321172c462abd9a`，调研 worktree `/home/w0n9/workspaces/audio-share-v063`（只读）[1]  
输出位置：主 worktree `docs/v0.6.0-curation` 分支的 `.scratch/research/`。本次唯一写入文件即本文。

**范围与边界**：本文只回答"是否应把 Android 客户端的 Compose UI 换成 Tauri（WebView + 前端）"。桌面服务端 GUI 已经是 Tauri 2.12.5 + Svelte 5 + Vite，其"是否改 GPUI"的评估结论是**不换**，见同目录 `gui-gpui-evaluation.md`；本文不重复、不推翻该结论，也不评估桌面端[2][4]。Tauri 只能替换 UI 层：前台 Service、`AudioTrack` 播放、`MediaSession`、通知、电池/后台限制、JNI 到 Rust 核心都必须继续由原生 Kotlin/Rust 实现，这不是可选项。

**方法**：本地只读核对源码与行数（本文所有本地行数均为本次实测，而非沿用旧文档）；外部只用一手来源——Tauri v2 官网文档及其 `tauri-docs` v2 分支源码、Tauri 官方 GitHub issue/PR、Android Developers 官方文档。每条外部事实附编号引用，末尾列可点击 URL 清单。未安装、未编译、未运行任何 Tauri Android 原型；所有"能否"类未实测结论按仓库 `docs/agents/evidence-policy.md` 标注为 NOT RUN 或注明来源属性。

---

## 结论（先给答案）

**不统一。保留 `android-client-v2` 的 Compose UI，不启动"Android 改 Tauri"的迁移。**

理由不是"Tauri 不能跑在 Android 上"——它能，而且是官方支持的一等平台[15][19]。决定性理由有四条：

1. **可共享的 UI 代码几乎不存在。** 桌面是两个输入框加状态展示的 420×640 服务端控制窗（含配对二维码的**展示**），手机是 Home / 扫码配对 / SAS 确认 / 会话态 / 错误引导 / 设置 / 关于的整套手机流程（含二维码**扫描**）。实测 Android v2 主源码 7,227 行中，纯 Compose UI 只有约 1.42k 行（20%），其余 80% 是与 UI 技术无关的 Service、播放、会话编排、JNI、签名与凭据[6][7]。"统一技术栈"最多统一这 20%，且这 20% 与桌面 UI 并不重叠。
2. **生命周期与本项目架构正面冲突。** 本项目的 `ControlSessionService` 是以 `mediaPlayback` 类型运行的前台服务，Activity 只是绑定它的薄壳；Tauri 官方 Android 插件文档确认一个窗口对应一个 `TauriActivity`，但**没有**关于"前台 Service 存活 + WebView 退到后台"的支持说明[16][18]。官方 issue #15671 报告的场景与本项目架构高度一致：FGS 保活时，Activity 被划掉后从启动器重进会得到永久空白的 WebView；即便用 `api.prevent_exit()` 阻止默认退出，仍无法恢复（issue 仍 open，报告者给出根因分析，未见维护者确认）[22]。这是风险，不是已证实的必然失败，但它落在最核心的会话连续性上。
3. **新增一层桥，而不是减少一层。** Tauri 官方文档明确写："Tauri doesn't directly provide a mechanism to call Rust from your plugin code"，并建议在 Android 上用 JNI 调用共享代码——**即使 WebView 挂起时**也可用[16]。也就是说 `ClientNative.kt` 这座 JNI 桥不会消失；Tauri 只会在其上再加一层 JS ↔ Rust ↔ Kotlin plugin 的 IPC。ADR 0002 里"in-process core 避免第二套 IPC"的理由在桌面成立，在 Android 反而不成立[5][16]。
4. **成本是确定的，收益是不确定的。** 迁移要重写约 1.4k 行 Compose UI、重接 binder 状态回放、重建 QR 扫描/通知/权限接线，并失去现成的 Robolectric/Compose UI 测试[6][7][14]；启动时间、内存、包体在本项目上**均未实测（NOT RUN）**，不能当作迁移理由[35]。

**什么条件下重新评估**：见第 6 节。当前不写"有条件统一"的折中方案——折中方案会掩盖上述第 2 条的真实阻塞风险。

---

## 1. 真正在权衡的是什么

需要先把两个伪命题去掉：

- **"Tauri 能不能跑 Android"不是问题。** Tauri v2 官方支持的平台包含 Android 与 iOS；移动端通过 `tauri android init` 生成一个 Android Studio 工程，用 `tauri::mobile_entry_point` 把启动代码放进 shared library，并可产出 AAB/APK，最低支持 Android 7.0（SDK 24）[15][19]。本项目 `minSdk = 26` 不构成门槛[14]。
- **"统一后可以删掉原生层"也不是问题。** Tauri 的 UI 是 WebView，插件层用 Kotlin/Java 写，Android 命令还被调度在主线程；官方特别警告长操作会造成 UI 冻结甚至 ANR[16]。前台 Service、音频输出、`MediaSession`、通知权限都不可能由 WebView 承担[29][30][34]。

因此唯一有效的权衡是：**共享 UI 代码的收益，是否大于 WebView 的开销、第二座 IPC 桥与生命周期风险。** 本文的答案是"不大于"。

---

## 2. Tauri v2 Android 的一手事实

### 2.1 支持现状与架构

- Tauri v2 架构文档说它直接用 WRY/TAO 做系统级调用，"HTML rendered in a Webview"，TAO 支持 Android；移动端入口被包成 shared library[15][19]。
- Android 工程由 Tauri 生成，但官方明确说"Tauri uses an Android Studio project under the hood"，官方发布流程也要求理解 Gradle、签名、Play Console[19]。换言之，选 Tauri **不会**消除 Android 原生工程知识需求。
- 移动端插件模板把实现拆成 `desktop.rs` / `mobile.rs`：桌面用 Rust 实现，移动侧发送消息给原生代码执行[16]。这意味着"一套 Rust 逻辑跑两端"在移动上并不自动成立。
- 官方插件已覆盖本项目所需的两类移动能力：notification（含 Android，Actions API 仅移动端）与 barcode-scanner（移动平台相机扫码，需 capability 权限）[20]。这是 Tauri 的**真实优点**，应在对比中承认；但后文本项目现有实现已经可用且有测试，替换属于净重写[6][14]。

### 2.2 WebView 版本与渲染一致性

- 官方 webview-versions 页面原文："Tauri uses the system Android WebView, which is based on Chromium." 且"Tauri does not bundle a WebView with your app"，"the runtime version depends on the device's currently selected WebView provider"；"On most production Android devices, WebView is an updatable system component"，但"Some Android images can use a different preinstalled provider"或允许在开发者设置中切换 provider，因此"web platform support is tied to that provider's Chromium/WebView version"[17]。
- 对照桌面端：Windows 是 WebView2、Linux 是 WebKitGTK，本来就已经是多引擎[4]。迁到 Tauri Android 是**再增加**第三种引擎/版本维度，而不是消除差异。
- 厂商定制系统的兼容性不是理论问题：官方 issue #7287 请求支持华为/HarmonyOS 的私有 WebView，2023-06 开、2026-11-05 仍有更新且仍 open[27]。对以中文用户和国产 ROM 为主要场景的项目，这是要计入的兼容性尾部风险。
- Android 官方 WebView 文档写明，WebView 基于 Chromium、有独立进程和 native code，"its baseline memory usage is higher than standard Android views"，且 WebView 从布局移除后仍持有内存，必须显式从父 `ViewGroup` 移除并调用 `destroy()` 以避免 Activity 泄漏和 OOM[35]。

### 2.3 启动与内存开销

- 官方 issue #12743（2025-02 开，2025-10 关闭）报告 Android 启动时出现黑/白屏闪烁；讨论中的可行 workaround 是**用 Kotlin 写原生 SplashScreen**，即必须补原生 UI 来遮 WebView 初始化过程[26]。
- 官方 issue #13554（open）报告从 APK assets 读取图片/音频时 WebView 资源拦截导致 OOM 崩溃，属于 WebView 资源路径上的真实问题[28]。
- 官方 Android WebView 文档也说明配置变更（旋转、IME 弹出/收起）会销毁 Activity 并重建 WebView，需要 `saveState`/`restoreState` 保状态[35]。

**证据边界**：本次**没有**本项目的 Tauri Android 冷启动时间、常驻内存或 APK 体积数据，全部为 **NOT RUN**。任何"WebView 更慢/更大"的定量说法在这里都不成立；能确认的只有"官方承认 WebView 基线内存高于标准 Android View"[35]。

### 2.4 与原生 Service 通信、生命周期（决定性）

这是与本架构最相关的一节。

- Tauri 移动多窗口文档说明 Android 的多窗口基于 Activity Embedding，"Each activity must extend `TauriActivity`"，多窗口要求 Android 12L（API 32）+[18]。这确认了 WebView 窗口与 Android Activity 的强绑定关系。
- 官方移动端插件文档列出可 hook 的生命周期事件为 `load`、`onNewIntent`（Android only）等；`onResume`/`onPause` 由 PR 8092 加入。官方 issue #14270 指出 `onCreate` 钩子在 PR 8092 中被移除且至今没有，2025-11 的评论仍称"没有找到 workaround"[16][23]。
- 同一文档还写明：Android 原生命令在主线程执行，长操作会导致 UI 冻结甚至 ANR；以及 Tauri 不提供从插件直接调用 Rust 的机制，需要 Android 上走 JNI[16]。
- **#15671（open，2026-07-07）**：在 Tauri 2.11.5 / tao 0.35.3 / wry 0.55.1、Android 14/15 模拟器上，报告者复现了"FGS 保活 + 从最近任务划掉 + 从启动器重进 → 纯白屏，无 WebView 创建、JS 不执行，二次重进也不恢复，只能杀进程"。报告者给出的三层根因包括：(a) 默认"最后一个窗口关闭即退出"会让 tao 调用 `std::process::exit(0)`，连带杀死 FGS，需要 `RunEvent::ExitRequested` + `api.prevent_exit()` 才能让进程存活；(b) tao 把一个 Activity 映射成一个 Window，而生成的 `WryActivity.onCreate` 用 `savedInstanceState ?: intent extras ?: hashCode()` 作为 id，划掉任务后重进两者都为空，新 Activity 得到新的 `hashCode()`，无法匹配已存的 WebView 属性，于是永远不创建 WebView；(c) `Event::Resumed`/`Suspended` 只按窗口派发，零窗口时 resume 被静默丢弃[22]。**属性说明**：这是报告者的复现与根因分析，issue 仍 open 且无维护者回复；它不能当作官方已确认的 bug，但它是与本项目架构同构的可复现失败路径，且官方目前没有文档说明相反的保证。
- 其他相邻风险：#16379（open，2026-10-22）报告在 `on_page_load` 调 `webview.url()` 会让 UI 线程冻 10 秒后 abort，Android 15 x86_64 模拟器 5/5 冷启动复现[24]；#16122（open，2026-09-23）报告 `startActivityForResult` 并发调用会覆盖 callback，导致先前的 invoke 永久挂起[25]。
- **官方文档空白**：Tauri 的调试页只描述桌面/WebInspector 的差异（Linux webkit2gtk、macOS Safari、Windows Edge DevTools），没有 Android 章节；Android 的 WebView 检查是通过 Chrome DevTools / Android Web Inspector，见 webview-versions 页[17][21]。

**结论**：Tauri 官方网站与文档仓库中，未找到"前台 Service 与后台 WebView 共存"的支持声明或设计说明；而 issue #15671 表明"进程活过 Activity"恰是 WebView 重建的薄弱点[17][21][22]。对把会话实现在 Service 里的本项目，这不是边角问题。

### 2.5 打包、签名与分发

- 官方 Google Play 文档：可 `tauri android build -- --aab` / `--apk`，默认打全 ABI，可用 `--split-per-abi`；首次上传必须在 Play Console 手动完成以验证签名和包标识；"Tauri currently does not offer a way to automate" Android release 创建流程[19]。
- 签名仍要求 Java Keystore、`keystore.properties` 与在生成工程 `app/build.gradle.kts` 里配 `release` signing config[19]。本项目现有 `build.gradle.kts` 已经有完整的外部 keystore/release signing 配置；Tauri 只是换一套 Gradle 工程来重写这些配置，而非省掉[14]。

---

## 3. Audio Share 的具体约束

### 3.1 两端 UI 形态与代码量（本次实测）

| 维度 | 桌面服务端 GUI | Android v2 客户端 |
|---|---|---|
| 技术栈 | Tauri 2.12.5 + Svelte 5.57.3 + Vite 8.3.6 | Compose + Material3，无 Navigation/DI 库 |
| 形态 | 420×640 单窗口，托盘/单实例/通知 | 全屏手机流程：Home、扫码/SAS 配对、确认、会话态、错误引导、设置、关于 |
| 代码量 | `gui/src` 3,502 行（Svelte 1,324 + TS 1,150 + CSS 1,028），另有 locale JSON 342 行（每语言 171 行） | 主源码 7,227 行 Kotlin；其中纯 Compose UI 约 1.42k 行 |
| 二维码 | 由 Rust 生成 svg，前端**展示** | CameraX + ZXing 本地**扫描**（无 Google Play Services） |
| 主要状态 | 服务端控制/设置/连接状态 | Service 持有的会话与配对快照，经 Binder 回放 |

[2][3][6][7][13][14]

关于行数差异：同目录 `gui-gpui-evaluation.md` 在提交 `ddeb939` 记录的是 2,783 行前端 + 310 行 locale；本文在 `9a86841` 实测为 3,502 + 342。差异来自分支演进，本文采用实测值，不沿用旧数[3][4]。

Android v2 主源码的构成（行数来自本次 `wc -l`）：

- 纯 UI：`AudioShareApp.kt` 1,344（内部约 30 个 Composable，含 Home/Pairing 各阶段/Session 各 Hero/Settings/About）+ `Navigation.kt` 45 + `AudioShareTheme.kt` 29 ≈ **1,418 行**。
- UI 状态机（不可简单丢弃）：`AppState.kt` 834（纯 reducer）、`UiStatePersistence.kt` 49、`UiPreferences.kt` 36。
- 必须保留的原生层：`MediaPlayer.kt` 908、`ControlSessionService.kt` 571、`session/`（LifecycleDriver 530 + Orchestrator 394 + Engine 38）、`ClientSession.kt` 369、`ClientNative.kt` 223、`ClientSigner.kt` 336、`CredentialStore.kt` 147、`QrScanner.kt` 112 等。
- 合计 7,227 行；因此**理论上可迁移上限约 20%**，而 `AppState` 若留在 Kotlin 侧，则前端只剩"渲染 + 事件转发"，共享价值进一步下降[6][7][12]。

### 3.2 原生层为什么不能跟着迁

- **前台服务**：Manifest 声明 `FOREGROUND_SERVICE`、`FOREGROUND_SERVICE_MEDIA_PLAYBACK`，`ControlSessionService` 的 `foregroundServiceType="mediaPlayback"`；`startForeground()` 传 `ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK`，并把 `MediaSession` token 挂到通知上[10][13]。Android 14（API 34）起必须为 FGS 声明类型并申请对应权限，`mediaPlayback` 类型的描述就是"Continue audio or video playback from the background"；Google Play 还要求在 Play Console 声明 FGS 类型[29]。
- **后台限制**：Android 8 起对后台 Service 施加限制，但 FGS 不受该限制；官方明确说"a service that plays audio should always be a foreground service"，且用 `startForegroundService()` 后必须在 5 秒内调 `startForeground()`，否则系统停止服务并判 ANR[30]。Android 12（API 31）起从后台启动 FGS 还有额外限制[31]。这些规则与 UI 技术无关。
- **音频**：当前播放路径是 `AudioTrack`（`MODE_STREAM`、`WRITE_NON_BLOCKING`、128 ms 缓冲、`AudioTrack.getTimestamp()` 漂移时钟、`setPlaybackRate` 变速率校正）——**不是 Oboe**[11]。Android 官方低延迟指南推荐 Oboe（API 27+ 内部走 AAudio）、低延迟 performance mode、exclusive sharing、48 kHz、data callback、避免回调内阻塞、double buffer；其对照表显示"全部推荐项"约 20 ms，而不开低延迟模式约 205 ms，且注明设备差异很大[32]。AOSP 的应用侧指南同样推荐 Oboe，Java 侧至少要用 `AudioTrack.PERFORMANCE_MODE_LOW_LATENCY`[33]。无论 UI 用什么，这条原生路径都必须保留。
- **MediaSession**：当前用的是平台 `android.media.session.MediaSession`，不是 Media3[10]。Android 官方说明 MediaSession 提供与外部控制器交互的通用通道（耳机按键、助手、Wear OS、系统媒体控件等），并明确"当在后台播放媒体时，MediaSession 和 player 需要放在作为前台服务运行的 `MediaSessionService`/`MediaLibraryService` 中"[34]。
- **JNI 核心**：`ClientNative.kt` 是到 Rust cdylib `audio_share_client_v2_native` 的低频 JNI 面（connect/auth/stop/state/stats/rebind），并记录了"JNIEnv 按值传递"的真机教训；包名/类名是 load-bearing 的 native 导出符号[12]。Tauri 官方建议移动插件调 Rust 仍走 JNI[16]，所以这座桥不会因迁移消失。

### 3.3 现有测试资产

Android v2 有一整套 Robolectric/Compose UI 测试（`DirectionAScreensTest`、`PairingScreenTest`、`SessionScreenDirectionATest`、`NavigationRestorationTest`、`AudioShareAppTest`、`StringResourceParityTest` 等）以及会话/重连/Fixture 回放测试[14]。改为 WebView 前端后，这部分 UI 测试无法复用，需要换成 Web 端测试（而 Tauri 官网的 Webdriver 测试文档主要面向桌面），这是迁移的隐性成本。

---

## 4. 收益与成本对照

| 维度 | 迁到 Tauri 的收益 | 迁到 Tauri 的成本 / 风险 | 判断 |
|---|---|---|---|
| 共享 UI 代码 | 桌面前端可理论上复用组件与样式 | 两端屏幕几乎无重叠；桌面 3.5k 行前端不能整体套到手机；实际共享上限约为字符串与设计 token | 收益接近零[2][3][7] |
| Rust 核心复用 | 不必再手写 JNI？ | 官方说插件不能直接调 Rust，Android 仍用 JNI；现有 `ClientNative.kt` 已可用且经过真机验证 | 不成立[12][16] |
| 原生能力 | notification / barcode-scanner 插件现成 | 现有 `QrScanner`/通知代码已可用且有测试；替换是重写；Android 命令在主线程，长操作 ANR | 中性偏负[14][16][20] |
| WebView 开销 | 无 | 基线内存高于标准 Android View；需 `destroy()` 防泄漏；provider 随设备变化；华为/HarmonyOS WebView 支持 issue 仍 open | 负[17][27][35] |
| 启动体验 | — | 启动闪烁需原生 Splash 掩盖（#12743 已关闭但 workaround 是原生代码） | 负[26] |
| 会话连续性 | — | #15671：FGS 保活 + 划掉任务重进得到永久白屏；官方文档对此无说明 | **重大风险，NOT RUN 在本项目** [22] |
| 调试与测试 | Chrome DevTools 可查 WebView | 官网 debug 页无 Android 章节；失去 Robolectric/Compose 测试；仍需 Android Studio/Gradle | 负[14][17][21] |
| 打包分发 | 官方生成 AAB/APK 流程 | 仍是 Android Studio 工程；release 创建自动化官方承认缺失；签名要重写配置 | 中性[14][19] |

---

## 5. 对 ADR 0002 与文档的影响

ADR 0002 选择 Tauri 的明确理由是：Rust 音频路径干净（`wasapi` / 官方 PipeWire bindings）、"Tauri has the most mature packaging and tray story"、in-process core 避免第二套 IPC[5]。逐条看：

1. **托盘、打包、单实例**是纯桌面议题，Android 上没有托盘，也不适用"AAB 打包比 GPUI 成熟"之外的比较——迁移目标是 Compose，不是另一个原生 UI 框架，Compose 的 Android 打包是平台默认能力。
2. **in-process core** 在桌面上指 GUI 与 core 链接进同一进程；在 Android 上现有设计本来就是 Service 进程内持有会话、JNI 到 Rust cdylib，与 UI 技术无关[10][12]。
3. 因此 **ADR 0002 继续有效，且不构成"Android 也应 Tauri"的理由**：它记录的是桌面服务端选型，没有覆盖 Android UI 栈。

建议（本文不执行）：若要把这个决定固化，应新写一份 Android UI 栈的 ADR，记录"保留 Compose + 拒绝 Tauri 统一"及第 6 节的重新评估触发条件；不要在 0002 上打补丁。本次未修改任何 ADR 或 GLOSSARY。

---

## 6. 建议与重新评估触发条件

**建议：不统一，维持 `android-client-v2` 的 Compose UI；桌面继续 Tauri + Svelte。**

这不是"两套技术栈无所谓"的妥协，而是因为在本项目里"统一"没有可兑现的收益：可替换的只是约 1.4k 行、且与桌面屏幕不重叠的 UI；同时要引入 WebView 内存/版本/启动成本、第二座 IPC 桥，并把最关键的会话连续性暴露在 #15671 那一类生命周期路径下[3][6][7][16][22][35]。

**重新评估的触发条件**（满足第 1 条且至少再满足一条时，才值得做原型；原型验收按第 7 节）：

1. **生命周期阻塞被解决**：Tauri 官方文档明确支持 FGS 与后台 WebView 共存，或 #15671 被修复并关闭、有可验证的"划掉任务后重进恢复正常 WebView"方案[22]。
2. **两端 UI 真正收敛**：手机端不再有扫码/SAS/会话 Hero 等桌面没有的屏幕，或出现一个跨桌面与手机的"同一套界面"产品要求。
3. **新增 iOS 客户端**：三端共享 Web UI 的收益才可能压过原生适配成本（并且仍需先解决第 1 条）。
4. **团队失去 Kotlin 维护能力**，而愿意接受原生 Service/音频/MediaSession 层继续外包或另招人——注意这条并不自动使 Tauri 划算，因为原生层不会消失。
5. **原型实测通过**：冷启动时间、常驻内存、任务移除后重进、FGS 保活、QR 扫描与通知链路全部达到当前 Compose 版本同等水平（证据标记 PASS，而非 EMULATOR 近似）。

---

## 7. 证据边界（本次未验证项）

以下均标 **NOT RUN**，没有本项目实测数据，不应作为决策依据：

- Tauri Android 冷启动时间、内存占用、APK/AAB 体积对比（无原型、无真机数据）[35]。
- #15671 的失败路径是否在本项目同样的 FGS + Binder 架构下复现——该 issue 报告于 Tauri 2.11.5 / 模拟器，本项目桌面侧用 Tauri 2.12.5，Android 侧尚未引入 Tauri[2][22]。
- 国产 ROM / 华为 WebView provider 下的渲染一致性；#7287 仍 open[27]。
- Tauri barcode-scanner 插件替换现有 CameraX + ZXing 的可行性与权限行为[13][20]。

这些缺口的方向很重要：它们全部是"迁移之后才可能变好/变坏"的未知量，而第 3、4 节的成本是迁移当下就确定要付的。

---

## 参考来源（URL 清单）

> 检索日期均为 2026-11-24。本地条目按仓库相对路径列出；外部条目均为一手来源，注明版本/日期或来源属性。

1. 本地基线：`integration/v0.6.3-gui-network` @ `9a86841`，worktree `/home/w0n9/workspaces/audio-share-v063`；证据标签规则见 `docs/agents/evidence-policy.md`。
2. 本地：`gui/src-tauri/Cargo.toml`（Tauri 2.12.5）、`gui/package.json`（Svelte 5.57.3、Vite 8.3.6、`@tauri-apps/api` 2.12.4）；另见 Tauri crates.io https://crates.io/crates/tauri
3. 本地实测：`gui/src` 3,502 行（Svelte 1,324 + TS 1,150 + CSS 1,028），`gui/src/locales/{en-US,zh-CN}.json` 各 171 行。
4. 本地：`.scratch/research/gui-gpui-evaluation.md`（分支 `docs/v0.6.0-curation`，提交 `ddeb939`；记录 2,783 行前端 + 310 行 locale，并声明 Android 客户端不在其范围内）。
5. 本地：`docs/adr/0002-server-rewrite-stack.md`（Rust / Tauri v2 / in-process core 的理由）。
6. 本地实测：`android-client-v2/app/src/main/java/dev/audioshare/v2/` 全部 Kotlin 主源码 `wc -l` = 7,227 行；`AudioShareApp.kt` 1,344、`AppState.kt` 834、`MediaPlayer.kt` 908、`ControlSessionService.kt` 571、`session/SessionLifecycleDriver.kt` 530、`session/SessionOrchestrator.kt` 394、`ClientSession.kt` 369、`ClientSigner.kt` 336、`ClientNative.kt` 223、`QrScanner.kt` 112、`AudioShareTheme.kt` 29、`Navigation.kt` 45。
7. 本地：`AudioShareApp.kt`（约 30 个 Composable：Home/Pairing 各阶段/Session Hero/Settings/About）、`Pairing.kt`。
8. 本地：`Navigation.kt`（`NavigationRestoration.reconcile`，Activity 重建时按 Service 回放的 outcome 校正屏幕）。
9. 本地：`MainActivity.kt`（`ComponentActivity` + `setContent { AudioShareTheme { AudioShareApp(...) } }`，只通过 `SessionController` Binder 与 Service 通信）。
10. 本地：`ControlSessionService.kt`（平台 `android.media.session.MediaSession`、`startForeground(..., FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)`、通知挂 mediaSession token）。
11. 本地：`MediaPlayer.kt`（`AudioTrack`、`MODE_STREAM`、`WRITE_NON_BLOCKING`、`getTimestamp()`、`setPlaybackRate` 漂移校正；128 ms 缓冲）。
12. 本地：`ClientNative.kt`（JNI 到 `audio_share_client_v2_native` cdylib；包名/类名对应 native 导出符号；JNIEnv 按值的真机教训）。
13. 本地：`android-client-v2/app/src/main/AndroidManifest.xml`（`FOREGROUND_SERVICE`、`FOREGROUND_SERVICE_MEDIA_PLAYBACK`、`CAMERA`、`POST_NOTIFICATIONS`；service `foregroundServiceType="mediaPlayback"`）与 `QrScanner.kt`（CameraX + ZXing，无 Google Play Services）。
14. 本地：`android-client-v2/app/build.gradle.kts`（minSdk 26 / targetSdk 36 / compileSdk 37、NDK 27.2.12479018、外部 keystore release signing）；`app/src/test/java/dev/audioshare/v2/` 下 Robolectric/Compose 测试；`app/src/main/res/values{,-zh-rCN}/strings.xml`；`app/src/main/res/values{,-night}/themes.xml`；另 `android-app/`（legacy Compose + Navigation Compose + ViewModel，主源码 3,103 行）。
15. Tauri v2 Docs, "What is Tauri?" / "Prerequisites" / "Tauri Architecture" — WebView、TAO 支持 Android、Android SDK/NDK 前置条件。 https://v2.tauri.app/start/ · https://v2.tauri.app/start/prerequisites/ · https://v2.tauri.app/concept/architecture/
16. Tauri v2 Docs, "Mobile Plugin Development"（源码 `tauri-docs` v2 分支 `src/content/docs/develop/Plugins/develop-mobile.mdx`）— Kotlin 插件 API、`desktop.rs`/`mobile.rs` 拆分、Android 命令在主线程且长操作会 ANR、"Tauri doesn't directly provide a mechanism to call Rust from your plugin code"、Android 用 JNI（"even when the application WebView is suspended"）、`onNewIntent`/`load` 生命周期。 https://v2.tauri.app/develop/plugins/develop-mobile/ · 源码 https://github.com/tauri-apps/tauri-docs/blob/v2/src/content/docs/develop/Plugins/develop-mobile.mdx
17. Tauri v2 Docs, "Webview Versions" — 使用系统 Android WebView（基于 Chromium）、不随包分发、provider 随设备变化且可切换、用 Chrome DevTools / Android Web Inspector 检查。 https://v2.tauri.app/reference/webview-versions/
18. Tauri v2 Docs, "Multi-Window on Mobile" — Android 多窗口基于 Activity Embedding，每个窗口的 Activity 必须继承 `TauriActivity`，要求 Android 12L（API 32）+。 https://v2.tauri.app/learn/mobile-multiwindow/
19. Tauri v2 Docs, "Google Play" / "Sign Android" / "Upgrade from Tauri 1" — 生成 Android Studio 工程、AAB/APK、最低 SDK 24、`tauri::mobile_entry_point` shared library、release 创建自动化缺失、Keystore/`keystore.properties`/Gradle 签名。 https://v2.tauri.app/distribute/google-play/ · https://v2.tauri.app/distribute/sign/android/ · https://v2.tauri.app/start/migrate/from-tauri-1/
20. Tauri v2 Docs, "Notification Plugin" / "Barcode Scanner Plugin" — Android 支持、Actions API 仅移动端、扫码插件需 capability 权限。 https://v2.tauri.app/plugin/notification/ · https://v2.tauri.app/plugin/barcode-scanner/
21. Tauri v2 Docs, "Debugging" — WebInspector 按平台区分（Linux webkit2gtk / macOS Safari / Windows Edge DevTools），页面无 Android 章节；inspector 默认仅在 dev/debug 构建启用。 https://v2.tauri.app/develop/debug/
22. GitHub, tauri-apps/tauri issue #15671 "[bug] Android: blank webview when relaunching the app after task removal while a foreground service keeps the process alive"，2026-07-07 开，仍 open；报告者复现于 tauri 2.11.5 / tao 0.35.3 / wry 0.55.1、Android 14/15 模拟器，含 `prevent_exit`、Activity id 与 `Event::Resumed` 派发的根因分析（报告者分析，未见维护者确认）。 https://github.com/tauri-apps/tauri/issues/15671
23. GitHub, tauri-apps/tauri issue #14270 "Add onCreate lifecycle hook to Android plugin"（2025-10-09 开，仍 open；提及 PR 8092 加入 onResume/onPause 但移除 onCreate）与 PR #8092。 https://github.com/tauri-apps/tauri/issues/14270 · https://github.com/tauri-apps/tauri/pull/8092
24. GitHub, tauri-apps/tauri issue #16379 "Android: `webview.url()` from `on_page_load` freezes the UI thread for 10 s, then aborts"，2026-10-22 开，仍 open；Android 15 x86_64 模拟器 5/5 冷启动复现，tauri 2.12.4 / wry 0.57.0。 https://github.com/tauri-apps/tauri/issues/16379
25. GitHub, tauri-apps/tauri issue #16122 "[bug] [android] Activity callback may be lost, causing the invoke to hang"，2026-09-23 开，仍 open。 https://github.com/tauri-apps/tauri/issues/16122
26. GitHub, tauri-apps/tauri issue #12743 "Black/White screen flashes on android start"，2025-02-19 开、2025-10-23 关闭；评论中的 Android workaround 是 Kotlin 原生 SplashScreen。 https://github.com/tauri-apps/tauri/issues/12743
27. GitHub, tauri-apps/tauri issue #7287 "[feat] Add support for Huawei WebView / HarmonyOS"，2023-06-24 开、2026-11-05 更新，仍 open。 https://github.com/tauri-apps/tauri/issues/7287
28. GitHub, tauri-apps/tauri issue #13554 "When apk load images… from assets, memory overflow occurs"，2025-06-02 开，仍 open（WebView 资源拦截 OOM）。 https://github.com/tauri-apps/tauri/issues/13554
29. Android Developers, "Foreground service types" — Android 14（API 34）起必须声明 FGS 类型；`mediaPlayback` + `FOREGROUND_SERVICE_MEDIA_PLAYBACK`；"Continue audio or video playback from the background"；Play Console 声明要求。 https://developer.android.com/develop/background-work/services/fgs/service-types
30. Android Developers, "Background Execution Limits" — Android 8 后台 Service 限制、FGS 豁免、"a service that plays audio should always be a foreground service"、`startForegroundService()` 与 5 秒 `startForeground()` 超时 ANR。 https://developer.android.com/about/versions/oreo/background
31. Android Developers, "Restrictions on starting a foreground service from the background" — Android 12（API 31）+ 的限制与豁免、while-in-use 权限相关限制。 https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start
32. Android Developers, "Low latency audio"（Oboe/AAudio 清单：低延迟模式、exclusive、48 kHz、data callback、避免回调阻塞、double buffer；对照表 20 ms vs 205 ms；设备差异大）。 https://developer.android.com/games/sdk/oboe/low-latency-audio
33. AOSP, "Audio latency for app developers" — 推荐 Oboe；Java 侧 `AudioTrack.PERFORMANCE_MODE_LOW_LATENCY`；NDK、回调与非阻塞通信清单。 https://source.android.com/docs/core/audio/latency/app
34. Android Developers, "Control and advertise playback using a MediaSession" — MediaSession 是播放器与外部控制器（耳机按键、助手、Wear OS、系统媒体控件、Android Auto）的通用接口；后台播放时 MediaSession 与 player 必须置于作为 FGS 运行的 `MediaSessionService`/`MediaLibraryService` 中。 https://developer.android.com/media/media3/session/control-playback
35. Android Developers, "Build web apps in WebView" — WebView 基于 Chromium、有独立进程/native code、基线内存高于标准 Android View、必须显式 `destroy()` 防泄漏、配置变更会重建 WebView 并需 `saveState`/`restoreState`。 https://developer.android.com/guide/webapps/webview
