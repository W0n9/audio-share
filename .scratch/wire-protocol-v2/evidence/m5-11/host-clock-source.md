# M5-11: hardware playout clock — host evidence

日期：2026-11-21。本文件只记录**实际执行过**的命令与结果。真机相关项一律 `NOT RUN`。

## 基线 / 合并状态

- 独立工作树：`.claude/worktrees/agent-af2aa961800adffe3`，初始 HEAD `99263a8`
  （`git reset --hard 99263a8` 后才开始修改；主工作树未被写入）。
- `integration/m5` 本地仍为 `99263a8`；`git merge --no-edit integration/m5`
  输出「已经是最新的」。
- `git ls-remote fork refs/heads/integration/m5` 无输出：fork 远端没有该分支。
- 实现 commit：本提交（代码与证据文件在同一 commit）。

## 实现要点

- Kotlin 报告播放头（单位：**帧**）：优先 `AudioTrack.getTimestamp()` 的
  `(framePosition, nanoTime)` 对，按报告时刻插值；`getTimestamp()` 不可用时退回
  `getPlaybackHeadPosition()`（32 位无符号，按整帧扩展）。读不到时送 `-1`，
  原生记为 fallback，绝不当作 0。
- 原生 drift 序列改用 `anchor + head`。`position_adjust_frames` /
  `drift_playback_step` 只保留在 accepted 路径和 §9 `submitted_position`，
  不对硬件头重复补偿。
- 时钟源切换、sink 重建（head 归零）重置窗口并另行计数；32 位回绕按
  `last > 0xF000_0000 && raw < 0x1000_0000` 扩展 2^32，不误判为重建。
- 诊断只走本地日志：`playback.drift_clock source=… hardware_reports=…
  accepted_fallbacks=… head_reanchors=…`。不新增协议字段。

## RED（实现前，head 被忽略）

命令：

```sh
cargo test --manifest-path android-client-v2/native/Cargo.toml \
  --features test-support --test clock_drift_session_contract \
  hardware_playback_head_keeps_every_batched_window_within_the_b_bound -- --exact --nocapture
```

- **FAIL**：`batch 480 mixer 240 drift 0: 248 windows outside ±100 ppm:
  [-102, -125, -126, -114, 106, 127, 108, 148]`。
  此时 `record_playback_with_head_at(..., Some(head))` 的 head 被占位实现忽略，
  估计器仍走 accepted 字节时钟。

accepted 字节时钟本身的复现（同一文件）：

```sh
cargo test --manifest-path android-client-v2/native/Cargo.toml \
  --features test-support --test clock_drift_session_contract \
  accepted_bytes_quantised_acceptance_exceeds_the_b_drift_bound -- --exact --nocapture
```

- **PASS**（该测试断言 bug 形态）：
  - `batch=1026 estimates=1090 exceeds_b=677 mean=83.81 control=75.00 min=-418 max=559`
  - `batch=2052 estimates=1090 exceeds_b=901 mean=73.33 control=75.00 min=-1201 max=1047`
  - `batch=4104 estimates=0 exceeds_b=0`（块长超过 60 ms 连续性预算，窗口直接饿死）
- 条件：双 48 kHz，注入 +75 ppm，1 ms 采集包，100 ± 20 ms 报告，accepted 进度
  向下量化到 1026/2052/4104 帧。`control` 是同 fixture 的精确 accepted 进度。

## GREEN（硬件头）

```sh
cargo test --manifest-path android-client-v2/native/Cargo.toml \
  --features test-support --test clock_drift_session_contract \
  hardware_playback_head_keeps_every_batched_window_within_the_b_bound -- --exact
```

- **PASS**。矩阵：batch 480/2052/4104 × 注入漂移 0/+75/−75，另加 batch 2052、
  48 帧量化的稳健性档。
- 断言：每个满窗 |raw| ≤ 100；长程均值在注入值 ±3 ppm 内；
  `correction_ppm` 从不到 ±100 轨，末值在注入值 ±10 ppm 内（saturation 修复）。

### 如实记录：head 模型曾经过一次修正

实现后先用 240 帧（5 ms 混音周期）量化 head、按抖动报告时刻采样：

- **FAIL**：`batch 480 mixer 240 drift 0: 43 windows outside ±100 ppm`。
- 960 帧量化：**FAIL**：`batch 2052 mixer 960 drift 0: 309 windows outside ±100 ppm`。

这不是判据放宽：它说明**裸 `getPlaybackHeadPosition()` 的混音周期量化**在抖动报告
时刻下仍可能越界。Kotlin 生产实现因此优先 `getTimestamp()` 的 frame+time 对并插值
到报告时刻（整数帧精度）；host GREEN 用 1 帧和 48 帧量化验证。裸 32 位计数器只作为
`getTimestamp()` 从未成功前的启动回退。该回退路径的逐窗精度 **NOT VERIFIED**，需真机
B 复测确认。

## 其它 host 验证

| 项目 | 命令 | 结果 |
| --- | --- | --- |
| Native 默认特性 | `cargo test --manifest-path android-client-v2/native/Cargo.toml` | **PASS**：247 项，0 失败 |
| Native `test-support` | `cargo test --manifest-path android-client-v2/native/Cargo.toml --features test-support` | **PASS**：247 项，0 失败（本次新增 7；历史 host-drift 证据记录的基线为 240） |
| Native Clippy | `cargo clippy --manifest-path android-client-v2/native/Cargo.toml --all-targets --features test-support -- -D warnings` | **PASS**：退出 0 |
| Rust fmt | `cargo fmt --manifest-path android-client-v2/native/Cargo.toml --all -- --check` | **PASS**：退出 0 |
| JVM debug 单元测试 | `ANDROID_HOME=/home/w0n9/Android/Sdk ./gradlew :app:testDebugUnitTest -Porg.gradle.java.installations.paths=$HOME/.cache/audio-share-refactor-worktrees/toolchain/jdk-21.0.13+9` | **PASS**：276 项，0 失败 |
| JVM release 单元测试 | 同上，`:app:testReleaseUnitTest`（两条任务同一次 invocation） | **PASS**：276 项，0 失败 |
| testSupport 源集编译 | `./gradlew :app:compileDebugKotlin -PtestSupport=true -Porg.gradle.java.installations.paths=…` | **PASS**：退出 0，生成 `TestSupportHarness.class` |

新增的 native 用例：accepted 复现、硬件头 GREEN、位置偏移不双重补偿、
缺失 head 的 fallback、AudioTrack 重建重锚、32 位回绕扩展。
新增 JVM 用例：32 位回绕扩展、重建重启、时间戳抖动保持、`getTimestamp` 插值、
帧单位与 `-1` 哨兵。

## NOT RUN

- 真机用例 B 复测（同一 ±100 ppm 判据、诊断覆盖、re-anchor、缓冲/服务端）：
  **NOT RUN**。因此本工单不写 `resolved`、不合并 beta。
- Android arm64 / APK 构建：**NOT RUN**。
- `getTimestamp()` 在 MIUI 扬声器路由上的可用性、刷新频率与过期行为：
  **NOT RUN**。短暂失效会退回 accepted 并重置 10 s 窗口（`accepted_fallbacks`
  可见），可能打破 `valid_estimates == reports`；需真机确认。
- 真机上 35/67 `correction_ppm` 顶在 ±100 的现象是否消除：host 矩阵已证明
  噪声饱和消失；真机 **NOT RUN**。
- 约 24.9 小时（48 kHz）的 32 位回绕：host 合成覆盖，真机 **NOT RUN**。
