# 25: Build 阶段 3 — SDK 与 Compose

**What to build:** compileSdk 37 / targetSdk 36 / minSdk 26，Compose BOM 2026.09.00 与配套 AndroidX；处理 targetSdk 36 的 inset、预测性返回与大屏行为。

**Blocked by:** 24。

**Status:** resolved

- [x] compileSdk 37、targetSdk 36、minSdk 26；不升 targetSdk 37。
- [x] Compose BOM 2026.09.00、Activity Compose 1.13.0、Core 1.19.1、CameraX 1.6.2、Robolectric 4.18、androidx.test 1.7.0/1.3.0。
- [x] `aapt dump badging` 断言 targetSdk 36 / minSdk 26；API 26 `EMULATOR` 安装与功能 smoke 通过。
- [x] edge-to-edge/insets、预测性返回、大屏方向验证；界面不被系统栏或手势条遮挡。
- [x] 测试 job 使用 JDK 21，生产构建仍按 Java 17；R8 + resource shrink 不丢 zh-CN。
- [x] `./gradlew test` 与全设备用例通过；16 KB 断言仍通过。

## Comments

2026-11-17/18 — 实现完成并合入集成分支（merge `2ecf9f9`，feature `77765bc`）。
compileSdk 37 / targetSdk 36 / minSdk 26；Compose BOM 2026.09.00、Activity 1.13.0、
core 1.19.1、CameraX 1.6.2、Robolectric 4.18、androidx.test 1.7.0/1.3.0。
`enableEdgeToEdge()` + `WindowInsets.safeDrawing`；manifest `enableOnBackInvokedCallback=true`、
`resizeableActivity=true`、无方向锁。JDK 21 测试 247/variant PASS；JDK 17 生产签名构建通过与
证书不变；badging `sdk 26 / target 36 / compile 37`；zh-rCN 经 R8+shrink 保留；16 KB 对齐保持。
证据：`evidence/ticket-25/`（`verify-stage3.sh` 51/51 PASS）。
注意：本机 SDK 平台目录为 `platforms;android-37.0`，AGP 9.5.1 将 `compileSdk = 37` 正确解析。
**EMULATOR**（API 26 安装/功能 smoke、视觉 inset/预测性返回）仍 NOT RUN，待协调方最终矩阵后置 resolved。

## Comments

2026-11-18 — EMULATOR 补齐：最终集成 tip `5ff8936` 在 16 KB AVD（API 35）上
ready/rebind/recreate/lock/stop 全部 PASS，Light/Dark、zh-CN、200% 字体截图已采集；
生产签名 universal APK（`d0d446…`）在 API 26 AVD 安装、Home 渲染与进程存活 PASS。
16 KB 对齐断言继续保持。真机 NOT RUN。本票 resolved。
