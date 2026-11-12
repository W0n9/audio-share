# 08: Android Client v2 control-channel session

**What to build:** Android Client 的 v2 控制通道会话：以 prototype `prototype/wire-protocol-v2-transport/android-client/` 为种子，把 native 会话对齐到已接受的 normative 规范与 schema，交付「能连上 06/07 的 Server 并完成认证、停在 READY」的客户端半边。媒体是后续阶段（Tracer-Bullet 第 4 步）。

**Blocked by:** 05

**Status:** resolved

**Type:** task

**Note:** 与 06/07 并行 —— 本工单以 fixtures 为对拍基准独立开发，交叉验证在 09。

## Requirements

- 新的 Android Client 代码骨架独立于 prototype 目录（建议仓库根 `android-client-v2/` 或复用现有 `android-app/` 新增模块 —— 实现者读 `android-app/` 构建配置后二选一并记录理由；**必须**保持 `android-app/`（v0.3.4 兼容线）行为与构建不受影响）。baseline：Android 8 / API 26 minSdk、compileSdk 35。
- Native Rust cdylib（从 prototype native crate 演化）：接入 `protos/wire-protocol-v2/control.proto` —— 用 **prost**（workspace 已有 0.13）做 Rust 侧 codegen，替换 prototype 的手写帧 codec；§5 Client 状态机逐行实现；保留 prototype 已验证的骨架：SPKI-pinned verifier（proto-core 单源）、JNI 按值 JNIEnv（教训）、direct ByteBuffer ring（本阶段仅保骨架，媒体路径不激活）、有界退避重连（±20% jitter）、NetworkCallback rebind 尝试 + 回退。
- 认证（pairing.md §9）：客户端实现 `AuthInit/Proof` 的签名侧 —— P-256 密钥优先 Android Keystore（PR-2；Keystore 不可用时测试期允许非 Keystore 测试钥 + 文档标注）；测试期 SPKI pin 来自预置 Server 公钥（与 07 的测试凭据预置同源的 test-only 配置；生产 pairing 第 6 阶段）。
- 控制消息收发走 prost `Frame` envelope（oneof band 原样）；非致命 402 保持连接语义；请求 id 配对；错误/日志红线同 07。
- Kotlin 侧 protobuf-kotlin-lite（`libs.versions.toml` 已有 4.28.2 + plugin 0.9.4，照 `android-app/` 接法）编译同一 `.proto` —— 两端同源 schema；JNI 面保持低频（connect/auth/stop/state/stats/rebind）。
- **Vector 回放测试**：Android 侧（native crate host 测试 + Kotlin JVM 测试可行处）对 `fixtures/wire-protocol-v2/` 的 control happy/error 案例（34+3）做双向字节对拍 —— Rust↔Kotlin 各自对同一 fixtures，不得互为编码器。

## Definition of done

- native crate host `cargo test` 绿（状态机 Client 表 + prost codec + verifier + 认证签名）。
- Vector 回放两端一致（byte-level，manifest 全绿）。
- Gradle 构建 arm64 release 绿；stripped `.so` 尺寸记录（对照 prototype 的 2.20 MiB 基线，明显膨胀要解释）。
- `android-app/` v0.3.4 线构建/测试不受影响。
- 真机安装冒烟（adb install + 启动到 IDLE 即可；完整握手联调在 09）。

### 2026-11-12 — implemented on `ticket/08-android-client-v2`（final commit：本条注释所在 commit）

除真机冒烟外全部 DoD 项完成；冒烟因设备不可达记 NOT RUN（见末条），故 Status 置 `ready-for-human`。

- **目录决策：仓库根 `android-client-v2/`（独立 gradle root）**，不并入 `android-app/`。理由：v0.3.4 线自带机器本地签名配置（`keystore.properties`）、compose UI 与 release signing —— 往该构建里嫁接第二套 cargo/NDK 工具链会威胁其 release 路径；独立 root 保证 `android-app/` 零改动（已验证：worktree 内 `assembleDebug` + unit tests 绿，仅差异是 gitignored 的 keystore.properties 本就不在 git 里）。版本镜像 android-app（AGP 8.7.3 / Kotlin 2.0.21 / Gradle 8.9 / protobuf-kotlin-lite 4.28.2 + plugin 0.9.4）。commit 链 `63bd3fe → 56223b1 → 3c1c314 → b20208d → 9e3d64e → 0157950 → 9e64818`，`phase-2/wire-protocol-v2` 已并回 @ `0df4d73`。
- **Native crate**（`android-client-v2/native/`，独立 crate 不入 workspace）：prost 0.13 codegen 编译冻结的 `protos/wire-protocol-v2/control.proto`（替换 prototype 手写 codec）；§5.3 Client 状态机逐行实现（含非致命 402 保持连接、Fatal→Error+Close 双发、CLOSED 吸收）；SPKI-pinned verifier；认证签名侧按 pairing.md §9.1 —— 单次 ECDSA-P256-SHA256 over RAW auth_context、64 字节 r||s P1363（Keystore 路线属生产 pairing 第 6 阶段，本票按票面允许用测试钥并已标注 test-only）；quinn 0.11 会话（ALPN audio-share/v2、无 0-RTT、5 s 握手超时）；有界退避（1 s 基数 ×2、30 s 封顶、±20% jitter）；NetworkCallback rebind + 失败回退；JNI 按值 JNIEnv（prototype 教训保留），7 个低频入口（createSession/connect/stop/destroy/getState/getStats/rebind）；direct ByteBuffer ring 随骨架保留但**媒体路径未激活**（Tracer-Bullet 第 4 步）。
- **测试：native host 83 + Kotlin JVM 18，全绿**。Vector 回放按 README 契约双向字节对拍：native 侧 41 个 control+limits 案例双向 + 状态机方向（client 发出的 Close(701)、Error+Close(403) 帧逐字节核对）+ 10 个 media 案例中 8 个（header/malformed/flags/epoch/wrap/floor）；Kotlin 侧以 protobuf-kotlin-lite 独立重建 `decoded` → 编码方向字节相等 + 解码 round-trip，覆盖全部 41 个 control+limits 案例与全部 10 个 media 案例（各带 manifest drift guard）。Rust↔Kotlin 各自读 fixtures，互不作为对端编码器。回放中发现并修复 fixture 缺陷一处：`error-201-pairing-declined` 的字节把 Error/Close 编成 401，与 decoded/manifest/断言的 201 矛盾，字节已按 spec 改为 varint `c9 01`（b20208d，38 个 control+limits 案例经 python 独立 walker 重新全验）。
- **stripped .so：2,355,888 B（2.25 MiB）**，对照 prototype 基线 2,305,128 B（2.20 MiB）**+50,760 B（+2.2%）** —— prost codegen 与 session/verify/auth 模块替换手写 codec 后的净增量，无异常膨胀（release profile：opt-level "z"、lto="fat"、codegen-units=1，未启用 panic=abort 因 JNI 需 catch_unwind）。`assembleRelease` 绿（R8 + shrinkResources 开、debug-signed 可安装；APK 2,467,726 B，JNI 7 符号经 llvm-nm 验证导出）。
- **`android-app/` 不受影响**：worktree 内 `assembleDebug` + `testDebugUnitTest` 绿（唯一环境差异是 gitignored 的 keystore.properties，非本票改动）。
- **真机冒烟：NOT RUN** —— MIUI 设备经 tailscale 不可达（ping 100% loss、`adb connect` 超时，与本票开工前一致）。release APK 已产出可安装；安装冒烟 + 与 06/07 的完整握手联调归 09（票面 Note）。
- 错误/日志红线：错误字符串不含凭据、私钥、原始媒体（createSession/connect/rebind 失败路径与 stats JSON 均为计数与状态名）。

### 2026-11-12 — device smoke test PASSED（设备恢复后补跑，resolved）

设备（Xiaomi M2007J22C / Android 12 / tailscale `100.117.189.121`）恢复在线后补跑真机安装冒烟：

- `adb install -r app-release.apk`（2,467,726 B，R8 + debug-signed）→ `Success`。
- `am start -W -n dev.audioshare.v2/.MainActivity` → COLD 启动 540 ms，无 FATAL/AndroidRuntime 崩溃，`mResumedActivity` 为该 Activity。
- screencap 确认 UI 显示 **IDLE** 状态、副标题「config missing: pass host/pinnedSpki/serverKey extras (test config)」—— 与「启动到 IDLE 即可」的票面验收一致（无 extras 时预期行为）。
- 完整握手联调（与 06/07 Server 的真机交叉验证）按票面 Note 归 09。

至此 08 的 DoD 全项完成，Status → `resolved`。冒烟所用 APK 构建于 `ticket/08-android-client-v2`（`1fb9de5`，已合入集成分支 `f727ba6`，与本条同源代码）。
