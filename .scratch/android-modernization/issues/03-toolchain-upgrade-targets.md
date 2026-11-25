# 03: 构建工具链升级目标与顺序

Labels: wayfinder:research
Type: research
Status: resolved
Blocked by: None (can start immediately)

## Question

在当前约束下，给出 Kotlin / AGP / Gradle / Compose BOM / Material3 / Navigation / DI / protobuf / NDK / Rust 的推荐目标版本与升级顺序。

约束：

- `minSdk 26` 不得抬高。
- NDK `27.2.12479018`，Gradle 调 cargo 构建，ABI `arm64-v8a` + `x86_64`。
- release 签名流程与 `-PtestSupport` / `-PdeviceTests` 语义必须保留。
- 当前 Gradle wrapper 8.9、Java 17 toolchain、compileSdk / targetSdk 35。
- 不改 Wire Protocol 行为；工具链升级不得要求改协议或 beta-spec 验收范围。

需要回答：现状版本；推荐目标；破坏性变更；是否触碰 minSdk 26；哪些升级必须先做；三个最高风险与可验证的缓解办法。用官方 release notes / compatibility table 作为来源。

研究子代理已启动；答案以版本矩阵 + 来源链接记录在本工单。

## Answer

已由只读子代理完成调研，完整版本矩阵、升级阶段与来源见 [`../research/toolchain-upgrade.md`](../research/toolchain-upgrade.md)。

结论：

- 推荐终点：**AGP 9.5.1 + Gradle 9.7.1 + Kotlin 2.4.21 + compileSdk 37 / targetSdk 36 / minSdk 26 + Compose BOM 2026.09.00**。
- 两个独立发布阻塞：**16 KB ELF 对齐**（当前 `.so` 的 LOAD 对齐为 4 KB）与 **targetSdk 36**（若未来走 Play；v2 beta 当前走 GitHub Release，平台行为变更仍需验证）。
- **不要升 targetSdk 37**：`ACCESS_LOCAL_NETWORK` 会成为强制运行时权限，直接影响局域网 QUIC 连接。
- 升级顺序：阶段 0（16 KB / CI，可并行）→ 阶段 1（AGP 8.13.2 + Gradle 8.13 + protobuf 0.10.1）→ 阶段 2（AGP 9.5.1 + Gradle 9.7.1 + built-in Kotlin 2.4.21）→ 阶段 3（compileSdk 37 + Compose 1.12 + targetSdk 36）→ 阶段 4（可选 baseline profile / DI）。
- 调研方式说明：使用 Explore 只读子代理，而非 `research` skill；未创建 `research/<name>` 分支，未 push（fork + 本地 tracker；push 需明确授权）。
