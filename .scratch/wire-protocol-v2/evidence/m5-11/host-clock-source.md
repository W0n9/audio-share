# M5-11: hardware playout clock — host evidence

日期：2026-11-21。本文件只记录**实际执行过**的命令与结果。真机相关项一律 `NOT RUN`。

## 基线 / 合并状态

- 独立工作树：`.claude/worktrees/agent-af2aa961800adffe3`，初始 HEAD `99263a8`
  （先 `git reset --hard 99263a8` 再改代码；主工作树未被写入，早先只有只读访问）。
- 首轮实现 commit：`d60dc9e`。两轴审查后的修复提交基于它，位于本次提交之上。
- `integration/m5` 本地仍为 `99263a8`；`git merge --no-edit integration/m5`
  输出「已经是最新的」。`git ls-remote fork refs/heads/integration/m5` 无输出：
  fork 远端没有该分支。
- 以下命令均从仓库根目录执行；JVM 命令先 `cd android-client-v2`。工作树没有
  `local.properties`，所以显式传 `ANDROID_HOME`。

## 实现要点（含审查修复）

- **32 位扩展单一所有者（Kotlin）**：`PlaybackHeadExtender` 把
  `getPlaybackHeadPosition()` 扩成单调 `Long`。回绕判据是「从计数器顶部到底部」；
  5 ms（按 sink 采样率换算，`sampleRate * 5 / 1000`，最少 1 帧）以内的回退视为抖动；
  超过容差的一次回退先保持上值（可能是 `getTimestamp` 插值年龄回跳），**连续两次**
  才认定轨道重建并把 base 归零。native 收到的契约是「已扩展、单调」，任何回退都只
  按重建处理（重锚 + 清窗），不再有 native 的 `2^32` 分支。
- **时钟源不乒乓**：`getTimestamp()` 成功过一次后，单次失败送 `-1`。native 在
  硬件源激活时把该报告计为 `hardware_missing_reports` 并**跳过这次观测**（不发布
  过期值、不清窗）；仅当距最后一个硬件报告达到 `HARDWARE_HEAD_GIVE_UP_MS = 500`
  时才切到 accepted、清窗并计 `head_give_up_events`。500 ms 依据：正常 100 ms 报告
  间隔的 5 倍，且超过 60 ms 窗口连续性预算的 8 倍，短于此的缺口仍可用线性拟合。
- **时间戳新鲜度**：`MAX_HEAD_AGE_NANOS = 200 ms`。依据：两个正常报告间隔；比这更
  旧的 presentation 时间不再是当前播放进度，视为本次读不到（native 按上面的规则
  跳过或最终切源）。200 ms 内的年龄用 `framePosition + age * rate` 插值到查询时刻。
- **诊断命名**：日志键与 `PlaybackClockStats` 字段完全一致：
  `source / hardware_reports / hardware_missing_reports / accepted_fallback_reports /
  head_reanchor_events / head_give_up_events`。只写本地日志，无协议字段。
- **位置记账**：drift 播放侧用 `anchor + head`；`position_adjust_frames` /
  `drift_playback_step` 只保留在 accepted 路径和 §9 `submitted_position`，不重复补偿。

## RED 证据（修复前）

### 1. accepted 字节时钟复现（首轮）

```sh
cargo test --manifest-path android-client-v2/native/Cargo.toml \
  --features test-support --test clock_drift_session_contract \
  accepted_bytes_quantised_acceptance_exceeds_the_b_drift_bound -- --exact --nocapture
```

实际输出片段（PASS，测试断言 bug 形态）：

```text
accepted batch=1026 estimates=1090 exceeds_b=677 mean=83.81 control=75.00 min=-418 max=559
accepted batch=2052 estimates=1090 exceeds_b=901 mean=73.33 control=75.00 min=-1201 max=1047
accepted batch=4104 estimates=0 exceeds_b=0
```

条件：双 48 kHz，注入 +75 ppm，1 ms 采集包，100 ± 20 ms 报告，accepted 进度向下
量化到 1026/2052/4104 帧。`control` 是同 fixture 的精确 accepted 进度。

### 2. 硬件头被忽略（首轮实现前）

```sh
cargo test --manifest-path android-client-v2/native/Cargo.toml \
  --features test-support --test clock_drift_session_contract \
  hardware_playback_head_keeps_every_batched_window_within_the_b_bound -- --exact --nocapture
```

实际输出片段（FAIL）：

```text
batch 480 mixer 240 drift 0: 248 windows outside ±100 ppm: [-102, -125, -126, -114, 106, 127, 108, 148]
```

### 3. 审查项 1：长时间运行后的重建被误判为回绕

```sh
cargo test --manifest-path android-client-v2/native/Cargo.toml \
  --features test-support --test clock_drift_session_contract \
  an_extended_long_run_head_then_rebuild_reanchors -- --exact --nocapture
```

实际输出片段（FAIL，修复前）：

```text
assertion `left == right` failed: an already-extended regression is a rebuild, not a 2^32 wrap
  left: 0
 right: 1
```

### 4. 审查项 2：单次缺失导致时钟源乒乓

```sh
cargo test --manifest-path android-client-v2/native/Cargo.toml \
  --features test-support --test clock_drift_session_contract \
  periodic_single_missing_heads_keep_the_hardware_window -- --exact --nocapture
cargo test --manifest-path android-client-v2/native/Cargo.toml \
  --features test-support --test clock_drift_session_contract \
  a_sustained_hardware_head_outage_switches_to_accepted -- --exact --nocapture
```

实际输出片段（修复前，均 FAIL）：

```text
every miss is counted: PlaybackClockStats { hardware_reports: 358,
hardware_missing_reports: 0, accepted_fallback_reports: 7,
head_reanchor_events: 0, head_give_up_events: 0 }

assertion `left == right` failed: exactly one sustained-outage switch
  left: 0
 right: 1
```

### 5. 审查项 3：单次年龄回跳被误判为重建

```sh
cd android-client-v2
ANDROID_HOME=$HOME/Android/Sdk ./gradlew :app:testDebugUnitTest \
  --tests 'dev.audioshare.v2.MediaPlaybackTest' \
  -Porg.gradle.java.installations.paths=$HOME/.cache/audio-share-refactor-worktrees/toolchain/jdk-21.0.13+9
```

实际输出片段（修复前，FAIL）：

```text
MediaPlaybackTest > playback_head_extender_restarts_after_a_track_rebuild FAILED
MediaPlaybackTest > head_rewind_tolerance_is_five_ms_at_the_sink_rate FAILED
MediaPlaybackTest > playback_head_extender_tolerates_a_single_age_rewind FAILED
```

## GREEN 证据

```sh
cargo test --manifest-path android-client-v2/native/Cargo.toml \
  --features test-support --test clock_drift_session_contract
```

实际输出片段：

```text
test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

```sh
cargo test --manifest-path android-client-v2/native/Cargo.toml
cargo test --manifest-path android-client-v2/native/Cargo.toml --features test-support
```

实际输出片段（两次运行各 248 项）：

```text
test result: ok. 248 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

```sh
cargo clippy --manifest-path android-client-v2/native/Cargo.toml \
  --all-targets --features test-support -- -D warnings
cargo fmt --manifest-path android-client-v2/native/Cargo.toml --all -- --check
```

实际结果：

```text
Finished `dev` profile [unoptimized + debuginfo] target(s)
fmt check exit 0
```

```sh
cd android-client-v2
ANDROID_HOME=$HOME/Android/Sdk ./gradlew :app:testDebugUnitTest :app:testReleaseUnitTest \
  -Porg.gradle.java.installations.paths=$HOME/.cache/audio-share-refactor-worktrees/toolchain/jdk-21.0.13+9
```

实际输出片段：

```text
BUILD SUCCESSFUL
```

从 `app/build/test-results/testDebugUnitTest/*.xml` 与
`app/build/test-results/testReleaseUnitTest/*.xml` 汇总：

```text
testDebugUnitTest:   tests=278 skipped=0 failures=0 errors=0
testReleaseUnitTest: tests=278 skipped=0 failures=0 errors=0
```

`MediaPlaybackTest` 本身 38 项。

```sh
cd android-client-v2
ANDROID_HOME=$HOME/Android/Sdk ./gradlew :app:compileDebugKotlin -PtestSupport=true \
  -Porg.gradle.java.installations.paths=$HOME/.cache/audio-share-refactor-worktrees/toolchain/jdk-21.0.13+9
```

实际输出片段：

```text
BUILD SUCCESSFUL
```

并确认生成 `TestSupportHarness.class`。

硬件头矩阵断言（batch 480/2052/4104 × 漂移 0/+75/−75，另加 48 帧量化的
2052 档）：逐窗 |raw| ≤ 100，长程均值在注入值 ±3 ppm 内，`correction_ppm` 从不到
±100 轨且末值在注入值 ±10 ppm 内。新增审查用例覆盖：

- 已扩展的长跑 head（> `0xF000_0000`）后重建 → `head_reanchor_events = 1`、清窗、
  有效位置不回退也不跳 2^32；
- 每 5 s 缺一次 head 共 36 s → 保持 `source=hardware`，窗口 span ≥ 10 s，
  `hardware_missing_reports ≥ 6`，`accepted_fallback_reports = 0`；
- 连续 7 次缺失（≥ 500 ms）→ `source=accepted`、`head_give_up_events = 1`、清窗，
  之后 accepted 路径复原 0 ppm；
- 单次大回跳保持上值，第二次才重建；5 ms 容差按采样率换算（48 kHz = 240 帧，
  8 kHz = 40 帧）。

## 如实记录：head 模型曾经过一次修正

实现后先用 240 帧（5 ms 混音周期）量化 head、按抖动报告时刻采样：

- **FAIL**：`batch 480 mixer 240 drift 0: 43 windows outside ±100 ppm`。
- 960 帧量化：**FAIL**：`batch 2052 mixer 960 drift 0: 309 windows outside ±100 ppm`。

这说明**裸 32 位 head 的混音周期量化**在抖动报告时刻下仍可能越界。Kotlin 生产实现
因此优先 `getTimestamp()` 的 frame+time 对并插值到报告时刻（整数帧精度）；host
GREEN 用 1 帧和 48 帧量化验证。裸 32 位计数器只在 `getTimestamp()` 从未成功前作为
启动回退，该回退路径的逐窗精度 **NOT VERIFIED**，需真机 B 复测确认。

## NOT RUN

- 真机用例 B 复测（同一 ±100 ppm 判据、诊断覆盖、re-anchor、缓冲/服务端）：
  **NOT RUN**。因此本工单不写 `resolved`、不合并 beta。
- Android arm64 / APK 构建：**NOT RUN**。
- `getTimestamp()` 在 MIUI 扬声器路由上的可用性、刷新频率与过期行为：
  **NOT RUN**。单次失效现在只跳过该次观测（不清窗）；连续失效 ≥ 500 ms 才退回
  accepted 并清窗。真实设备的失效频率与对 `valid_estimates == reports` 的影响需
  真机确认。
- 真机上 35/67 `correction_ppm` 顶在 ±100 的现象是否消除：host 矩阵已证明噪声
  饱和消失；真机 **NOT RUN**。
- 约 24.9 小时（48 kHz）的 32 位回绕：Kotlin 纯函数与 native 已扩展契约均有 host
  覆盖；真机长时间运行 **NOT RUN**。
- 裸 `getPlaybackHeadPosition()` 启动回退路径的逐窗精度：host 模型显示可能越界，
  真机 **NOT RUN**。
