# android-client-v2 提高 minSdk 的收益、代价与验证边界

日期：2026-11-23
性质：只读调研 / 决策支持材料
范围：仅 `android-client-v2`。v1 `android-app`（minSdk 23）已冻结，不在本报告范围内。

> **范围与前提**
>
> - 现状：`minSdk = 26`（Android 8.0），`targetSdk = 36`，`compileSdk = 37`
>   （`android-client-v2/app/build.gradle.kts:101,121,122`）。
> - 仓库现有硬约束「minSdk 26 不得抬高」见
>   [`.scratch/android-modernization/map.md:16`](../map.md) 与
>   [`issues/03-toolchain-upgrade-targets.md:14`](../issues/03-toolchain-upgrade-targets.md)。
>   本报告**只为是否放开该约束提供依据，不改变现有决议**，也不修改任何构建文件、不提交代码。
> - 已有事实不重复调研：依赖/工具链矩阵见
>   [`research/toolchain-upgrade.md`](toolchain-upgrade.md)（Robolectric 测试侧最低 SDK 24；
>   Compose 1.12 在 API 26–28 仅 WCG 回退 sRGB；AAudio/Oboe 替换在 out-of-scope）。
> - 证据状态：本报告未改代码、未运行测试；所有验证建议均为 `NOT RUN`。
>   代码事实来自阅读，API 事实来自官方文档；无法直接核实的条目会标注「未核实」。

## 0. 结论速览

- **推荐：维持 `minSdk 26`，不升档。** 没有找到足以重开现有硬约束的收益：
  所有升档收益要么是删除 1–3 处兼容分支（纯维护性），要么是可选平台能力
  （SplashScreen、动态取色、per-app language、24/32-bit PCM 编码），
  要么是 targetSdk 强制项（与 minSdk 无关，且项目已处理）。
- **如果未来因外部原因必须升：推荐 29。** 29 是成本/收益最平衡的档位：
  能同时删掉 StrongBox 版本判断和 `startForeground` 的 Q 兼容分支，
  让 foreground service type 调用无条件化；代价约 2.6% 活跃设备，
  现有 API 31 真机与 API 35 AVD 仍可安装，只需补一个 API 29 的 floor AVD。
- **31 是现有验证设备的最高可用档，不是推荐档。** 真机正好是 API 31；
  升到 31 仍能安装，但 API 26 AVD 报废，beta-spec 的「API 26 runtime」证据失效。
- **> 31 不可接受。** 唯一真机（Android 12 / API 31）将无法安装 APK；
  `EMULATOR` 证据按 `docs/agents/evidence-policy.md` 不能替代可听音质、时延、
  underrun、MIUI 行为、真实网络切换等真机验收项，beta-spec 的真机验收无法完成。
- **音频不是升档理由。** AAudio 在 API 26 已可用；升 minSdk 不带来新的音频引擎能力；
  Oboe/AAudio 替换仍 out-of-scope。API 31 的 24/32-bit AudioTrack 编码只对未来的
  格式扩展有意义，当前 Rust 闸门只放行 F32LE/S16LE。

## 1. 背景与约束状态

### 1.1 当前构建事实

| 项 | 当前值 | 出处 |
|---|---|---|
| `minSdk` | 26（Android 8.0） | `android-client-v2/app/build.gradle.kts:121` |
| `targetSdk` | 36 | `android-client-v2/app/build.gradle.kts:122` |
| `compileSdk` | 37 | `android-client-v2/app/build.gradle.kts:101` |
| ABI | `arm64-v8a`（真机）+ `x86_64`（模拟器/CI） | `android-client-v2/app/build.gradle.kts:23-40` |
| NDK | `27.2.12479018` | `android-client-v2/app/build.gradle.kts:15-21`、`native/.cargo/config.toml` |
| native API wrapper | `android26-clang`（两 ABI、linker 与 `CC_*` 共四处） | `android-client-v2/native/.cargo/config.toml:7,19,28,30` |
| 签名 | V1 关闭，V2/V3 开启 | `android-client-v2/app/build.gradle.kts:104-115` |
| 前台服务 | `mediaPlayback` type + `FOREGROUND_SERVICE` / `FOREGROUND_SERVICE_MEDIA_PLAYBACK` | `android-client-v2/app/src/main/AndroidManifest.xml:8-9,20-23` |

补充：Client 身份签名所需的 Keystore EC P-256 在 API 26 已可用，
这也是当前 baseline 的安全理由之一（`docs/wire-protocol-v2/pairing.md:41`）；
升档不会带来身份签名能力的关键新增，StrongBox（API 28）只是可选的硬件后端。

### 1.2 约束状态（本报告不改变任何一条）

| 约束 | 出处 | 与升 minSdk 的关系 |
|---|---|---|
| 不改 Wire Protocol 行为 | `map.md:16` | 升 minSdk 本身不改协议；但不得借升档扩大格式/验收范围 |
| 不改 beta-spec 验收范围 | `map.md:16` | **任何升档都会改变最低 API 验收项**，需要单独决策 |
| minSdk 26 不得抬高 | `map.md:16`、`issues/03-toolchain-upgrade-targets.md:14` | 本报告的建议是**继续遵守**该约束 |
| Baseline 保持 Android 8 / API 26 | `docs/adr/0004-wire-protocol-v2-transport.md:19`（决策 5） | 升档会与已接受的 ADR 冲突，需要新的 ADR 取代 |
| beta 验收含 API 26 runtime | `.scratch/wire-protocol-v2/beta-spec.md:281` | 升档后必须改写为新的最低 API |
| beta 验收含一台真实 arm64/API 31 设备 | `.scratch/wire-protocol-v2/beta-spec.md:283-286` | **minSdk > 31 时该条无法执行** |
| API 26 runtime presence/installability 属 `EMULATOR` 可满足项 | `docs/agents/evidence-policy.md:30` | 升档后该项作废或改写 |
| `EMULATOR` 不可替代真机音频/MIUI/真实网络等 | `docs/agents/evidence-policy.md:34-43` | **minSdk > 31 时真机验收不可完成** |

`android-client-v2/README.md:92` 也写明测试工装「不要求提高 API 26 baseline」；
`docs/adr/0004-wire-protocol-v2-transport.md:19` 的措辞是「no dependency may raise it」。
这两处进一步说明：升 minSdk 不是一次普通构建调整，而是治理层面的决策。

## 2. 「现在就能做」与「升 minSdk 才解锁」

### 2.1 现在就能删：minSdk 26 下已经是死分支的三处 `SDK_INT >= O`

`ControlSessionService.kt` 中三处 `Build.VERSION.SDK_INT >= Build.VERSION_CODES.O`
在当前 `minSdk = 26` 下恒为真（`O` 即 API 26，
见 [Build.VERSION_CODES](https://developer.android.com/reference/android/os/Build.VERSION_CODES)）。
删除它们**不需要**提升 minSdk，也不改变 API 26+ 上的行为。

| 位置 | 当前判断 | 为什么是死分支 | 现在可做的清理 |
|---|---|---|---|
| `ControlSessionService.kt:163-167` | `SDK_INT >= O` 时 `startForegroundService`，否则 `startService` | minSdk 26 ⇒ 所有设备都 ≥ 26 | 直接调用 `startForegroundService(intent)`，删除 else |
| `ControlSessionService.kt:356-361` | `SDK_INT >= O` 时创建 notification channel | 同上 | 无条件调用 `createNotificationChannel(...)` |
| `ControlSessionService.kt:384-388` | `SDK_INT >= O` 时用 `Notification.Builder(this, CHANNEL_ID)`，否则用已废弃构造器 | 同上 | 无条件使用 channel 构造器，删除 else 与 `@Suppress("DEPRECATION")` |

验证建议（本报告未执行，状态 `NOT RUN`）：

- JVM：`./gradlew test -PnoNative=true`。仓库约定测试 JVM 用 JDK 21，生产字节码仍是 Java 17
  （`android-client-v2/app/build.gradle.kts:218-231`），与 minSdk 无关。
- `EMULATOR`：在 `audio-share-api26` 上安装、启动前台服务、检查通知与 Stop action；
  记录 AVD 名、API 26、x86_64、构建 flag，真机仍记 `NOT RUN`。
- 这些改动不触碰 manifest、native、签名或 Wire Protocol，风险低。

注意：`issues/07-kernel-audio-refactor-scope.md:68-76` 所说的「删除 Kotlin 死分支」
指的是 `MediaPlayer.kt` 的 U8/S32 不可达格式分支，**不包含**上面三处 SDK 判断。
因此这三处仍是独立的、现在就能做的清理项。

### 2.2 升档才解锁的现有代码清理：总量只有 1–3 处

把「升档后能删的现有兼容代码」单独列出来，收益非常有限。注意：这些平台 API 在
minSdk 26 下大多已经可以通过运行时版本判断/AndroidX 使用；升档的差别主要是
「无条件调用」和「删掉判断」，**不是新增用户可见能力**。

| 目标 minSdk | 额外可删的现有分支 | 位置 | 性质 |
|---|---|---|---|
| 28 | `preferStrongBox && SDK_INT >= P` 中的版本判断 | `ClientSigner.kt:193` | 删掉一个条件，StrongBox 仍要 catch 不可用异常 |
| 29 | `SDK_INT >= Q` 的 `startForeground` 兼容分支 | `ControlSessionService.kt:402-406` | 改为无条件 typed `startForeground` |
| 33 | `SDK_INT < TIRAMISU` 的提前 return | `MainActivity.kt:161` | 删除一行版本判断 |
| 34 / 35 / 36 | 无 | — | 没有新增可删分支 |

也就是说：即使一路升到 36，现有 Kotlin 代码也只多删这几行。
这还不包括升档本身带来的 AVD、native wrapper、验收文档改动。

### 2.3 音频：升档不带来新引擎，不是理由

- AAudio 是 Android O（API 26）引入的 C API；Oboe 是它的 C++ 包装，
  在 AAudio 不可用时回退 OpenSL ES
  （[AAudio 官方文档](https://developer.android.com/ndk/guides/audio/aaudio/aaudio)）。
  本项目当前使用 Java `AudioTrack`（`MediaPlayer.kt:198-233`），
  并且 M5 的缓冲/漂移/underrun 工作都是围绕 `AudioTrack` 做的；
  AAudio/Oboe 替换明确在 out-of-scope（`map.md:61`）。
- 升 minSdk 到 27–36 不会让 AAudio 出现「新的引擎能力」：26 已经可用；
  更高档位只可能提供 AAudio 的新属性/行为，而本项目并未使用 AAudio。
- API 31 新增 `AudioFormat.ENCODING_PCM_24BIT_PACKED` / `ENCODING_PCM_32BIT`
  （[AudioFormat](https://developer.android.com/reference/android/media/AudioFormat)），
  但当前 Rust 闸门只接受 F32LE(1) / S16LE(3)，
  Kotlin 也只映射这两种（`MediaPlayer.kt:133-147`）；接受 S24/S32 是
  **Wire Protocol / 验收范围变更**，不是升 minSdk 就能获得的现有代码简化。
- API 28 的 `AudioTrack.getMetrics()`（Android 9 媒体指标）可用于诊断，
  但诊断导出在 map 的 out-of-scope；且它是 API 调用，不是 minSdk 收益。

**结论：音频不构成任何档位的升档理由。**

## 3. 按候选 API 档位逐一核实

### 3.1 总表

| 档位 | 与本项目可能相关的「新解锁」 | 现有代码可删/简化 | 结论 |
|---|---|---|---|
| 27 | Autofill、`SharedMemory`、`WallpaperColors`、NNAPI 1.0 等 | 无 | **无实质收益** |
| 28 | StrongBox、bionic `getrandom`、`ImageDecoder`、`AudioTrack.getMetrics` | StrongBox 版本判断 1 处 | 仅一行死代码 |
| 29 | FGS types / typed `startForeground`、ELF TLS | Q 分支 1 处 + StrongBox 判断 | 少量兼容代码 |
| 30 | `WindowInsets.Type`、`WindowMetrics`、package visibility | 无 | **无实质收益** |
| 31 | SplashScreen、动态取色、24/32-bit PCM 编码、PendingIntent mutability | 无新增（28/29 已含） | 可选能力，不必须 |
| 32 | 12L 大屏 / taskbar / insets | 无 | **无实质收益** |
| 33 | POST_NOTIFICATIONS、LocaleManager/localeConfig、predictive back | 通知权限判断 1 处 | 一行死代码 + 可选语言能力 |
| 34 | FGS types 强制、`registerReceiver` flags 强制（均 targetSdk） | 无 | **无实质收益** |
| 35 | edge-to-edge 强制、16 KB page size（targetSdk/设备） | 无 | **无实质收益** |
| 36 | adaptive/edge-to-edge/predictive back（targetSdk） | 无 | **无实质收益，构建成本最高** |

先给一个总原则：**minSdk 只保证「该 API 在设备上存在」，不改变 targetSdk 决定的行为。**
下面每个档位都把「minSdk 新增」与「targetSdk 强制」分开写；
targetSdk 强制项的汇总见 §3.12。

### 3.2 API 27（Android 8.1）

**新解锁的 API/平台行为。** Android 8.1 的新增内容包括 Autofill 改进
（`BaseAdapter.setAutofillOptions`、`AutofillManager.notifyViewVisibilityChanged`）、
`SharedMemory`/`ASharedMemory`、`WallpaperColors`、`MediaMetadataRetriever.getScaledFrameAtTime`、
NNAPI 1.0、指纹错误码、Conscrypt 算法变化，以及「通知提示音每秒最多一次」的平台行为
（[Android 8.1 features](https://developer.android.com/about/versions/oreo/android-8.1)）。
`EditText.getText()` 从 API 27 起返回 `Editable`（向后兼容）。

**对本项目意味着什么。** 本项目没有表单 Autofill、没有跨进程共享内存、没有动态壁纸、
没有 ML、没有视频缩略图；UI 是 Compose，不直接依赖 `EditText.getText()` 的返回类型。
「通知提示音限速」是 Android 8.1+ 设备上的平台行为，**与 minSdk 无关**；
前台服务通知是 `IMPORTANCE_LOW`，也不依赖提示音。

**代码/构建/验证上的具体影响。** 没有可删分支、没有构建改动、没有新增验证项。
**对本项目无实质收益。**

### 3.3 API 28（Android 9）

**新解锁的 API/平台行为。**

- StrongBox Keymaster：`KeyGenParameterSpec.Builder.setIsStrongBoxBacked`
  **Added in API level 28**
  （[官方参考](https://developer.android.com/reference/android/security/keystore/KeyGenParameterSpec.Builder#setIsStrongBoxBacked(boolean))），
  以及 `StrongBoxUnavailableException`。设备支持时密钥在独立安全芯片中生成/使用。
- bionic 在 Android 9（API 28）新增 `getrandom()` / `getentropy()`
  （bionic status 页面把二者列在 P/API 28；本次直接抓取该页受网络限制，
  版本号按该页搜索结果标注，见文末来源）。
- `ImageDecoder`、`DisplayCutout`、`BiometricPrompt`（平台版）、
  `AudioTrack.getMetrics()`、APK Signature Scheme v3 等
  （[Android 9 features](https://developer.android.com/about/versions/pie/android-9.0)）。

**对本项目意味着什么。**

- **StrongBox 是唯一直接的代码收益**：`ClientSigner.kt:193` 当前写
  `if (preferStrongBox && SDK_INT >= P)`；minSdk 28 后 `SDK_INT >= P` 恒真，
  可删掉版本判断。仍然必须 catch `ProviderException`/不可用异常并回退到普通
  Keystore（代码已经这样做），所以这不是「StrongBox 变成必选」，只是少一个条件。
- **bionic `getrandom` 没有实质收益。** Rust 侧 `ring 0.17.14` 与 `rand_core 0.6.4`
  通过 `getrandom 0.2.17` 取随机数（`native/Cargo.toml:34-46`；
  `Cargo.lock` 的 `getrandom 0.2.17`；`cargo tree` 可复现）。该 crate 在
  aarch64/x86_64 Android 上默认走 `linux_android_with_fallback`：
  运行时 `dlsym("getrandom")`，不可用时回退 `/dev/urandom`
  （本机 cargo registry 的 `getrandom-0.2.17/src/linux_android_with_fallback.rs`）。
  对当前 release `.so` 的符号检查也显示：导入 `dlsym@LIBC` 和一个 **weak** `getrandom`，
  没有硬依赖 API 28 符号。因此升到 28 只是让 bionic 的 `getrandom` 可用、
  少走一次回退，**不增加能力，也没有可测量的收益证据**。
- `AudioTrack.getMetrics()` 属诊断能力，诊断导出 out-of-scope；
  `ImageDecoder`/`BiometricPrompt`/`DisplayCutout` 未使用或已由 Compose/AndroidX 处理。

**代码/构建/验证上的具体影响。**

- 代码：删除 `ClientSigner.kt:193` 的版本判断（约 2 行）。
- 构建：`native/.cargo/config.toml` 四处 `android26-clang` 改为 `android28-clang`
  （linker 与 `CC_*`），重新构建 `ring`/native，并重跑 16 KB 对齐断言
  （`llvm-readelf`、`zipalign -c -P 16`）。NDK 27.2 本地确实提供 `android28-clang`
  （已核对本机 NDK 目录），不需要升级 NDK。
- 验证：`audio-share-api26` 不再能安装；需要新增一个 API 28 的 floor AVD。
  真机 API 31 仍可安装；beta-spec 的「API 26 runtime」证据必须改写。
- **结论：收益仅一行死代码 + native wrapper 现代化；不显著。** 设备损失见 §4。

### 3.4 API 29（Android 10）

**新解锁的 API/平台行为。**

- **Foreground service types**：`android:foregroundServiceType` manifest 属性、
  `Service.startForeground(int, Notification, int)`（**Added in API level 29**，
  [官方参考](https://developer.android.com/reference/android/app/Service#startForeground(int,%20android.app.Notification,%20int))）、
  `ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK`
  （**Added in API level 29**，
  [官方参考](https://developer.android.com/reference/android/content/pm/ServiceInfo#FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)）。
- **ELF TLS**：官方 Android 10 NDK 说明，以 min API 29 构建的应用可以用 ELF TLS
  替代 `emutls`
  （[Android 10 features — NDK](https://developer.android.com/about/versions/10/features#ndk)）。
- 其他：scoped storage（targetSdk 29）、dark theme、`DnsResolver`、thermal API、
  playback capture、fdsan 等
  （[Android 10 features](https://developer.android.com/about/versions/10/features)）。

**对本项目意味着什么。**

- **唯一直接的代码收益**是 `ControlSessionService.kt:402-406` 的 Q 分支：
  minSdk 29 后可以无条件调用
  `startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)`；
  manifest 已经声明 `mediaPlayback`（`AndroidManifest.xml:23`），type 是 manifest 的子集，
  不会抛 `IllegalArgumentException`。同时 28 的 StrongBox 判断也可删。
- **ELF TLS**：理论上 native 可用更现代的线程局部存储实现，但当前 release `.so`
  的符号检查未发现 `__emutls_get_address` 导入，也没有性能测量；本报告不能证明
  升到 29 会带来可测量的运行时收益，标为「未量化 / 未核实」。
- **Compose 1.12 的 WCG 回退**：`toolchain-upgrade.md` 记录 Compose 1.12 在 API 26–28
  仅 WCG 回退 sRGB；升到 29 后这段回退不再可达。但本应用未使用宽色域，
  因此没有用户可见收益（仅少一个不再需要的兼容路径）。
- scoped storage / dark theme / DnsResolver / thermal 与本项目无关：
  应用只写 app-private 文件、已有自己的主题、QUIC 不依赖平台 DNS resolver、
  未使用 thermal API。

**代码/构建/验证上的具体影响。**

- 代码：删除 `ControlSessionService.kt:402-406` 的 Q 分支；顺带删除 StrongBox 判断。
- 构建：`native/.cargo/config.toml` 四处改为 `android29-clang`，重新构建 native + 16 KB 断言。
  NDK 27.2 本地提供 `android29-clang`。
- 验证：需要新增 API 29 floor AVD；API 35 AVD 与 API 31 真机仍可用；
  beta-spec 的 API 26 runtime 证据必须改写。
- **结论：少量兼容代码 + 更现代的 native wrapper；仍不显著。** 设备损失见 §4。

### 3.5 API 30（Android 11）

**新解锁的 API/平台行为。** `WindowInsets.Type`、`WindowInsetsController`、
`WindowMetrics`；package visibility（targetSdk 30）；一次性权限与权限自动重置；
APK Signature Scheme v2 强制（targetSdk 30）；压缩 `resources.arsc` 限制（targetSdk 30）；
heap pointer tagging 等
（[Android 11 features](https://developer.android.com/about/versions/11/features)、
[Android 11 behavior changes](https://developer.android.com/about/versions/11/behavior-changes-11)）。

**对本项目意味着什么。** Compose/AndroidX 已经处理 insets；项目不查询已安装应用列表、
不访问外部存储、没有自定义权限 UI；QUIC 走自己的 socket/证书路径，不依赖 package visibility。
`WindowMetrics` 等 API 在 AndroidX 中已有兼容实现，无需为了它们抬 minSdk。

**代码/构建/验证上的具体影响。** 没有可删分支、没有构建改动、没有新增验证项。
**对本项目无实质收益。** 设备损失 5.7%（§4），不抵任何收益。

### 3.6 API 31（Android 12）

**新解锁的 API/平台行为。**

- **SplashScreen**：Android 12 引入系统启动画面 API（`android.window.SplashScreen`）；
  AndroidX `core-splashscreen` 把同一体验带到 API 23
  （[SplashScreen 指南](https://developer.android.com/develop/ui/views/launch/splash-screen)）。
- **动态取色（Material You）**：Android 12 引入 dynamic color
  （[动态取色指南](https://developer.android.com/develop/ui/views/theming/dynamic-colors)）。
- **PendingIntent mutability**：`FLAG_IMMUTABLE` 自 API 23 存在；`FLAG_MUTABLE` 于 API 31 新增。
  从 Android 12（S）开始，创建 PendingIntent 必须显式指定 mutability
  （[PendingIntent 参考](https://developer.android.com/reference/android/app/PendingIntent)）。
- **AudioFormat**：`ENCODING_PCM_24BIT_PACKED` 与 `ENCODING_PCM_32BIT`
  **Added in API level 31**（[AudioFormat](https://developer.android.com/reference/android/media/AudioFormat)）。
- 其他：`Notification.Action.setAuthenticationRequired`、`setDevicePropertiesAttestationIncluded`
  （targetSdk 31）、audio focus fade、FGS 后台启动限制（targetSdk 31）、
  `android:exported` 强制（targetSdk 31）等
  （[Android 12 features](https://developer.android.com/about/versions/12/features)）。

**对本项目意味着什么。**

- **SplashScreen 不是升档理由**：当前主题是 `android:Theme.Material.Light.NoActionBar`
  （`res/values/themes.xml`），Android 12+ 会使用系统默认启动画面；
  若要做品牌化/退出动画，`core-splashscreen` 在 API 23 就能用，不需要抬 minSdk。
- **动态取色不是升档理由**：`AudioShareTheme.kt:24-26` 使用静态
  `lightColorScheme()`/`darkColorScheme()`；ticket 10 已明确「不做动态取色（API 31+）」
  （`issues/10-ui-implementation-scope.md:45`）。而且 `dynamicLightColorScheme` 可以在
  minSdk 26 下用运行时版本判断调用——升档只是删掉一个判断，产品决议仍然是「不做」。
- **PendingIntent 不是升档理由**：`ControlSessionService.kt:362` 已经使用
  `FLAG_UPDATE_CURRENT or FLAG_IMMUTABLE`，而 `FLAG_IMMUTABLE` 自 API 23 就有；
  Android 12 的强制是 targetSdk 31 行为，项目 targetSdk 36 已满足，无需改代码。
- **24/32-bit PCM 编码**：API 31 确实新增了 AudioTrack 编码常量，但当前 Rust 闸门
  只放行 F32LE/S16LE，Kotlin 也只映射这两种（`MediaPlayer.kt:133-147`）；
  要使用 S24/S32 必须改 Wire Protocol/格式接受面，属独立决策，**不是现有代码简化**。
- **`setAuthenticationRequired`**：可以让通知 Stop action 要求解锁；当前 beta 验证锁屏连续播放，
  通知 Stop 是锁屏/通知上的操作入口，要求解锁会使该路径在锁屏时失效，不应采用。
- **设备属性 attestation**：可选安全加固，不是必需。
- FGS 后台启动限制、audio focus fade、`android:exported` 等是 targetSdk 31 行为，
  项目已 targetSdk 36；与本项目相关的部分已处理或不受影响。

**代码/构建/验证上的具体影响。** 升到 31 没有新增可删分支（28/29 的两处已在前面）；
构建上 `android31-clang` 本地可用；验证上 **真机 API 31 正好成为 floor**，
`audio-share-api26` 报废；beta-spec 的 API 26 runtime 证据必须改写。
**收益是可选平台能力，不是必须；设备损失 11.8%。不推荐。**

### 3.7 API 32（Android 12L）

**新解锁的 API/平台行为。** 12L 主要是大屏系统 UI、taskbar、分屏、activity embedding、
media projection 缩放和 inset 变化
（[Android 12L summary](https://developer.android.com/about/versions/12/12L/summary)）。

**对本项目意味着什么。** Compose `Scaffold` 已经消费系统栏 insets；ticket 10 明确
横屏/平板专用布局延后；项目不用 media projection / activity embedding。
12L 没有解决本项目的任何现有问题。

**代码/构建/验证上的具体影响。** 无可删分支、无构建改动、无新增验证项。
**对本项目无实质收益。** 设备占比在现有数据中与 API 31 合并为 7.1%，无法单独拆分（§4）。

### 3.8 API 33（Android 13）

**新解锁的 API/平台行为。**

- **POST_NOTIFICATIONS 运行时权限**：Android 13（API 33）引入；targetSdk 33+ 的应用
  需要请求它，targetSdk ≤32 的应用由系统在首次创建通知渠道后弹窗
  （[通知权限指南](https://developer.android.com/develop/ui/views/notifications/notification-permission)）。
- **Per-app language**：`android.app.LocaleManager` **Added in API level 33**
  （[官方参考](https://developer.android.com/reference/android/app/LocaleManager)）；
  系统设置通过 `android:localeConfig`（或 AGP `generateLocaleConfig`）展示应用语言
  （[per-app languages 指南](https://developer.android.com/guide/topics/resources/app-languages)）。
- **Predictive back**：Android 13 引入，targetSdk 33 起可 opt-in，Android 16/targetSdk 36
  在支持设备上默认启用
  （[predictive back 指南](https://developer.android.com/guide/navigation/custom-back/predictive-back-gesture)、
  [Android 16 behavior changes](https://developer.android.com/about/versions/16/behavior-changes-16)）。
- 其他：themed app icons、Keystore/KeyMint 错误层级、ART JNI 加速、
  media controls 从 `PlaybackState` 派生（targetSdk 33）、granular media permissions、
  `NEARBY_WIFI_DEVICES` 等
  （[Android 13 features](https://developer.android.com/about/versions/13/features)、
  [Android 13 behavior changes](https://developer.android.com/about/versions/13/behavior-changes-13)）。

**对本项目意味着什么。**

- **POST_NOTIFICATIONS**：`MainActivity.kt:161` 的 `SDK_INT < TIRAMISU` 提前 return
  在 minSdk 33 后变成死分支，可删一行；manifest 已声明权限（`AndroidManifest.xml:7`）。
  这是唯一的现有代码收益。
- **Per-app language**：ticket 10 已完成 en + zh-CN 资源化
  （`res/values` + `res/values-zh-rCN`），并且明确「跟随系统语言；不做应用内语言选择器」
  （`issues/10-ui-implementation-scope.md:71-78, 96`）。系统 per-app language 可以在
  **不升 minSdk** 的情况下通过 `android:localeConfig`/AGP `generateLocaleConfig` 启用，
  只影响 Android 13+ 设备；`LocaleManager` 只有在做应用内语言选择器或
  `setOverrideLocaleConfig` 时才需要，而这两项都不在本轮范围。
  此外，官方指南说明：若要在 Android 12 及以下用 AppCompat 回退实现应用内选择器，
  Activity 必须继承 `AppCompatActivity`；本项目是 `ComponentActivity`
  （`MainActivity.kt:25`），需要引入 AppCompat 而不是抬 minSdk。**所以 per-app language
  不是升档理由。**
- **Predictive back**：manifest 已设置 `android:enableOnBackInvokedCallback="true"`
  （`AndroidManifest.xml:28`），Compose 使用 `BackHandler`
  （`AudioShareApp.kt:87`），targetSdk 36 已在支持的设备上默认启用；
  升 minSdk 不改变任何行为。**不是升档理由。**
- Themed icons、Keystore 错误层级、ART JNI 加速、media controls 等：
  可选或平台行为，不是 minSdk 收益。项目没有设置 `PlaybackState`，
  Android 13+ 会按 `MediaStyle` action 回退显示，不影响现有 Stop action。

**代码/构建/验证上的具体影响。** 删 `MainActivity.kt:161` 一行；构建上
`android33-clang` 本地可用；验证上 **真机 API 31 无法安装**（>31），
只能靠 API 35 AVD 的 `EMULATOR` 证据，真机音频/MIUI 验收不可完成。
**设备损失 18.9%（相对当前 API 26+ 基数约 19.1%）。不接受。**

### 3.9 API 34（Android 14）

**新解锁的 API/平台行为（基本都是 targetSdk 34 强制项）。**

- **FGS types 强制**：targetSdk 34+ 必须为每个前台服务声明至少一个 type
  （[Android 14 behavior changes](https://developer.android.com/about/versions/14/behavior-changes-14#fgs-types-required)）；
  `mediaPlayback` 需要 `FOREGROUND_SERVICE_MEDIA_PLAYBACK` 权限
  （[FGS types](https://developer.android.com/develop/background-work/services/fg-service-types)）。
- **`registerReceiver` export flags 强制**：targetSdk 34+，运行时注册的 receiver
  必须指定 `RECEIVER_EXPORTED`/`RECEIVER_NOT_EXPORTED`；仅注册系统广播时例外
  （[Android 14 behavior changes](https://developer.android.com/about/versions/14/behavior-changes-14#runtime-receivers-exported)）。
- 其他：implicit intent 限制、dynamic code loading 只读、partial photo access、
  MediaProjection 每次同意、JobScheduler 超时 ANR 等。

**对本项目意味着什么。** manifest 已经声明 `foregroundServiceType="mediaPlayback"`
与 `FOREGROUND_SERVICE_MEDIA_PLAYBACK`（`AndroidManifest.xml:9,23`）；
项目**没有** `registerReceiver`，也没有 `<receiver>`
（`ForegroundServiceContractTest.kt:34-41` 明确断言无 boot receiver）；
没有向未导出组件发 implicit intent、没有动态加载代码、没有照片/MediaProjection/JobScheduler。
所有相关要求都是 targetSdk 34 行为，项目 targetSdk 36 已满足，且**与 minSdk 无关**。

**代码/构建/验证上的具体影响。** 没有可删分支、没有构建改动；
**对本项目无实质收益。** 设备损失 29.8%（相对当前基数约 30.0%）；且真机无法安装。不接受。

### 3.10 API 35（Android 15）

**新解锁的 API/平台行为（targetSdk 35 或设备级）。**

- **edge-to-edge 强制**：targetSdk 35 的应用在 Android 15 上默认 edge-to-edge，
  必须处理 insets
  （[Android 15 behavior changes — edge-to-edge](https://developer.android.com/about/versions/15/behavior-changes-15#edge-to-edge-enforcement)）。
- **16 KB page size**：Android 15 起 AOSP 支持 16 KB 设备；targetSdk 35+ 的 64 位应用
  从 2027-02-01 起在 Google Play 上必须支持
  （[page sizes 指南](https://developer.android.com/guide/practices/page-sizes)）。
- 其他：dataSync/mediaProcessing FGS 6 小时超时、`BOOT_COMPLETED` 启动 FGS 限制、
  `SYSTEM_ALERT_WINDOW` 启动 FGS 限制、targetSdk 35 的 audio focus 限制、
  `Service.onTimeout`、TLS 1.0/1.1 禁用等。

**对本项目意味着什么。** edge-to-edge 已由 `enableEdgeToEdge()`（`MainActivity.kt:63`）
与 Scaffold insets 处理，`TargetSdkPlatformBehaviorTest.kt:46-54` 还断言了
`decorFitsSystemWindows == false`；16 KB 已由 `native/.cargo/config.toml:11-14,20-23`
的 `max/common-page-size=16384` 与 ticket 14 的证据覆盖；项目使用 `mediaPlayback`
而不是 dataSync/mediaProcessing；没有 boot-start、没有 overlay、没有请求 audio focus
（全仓 grep 无 `requestAudioFocus`）、没有平台 TLS。**这些都与 minSdk 无关，且已处理。**

**代码/构建/验证上的具体影响。** 没有可删分支、没有构建改动；
**对本项目无实质收益。** 设备损失 41.3%（相对当前基数约 41.6%）；真机无法安装。不接受。

### 3.11 API 36（Android 16）

**新解锁的 API/平台行为（targetSdk 36）。**

- **edge-to-edge opt-out 失效**：targetSdk 36 在 Android 16 上不能再用
  `windowOptOutEdgeToEdgeEnforcement` 退出 edge-to-edge
  （[Android 16 behavior changes — edge-to-edge](https://developer.android.com/about/versions/16/behavior-changes-16#edge-to-edge)）。
- **predictive back 默认启用**：targetSdk 36 在 Android 16+ 上默认启用，
  `onBackPressed` 不再被调用
  （[Android 16 behavior changes — predictive back](https://developer.android.com/about/versions/16/behavior-changes-16#predictive-back)）。
- **adaptive layouts**：targetSdk 36 在大屏（sw ≥ 600dp）忽略 orientation/resizability/
  aspect ratio 限制
  （[Android 16 behavior changes — adaptive layouts](https://developer.android.com/about/versions/16/behavior-changes-16#adaptive-layouts)）。
- Local Network Protection、MediaStore version lockdown、safer intents、health permissions 等。

**对本项目意味着什么。** 这三项 targetSdk 36 行为项目已经按现状处理：
`enableEdgeToEdge()`、`enableOnBackInvokedCallback="true"` + Compose `BackHandler`、
`resizeableActivity="true"` 且无方向锁（`TargetSdkPlatformBehaviorTest.kt:57-67`）。
Local Network Protection 是**未来版本**的风险，不是 API 36 的 minSdk 收益；
按 `toolchain-upgrade.md` 的判断，targetSdk 37 会让 `ACCESS_LOCAL_NETWORK` 成为强制运行时权限，
因此不要升 targetSdk 37。
**升 minSdk 到 36 不会改变这些 targetSdk 行为，也没有新的 minSdk-gated API 可简化代码。**

**代码/构建/验证上的具体影响（成本最高）。**

- 本地 NDK 27.2 的 clang wrapper 只到 `android35-clang`，**没有 `android36-clang`**
  （已核对 `/home/w0n9/Android/Sdk/ndk/27.2.12479018/.../bin/`）；
  要升到 36 必须换一个带 `android36-clang` 的 NDK（例如 r28+；本报告未逐一核实
  各 NDK 版本的 wrapper 列表）或另找 wrapper，这本身是工具链决策。
- 现有 4 个 AVD 中三个是 API 35、一个是 API 26；minSdk 36 时**一个都装不上**，
  需要新建 API 36 AVD。
- 真机 API 31 无法安装；真机音频/MIUI/真实网络验收全部不可完成。
- **对本项目无实质收益。** 设备损失 57.6%（相对当前基数约 58.1%）。不接受。

### 3.12 targetSdk 强制项（与 minSdk 无关，单独提示）

下表把容易被误写成「升 minSdk 收益」的 targetSdk 强制项单独列出。它们**不会**因为
minSdk 提高而变得更容易满足，也不会因为 minSdk 保持 26 而失效；项目当前 targetSdk 36
已经承受这些约束。

| 强制项 | 触发条件 | 项目现状 | 与 minSdk 的关系 |
|---|---|---|---|
| PendingIntent mutability | targetSdk 31 | 已用 `FLAG_IMMUTABLE`（`ControlSessionService.kt:362`），该 flag 自 API 23 可用 | 无关 |
| POST_NOTIFICATIONS 请求 | targetSdk 33 | 已在 33+ 请求（`MainActivity.kt:160-165`），manifest 已声明 | minSdk 33 只删掉版本判断 |
| FGS types 强制 + `FOREGROUND_SERVICE_MEDIA_PLAYBACK` | targetSdk 34 | manifest 已声明 type 与权限 | 无关 |
| `registerReceiver` export flags | targetSdk 34 | 无 `registerReceiver`、无 `<receiver>` | 无关（不适用） |
| edge-to-edge | targetSdk 35/36 | `enableEdgeToEdge()` + Scaffold insets | 无关 |
| predictive back 默认 | targetSdk 36 | manifest flag + Compose `BackHandler` | 无关 |
| adaptive orientation/resizability | targetSdk 36 | `resizeableActivity=true`、无方向锁 | 无关 |
| 16 KB page size | targetSdk 35 + 64 位设备（Play 2027-02-01） | rustflags + ticket 14 证据 | 无关（设备/分发要求） |
| `ACCESS_LOCAL_NETWORK` | targetSdk 37（未来） | 未采用；`toolchain-upgrade.md` 明确不升 37 | 无关；升 minSdk 不能规避 |

## 4. 代价：设备覆盖率

### 4.1 数据来源与日期

- **官方 dashboard**：`https://developer.android.com/about/dashboards` 仍在线，
  但页面自述的快照是「7-day period ending on **November 24, 2025**」；
  截至本报告日期（2026-11-23）约 12 个月未更新。页面同时引导开发者改用
  Google Play Console 的 **Reach and devices** 获取更细的分发数据
  （需要应用/Play Console 权限）。研究时其平台分布表位于一个 iframe，
  该 iframe 返回 HTTP 502，未能取得逐档数值；因此本报告**不用它**作为当前数值来源。
- **当前替代来源**：[apilevels.com](https://apilevels.com/)。
  该页自述：数据来自 **Statcounter GlobalStats**，覆盖全球手机与平板设备，
  由站点脚本 `usage_reporter.py` 汇总；**最后更新 2026-11-03，使用 2026 年 10 月数据**。
- **交叉核对**：[Statcounter Android Version Market Share Worldwide — October 2026](https://gs.statcounter.com/android-version-market-share)
  显示 Android 16 37.71%、15 16.34%、14 11.45%、13 10.9%、12 7.11%、11 6.08%，
  与 apilevels 的分段占比一致（apilevels 的 API 36+ 41.6% 还包含 API 37 的 4.0%）。

**偏差说明（重要）：** Statcounter 统计的是 Web 流量，不是 Google Play 安装量，
也不是本项目的真实用户。本项目通过 GitHub Release 分发，用户可能集中在
MIUI/中国区/较旧设备；这些数字只能作为方向性参考，不能当作本项目用户的精确比例。
百分比四舍五入到 0.1%，累计损失有约 ±0.1 个百分点的误差。

### 4.2 版本段占比（apilevels.com，2026-10 数据，页面更新 2026-11-03）

| Android 版本 | API | 该段占全部活跃设备 | 累计 API ≥ |
|---|---|---:|---:|
| 8.0 Oreo | 26 | 0.2% | 99.2%（26+） |
| 8.1 Oreo | 27 | 0.8% | 99.0%（27+） |
| 9 Pie | 28 | 1.6% | 98.2%（28+） |
| 10 | 29 | 3.1% | 96.6%（29+） |
| 11 | 30 | 6.1% | 93.5%（30+） |
| 12 / 12L | 31 / 32 | 7.1%（合并） | 87.4%（31+） |
| 13 | 33 | 10.9% | 80.3%（33+） |
| 14 | 34 | 11.5% | 69.4%（34+） |
| 15 | 35 | 16.3% | 57.9%（35+） |
| 16 | 36 | 37.6% | 41.6%（36+） |
| 17 | 37 | 4.0% | 4.0%（37+） |

当前 minSdk 26 的覆盖是 **99.2%**（API 25 及以下的 0.8% 已经被排除）。
下表的「放弃占全部活跃设备」指**相对当前 minSdk 26 新放弃**的设备占全部活跃设备的比例
（不是「目标 API 以下的所有设备」，后者还要加上已排除的 0.8%）；
「相对当前 API 26+ 覆盖的损失」= 新放弃占比 ÷ 99.2%。

### 4.3 重点档位的累计覆盖率损失

| 目标 minSdk | 放弃的版本段 | 放弃占全部活跃设备 | 相对当前 API 26+ 覆盖的损失 | 剩余覆盖（全部设备） |
|---|---|---:|---:|---:|
| **28** | 26–27 | **1.0%** | **约 1.0%** | 98.2% |
| **29** | 26–28 | **2.6%** | **约 2.6%** | 96.6% |
| **31** | 26–30 | **11.8%** | **约 11.9%** | 87.4% |
| **33** | 26–32 | **18.9%** | **约 19.1%** | 80.3% |
| **34** | 26–33 | **29.8%** | **约 30.0%** | 69.4% |
| **35** | 26–34 | **41.3%** | **约 41.6%** | 57.9% |
| （参考）36 | 26–35 | 57.6% | 约 58.1% | 41.6% |

说明：

- API 32 无法从 apilevels/Statcounter 的公开表中单独拆出（12/12L 合并为 7.1%）。
  因此「升到 32」的精确损失不可得；可以确定的是 **31+ 的累计损失为 11.8%**，
  32+ 的累计损失位于 **11.8%–18.9%** 区间（取决于 API 31-only 的占比）。
- 这些数字是「活跃设备」占比，不是「安装量」或「收入」占比；对一个小众、
  自托管、GitHub 分发的应用，实际用户分布可能显著偏离。
- 如果未来官方 dashboard 恢复更新，应以官方 Play 分布为准；当前它不能提供
  2026 年的当前数据。

## 5. 验证设备边界与证据分层

### 5.1 现有验证设备

| 设备 | API / ABI | 类型 | 备注 |
|---|---|---|---|
| `audio-share-api26` | API 26 / x86_64 | AVD | 当前最低 API floor |
| `audio-share-api35` | API 35 / x86_64 | AVD | 新平台行为主验证 |
| `audio-share-api35-16k` | API 35 / x86_64 / 16 KB pages | AVD | 16 KB 证据 |
| `audio-share-api35-t30` | API 35 / x86_64 | AVD | 测试/兼容矩阵 |
| Xiaomi M2007J22C（Redmi Note 9 系） | Android 12 / API 31 / arm64-v8a / MIUI | 真机 | **唯一真机** |

### 5.2 各档位下设备是否仍可用

| 目标 minSdk | api26 AVD | api35 AVD ×3 | API 31 真机 | 验证边界结论 |
|---|---|---|---|---|
| 26（现状） | ✅ | ✅ | ✅ | 不变 |
| 27–30 | ❌ 不可安装 | ✅ | ✅ | 需补一个 API 27/28/29/30 的 floor AVD；真机仍可跑音频/MIUI 关键路径 |
| 31 | ❌ | ✅ | ✅（正好是 floor） | 真机成为最低 API；无法再用真机验证「低于 31」的行为；beta-spec 的 API 26 runtime 证据必须改写 |
| 32–34 | ❌ | ✅ | ❌ 不可安装 | **真机验收不可完成**；只能靠 AVD，`EMULATOR` 不能替代音频/MIUI/真实网络 |
| 35 | ❌ | ✅（35 正好是 floor） | ❌ | 三个 API 35 AVD 可用；仍无真机验收 |
| 36 | ❌ | ❌（全部是 35） | ❌ | 现有设备/AVD 全部不可用；需 API 36 AVD + NDK 28+ wrapper |

**补充说明：** 「现有验证设备仍可用」要分设备看。**真机**在 minSdk ≤31 时一直可安装；
但 `audio-share-api26` 只在 minSdk ≤ 26 时可安装，一旦升到 27+ 就必须新建最低 API 的
floor AVD。API 35 AVD 在 minSdk ≤35 时可用；minSdk 36 时全部不可用。

### 5.3 对 `docs/agents/evidence-policy.md` 证据分层的含义

- `EMULATOR` 是正式证据状态，可满足：功能 UI、协议/状态机、Android 生命周期
  （前台 Service、ready、rebind、recreate、lock、stop）、**API 26 runtime 存在性与可安装性**、
  release APK 安装/R8/资源打包、模拟器软件实现的 Keystore
  （`docs/agents/evidence-policy.md:26-32`）。
- `EMULATOR` **永远不能替代**：可听音质、端到端可听时延（p95/p99）、underrun、
  `AudioTrack` 缓冲结论；MIUI/vendor 行为（如 `Network.bindSocket` EPERM）与真实
  Wi-Fi↔蜂窝切换；60 分钟双真机 gate、多 Client 并发音质、Server soak 音质；
  IPv6 与真实动态 MTU；最终 RC 验收（`docs/agents/evidence-policy.md:34-43`）。
- 因此：
  - **minSdk 27–30**：真机仍是 API 31，可以继续承担音频/MIUI/真实网络验收；
    最低 API 的功能/安装性由新 floor AVD 以 `EMULATOR` 覆盖。beta-spec 的
    「API 26 runtime」项需要改成新 floor；这是验收范围变更，不是纯构建变更。
  - **minSdk 31**：真机仍能安装，beta-spec 的「一台真实 arm64/API 31 设备」
    （`beta-spec.md:283-286`）仍可执行；但「API 26 runtime」
    （`beta-spec.md:281`）失效，且真机无法再验证「低于 floor」的行为。
  - **minSdk > 31**：真机完全不能安装。beta-spec 的真机 30 分钟播放、锁屏/唤醒、
    断网恢复、revoke/re-pair 等条目只能变成 `NOT RUN` 或 `WAIVED`；
    `EMULATOR` 不能把它们升级成 `PASS`。这不是「证据等级不同」，
    而是**现有 beta 验收无法执行**，除非购买/借用新的 API ≥ 新 floor 的真机，
    或修改 beta-spec 验收范围（与 `map.md:16` 的第二条硬约束冲突）。

## 6. 结论与建议

### 6.1 推荐档位

**推荐：维持 `minSdk 26`。** 理由按重要性排序：

1. **没有显著收益。** 所有升档收益都是「删 1–3 处兼容分支」或「获得可选平台能力」；
   没有用户可见的功能、音频、性能或安全收益是「必须靠抬 minSdk 才能得到」的。
   StrongBox 已经在 26 下按需使用；动态取色/per-app language/SplashScreen 要么
   被 ticket 10 延后，要么有 AndroidX/运行时判断的替代路径。
2. **升档会改变验收范围。** beta-spec 明确要求 API 26 runtime 证据
   （`beta-spec.md:281`），map.md 又把「不改 beta-spec 验收范围」列为硬约束
   （`map.md:16`）。抬 minSdk 不是一次普通构建调整，需要同时重开这两个决议。
3. **>31 直接破坏真机验收。** 唯一真机是 API 31；>31 时连安装都不行，
   `EMULATOR` 不能替代证据政策明确列出的真机项。
4. **31 是最高可用档，但收益不配 11.8% 的设备损失。** 升到 31 只让真机成为 floor、
   让 API 26 AVD 报废，并没有解决任何现有问题。

**如果未来因外部原因必须升，推荐 29。** 29 能同时获得 28/29 的代码清理
（StrongBox 判断 + Q 分支），让 foreground service type 调用无条件化，
native wrapper 也进入 API 29；代价约 2.6% 活跃设备，现有 API 31 真机与 API 35 AVD
仍可用，只需补一个 API 29 floor AVD。28 只比 29 少损失约 1.6 个百分点，
却只换来一行 StrongBox 判断，收益更弱，因此不作为首选。

**31 是现有验证设备允许的最高档，不是推荐档。** 超过 31 不可接受，
除非同时解决真机硬件和 beta 验收范围两个问题。

### 6.2 「收益显著」与「只是删死代码」的区分

| 类别 | 内容 | 判断 |
|---|---|---|
| 显著收益 | 无 | 没有任何档位带来必须的用户可见/音频/性能/安全收益 |
| 现在就能做（不升档） | 3 处 `SDK_INT >= O` 死分支 | 纯清理，建议单独做 |
| 升 28 | 删 StrongBox 版本判断；native wrapper 28 | 一行死代码 |
| 升 29 | 删 Q 分支；typed FGS 无条件化；wrapper 29 | 少量兼容代码 |
| 升 33 | 删通知权限判断；LocaleManager/localeConfig（不升档也可启用系统级 per-app language） | 一行死代码 + 可选语言能力 |
| 升 34/35/36 | 无现有代码可删；targetSdk 项已处理 | 无收益，成本递增 |
| 音频 | AAudio 已在 26；Oboe/AAudio 替换 out-of-scope | 升档不带来新引擎 |

### 6.3 如果真要升，必须同步改什么

按影响面从大到小：

1. **治理/验收文件（不是构建文件）**
   - `map.md:16` 的硬约束、`issues/03-toolchain-upgrade-targets.md:14` 的约束；
   - `docs/adr/0004-wire-protocol-v2-transport.md:19`（决策 5，baseline API 26）；
     升档需要新的 ADR 取代，而不是静默修改。
   - `.scratch/wire-protocol-v2/beta-spec.md:281`（API 26 runtime）和
     `:283-286`（真实 API 31 设备）；`docs/agents/evidence-policy.md:30`（API 26 项）。
   - `android-client-v2/README.md:92` 的「不要求提高 API 26 baseline」表述。
2. **构建与 native**
   - `android-client-v2/app/build.gradle.kts:121` 的 `minSdk`，以及 `:110/:120` 的注释。
   - `android-client-v2/native/.cargo/config.toml:7,19,28,30` 四处 `android26-clang`
     改为目标 API 的 wrapper；重新构建 `ring`/native；重跑 16 KB 对齐断言。
   - 目标 36 时 NDK 27.2 无 `android36-clang`，必须换用带 API 36 wrapper 的 NDK
     （例如 r28+，需核实），这是独立的工具链变更，需重新验证 `ring` 的 cc-rs 构建与 ELF 对齐。
3. **测试与验证矩阵**
   - `TargetSdkPlatformBehaviorTest.kt:42` 的 `assertEquals(26, minSdkVersion)`
     必须改成新值，测试名 `app_targets_api_36_on_the_api_26_baseline` 也要改。
   - 新建目标 API 的 floor AVD；`audio-share-api26` 从矩阵移除（目标 >26 时）。
   - 目标 36 时现有 API 35 AVD 全部不可用，需 API 36 AVD。
   - 目标 >31 时必须解决真机：购买/借用 API ≥ 新 floor 的 arm64 真机，
     或明确修改 beta 验收范围；`EMULATOR` 不能替代音频/MIUI/真实网络项。
4. **依赖下限**
   - 现有依赖不强制变化：Robolectric 测试侧最低 SDK 24，Compose 1.12 未抬高 minSdk，
     Activity Compose/CameraX 等下限 ≤26（`research/toolchain-upgrade.md:54,56,59-65,71`）。
     升 minSdk 不会让这些依赖失效，也不会强迫升级。
   - Compose 1.12 在 API 26–28 的 WCG→sRGB 回退在升到 29+ 后不再可达；
     本应用未使用宽色域，收益为零。
5. **代码清理（如果目标档位覆盖）**
   - 28：`ClientSigner.kt:193`；29：`ControlSessionService.kt:402-406`；
     33：`MainActivity.kt:161`。
   - 三处 `SDK_INT >= O`（`ControlSessionService.kt:163,356,384`）**现在就能删**，
     不要等到升档。
6. **重测范围**
   - JVM：`./gradlew test -PnoNative=true`（JDK 21）。
   - native：`cargo test`、release 构建、`llvm-readelf` 16 KB 断言、`zipalign -c -P 16`。
   - `EMULATOR`：目标 floor AVD 的安装/启动/配对/播放/锁屏/rebind/recreate/stop。
   - 真机：API 31 的配对、播放、锁屏、断网重连；目标 >31 时这些变为不可执行。
   - beta-spec 的 API 26 runtime 项按新 floor 重写；这不属于「重测」，而是验收范围变更。

### 6.4 未核实 / 待补

- bionic status 页面的直接抓取受网络限制；`getrandom`/`getentropy` 属 P/API 28
  来自该页的搜索摘要，未逐字复核。该条不影响结论：Rust 侧已用 dlsym + `/dev/urandom`
  回退，release `.so` 也没有硬依赖 API 28 符号。
- ELF TLS 对当前 Rust `.so` 的实际性能影响未测量；符号检查未发现 `__emutls_get_address`，
  因此不能证明升 29 有可测量收益。
- 没有 `Application.getPageSize()` 的官方 API 证据；本报告不把它列为 API 36 收益。
- 设备占比是 Statcounter/apilevels 的全球 Web 流量数据，不是本项目的安装分布；
  实际损失可能偏离。官方 dashboard 的当前快照停留在 2025-11-24，不能用于 2026 年数值。
- APK 体积/启动性能是否因删分支而变化：未测量，预计可忽略；本报告不声称有收益。

## 来源

### 官方 Android / API 文档

- Build.VERSION_CODES（O=26、P=28、Q=29、S=31、TIRAMISU=33、UPSIDE_DOWN_CAKE=34、VANILLA_ICE_CREAM=35、BAKLAVA=36）：https://developer.android.com/reference/android/os/Build.VERSION_CODES
- Android 8.1（API 27）features：https://developer.android.com/about/versions/oreo/android-8.1
- Android 9（API 28）features：https://developer.android.com/about/versions/pie/android-9.0
- Android 10（API 29）features（含 FGS types、ELF TLS）：https://developer.android.com/about/versions/10/features
- Android 11（API 30）features：https://developer.android.com/about/versions/11/features
- Android 11（API 30）behavior changes：https://developer.android.com/about/versions/11/behavior-changes-11
- Android 12（API 31）features（SplashScreen、Material You、audio focus）：https://developer.android.com/about/versions/12/features
- Android 12L（API 32）summary：https://developer.android.com/about/versions/12/12L/summary
- Android 13（API 33）features（per-app language、predictive back）：https://developer.android.com/about/versions/13/features
- Android 13（API 33）behavior changes：https://developer.android.com/about/versions/13/behavior-changes-13
- Android 13 通知运行时权限：https://developer.android.com/develop/ui/views/notifications/notification-permission
- Android 14（API 34）behavior changes（FGS types、registerReceiver flags）：https://developer.android.com/about/versions/14/behavior-changes-14
- Android 15（API 35）behavior changes（edge-to-edge、FGS、audio focus）：https://developer.android.com/about/versions/15/behavior-changes-15
- Android 16（API 36）behavior changes（edge-to-edge、predictive back、adaptive layouts）：https://developer.android.com/about/versions/16/behavior-changes-16
- 16 KB page size：https://developer.android.com/guide/practices/page-sizes
- Predictive back：https://developer.android.com/guide/navigation/custom-back/predictive-back-gesture
- Per-app languages：https://developer.android.com/guide/topics/resources/app-languages
- Dynamic color：https://developer.android.com/develop/ui/views/theming/dynamic-colors
- SplashScreen（含 core-splashscreen API 23）：https://developer.android.com/develop/ui/views/launch/splash-screen
- Foreground service types：https://developer.android.com/develop/background-work/services/fg-service-types
- AAudio：https://developer.android.com/ndk/guides/audio/aaudio/aaudio
- AudioFormat（API 31 的 24/32-bit 编码）：https://developer.android.com/reference/android/media/AudioFormat
- `KeyGenParameterSpec.Builder.setIsStrongBoxBacked`（API 28）：https://developer.android.com/reference/android/security/keystore/KeyGenParameterSpec.Builder#setIsStrongBoxBacked(boolean)
- `Service.startForeground(int, Notification, int)`（API 29）：https://developer.android.com/reference/android/app/Service#startForeground(int,%20android.app.Notification,%20int)
- `ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK`（API 29）：https://developer.android.com/reference/android/content/pm/ServiceInfo#FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK
- `PendingIntent`（FLAG_IMMUTABLE API 23、FLAG_MUTABLE API 31）：https://developer.android.com/reference/android/app/PendingIntent
- `LocaleManager`（API 33）：https://developer.android.com/reference/android/app/LocaleManager
- bionic status（`getrandom`/`getentropy` 列在 P/API 28；直接抓取受网络限制）：https://android.googlesource.com/platform/bionic/+/HEAD/docs/status.md

### 设备分布

- 官方 dashboard（公开快照截至 2025-11-24；当前数值不采用）：https://developer.android.com/about/dashboards
- apilevels.com（Statcounter，2026-10 数据，页面更新 2026-11-03）：https://apilevels.com/
- Statcounter Android version market share（October 2026）：https://gs.statcounter.com/android-version-market-share

### 仓库内证据（代码/决策）

- `android-client-v2/app/build.gradle.kts:101,110,120-122`（compileSdk/targetSdk/minSdk/签名注释）
- `android-client-v2/app/src/main/AndroidManifest.xml:7-9,20-23,28-29`
- `android-client-v2/app/src/main/java/dev/audioshare/v2/ControlSessionService.kt:163,356,384,402`
- `android-client-v2/app/src/main/java/dev/audioshare/v2/ClientSigner.kt:193`
- `android-client-v2/app/src/main/java/dev/audioshare/v2/MainActivity.kt:25,63,160-165`
- `android-client-v2/app/src/main/java/dev/audioshare/v2/MediaPlayer.kt:133-147,198-233`
- `android-client-v2/app/src/main/java/dev/audioshare/v2/AudioShareTheme.kt:24-26`
- `android-client-v2/native/.cargo/config.toml:7,11-14,19-23,28-31`
- `android-client-v2/app/src/test/java/dev/audioshare/v2/TargetSdkPlatformBehaviorTest.kt:42`
- `android-client-v2/app/src/test/java/dev/audioshare/v2/ForegroundServiceContractTest.kt:34-41`
- `.scratch/android-modernization/map.md:16,61`
- `.scratch/android-modernization/issues/03-toolchain-upgrade-targets.md:14`
- `.scratch/android-modernization/issues/07-kernel-audio-refactor-scope.md:68-76`
- `.scratch/android-modernization/issues/10-ui-implementation-scope.md:45,71-78,96`
- `.scratch/android-modernization/research/toolchain-upgrade.md:54,56,59-65,71,137,193`
- `.scratch/wire-protocol-v2/beta-spec.md:281,283-286`
- `docs/agents/evidence-policy.md:26-43`
- `docs/adr/0004-wire-protocol-v2-transport.md:19`
- `docs/wire-protocol-v2/pairing.md:41`
- `android-client-v2/README.md:92`
- 本机验证：NDK 27.2 的 `bin/` 提供 `android21`–`android35` clang wrapper，无 `android36`；
  release `.so` 导入 `dlsym@LIBC` 与 weak `getrandom`；`cargo tree` 显示
  `getrandom 0.2.17` 由 `ring 0.17.14` 与 `rand_core 0.6.4` 使用。