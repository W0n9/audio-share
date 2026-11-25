# 09: 构建升级落地、CI 与 baseline profile 范围

Labels: wayfinder:grilling
Type: grilling
Status: resolved
Blocked by: 03

## Question

基于「构建工具链升级目标与顺序」的版本矩阵，决定落地方式：

1. **升级节奏**：一次性升到目标版本，还是按 Gradle → AGP → Kotlin → Compose 渐进；每一步的绿色门禁是什么。
2. **CI**：现有构建 / 测试 / interop / JVM / release 签名检查如何扩展；是否加入 baseline profile 生成与 Macrobenchmark。
3. **baseline profile / Macrobenchmark**：覆盖哪些启动与播放场景；是否在 beta 并行期间执行，还是 stable gate 后再做。
4. **兼容性门禁**：`minSdk 26`、release 签名、`-PtestSupport` / `-PdeviceTests`、cargo NDK 构建，任何升级都不得破坏。
5. **回滚**：升级失败时如何回到当前可发布构建。

已知约束（来自 [02](02-beta-freeze-and-merge-point.md#answer)）：`release.yml` 硬校验 `versionName`；重构分支禁改 `VERSION` / `versionName` / `versionCode`；升级落地不得让 beta 并行期间的发布校验失败。

产出是升级顺序、CI 门禁与回滚点，不是实现。

## Answer

**结论（用户逐条确认）：四阶段渐进升级并保留桥接阶段；16 KB 与 CI SDK 先行；新增独立
`android-ci` workflow 只做可自动化门禁；Baseline Profile / Macrobenchmark 排除本图；
每阶段固定兼容性断言；每阶段一个 `--no-ff` merge，失败即 revert。**

### 1. 升级节奏与阶段门禁

| 阶段 | 内容 | 绿色门禁 |
|---|---|---|
| **0（并行）** | Rust 链接加 `-Wl,-z,max-page-size=16384` / `common-page-size=16384`；CI SDK 升 build-tools 36.0.0 / platforms android-37；NDK 保持 27.2.12479018 | `llvm-readelf` 显示 `Align 2**14` 且 RELRO 末端 0x4000 对齐；`zipalign -c -P 16`；`check_elf_alignment.sh` 输出 `ALIGNED`（外部脚本，需 vendor；未 vendoring 时以 `llvm-readelf` + `zipalign` 断言为准）；16 KB 模拟器跑 ready/rebind/recreate/lock/stop |
| **1（桥接）** | AGP 8.7.3 → **8.13.2**；Gradle 8.9 → **8.13**；protobuf 插件 **0.10.1**；Kotlin 暂留 **2.0.21** | `./gradlew test`；生产签名 release 构建；`-PtestSupport=true -PdeviceTests=true` 测试构建；generated protobuf diff + fixture replay；任务名 `packageRelease` / `merge*JniLibFolders` 仍存在 |
| **2（AGP 9）** | AGP **9.5.1** + Gradle **9.7.1**；移除 `org.jetbrains.kotlin.android`；built-in Kotlin + KGP **2.4.21**；Compose 编译器插件 2.4.21；迁移 `jvmToolchain`、`src/testSupport` source set、jniLibs 任务 API；R8 strict full mode keep 规则 | 双 ABI `.so`；生产证书 = beta keystore；测试证书 = debug；`-PtestSupport` 产物无 fixture 路径；mapping/configuration 与阶段 1 对比；完整设备用例在 `EMULATOR` 通过 |
| **3（SDK/依赖）** | compileSdk **37**、targetSdk **36**、minSdk **26**；Compose BOM **2026.09.00**；Activity Compose 1.13.0；Core 1.19.1；CameraX 1.6.2；Robolectric 4.18；androidx.test 1.7.0 / ext 1.3.0 | `aapt dump badging` targetSdk 36 / minSdk 26；API 26 `EMULATOR` 安装与 functional smoke；edge-to-edge / insets / 预测性返回 / 大屏方向验证；Compose 屏测试全绿 |

- compileSdk 37 与 targetSdk 36 并存；**不升 targetSdk 37**（`ACCESS_LOCAL_NETWORK`）。
- 应用字节码 toolchain 保持 **Java 17**；Robolectric 4.18 的 SDK 36 测试需要 **JDK 21**，
  因此 CI 测试 job 用 JDK 21，生产构建仍按 17 编译。
- **JDK 25 裁决：不采用。** AGP 9.5 官方 JDK 基线仍是 17（25 只是修掉了警告，不是默认支持）；
  应用字节码目标受 Android API/Java 版本表限制，保持 17；Robolectric 没有正式的 JDK 25
  支持声明。若未来 AGP/Robolectric 明确支持，再作为独立决策评估，不在本轮引入。
- 所有阶段只在 `refactor/android-build` worktree 落地，beta 分支不动。

### 2. 阶段 0 的并行边界

- 阶段 0 **只做 16 KB 对齐与 CI SDK 升级**，可与 UI/架构轴并行开始。
- targetSdk 36 留在阶段 3：需要 compileSdk ≥ 36，并验证 edge-to-edge / 预测性返回等平台行为。
- 两者都通过后再进入桥接阶段；不改 `.proto`、ABI、cargo target、JNI 接口或签名配置。

### 3. CI 范围

- 新增独立 **`android-ci`** workflow（PR / refactor 分支触发），包含：
  - JVM/Robolectric 测试（JDK 21）；
  - `-PdeviceTests=true -PtestSupport=true` 的测试签名 release 构建；
  - 双 ABI `.so`、JNI 符号、16 KB 对齐、`zipalign -P 16`、证书断言（测试构建 = debug）；
  - `-PtestSupport=false` 时 fixture 路径不存在；
  - `.proto` 未改动与生成源码 diff。
- **CI 不跑模拟器/真机**：`EMULATOR` 与真机项按 08 手工执行；CI 只强化可自动化部分。
- `release.yml` 仅更新 SDK 版本，保留签名、`versionName` 硬校验与产物哈希流程。

### 4. Baseline Profile / Macrobenchmark

- **排除本图**，记入 map 的 Out of scope。
- 理由：需要专用 benchmark 构建与设备、对 beta 验收无贡献、会拉长工具链轴的验证链；
  留作 stable gate / beta 之后的独立工作。

### 5. 兼容性门禁（每阶段固定）

1. `minSdk 26`：`aapt dump badging` + API 26 `EMULATOR` 安装。
2. 生产 release 证书 = beta keystore；测试构建证书 = debug；无凭据时打包失败。
3. `-PtestSupport` / `-PdeviceTests` 语义与 native feature、source set、proguard 不变。
4. 双 ABI `.so` 存在；JNI 符号与 `ClientNative` 包名不变。
5. `packageRelease` / `merge*JniLibFolders` 或替代 API 的任务图存在。
6. R8 mapping 对比；strict full mode keep 规则生效。
7. `.proto` 无改动；fixture replay 与 `cargo test` 通过。
8. 版本号、`VERSION`、`versionName`、`versionCode` 不变；`release.yml` 校验不破。

### 6. 回滚模型

- 每阶段一个 `--no-ff` merge commit；失败时 **revert 该 merge**，回到上一阶段，不重写历史。
- 最低可用回退点：阶段 1（AGP 8.13.2 + Gradle 8.13 + protobuf 0.10.1 + Kotlin 2.0.21），
  承载 16 KB 对齐；**不含 targetSdk 36**（那属于阶段 3），不必退回 8.7.3。
- 回滚不触碰 beta 分支、签名材料、版本号或 `native/Cargo.lock` 语义。
- 失败原因与最后可用阶段记录在 build 轴证据中，供 08 的 manifest 引用。

### 对后续工单的约束

- 08：CI 结果只能强化其门槛；`EMULATOR`/真机项不可由 CI 替代。
- 12：Robolectric 4.18 + JDK 21 的测试运行方式、现有屏测试的迁移纳入资产处置。
- 03：目标矩阵不变；若阶段 2/3 发现版本不兼容，回写 03 的 research 文档而不是改目标自行降级。

### 证据

本票为决策票，未改动代码；版本与风险事实来自 [research/toolchain-upgrade.md](research/toolchain-upgrade.md)
及构建/CI 文件。未建分支、未 commit。
