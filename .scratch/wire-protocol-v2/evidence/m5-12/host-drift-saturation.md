# M5-12: 漂移修正的噪声饱和 — host 证据

日期：2026-11-22（含两轴审查后的修订）。分支 `fix/m5-12-drift-saturation`，
基线 `integration/m5` @ `51a7aae`。本文件只记录**实际执行过**的命令与结果。
状态词仅使用 `PASS` / `FAIL` / `NOT RUN` / `WAIVED` / `EMULATOR`；真机复测与
听感 A/B 均为 `NOT RUN`。

## 0. 第 0 步：执行链路与闭环

### 0.1 速率修正确实作用到 `AudioTrack`

| 环节 | 位置 |
|---|---|
| 控制器产出 `rate_ppm` | `native/src/drift.rs:471`（`update_from_residual`） |
| 会话读出 | `native/src/media_session.rs:862` `drift_correction_ppm()` |
| JNI | `native/src/jni.rs:664` `Java_dev_audioshare_v2_ClientNative_driftCorrectionPpm` |
| Kotlin external | `app/.../ClientNative.kt:212` |
| 播放循环取用 | `app/.../MediaPlayer.kt:693` `sink.setPlaybackRateHz(playbackRateHz(...))` |
| ppm → 整数 Hz | `app/.../MediaPlayer.kt:26` `playbackRateHz` |
| 平台 API | `app/.../MediaPlayer.kt:342` `setPlaybackRateHz` → 直接调用 `track.setPlaybackRate(rateHz)`（:344） |

结论：修正**真的**通过 `AudioTrack.setPlaybackRate` 作用到播放速率，不是纯日志现象。
符号正确：`+ppm`（采集时钟领先、缓冲增长）→ 更高的 Hz → 播放更快。单位正确：ppm
映射为整数 Hz。

### 0.2 闭环：估计量是“残差”，不是开环漂移

`MediaPlayer.playbackHeadFrames()` 用 `getTimestamp()` 的
`(framePosition, nanoTime)` 对，并用 `appliedRateHz` 插值到查询时刻
（`MediaPlayer.kt:320`，helper `interpolatedHeadFrames` 在 `:116`）。因此播放头按
**已施加的速率**前进。估计器观察的是 `capture - playback` 的斜率：

```
residual ≈ d - applied        （d = 真实时钟差，applied = 实际施加速率）
```

旧控制器把 `residual` 当成**绝对目标**，闭环稳态于是 `applied = d - applied`，
即 `applied = d/2`。修复后控制器用

```
open_loop = residual + mean(实际施加的修正)
```

重建 `d`，再对 `d` 做长基线/阻尼平滑。旧的 `DriftController::update` 保留为开环
语义的兼容入口；生产路径 `MediaSession` 使用 `update_from_residual`
（`native/src/media_session.rs:654`）。

### 0.3 执行器量化与命令上限

- `AudioTrack.setPlaybackRate` 只接受整数 Hz（`playbackRateHz` 返回 `Int`）。
  48 kHz 下 1 Hz ≈ 20.83 ppm。
- 旧 `playbackRateHz` 先 clamp ppm 到 ±100 再 `roundToInt`：48 kHz 下 +100 ppm →
  5 Hz → **+104.2 ppm，越过 §8 的 ±100 ppm 上限**。已改为按 Hz 域夹紧：
  48 kHz 最多 4 Hz = 83.3 ppm；8 kHz 下没有任何整数 Hz 落在 ±100 ppm 内，因此为 0。
- native 侧 `actuator_ppm`（`native/src/drift.rs:644`）镜像同一映射，并**使用
  `DriftConfig.max_correction_ppm`**（不是全局常量）：配置调小时，重建也不会记录
  越界速率；默认 100 ppm 行为不变。
- 控制目标本身也夹到执行器可达上限 `actuator_ppm(max_correction_ppm, ...)`，
  因此 `applied_ppm`/`correction_ppm` 不会再报一个平台达不到的 100 ppm。§8 的
  ±100 ppm 仍是**上界**（不许超过），不是必须达到的命令值。

| 采样率 | §8 上限 | 可达整数 Hz | 实际 ppm |
|---|---|---|---|
| 48 kHz | 100 | ±4 Hz | ±83 |
| 44.1 kHz | 100 | ±4 Hz | ±90 |
| 8 kHz | 100 | 0 Hz | 0 |

- 由此得到一个诚实的能力边界：真实漂移在 84–100 ppm 时，48 kHz 速率修正只能到
  83 ppm，剩余量交给 §8 既有整帧干预。**真机验收的“顶轨比例”判据只对
  \|d\| ≤ 83.3 ppm 有意义**；超过时修正停在执行器物理上限，不是噪声饱和。
  这不是放宽判据，而是写清适用域。

### 0.4 SRC 被拒时的状态分叉

`setPlaybackRateHz`（`MediaPlayer.kt:342`）在 `setPlaybackRate` 返回非 `SUCCESS`
时只写 `Log.w("AudioTrackSink", "setPlaybackRate(...) rejected")` 并保持上一档速率；
native 的 `applied_ppm` 仍认为修正已生效。**若该路由不支持 SRC，速率修正整体失效，
“顶轨”就只是日志现象。** 现有三轮证据只记录了 `source=hardware`，没有记录
`setPlaybackRate rejected` 计数，因此“该路由接受 SRC”本轮为 **`NOT RUN`**。
下次真机复测必须 `grep 'setPlaybackRate(.*) rejected'`；若出现，本工单按 `FAIL` /
`NOT RUN` 记录，不能宣称噪声饱和已消除。本轮未新增计数器。

**这是一个明确的回归风险。** 修复后的重建公式假设实际速率 = 命令速率。若 SRC 被拒，
实际速率保持 0，残差仍为 `d`，而 `open_loop = residual + mean(commanded)` 会变成
正反馈并一路顶到执行器上限；旧控制器在同样条件下只会收敛到 `d`。要根治，需要
Kotlin 把“实际接受的 Hz”回报给 native（新增 JNI 上行），或由 native 用回报的实际
速率重建。本轮明确**不做**这个上行改动（超出 M5-12 范围），因此在非 SRC 路由上，
本修复不退化为更差的唯一前提是设备确实接受 `setPlaybackRate`。真机复测必须先确认
这一点（`NOT RUN`）。

### 0.5 缓冲误差 PI：未采用

真机 `buffer_current_ms` 长期 0–28 ms（目标 60 ms）。若这是 ring 几何或
`submitted_position` 口径的结构性偏差，直接用缓冲误差驱动 PI 会朝 −100 ppm 反向
饱和。根因未查明，本工单不采用 PI，只保留 §8 既有整帧干预逻辑。

## 1. Host 噪声模型

闭环模型（`tests/clock_drift_contract.rs` 的 `ClosedLoop`）：

- 报告周期 100 ms ± 3 ms：`[100, 103, 99, 101, 100, 102, 98, 100]`。
- 采集时钟按 `RATE * (1 + drift/1e6)` 前进；播放时钟按**整数 Hz 执行器实际达到的
  速率**前进（独立 oracle 在 `tests/common/mod.rs`，不调用生产映射）。
- 位置测量噪声：每 `hold_ms` 重新抽取一个偏移，均匀整数落在
  `[-q/2, +q/2]` 帧（`q` = 10/15/20 ms），即 10–20 ms 的量化台阶，不是白噪声。
- 每个报告：`observe → ppm()/residual_ppm() → update_from_residual`；不使用私有函数。
- 顶轨定义：测量窗内 `|correction_ppm| == 100` 的周期占比。
- **长基线 180 s + 20 s 一阶阻尼**。180 s 是工单授权的“更长积分”，不是 spec §8
  原文；§8 的 10 s 测量窗语义（`ppm()`/`drift_ppm`）完全未变。
- 测量窗 240–300 s（598 个控制周期），让 180 s 基线先填满并稳定。
- 两种子 `0x6d5a56da`、`0x12345678`；漂移 0 / ±75 /（适用域用例）+90 ppm。

标定结果（RED 运行里 10 s 原始估计的范围）：约 `-2,087 … +1,692 ppm`，与真机
`-1,807 … +723` / `-1,294 … +1,209` / `-467 … +532` 同量级。

暖机期（60–170 s）使用同一残差语义，但把“已施加修正的均值”对齐到 **10 s 测量窗**
（不是 180 s），否则短窗残差与长窗均值错配，会留下系统性偏差；180 s 基线填满后，
残差与均值都改用 180 s，两者始终同跨距。

## 2. RED（未修改实现）

命令：

```sh
cd android-client-v2/native
cargo test --test clock_drift_contract \
  quantised_position_noise_must_not_saturate_the_rate_correction -- --nocapture
cargo test --test clock_drift_contract \
  a_real_seventy_five_ppm_drift_is_tracked_without_long_term_saturation -- --nocapture
```

修复前（提交 `de6573b`）的实际输出片段：

```text
M5-12 drift=0 q=10ms hold=1000ms seed=0x6d5a56da: rail=55.0% mean_applied=+3.4 max|applied|=100 raw=[-780,917]
M5-12 drift=0 q=15ms hold=1000ms seed=0x6d5a56da: rail=68.4% mean_applied=+9.0 max|applied|=100 raw=[-1028,1059]
M5-12 drift=0 q=20ms hold=2000ms seed=0x6d5a56da: rail=68.6% mean_applied=+8.8 max|applied|=100 raw=[-2079,1705]
M5-12 drift=+75 q=10ms hold=1000ms seed=0x6d5a56da: rail=55.5% mean_applied=+20.3 err=-54.7 raw=[-707,990]
M5-12 drift=+75 q=20ms hold=2000ms seed=0x6d5a56da: rail=69.9% mean_applied=+15.5 err=-59.5 raw=[-2012,1766]
M5-12 drift=-75 q=20ms hold=2000ms seed=0x12345678: rail=65.7% mean_applied=-9.5 err=+65.5 raw=[-1327,1680]
...
test result: FAILED. 0 passed; 1 failed
```

汇总（12 个无漂移单元 + 24 个 ±75 单元）：

| 指标 | RED（旧实现） | 阈值 |
|---|---|---|
| 无漂移顶轨比例 | 53.2 % … 68.6 %（中位 62.7 %） | < 10 % → **FAIL** |
| ±75 顶轨比例 | 53.2 % … 69.9 %（中位 63.7 %） | < 10 % → **FAIL** |
| ±75 稳态误差 | −82.0 … +86.9 ppm | ≤ 25 ppm → **FAIL** |
| 无噪声 drift=0 | mean_applied = 0.0 | ≤ 5 ppm → PASS（守门用例） |

失败原因有两层：10 s 窗口噪声把目标推过 ±100；闭环残差语义又让真实漂移只被修正
约一半（`d/2`）。RED 输出是 `de6573b` 在旧实现上的真实运行；后续审查提交只改了
命名/上限/暖机语义，未改变失效模式。

## 3. 三种方案取舍

| 方案 | 做法 | Host 结果 | 结论 |
|---|---|---|---|
| (a) 长时常数 EMA | 对 10 s 估计做 τ=30–120 s EMA 后再驱动 | 漂移 0 时仍见 11–45 % 顶轨；±75 时 10–45 %。10 s 估计本身相关时间已达 ~10 s，EMA 压不住低频游走 | 单独用不足 |
| (b) 小估计死区/滞回 | 把 \|estimate\| 小于阈值时置零 | 要吞掉 ±1,000–1,800 ppm 噪声，死区必须 >1,800 ppm，连真实 ±75 ppm 一起吞掉 | 否决 |
| (c) 更长基线斜率（选用） | 180 s 最小二乘残差斜率驱动控制；10 s 窗口只保留诊断 | 见 §4：0 % 顶轨，误差 ≤12.8 ppm | **采用** |
| 缓冲误差 PI | 按 `buffer_current_ms − target` 驱动 | 真机缓冲 0–28 ms，会反向饱和；根因未明 | 不采用 |

最终实现是 (c) + 轻量 (a)：180 s 直接斜率重建开环漂移，再做 20 s 一阶阻尼。
180 s 依据：量化斜率误差约 `q/T`，10 ms / 180 s ≈ 55 ppm，再经最小二乘与阻尼降到
个位数 ppm；最坏 ±100 ppm 未修正漂移在 180 s 内只积累约 18 ms，仍在 60 ms 目标 /
200 ms 硬限内。离线 Python 标定曾在 `q=20 ms, hold=3 s` 等更严苛单元上比较 60/90/
120/150/180 s；180 s + 20 s 阻尼是唯一在全部单元为 0 % 顶轨的档位。该标定只是
**离线模型**，不是 Rust 测试结果。

## 4. GREEN

### 4.1 命令上限（审查项 4）

```text
sample_rate 48 kHz : control target saturates at 83 ppm (4 Hz), never 100
sample_rate 44.1 kHz: 90 ppm (4 Hz)
sample_rate 8 kHz  : 0 ppm (no whole-Hz step inside ±100 ppm)
```

测试：`clock_drift_contract::the_controller_command_is_capped_at_the_actuator_limit`、
`drift::tests::the_controller_command_never_exceeds_the_actuator_limit`、
`drift::tests::integer_hz_actuator_stays_inside_the_ppm_bound`。

### 4.2 噪声矩阵

```text
M5-12 drift=0 q=10ms hold=1000ms seed=0x6d5a56da: rail=0.0% mean_applied=-1.3 max|applied|=2 raw=[-714,878]
M5-12 drift=0 q=15ms hold=1000ms seed=0x12345678: rail=0.0% mean_applied=-9.6 max|applied|=17 raw=[-1520,920]
M5-12 drift=0 q=20ms hold=2000ms seed=0x12345678: rail=0.0% mean_applied=+15.2 max|applied|=16 raw=[-1250,1664]
M5-12 drift=+75 q=10ms hold=1000ms seed=0x6d5a56da: rail=0.0% mean_applied=+75.5 err=+0.5 raw=[-722,870]
M5-12 drift=+75 q=20ms hold=2000ms seed=0x12345678: rail=0.0% mean_applied=+83.0 err=+8.0 raw=[-1238,1676]
M5-12 drift=-75 q=15ms hold=2000ms seed=0x12345678: rail=0.0% mean_applied=-81.4 err=-6.4 raw=[-926,977]
M5-12 noiseless drift=0: mean_applied=+0.0 max|applied|=0
```

| 指标 | GREEN（修复后） | 阈值 | 结论 |
|---|---|---|---|
| 无漂移顶轨比例 | 12/12 单元 0.0 % | < 10 % | **PASS** |
| ±75 顶轨比例 | 24/24 单元 0.0 % | < 10 % | **PASS** |
| ±75 稳态误差 | −7.7 … +12.8 ppm | ≤ 25 ppm | **PASS** |
| 无噪声 drift=0 | mean_applied = 0.0，max 0 | ≤ 5 ppm | **PASS** |
| §8 ±100 ppm 上界 | 命令始终 ≤ 83（48 kHz 可达上限），≤ 100 | 不变 | **PASS** |

### 4.3 暖机期（审查项 6）

无噪声、真实 +75 ppm，测量 60–170 s（180 s 基线尚未填满）：

```text
M5-12 warm-up drift=+75 (60-170 s): mean_applied=+74.5 max|applied|=77
```

不是旧的 `d/2 = 37.5`。暖机期仍可能因 10 s 噪声顶轨；**2026-11-22 维护者裁决**：真机顶轨
比例只统计 `residual_ppm != none` 的快照，整段会话（含暖机/重建）比例单列、不混入判据，
复测会话至少 8–10 分钟。这不改变 §8 判据，只明确新控制器的适用域。

### 4.4 84–100 ppm 适用域（审查项 5）

真实 +90 ppm、48 kHz：

```text
M5-12 drift=+90: mean_applied=+83.0 max|applied|=83 residual_mean=+6.9
```

命令停在执行器上限 83 ppm；残差约 7 ppm 仍在；同一控制器在持续 >20 ms 缓冲偏差下
仍产生 §8 整帧干预（该用例同时断言 `frame_adjust != 0`）。真机“顶轨比例”判据只对
\|d\| ≤ 83.3 ppm 有意义。

### 4.5 生产会话路径（审查项 10）

`MediaSession::record_playback_with_head_at` 闭环，300 s，0 / +75 / −75：

```text
M5-12 session drift=+0: rail=0.0% mean=-4.3 samples=599
M5-12 session drift=+75: rail=0.0% mean=+71.9 samples=599
M5-12 session drift=-75: rail=0.0% mean=-81.2 samples=599
```

## 5. 执行的验证命令

Rust（`android-client-v2/native`）：

```sh
cargo test
cargo test --features test-support
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

实际结果：

```text
cargo test                        : 10 个 test binary，260 passed, 0 failed
cargo test --features test-support: 10 个 test binary，260 passed, 0 failed
cargo clippy --all-targets ...    : Finished `dev` profile，无 warning
cargo fmt --check                 : exit 0
```

其中：

```text
clock_drift_contract           : 27 passed
clock_drift_session_contract   : 25 passed（24 个既有用例 + 1 个 M5-12 闭环会话用例）
```

JVM（Kotlin 无本轮改动，但仍复跑；`ANDROID_HOME` + JDK 21）：

```sh
cd android-client-v2
ANDROID_HOME=$HOME/Android/Sdk ./gradlew :app:testDebugUnitTest --offline \
  -Porg.gradle.java.installations.paths=$HOME/.cache/audio-share-refactor-worktrees/toolchain/jdk-21.0.13+9
```

实际结果：

```text
BUILD SUCCESSFUL
testDebugUnitTest: tests=278 failures=0 errors=0 skipped=0
```

这是 **host JVM** 结果，不是真机结果。

## 6. 诊断字段（保持 + 加法）

- `playback.drift`（`native/src/session.rs:1241`）新增 `residual_ppm=…`：
  它是驱动控制器的 180 s 长基线**残差**（`d − applied`）；`drift_ppm` 仍是 §8
  10 s 原始值，语义未变、未钳位。
- `playback.drift_window status=rebuilding`（`:1253`）同样新增 `residual_ppm=…`。
- 没有删除或重命名任何既有 wire/日志字段；§9 wire 协议未改。
- `MediaSession::drift_residual_ppm()`（`native/src/media_session.rs:821`）只是
  **会话日志入口**，并未在 `jni.rs` 暴露；它不是 JNI 接口。
- 真机验收口径：顶轨比例只统计 `residual_ppm != none` 的快照；暖机/重建段单列。

## 7. 因审查修改的既有断言

| 测试 | 旧断言 | 新断言 | 理由 |
|---|---|---|---|
| `the_controller_clamps_correction_at_one_hundred_ppm` | 100 / −100 | 83 / −83（并断言 ≤100），改名为 `..._at_the_reachable_ceiling` | 48 kHz 整数 Hz 只能到 4 Hz = 83 ppm；§8 100 ppm 仍是上界 |
| `a_zero_estimate_slews_back_to_zero` | 先到 100 | 先到 83 | 同上 |
| session `a_synthetic_one_hundred_ppm_capture_clock_is_measured_and_clamped` | 100 | 83（并断言 ≤100） | 同上 |
| session `the_correction_never_exceeds_plus_or_minus_one_hundred_ppm` | 100 / −100 | 83 / −83（并断言 ≤100） | 同上；测试名仍是“不得超过 ±100” |
| `impairment_matrix` row8 | `measurement.clamp(-100,100)` | ±83（并断言 ≤100） | 同上 |
| session `one_stalled_delivery_invalidates_old_drift_and_recovers_real_nonzero_drift` | 修正 74–76 | 74–83 | 该 fixture 的 accepted 时钟是开环；残差语义把开环漂移识别为 `residual + applied`，因此饱和到 83。测试真正钉住的“停滞作废旧窗、恢复真实漂移”不变 |
| Kotlin `MediaPlaybackTest`（上一轮） | 48005 / 47995 | 48004 / 47996 | Hz 域夹紧后 100 ppm 只能到 4 Hz |
| 窗口重建 | 修正回 0 | 保持修正回 0 | 有测量时用残差语义；`measurement` 与 `residual` 都缺失（重建）时目标仍为 0，保持既有行为 |

## 8. 残余风险 / NOT RUN

- **真机复测（顶轨比例、缓冲、source、SRC 接受）**：`NOT RUN`。验收口径见 §6；
  必须先确认无 `setPlaybackRate(...) rejected`。
- **听感 A/B**：`NOT RUN`，人工门。
- **SRC 是否被设备路由接受**：`NOT RUN`；若被拒，新的重建可能正反馈顶到执行器上限
  （旧控制器只会收敛到 `d`）。根治需要“实际接受速率”上行回报，超出本工单范围。
- **暖机期与每次重建后的 180 s**：已改为残差语义，不再系统性半修正；但 10 s 噪声
  仍可能顶轨。真机统计需排除 `residual_ppm=none` 段。
- **accepted-bytes 回退时钟**：生产证据 `source=hardware` 全程有效；若切到 accepted，
  重建假设 accepted 进度也反映已施加速率。该假设在真机为 `NOT RUN`。
- **8 kHz 等低采样率**：±100 ppm 内没有整数 Hz 可达，执行器为 0 ppm；真机 `NOT RUN`，
  依赖 §8 整帧干预。
- **84–100 ppm 真实漂移**：48 kHz 只能补到 83 ppm，剩余由整帧干预兜底；该适用域
  在真机 `NOT RUN`。
- **§8 其它判据**：±100 ppm 上限、60/200 ms、20 ms/5 s 持续偏差、re-anchor 触发、
  wire 协议均未改动。

## 9. 提交

- `de6573b` test(m5-12): reproduce noise-driven drift correction saturation (RED)
- `360b7fb` fix(m5-12): clamp the integer-Hz executor inside the §8 ppm bound
- `d6307b7` fix(m5-12): drive drift control from a long baseline and close the rate loop
- `6d00114` fix(m5-12): reset damped control state and cover the session path
- `50b20f8` / `8b606d3` docs(m5-12): 首轮 host 证据
- `c779e73` fix(m5-12): address two-axis review (residual naming, actuator cap, warm-up)
- `18daffe` fix(m5-12): align warm-up residual and applied means to the same span
- 本条证据文件单独提交。
