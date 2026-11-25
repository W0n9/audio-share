# 构建工具链升级调研 — android-client-v2

日期：2026-11-16
方式：只读子代理调研；未创建 `research/<name>` 分支，未 push（fork + 本地 tracker；push 需明确授权）。
范围：`android-client-v2/` 及其 native / CI 构建。

## 结论摘要

- 推荐终点：**AGP 9.5.1 + Gradle 9.7.1 + Kotlin 2.4.21 + compileSdk 37 / targetSdk 36 / minSdk 26 + Compose BOM 2026.09.00**。
- 两个不能靠 AGP 升级解决的独立发布阻塞：
  - **16 KB ELF 对齐**：targetSdk 35+ 在 64 位设备上受 Play 2027-02-01 起的强制要求。当前 arm64 release `.so` 实测 `LOAD Align 0x1000`，`GNU_RELRO` 末端 `0x266000` 不是 `0x4000` 倍数。
  - **targetSdk 36**：若通过 Play 分发，2026-08-31 起新应用/更新必须 target 36（延期窗口 2026-11-01 已过）。v2 beta 目前走 GitHub Release，Play 要求只在未来走 Play 时变成硬阻塞；targetSdk 36 的平台行为变更仍需验证。
- **不要升 targetSdk 37**：API 37 会让 `ACCESS_LOCAL_NETWORK` 成为强制运行时权限，而本应用核心功能就是通过 QUIC 连接局域网 Server；同时收紧后台音频与 native DCL。`compileSdk 37` 与 `targetSdk 36` 可以并存。

## 现状

| 组件 | 当前版本 | 约束 / 证据 |
|---|---|---|
| AGP | 8.7.3 | `gradle/libs.versions.toml:5` |
| Gradle wrapper | 8.9 | `gradle/wrapper/gradle-wrapper.properties:4`，带 SHA-256 |
| Kotlin / KGP | 2.0.21 | `libs.versions.toml:6`；K2 自 2.0 起默认 |
| Compose 编译器插件 | 2.0.21 | `libs.versions.toml:46` |
| Compose BOM | 2024.10.01 | `libs.versions.toml:12` |
| Activity Compose | 1.9.3 | `libs.versions.toml:11` |
| Core KTX | 1.13.1 | `libs.versions.toml:10` |
| CameraX | 1.4.0 | `libs.versions.toml:13` |
| ZXing | 3.5.3 | `libs.versions.toml:14`；本地解码 |
| Navigation Compose | 未使用 | v2 单 Activity；仅 v1 使用 2.8.3 |
| Hilt / Koin | 未使用 | 手工构造 Service / Controller |
| protobuf 插件 | 0.9.4 | `libs.versions.toml:7` |
| protobuf-kotlin-lite / protoc | 4.28.2 / 4.28.2 | lite codegen |
| compileSdk / targetSdk | 35 / 35 | `app/build.gradle.kts:98, 119` |
| minSdk | 26 | 硬约束 |
| Java / Kotlin toolchain | 17 | `app/build.gradle.kts:181-183`；CI Temurin 17 |
| NDK | 27.2.12479018（r27c LTS） | `app/build.gradle.kts:20`、`native/.cargo/config.toml`、release.yml |
| ABI | 默认 arm64-v8a；beta 可 arm64-v8a + x86_64 | `app/build.gradle.kts:26-39, 125-129` |
| Rust | workspace MSRV 1.90；本机 rustc 1.98.1 | 根 `Cargo.toml:18`；无 `rust-toolchain.toml` |
| native 构建 | Gradle `Exec` 调 `cargo build --release --target ...`，NDK `llvm-strip` 后拷入 jniLibs；禁用 `cargo-ndk` | `app/build.gradle.kts:268-324` |
| release 签名 | keystore 环境变量或仓库外 properties；V1 关闭、V2/V3 开启；无凭据时 `packageRelease` 失败 | `app/build.gradle.kts:41-77, 100-113, 326-339` |
| 测试构建语义 | `-PtestSupport=true` / `-PdeviceTests=true`；测试构建用 debug 签名 | `app/build.gradle.kts:79-94, 148-179, 200-203` |
| 单元测试 | Robolectric 4.14；Compose 测试跑 JVM | `libs.versions.toml:15,39` |
| R8 | release 开启 minify + shrinkResources | `app/build.gradle.kts:155-165` |
| Baseline Profile / Macrobenchmark | 未使用 | 无相关模块 |
| CI | JDK 17；build-tools 35.0.0；platforms 35；NDK 27.2.12479018 | `.github/workflows/release.yml:24-43, 73-80` |

## 推荐目标

| 组件 | 推荐版本 | 理由 / 关键变更 | minSdk 26 |
|---|---|---|---|
| AGP | **9.5.1** | 当前稳定线；支持 max API 37；内置 Kotlin；AGP 9 是 Compose 1.12 前提；旧 Variant API 移除、R8 更严格 | 无 |
| Gradle wrapper | **9.7.1**（零 caveat 备选 9.6.0） | AGP 9.5 最低/默认 9.6.0；Kotlin 2.4.20 完全兼容到 9.7.0；不要用 9.9.1 | 无 |
| Kotlin / KGP | **2.4.21** | 当前稳定 bugfix；K2 默认；K1 移除；Compose 编译器插件版本必须一致 | 无 |
| Compose 编译器插件 | **2.4.21** | 与 KGP 同版本 | 无 |
| Compose BOM | **2026.09.00** | 映射 ui/runtime/foundation 1.12.1、material3 1.4.0；要求 compileSdk 37 + AGP ≥ 9.2 | 无（API 26–28 WCG 回退 sRGB） |
| Material3 | **1.4.0**（BOM） | 当前稳定 | 无 |
| Activity Compose | **1.13.0** | 与 Compose 1.12 搭配；1.12 起默认 minSdk 23 | 无 |
| Navigation Compose | **2.10.2**（仅在需要时） | 当前稳定；2.10 起 minSdk 24；v2 当前不需要 | 无 |
| androidx.core | **1.19.1**（建议 `core` 替代 `core-ktx`） | 1.19 起 core-ktx 为空兼容 artifact | 无 |
| CameraX | **1.6.2** | 当前稳定；minSdk 23；1.6 修复 Android 17 动态范围崩溃 | 无 |
| protobuf-gradle-plugin | **0.10.1** | 0.10.0 增加 AGP 9 支持；0.10.1 移除 Gradle 9.8 `setVisible` 弃用 | 无 |
| protobuf-kotlin-lite / protoc | **4.36.2 / 4.36.2** | Maven Central 稳定；不改 `.proto` | 无 |
| KSP | **2.3.14**（仅在使用 Hilt 时） | 支持 KGP 2.3.20–2.4.x、AGP 8.13+、Gradle 9.x、JDK 17 | 无 |
| Hilt | **2.60.1**（仅在决定引入 DI 时） | 最低 minSdk 23；要求 AGP 9.0+、Gradle 9.1+；需要 KSP | 无 |
| Koin | **4.2.2**（Hilt 的替代） | 运行时 DI，无代码生成；采用前需查 POM 的 minSdk 声明 | 待核实 |
| Robolectric | **4.18** | 支持 SDK 36/37；最低 minSdk 24；SDK 36 测试需 JDK 21 | 测试侧 minSdk 24 |
| androidx.test | **1.7.0 / ext.junit 1.3.0** | 当前稳定 | 无 |
| Rust | 保持 MSRV 1.90，使用 stable（本机 1.98.1） | Android targets 仍为 Tier 2，无需改 target | 无 |
| NDK | **保持 27.2.12479018** + 16 KB 链接参数 | r27 LTS；NDK 27+ 支持 16 KB 但默认不开启；继续使用 API 26 clang wrapper | 无 |
| compileSdk | **37** | Compose 1.12 强制；AGP 9.5 最高支持 37 | 无 |
| targetSdk | **36** | 平台行为目标；若走 Play 则是当前要求。不要 37 | 无 |
| minSdk | **26（不变）** | 硬约束 | — |
| CI SDK | build-tools **36.0.0**、platforms **android-37**、NDK 27.2.12479018 | AGP 9.5 最低 Build Tools 36.0.0 | 无 |

## 升级顺序与阻塞

```text
16 KB ELF 对齐（NDK 27.2 + rustflags）──┐
targetSdk 36（若走 Play）──────────────┼── 与 AGP 升级无依赖，可并行
CI: build-tools 36 / platforms 37 ─────┘

AGP 9.5.1 ──需要──> Gradle ≥ 9.6.0
AGP 9.5.1 ──需要──> 移除 org.jetbrains.kotlin.android（built-in Kotlin + new DSL）
AGP 9.5.1 ──需要──> protobuf-gradle-plugin ≥ 0.10.0
AGP 9.5.1 ──提供──> R8 ≥ 9.1.29 ──需要──> Kotlin 2.4.x
AGP ≥ 9.2 ──需要──> Compose 1.12 / BOM 2026.09.00
Compose 1.12 ──需要──> compileSdk 37
compileSdk 37 ──需要──> platforms;android-37
targetSdk 37 ──需要──> ACCESS_LOCAL_NETWORK（本应用不应采用，保持 36）
```

关键结论：

1. Gradle 不能单独先升；AGP 8.7.3 不能配 Gradle 9.x，AGP 9.5 又要求 Gradle ≥ 9.6.0。Gradle 与 AGP 成对升级。
2. Kotlin 2.4 不能先于 AGP 9：Kotlin 2.4 要求 R8 9.1.29，AGP 8.13.2 自带的 R8 8.13.19 只声明支持 Kotlin 2.3；release 开启 R8，必须先 AGP 9.5 再 Kotlin 2.4.21。
3. Compose 升级最后做：Compose 1.12 强制 compileSdk 37 + AGP ≥ 9.2。
4. protobuf 插件 0.10.1 必须在 AGP 9 之前或同时到位。
5. 16 KB 与 targetSdk 36 是独立发布阻塞，不会随 AGP 升级自动解决。

### 推荐执行顺序

**阶段 0（并行，无 AGP 依赖）**

- 16 KB native 对齐：在 `native/.cargo/config.toml` 的两个 target 下增加：
  ```toml
  rustflags = [
    "-C", "link-arg=-Wl,-z,max-page-size=16384",
    "-C", "link-arg=-Wl,-z,common-page-size=16384",
  ]
  ```
  或在 Gradle `Exec` / CI 设置 `RUSTFLAGS`。不改 `.proto`、ABI、cargo target 或 JNI 接口。
- 验证 `.so`：`llvm-readelf -Wl ... | grep -E 'LOAD|RELRO'`，期望 `LOAD Align 2**14`，且 `(GNU_RELRO.VirtAddr + MemSiz) % 0x4000 == 0`（或 RELRO 后无 writable data）。
- 验证 APK：`zipalign -c -P 16 -v 4 ...`，以及 `check_elf_alignment.sh` 输出 `ALIGNED`。
- 运行时：`adb shell getconf PAGE_SIZE` 期望 16384；在 16 KB 模拟器/设备上跑 `ready/rebind/recreate/lock/stop`。
- CI 更新：build-tools 35.0.0 → 36.0.0，platforms 35 → 37；NDK 保持 27.2.12479018。
- 可选：新增独立 `benchmark` 模块（`androidx.benchmark` 1.5.0、`benchmark-macro-junit4` 1.5.0、`profileinstaller` 1.4.1）；profile 生成构建 `isMinifyEnabled=false`，不改 release R8 语义。

**阶段 1（桥接，建议但可跳过）**

- AGP 8.7.3 → **8.13.2**，Gradle 8.9 → **8.13**。
- Kotlin 保持 2.0.21 或升到 **2.3.21**；不要在这一阶段上 2.4。
- protobuf 插件 0.9.4 → **0.10.1**，runtime/protoc 4.28.2 → **4.36.2**；冻结 `.proto`，对比生成源码差异。
- 跑完整 release 构建与 Robolectric 单测，确认签名与 `-P` 语义未变。

**阶段 2（AGP 9 + built-in Kotlin）**

- AGP **9.5.1**，Gradle **9.7.1**（零 caveat 用 9.6.0）。
- 删除根与 app 的 `org.jetbrains.kotlin.android` 插件及 version catalog 条目；保留 `org.jetbrains.kotlin.plugin.compose` 2.4.21。
- 用 `buildscript { classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.4.21") }` 覆盖 AGP 内置 KGP。
- `android { kotlin { jvmToolchain(17) } }` 改为顶层 `kotlin { jvmToolchain(17) }` 或 `compileOptions`。
- `src/testSupport/java` 只保留 `kotlin.directories +=`，不再同时进 `java.srcDir`。
- 验证 `merge*JniLibFolders`、`packageRelease` 任务名；若不存在，改用 `androidComponents` 的 `sources.jniLibs.addGeneratedSourceDirectory` 等 API。
- 验证 `android.r8.strictFullModeForKeepRules` 默认 true 下的 keep 规则。
- 验证 `-PtestSupport=true` / `-PdeviceTests=true` 的 native feature、source set、debug 签名、proguard 语义。

**阶段 3（compileSdk 37 + Compose 1.12 + targetSdk 36）**

- `compileSdk = 37`，`targetSdk = 36`，`minSdk = 26` 不变。
- Compose BOM → **2026.09.00**；Activity Compose → 1.13.0；Core → 1.19.1；CameraX → 1.6.2；Robolectric → 4.18；androidx.test → 1.7.0 / 1.3.0。
- 处理 targetSdk 36 行为：edge-to-edge / insets、预测性返回、大屏方向限制。

**阶段 4（可选）**

- Baseline Profile / Macrobenchmark 接入。
- 若确需 DI，再引入 Hilt 2.60.1 + KSP 2.3.14；否则不引入。

## 三个最高风险与缓解

### 风险 1：native `.so` 未做 16 KB 对齐

证据（当前 arm64 release `.so`）：`LOAD ... Align 0x1000`；`GNU_RELRO VirtAddr 0x24e0d0 MemSiz 0x17f30`，末端 `0x266000` 非 `0x4000` 倍数。NDK r27 默认不生成 16 KB 对齐。

缓解：

1. 给 Rust 最终链接加 `-Wl,-z,max-page-size=16384` 与 `-Wl,-z,common-page-size=16384`。
2. `llvm-readelf` 断言 `Align 2**14` 与 RELRO 末端对齐。
3. `zipalign -c -P 16` 与 `check_elf_alignment.sh` 断言 `ALIGNED`。
4. 16 KB 设备/模拟器上跑五个设备用例。
5. 若允许偏离硬约束，可改用 NDK 28.2.13676358（16 KB 默认对齐）；保持 27.2 + 参数同样满足。

### 风险 2：AGP 9 的 new DSL + built-in Kotlin 破坏自定义 cargo/签名/测试构建

会碰到的位置：

- `app/build.gradle.kts:181-183`：`android { kotlin { jvmToolchain(17) } }` 需迁移。
- `app/build.gradle.kts:200-203`：同一目录同时进 `java.srcDir` 与 `kotlin.srcDir`；built-in Kotlin 只支持 `AndroidSourceSet.kotlin`。
- `app/build.gradle.kts:322-324`：按任务名匹配 `merge*JniLibFolders`。
- `app/build.gradle.kts:329-339`：按 `packageRelease` 任务名做签名守卫。
- `app/build.gradle.kts:155-165`：R8 strict full mode 下 `-keep class A` 不再隐式保留默认构造器。

缓解（可执行断言）：

1. 对比生产/测试两套 APK 证书：生产 = beta keystore；`-PtestSupport/-PdeviceTests` = debug key。
2. `unzip -l` 断言两个 ABI 的 `.so` 存在；`-PtestSupport=false` 时 fixture 路径不存在。
3. `./gradlew tasks --all | grep -E 'packageRelease|JniLibFolders'` 与 `--dry-run` 验证任务图。
4. 对比升级前后 `mapping.txt` / `configuration.txt`；对 device-tests 跑完整设备套件。
5. 回退开关 `android.builtInKotlin=false` / `android.newDsl=false` 只能用一个版本窗口，AGP 10 将移除。

### 风险 3：targetSdk 36/37 行为变更与局域网权限

证据：当前 `targetSdk = 35`；全仓未发现 `enableEdgeToEdge`、`onBackPressed`、`BackHandler`、`windowOptOutEdgeToEdge`；targetSdk 37 会强制 `ACCESS_LOCAL_NETWORK`。

缓解：

1. 明确 `compileSdk = 37`、`targetSdk = 36`；不要设 37。
2. `aapt dump badging` 断言 `targetSdkVersion:'36'`。
3. Android 16 模拟器/设备验证 edge-to-edge、预测性返回、大屏方向；必要时在 `MainActivity` 调用 `enableEdgeToEdge()` 并使用 `WindowInsets`。
4. 网络回归：`ready` 到达 READY；`rebind` 同步收到 READY；`recreate`/`lock` 对比 Server 连接计数；`stop` 检查前台标志清理。

## 硬约束判断

| 硬约束 | 判断 | 说明 |
|---|---|---|
| minSdk 26 不得抬高 | 可满足 | 推荐依赖的 minSdk 均 ≤ 26；Compose 1.12 未抬高 minSdk，API 26–28 仅 WCG 回退 sRGB。 |
| compileSdk/targetSdk 可升 | 可满足，targetSdk 只到 36 | compileSdk 37 是 Compose 1.12 硬要求；targetSdk 37 会强制 `ACCESS_LOCAL_NETWORK` 并收紧后台音频。 |
| Gradle 调 cargo、NDK 27.2、两个 ABI | 可满足 | 保留 `Exec` cargo 调用、NDK 27.2、arm64-v8a + x86_64；追加 16 KB 链接参数。 |
| release 签名与 `-PtestSupport` / `-PdeviceTests` 语义 | 可满足 | 不改签名配置与 flag 分支；迁移 built-in Kotlin source set / jvmToolchain，验证 R8 keep 规则与任务名。 |
| 不改 Wire Protocol / beta-spec 验收范围 | 可满足 | 只升 protobuf 插件/runtime，不改 `protos/wire-protocol-v2/*.proto`；用生成源码 diff、`cargo test`、`ControlFixtureReplayTest` 与设备用例证明行为不变。 |

## 来源

- AGP 9.5 release notes: https://developer.android.com/build/releases/agp-9-5-0-release-notes
- AGP 9.0 built-in Kotlin / new DSL / Variant API / R8: https://developer.android.com/build/releases/agp-9-0-0-release-notes
- AGP/Gradle 兼容: https://developer.android.com/build/releases/gradle-plugin
- 迁移到 built-in Kotlin: https://developer.android.com/build/migrate-to-built-in-kotlin
- AGP/D8/R8 对 Kotlin 支持表: https://developer.android.com/build/kotlin-support
- Gradle releases: https://gradle.org/releases/
- Kotlin releases: https://kotlinlang.org/docs/releases.html
- Kotlin 2.4 新特性 / K1 移除: https://kotlinlang.org/docs/whatsnew24.html
- Compose BOM 映射: https://developer.android.com/develop/ui/compose/bom/bom-mapping
- Compose 依赖与 compileSdk 37 / AGP 9: https://developer.android.com/develop/ui/compose/setup-compose-dependencies-and-compiler
- Compose UI release notes: https://developer.android.com/jetpack/androidx/releases/compose-ui
- Material3 release notes: https://developer.android.com/jetpack/androidx/releases/compose-material3
- Activity release notes: https://developer.android.com/jetpack/androidx/releases/activity
- Navigation release notes: https://developer.android.com/jetpack/androidx/releases/navigation
- Core release notes: https://developer.android.com/jetpack/androidx/releases/core
- CameraX release notes: https://developer.android.com/jetpack/androidx/releases/camera
- androidx.test release notes: https://developer.android.com/jetpack/androidx/releases/test
- Robolectric releases: https://github.com/robolectric/robolectric/releases
- Hilt releases: https://github.com/google/dagger/releases
- Koin releases: https://github.com/InsertKoinIO/koin/releases
- protobuf-gradle-plugin releases: https://github.com/google/protobuf-gradle-plugin/releases
- Baseline Profiles overview: https://developer.android.com/topic/performance/baselineprofiles/overview
- Rust Android targets: https://doc.rust-lang.org/rustc/platform-support/android.html
- NDK revision history: https://developer.android.com/ndk/downloads/revision_history
- 16 KB page size: https://developer.android.com/guide/practices/page-sizes
- Play target API 要求: https://developer.android.com/google/play/requirements/target-sdk
- Android 16 behavior changes: https://developer.android.com/about/versions/16/behavior-changes-16
- Android 17 behavior changes: https://developer.android.com/about/versions/17/behavior-changes-17
