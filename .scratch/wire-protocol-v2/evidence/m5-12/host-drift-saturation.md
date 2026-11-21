# M5-12: 漂移修正的噪声饱和 — host 证据

日期：2026-11-22。分支 `fix/m5-12-drift-saturation`，基线 `integration/m5` @
`51a7aae`。本文件只记录**实际执行过**的命令与结果；真机复测与听感 A/B 一律
`NOT RUN`。

## 0. 第 0 步：执行链路与闭环（首要发现）

### 0.1 速率修正确实作用到 `AudioTrack`

链路（均为当前分支行号）：

| 环节 | 位置 |
|---|---|
| 控制器产出 `rate_ppm` | `native/src/drift.rs:423`（`update_with_control`） |
| 会话读出 | `native/src/media_session.rs:848` `drift_correction_ppm()` |
| JNI | `native/src/jni.rs:664` `Java_dev_audioshare_v2_ClientNative_driftCorrectionPpm` |
| Kotlin external | `app/.../ClientNative.kt:212` |
| 播放循环取用 | `app/.../MediaPlayer.kt:693` `sink.setPlaybackRateHz(playbackRateHz(...))` |
| ppm → 整数 Hz | `app/.../MediaPlayer.kt:26` `playbackRateHz` |
| 平台 API | `app/.../MediaPlayer.kt:342` `setPlaybackRateHz` → 直 `track.setPlaybackRate(rateHz)`（:344） |

结论：修正**真的**通过 `AudioTrack.setPlaybackRate` 作用到播放速率，不是纯日志现象。
符号正确：`+ppm`（采集时钟领先、缓冲增长）→ 更高的 Hz → 播放更快。单位正确：ppm
映射为整数 Hz。

### 0.2 闭环：估计量是“残差”，不是开环漂移

`MediaPlayer.playbackHeadFrames()` 用 `getTimestamp()` 的
`(framePosition, nanoTime)` 对，并用 `appliedRateHz` 插值到查询时刻
（`MediaPlayer.kt:320`，helper `interpolatedHeadFrames` 在 `:116`）。因此播放头按
**已施加的速率**前进。估计器观察的是 `capture - playback` 的斜率：

```
estimate ≈ d - applied        （d = 真实时钟差，applied = 当前修正）
```

旧控制器把 `estimate` 当成**绝对目标**（`applied → estimate`），闭环稳态于是

```
applied = d - applied   ⇒   applied = d / 2
```

即真实 ±75 ppm 只被修正一半。这一半修正误差与噪声饱和是两个独立缺陷；host
闭环模型（见 §1）复现了二者。修复后控制器用

```
open_loop = residual + mean(实际施加的修正)
```

重建 `d`，再对 `d` 做长基线平滑。旧的 `DriftController::update` 保留为开环语义的
兼容入口；生产路径 `MediaSession` 改用 `update_with_control`。

### 0.3 执行器量化：整数 Hz，且旧映射越界

- `AudioTrack.setPlaybackRate` 只接受整数 Hz（`playbackRateHz` 返回 `Int`）。
  48 kHz 下 1 Hz ≈ 20.83 ppm。
- 旧 `playbackRateHz` 先 clamp ppm 到 ±100 再 `roundToInt`：48 kHz 下 +100 ppm →
  5 Hz → **+104.2 ppm，越过 §8 的 ±100 ppm 上限**。已在
  `MediaPlayer.kt:26` 改为按 Hz 域夹紧：48 kHz 最多 4 Hz = 83.3 ppm；8 kHz 下没有
  任何整数 Hz 落在 ±100 ppm 内，因此为 0（由 §8 整帧干预兜底）。
- native 侧 `actuator_ppm`（`native/src/drift.rs:558`）复刻同一映射，控制器在重建
  时记录**实际**施加的 ppm，而不是命令值；host 模型同样按整数 Hz 台阶仿真。

### 0.4 SRC 被拒时的状态分叉

`setPlaybackRateHz`（`MediaPlayer.kt:342`）在 `setPlaybackRate` 返回非 `SUCCESS`
时只写 `Log.w("AudioTrackSink", "setPlaybackRate(...) rejected")` 并保持上一档速率；
native 的 `applied_ppm` 仍认为修正已生效。**若该路由不支持 SRC，速率修正整体失效，
“顶轨”就只是日志现象。** 现有三轮证据只记录了 `source=hardware`，没有记录
`setPlaybackRate rejected` 计数，因此“该路由接受 SRC”在本轮 **NOT VERIFIED**。
下次真机复测必须 `grep 'setPlaybackRate(.*) rejected'`；若出现，本工单按 `FAIL` /
`NOT RUN` 记录，不能宣称噪声饱和已消除。本轮未新增计数器。

### 0.5 缓冲误差 PI：未采用

真机 `buffer_current_ms` 长期 0–28 ms（目标 60 ms）。若这是 ring 几何或
`submitted_position` 口径的结构性偏差，直接用缓冲误差驱动 PI 会朝 −100 ppm 反向
饱和。根因未查明，本工单不采用 PI，只保留 §8 既有整帧干预逻辑。

## 1. Host 噪声模型

闭环模型（tests/clock_drift_contract.rs 的 `ClosedLoop`）：

- 报告周期 100 ms ± 3 ms：`[100, 103, 99, 101, 100, 102, 98, 100]`。
- 采集时钟按 `RATE * (1 + drift/1e6)` 前进；播放时钟按**整数 Hz 执行器实际达到的
  速率**前进。
- 位置测量噪声：每 `hold_ms` 重新抽取一个偏移，均匀整数落在
  `[-q/2, +q/2]` 帧（`q` 为 10/15/20 ms），即 10–20 ms 的量化台阶，不是白噪声。
- 每个报告：`observe → ppm()/control_ppm() → update(_with_control)`；不使用私有函数。
- 顶轨定义：测量窗内 `|correction_ppm| == 100` 的周期占比。
- 控制窗 180 s，重建后再做 20 s 一阶低通；测量窗为 240–300 s（598 个控制周期），
  让控制窗先填满并稳定。
- 两种子 `0x6d5a56da`、`0x12345678`；漂移 0 / ±75 ppm。

标定结果（RED 运行里 10 s 原始估计的范围）：约 `-2,079 … +1,754 ppm`，与真机
`-1,807 … +723` / `-1,294 … +1,209` / `-467 … +532` 同量级。

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
M5-12 noise-only control failures:
noise alone pinned the correction at the ±100 ppm rail (q=10ms hold=1000ms seed=0x6d5a56da: 54.7%)
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
约一半（`d/2`）。

## 3. 三种方案取舍

| 方案 | 做法 | Host 结果 | 结论 |
|---|---|---|---|
| (a) 长时常数 EMA | 对 10 s 估计做 τ=30–120 s EMA 后再驱动 | 漂移 0 时仍见 11–45 % 顶轨；±75 时 10–45 %。10 s 估计本身相关时间已达 ~10 s，EMA 压不住低频游走 | 单独用不足 |
| (b) 小估计死区/滞回 | 把 \|estimate\| 小于阈值时置零 | 要吞掉 ±1,000–1,800 ppm 噪声，死区必须 >1,800 ppm，连真实 ±75 ppm 一起吞掉 | 否决 |
| (c) 更长基线斜率（选用） | 180 s 最小二乘斜率驱动控制；10 s 窗口只保留诊断 | 见 §4：0 % 顶轨，误差 ≤16.2 ppm | **采用** |
| 缓冲误差 PI | 按 `buffer_current_ms − target` 驱动 | 真机缓冲 0–28 ms，会反向饱和；根因未明 | 不采用 |

最终实现是 (c) + 轻量 (a)：180 s 直接斜率重建开环漂移，再做 20 s 一阶阻尼。
180 s 依据：量化斜率误差约 `q/T`，10 ms / 180 s ≈ 55 ppm，再经最小二乘与阻尼降到
个位数 ppm；最坏 ±100 ppm 未修正漂移在 180 s 内只积累约 18 ms，仍在 60 ms 目标 /
200 ms 硬限内。离线 Python 标定曾在 `q=20 ms, hold=3 s` 等更严苛单元上比较 60/90/
120/150/180 s；180 s + 20 s 阻尼是唯一在全部单元为 0 % 顶轨的档位。该标定只是
**离线模型**，不是 Rust 测试结果。

## 4. GREEN（提交 `360b7fb` + `d6307b7`）

同一命令、同一模型；实际输出片段：

```text
M5-12 drift=0 q=10ms hold=1000ms seed=0x6d5a56da: rail=0.0% mean_applied=-0.3 max|applied|=1 raw=[-714,878]
M5-12 drift=0 q=20ms hold=2000ms seed=0x6d5a56da: rail=0.0% mean_applied=-5.5 max|applied|=8 raw=[-2087,1684]
M5-12 drift=0 q=20ms hold=2000ms seed=0x12345678: rail=0.0% mean_applied=+14.8 max|applied|=16 raw=[-1251,1663]
M5-12 drift=+75 q=10ms hold=1000ms seed=0x6d5a56da: rail=0.0% mean_applied=+76.0 err=+1.0 raw=[-722,870]
M5-12 drift=+75 q=20ms hold=2000ms seed=0x12345678: rail=0.0% mean_applied=+91.2 err=+16.2 raw=[-1238,1676]
M5-12 drift=-75 q=15ms hold=2000ms seed=0x6d5a56da: rail=0.0% mean_applied=-83.3 err=-8.3 raw=[-1403,1073]
M5-12 noiseless drift=0: mean_applied=+0.0 max|applied|=0
test result: ok. 1 passed; 0 failed
```

汇总：

| 指标 | GREEN（修复后） | 阈值 | 结论 |
|---|---|---|---|
| 无漂移顶轨比例 | 12/12 单元 0.0 % | < 10 % | **PASS** |
| ±75 顶轨比例 | 24/24 单元 0.0 % | < 10 % | **PASS** |
| ±75 稳态误差 | −11.1 … +16.2 ppm | ≤ 25 ppm | **PASS** |
| 无噪声 drift=0 | mean_applied = 0.0，max 0 | ≤ 5 ppm | **PASS** |
| §8 ±100 ppm 上界 | `max|applied|` 始终 ≤ 100（命令值）；整数 Hz 实际值 48 kHz ≤ 83.3 | 不变 | **PASS** |

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
cargo test                        : 10 个 test binary，254 passed, 0 failed
cargo test --features test-support: 10 个 test binary，254 passed, 0 failed
cargo clippy --all-targets ...    : Finished `dev` profile，无 warning
cargo fmt --check                 : exit 0
```

其中：

```text
clock_drift_contract           : 23 passed
clock_drift_session_contract   : 24 passed（既有用例全部保持通过）
```

JVM（因改动了 Kotlin 执行器；`ANDROID_HOME` + JDK 21）：

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

- `playback.drift`（`native/src/session.rs:1241`）新增 `control_ppm=…`：
  它是驱动控制器的 180 s 残差估计；`drift_ppm` 仍是 §8 10 s 原始值，语义未变、
  未钳位。
- `playback.drift_window status=rebuilding`（`:1253`）同样新增 `control_ppm=…`。
- 没有删除或重命名任何既有字段；§9 wire 协议未改。
- `MediaSession::drift_control_ppm()`（`native/src/media_session.rs:821`）是新的
  JNI/日志只读入口。

## 7. 残余风险 / NOT RUN

- **真机复测（顶轨比例、缓冲、source、SRC 接受）**：`NOT RUN`。本轮只准备工装；
  必须另跑长会话并按本文件 §0.4 检查 `setPlaybackRate rejected`。
- **听感 A/B（真实 ±75 ppm 下的音高抖动/平滑是否可闻）**：`NOT RUN`，人工门。
- **前 180 s**：控制窗未满时沿用旧的 10 s 控制器（既有会话测试依赖该行为）。
  这 180 s 内仍可能噪声顶轨；真实 ±75 ppm 在这段时间约积累 13.5 ms。修复效果从
  控制窗填满后开始。短于 ~3 min 的会话不会进入新稳态。
- **accepted-bytes 回退时钟**：生产证据 `source=hardware` 全程有效；若 `getTimestamp`
  连续不可用而切到 accepted，重建公式假设 accepted 进度也反映已施加速率。若该路径
  实际仍是开环，重建可能高估开环漂移。未在真机验证；保留为风险。
- **8 kHz 等低采样率**：±100 ppm 内没有整数 Hz 可达，执行器为 0 ppm；本工单未在
  真机验证该路由，依赖 §8 整帧干预。
- **§8 其它判据**：±100 ppm 上限、60/200 ms、20 ms/5 s 持续偏差、re-anchor 触发、
  wire 协议均未改动。

## 8. 提交

- `de6573b` test(m5-12): reproduce noise-driven drift correction saturation (RED)
- `360b7fb` fix(m5-12): clamp the integer-Hz executor inside the §8 ppm bound
- `d6307b7` fix(m5-12): drive drift control from a long baseline and close the rate loop
- 本条证据文件单独提交。
