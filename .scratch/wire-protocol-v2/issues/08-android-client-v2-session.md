# 08: Android Client v2 control-channel session

**What to build:** Android Client 的 v2 控制通道会话：以 prototype `prototype/wire-protocol-v2-transport/android-client/` 为种子，把 native 会话对齐到已接受的 normative 规范与 schema，交付「能连上 06/07 的 Server 并完成认证、停在 READY」的客户端半边。媒体是后续阶段（Tracer-Bullet 第 4 步）。

**Blocked by:** 05

**Status:** claimed

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
