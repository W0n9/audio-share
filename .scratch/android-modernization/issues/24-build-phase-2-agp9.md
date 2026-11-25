# 24: Build 阶段 2 — AGP 9

**What to build:** 升到 AGP 9.5.1 + Gradle 9.7.1 + built-in Kotlin + KGP 2.4.21；迁移 source set、jniLibs 任务与 R8 规则，生产/测试构建语义不变。

**Blocked by:** 23。

**Status:** resolved

- [x] AGP 9.5.1、Gradle 9.7.1、Kotlin/KGP 2.4.21、Compose 编译器插件 2.4.21；移除 `org.jetbrains.kotlin.android`。
- [x] `jvmToolchain(17)` 与 `src/testSupport` source set 迁移完成；jniLibs 打包任务或其替代 API 可用。
- [x] R8 strict full mode 下 keep 规则生效；mapping/configuration 与桥接阶段对比无意外变化。
- [x] 生产证书 = beta keystore；测试构建证书 = debug；`-PtestSupport=false` 不携带 fixture 路径。
- [x] `./gradlew test` 全绿；`EMULATOR` 全设备用例通过。

## Comments

2026-11-17 — 实现完成并合入集成分支（merge `0c892a5`，feature `169cc5e`）。
Gradle 9.7.1（官方 SHA-256）、AGP 9.5.1、KGP/Compose 编译器 2.4.21；移除
`org.jetbrains.kotlin.android`，built-in Kotlin + top-level `kotlin { jvmToolchain(17) }`，
`kotlin.directories` / `jniLibs.directories` 迁移；未使用 `builtInKotlin=false` / `newDsl=false`。
JVM 244 tests/variant PASS；生产签名证书与双 ABI 不变；16 JNI 符号/ABI；R8 strict keep 静态核验通过；
proto 生成零 diff；16 KB 对齐保持；cargo test 全绿。证据：`evidence/ticket-24/`。
仅新增兼容标志 `android.onlyEnableUnitTestForTheTestedBuildType=false`；
`kotlin("test")` 改为 `kotlin-test-junit:2.4.21`（built-in Kotlin 不选 JUnit 变体）。
**EMULATOR** 全设备用例仍 NOT RUN，待最终构建统一采集后置 resolved。

## Comments

2026-11-18 — EMULATOR 补齐：最终集成 tip `5ff8936`（含 AGP 9.5.1 / Gradle 9.7.1 /
Kotlin 2.4.21 与后续阶段）在 16 KB AVD 上 ready/rebind/recreate/lock/stop 全部 PASS；
双 ABI APK 在 API 26 AVD 安装并启动 PASS。真机 NOT RUN。本票 resolved。
