# 漂移速率控制的生产实践调研

> 调研日期：2026-11-22。源码按各上游 `master` / `main` 当日抓取，**未固定到 tag/commit**；引用的行为可能在后续版本变化。
>
> 标记约定：
> - **【源码】** 直接读到实现或常量。
> - **【文档】** 官方 API 文档、RFC、数据手册或厂商应用说明。
> - **【推断】** 由源码/文档推导出的结论，上游没有直接这么写，或尚未在本项目实测。
>
> 本文只服务 M5-12 的方案选择，不改协议、不改 §8 的 ±100 ppm / 60–200 ms / 20 ms+5 s 判据。仓库现状见 `android-client-v2/native/src/drift.rs`、`android-client-v2/app/src/main/java/dev/audioshare/v2/MediaPlayer.kt`。

## 0. 约束、现状与一个先算清楚的量

### 0.1 我们的约束

`drift.rs` 当前实现（【源码】）：

- 估计：10 s 滚动窗口，对 `capture_frames - playback_frames` 做最小二乘斜率，输出 ppm；`MIN_WINDOW_SAMPLES = 10`；遇到相邻观测跳变 >60 ms 或停顿时清窗（`DriftEstimator::discontinuous`）。
- 控制：`target = clamp(estimate, ±100)`，每个 ~100 ms 更新最多改变 10 ppm（`RATE_SLEW_PPM_PER_UPDATE`），没有速率死区。
- 缓冲：只有 `|buffer_current_ms - target_ms| > 20 ms` 持续 >5 s 才整帧增删（`DriftController::update` 第二段）；速率环完全不看缓冲误差。
- 目标 60 ms，硬上限 200 ms；`AudioTrack` 侧由 `playbackRateHz(sampleRate, ppm)` 换算成整数 Hz。

真机结果（M5-11/48 kHz）：`correction_ppm` 118/186 个快照顶在 ±100；长期中位数 −1 ppm、均值 −9.6 ppm；缓冲 0–28 ms（目标 60）。这说明**长期真实漂移接近 0，而逐窗估计噪声在 1,000–1,800 ppm 量级**，控制器把噪声当成速率差，长期在 ±100 轨上。

### 0.2 10 s 窗口为什么必然不够：量化阶梯的数量级

位置量化 10–20 ms 时，10 s 基线对速率的可分辨能力约为

```text
slope error  ~  q / T          （q = 位置台阶，T = 基线长度）
```

这不是白噪声平均，而是阶梯/锯齿的确定性误差量级；它与实测 `1,000–1,800 ppm ≈ (10–18 ms)/10 s` 吻合（【推断】，由量纲和实测数量级对照）。

| 位置台阶 q | T=10 s | 60 s | 120 s | 300 s | 600 s |
|---:|---:|---:|---:|---:|---:|
| 10 ms | 1,000 ppm | 167 ppm | 83 ppm | 33 ppm | 17 ppm |
| 20 ms | 2,000 ppm | 333 ppm | 167 ppm | 67 ppm | 33 ppm |

若把量化噪声近似成独立白噪声，最小二乘斜率的标准差为

```text
σ_slope = σ_y * sqrt(12) / (T * sqrt(N))
```

其中 `N` 是窗口内样本数。以 `N=100`（10 Hz × 10 s）、`σ_y = q/sqrt(12)` 代入：q=10 ms 时约 100 ppm，q=20 ms 时约 200 ppm。实测 1,000–1,800 ppm 高于这个值，说明噪声是**相关阶梯**而不是独立白噪声；无论用哪个模型，结论相同：**10 s 窗口在 10–20 ms 量化下无法分辨 100 ppm；要得到 ~30–80 ppm 的控制输入，基线需要 100–600 s 量级（q=10 ms 取 100–300 s，q=20 ms 取 200–600 s；多窗平均可再放宽）**。

这也是 Android 自己给出的使用建议：`AudioTrack.getTimestamp()` 的 Javadoc 明确说，时钟稳定后**每 10 s 到 1 分钟查询一次**，并警告“successive timestamp reports 的短期差异没有意义”，高分辨率映射应在应用层基于低分辨率时间戳实现（【文档】，见 §5）。

### 0.3 执行器的分辨率比估计器更粗：整数 Hz

Android `AudioTrack.setPlaybackRate(int sampleRateInHz)` 的参数是**整数 Hz**（【文档】/【源码】）：

- Java：`AudioTrack.java#setPlaybackRate(int)`，有效范围 1 Hz 到 2× `getNativeOutputSampleRate()`。
- JNI：`native_set_playback_rate` → `AudioTrack::setSampleRate(uint32_t)`（`frameworks/av/media/libaudioclient/AudioTrack.cpp`），保存的是整数 `mSampleRate`。

换算到 ppm：

| 采样率 | 1 Hz ≈ | 75 ppm 实际可设 | 100 ppm 实际可设 |
|---:|---:|---:|---:|
| 48,000 | 20.83 ppm | 4 Hz = 83.3 ppm | `round(4.8)=5 Hz` = **104.2 ppm** |
| 44,100 | 22.68 ppm | 3 Hz = 68.0 ppm 或 4 Hz = 90.7 ppm | 4 Hz = 90.7 ppm |

本项目 `playbackRateHz()` 先 `clamp(ppm, ±100)`，再 `roundToInt(ppm * sampleRate / 1e6)`，最后 `sampleRate + delta`（【源码】，`MediaPlayer.kt`）。因此 48 kHz 下请求 +100 ppm 会落到 48,005 Hz，即 **+104.2 ppm，已经越过 §8 的 ±100 上限**。这不是控制器逻辑错误，而是“ppm 域 clamp、Hz 域执行”的边界问题。反方向的 −100 ppm 同样会到 47,995 Hz（−104.2 ppm）。

**直接含义**：任何速率控制器的输出分辨率下限是 20.8–22.7 ppm；死区/滞回至少要覆盖 1 个 LSB（约 21 ppm），否则会在相邻整数 Hz 之间极限环。若需要真正细于 21 ppm 的连续修正，必须在 native PCM 生产端做小数比重采样，而不是只靠 `AudioTrack.setPlaybackRate`。

另外，`AudioTrack::setSampleRate` 在以下情况直接返回 `INVALID_OPERATION`（【源码】）：offload/direct track、`AUDIO_OUTPUT_FLAG_FAST`、haptic channel。当前 `AudioTrack.Builder` 没有设置 `PERFORMANCE_MODE_LOW_LATENCY`（【源码】，`MediaPlayer.kt`），所以现在能生效；未来若改成 FAST track，速率修正会静默失败。`setPlaybackRateHz` 目前会记录 `setPlaybackRate(...) rejected` 并保留上次速率（【源码】），这一点是安全的。

## 1. WebRTC NetEq

### 估计：没有漂移速率估计器

NetEq 的 `TimestampScaler` 只做**编解码器时间戳与 RTP clockrate 的比例换算**，不做墙钟漂移估计：

- `modules/audio_coding/neteq/timestamp_scaler.cc`：`TimestampScaler::ToInternal` 用 `numerator_ = decoder sample rate`、`denominator_ = RTP clockrate`，按 `internal_diff * numerator / denominator` 累积；`numerator_ == denominator_` 时原样返回。没有斜率、没有 ppm、没有窗口。
- `modules/audio_coding/neteq/delay_manager.h`：`DelayManager` 用 `UnderrunOptimizer` 维护到达延迟直方图，默认 `quantile = 0.95`、`forget_factor = 0.983`、`resample_interval_ms = 500`（可被 field trial 改），输出目标延迟 `TargetDelayMs()`；没有“频率偏差”状态。
- `modules/audio_coding/neteq/buffer_level_filter.cc`：`BufferLevelFilter::Update` 是一阶 IIR，`filtered = level_factor/256 * filtered + (1 - level_factor/256) * buffer`；`level_factor` 按目标延迟在 251…254 之间选，即每次更新保留 98.0%–99.2%，时间常数约 50–130 次更新（【推断】，由 `-1/ln(f)` 算得）。
- `modules/audio_coding/neteq/decision_logic.cc`：`kTargetLevelWindowMs = 100`、`kDelayAdjustmentGranularityMs = 20`；`ExpectedPacketAvailable` 中
  `high_limit = TargetLevelMs() + packet_arrival_history_->GetMaxDelayMs() + 20 ms`，
  `playout_delay >= high_limit * 4` → `FastAccelerate`，`>= high_limit` → `Accelerate`，`< TargetLevelMs()` → `PreemptiveExpand`。
- `accelerate.cc`：至少约 30 ms 输入；时间伸缩以 15 ms 为单位（`k15ms = 120` samples at 8 kHz）；`time_stretch.cc` / `expand.cc` 做相关/重叠相加。

### 控制：用时间伸缩吸收漂移

NetEq **不区分“网络抖动”与“晶振漂移”**。漂移表现为到达延迟的缓慢单调变化，最终推动滤波后的缓冲水平越过 `high_limit` 或低于目标，触发 accelerate / preemptive expand。也就是说，它用**偶发的 ~15 ms 时间伸缩**代替连续的速率微调（【源码】+【推断】）。

### 参数与已知问题

- 目标延迟由到达延迟的 95 分位决定，并随 `forget_factor=0.983` 缓慢遗忘（在 500 ms 重采样下约 30 s 量级，【推断】）。
- 缓冲水平滤波的时间常数约 0.5–1.3 s（每 10 ms 一次 `GetAudio`；50–130 次更新，【推断】）。
- 时间伸缩会改变音高/时长感知；15 ms 粒度、相关阈值、低能量语音的特殊处理都是为语音设计的，不适合“连续音乐、只允许 ±100 ppm 的轻微音高变化”的场景。
- 它没有“长期频率偏差”这个可观测状态，因此无法做 feed-forward；在固定 60 ms 目标的 PCM 流里，到达延迟历史也不携带我们需要的晶振信息。
- NetEq 不是速率匹配器，而是**时间伸缩器**。

**可借鉴的**：对缓冲水平做滤波、用“目标 + 到达历史 + 20 ms”形成上下门限、以及**用较大滞回避免频繁动作**。**不可借鉴的**：把时间伸缩当作 ±100 ppm 的执行器。

## 2. PulseAudio

### 2.1 `pa_smoother` / `pa_smoother_2`：时间戳平滑，不是速率控制器

`src/pulsecore/time-smoother.c`：

- 结构：最多 `HISTORY_MAX = 64` 个 `(本地时间 x, 远端时间 y)` 样本，按 `history_time` 丢弃旧样本，`min_history` 保底。
- 估计：`avg_gradient()` 对历史做**线性回归**求斜率；`pa_smoother_put()` 把新测量加入历史，并让估计函数在 `adjust_time` 内用三次多项式（Hermite/B-spline 式）平滑到新斜率，保证一阶导数连续；`monotonic` 时把负斜率钳到 0。
- 参数（调用点，【源码】）：
  - `src/pulse/stream.c`：`SMOOTHER_HISTORY_TIME = 5 s`、`SMOOTHER_ADJUST_TIME = 1 s`、`SMOOTHER_MIN_HISTORY = 4`。
  - `src/modules/module-tunnel.c`：history 2 s、adjust 1 s、min 10。
  - `src/modules/alsa/alsa-sink.c`：`SMOOTHER_WINDOW_USEC = 10 s`（部分构建 15 s）、`SMOOTHER_ADJUST_USEC = 1 s`、min 5；`update_smoother()` 把 `write_count - delay*frame_size` 与 htimestamp 喂进去，用于 `sink_get_latency()`。
- 已知问题：回归**未加权**（源码里 FIXME 明确说“更近的样本应更重要”）；没有鲁棒/离群处理；假定单调；2 s 的 tunnel 窗口对 100 ppm 远远不够；它输出的是平滑后的时间戳/延迟，不直接驱动采样率。

`src/pulsecore/time-smoother_2.c` 是另一套实现：

- 用 `time_factor` 表示声卡时间/系统时间之比；对 `time_factor` 做 Kalman 滤波，再经过二阶低通。
- 低通时间常数写成 `temp = smoother_window_time / (2π)`，两阶分别用 `temp/8` 和 `temp`（即窗口的 1/(16π) 与 1/(2π)）；【源码】。
- ALSA 用 10–15 s 窗口（`SMOOTHER_WINDOW_USEC`），所以二阶低通的时间常数约 0.2–0.3 s 和 1.6–2.4 s 量级（【推断】）。
- 同样用于**延迟/时间戳估计**（`pa_smoother_2_get_delay`），不是速率执行器。

**对第 1 问的对应**：PulseAudio 的“长基线 + 低通”思路可用，但 10–15 s 基线在 10–20 ms 量化下仍不足；它没有解决“位置量化”的问题，只是把延迟报告平滑了。

### 2.2 `module-loopback`：P + 漂移前馈 + Kalman 延迟滤波

`src/modules/module-loopback.c` 的 `rate_controller()` 与 `adjust_rates()`（【源码】）：

- 估计：
  - 每次 `adjust_rates()` 读取 source/sink 的 latency snapshot 与 memblockq 长度，算 `current_latency`；
  - `latency_difference = current_latency - final_latency`；
  - Kalman 滤波：`filtered_latency` 由测量值与预测值按 `kalman_variance`/`latency_variance` 加权；`kalman_variance` 每次更新后加 `latency_variance/4 + 200`（防止过度自信）；
  - 漂移：`latency_drift = latency_difference_at_base_rate - last_latency_difference`，`drift_rate = latency_drift * old_rate / real_adjust_time + old_rate - base_rate`；绝对值超过 `base_rate/100`（1%）就丢弃；然后经过两级一阶低通，`FILTER_PARAMETER = 0.125`（每 1 s 一次，单级约 8 s，两级约 16 s 量级，【推断】）。
- 控制：
  - `rate_controller()` 是一个带权 P 控制器：`new_rate_1` 限制每周期最多 2‰ 变化；`new_rate_2` 限制离 base rate 最多 1%；按“是否已接近目标”选择权重；
  - 最终 `new_rate += drift_compensation_rate`，再钳到 `[base*0.99, base*1.01]`；
  - 默认 `DEFAULT_LATENCY_MSEC = 200`、`DEFAULT_ADJUST_TIME_USEC = 1 s`、`DEFAULT_ADJUST_THRESHOLD_USEC = 250 µs`；
  - `fast_adjust_threshold_msec` 可配置，超过就直接丢/插样本；
  - underrun >2 次时把目标延迟提高 5 ms（`underrun_latency_limit`）；
  - source/sink 变化、挂起、时间跳变时把 `drift_compensation_rate`/`drift_filter` 清零。
- 已知问题：
  - 1% 的范围、2‰ 的单步，比我们的 ±100 ppm 粗一个数量级以上；1 s 的调整周期也偏快；
  - 依赖 latency snapshot 的准确性，路由/设备切换会产生跳变；
  - 代码注释自己说 2‰ 可视为不可闻，但这是针对它自己的 200 ms 缓冲和 1 s 周期；
  - Kalman 的方差/过程噪声是经验值，没有严格建模。

**可借鉴**：P + 慢漂移前馈 + 延迟低通 + 状态变化时重置 + 用 underrun 反馈调整目标延迟。**不可照搬**：1%/2‰ 的范围与步长、1 s 的带宽。

### 2.3 `module-rtp-recv`：5 s 一次、自适应 α、20 Hz 死区

`src/modules/rtp/module-rtp-recv.c` 的 rate 更新（【源码】）：

- `RATE_UPDATE_INTERVAL = 5 s`；
- `estimated_rate = current_rate * T / (T + last_latency - latency)`，即用一次 5 s 延迟变化估计速率；
- `avg_estimated_rate` 用自适应 α：α 默认 0.02；仅当 `|estimated_rate - avg_estimated_rate| > 1 Hz` 时，按 ratio 公式重算并夹在 0.02–0.8（源码注释描述的“稳定时 α→1、突变时 α→0”与这段实现并不完全一致，读取时以代码为准）。
- `new_rate = (T + latency/4 - intended_latency/4) / T * avg_estimated_rate`，即用 `a=7` 的渐进策略把延迟拉回目标；
- 合理性：`0.8×`–`1.25×` 之外放弃本次调整；与 base rate 相差 **±20 Hz 以内不调整**；每次相对当前速率最多 **2‰**。
- 已知问题：20 Hz 在 48 kHz 是 **417 ppm**、44.1 kHz 是 **454 ppm**，比我们整个 ±100 ppm 预算还宽；2‰ = 2,000 ppm 的单步也远大于我们的范围。它适用于“RTP 接收端允许几百 ppm、用较大缓冲”的场景，不适合 ±100 ppm。
- 可借鉴：自适应 α、绝对范围 sanity check、渐进拉回目标延迟；不可照搬参数。

### 2.4 `module-echo-cancel`：漂移交给 AEC 引擎，而不是速率环

`src/modules/echo-cancel/module-echo-cancel.c`（【源码】）：

- `time_callback()` 每 `DEFAULT_ADJUST_TIME_USEC = 1 s` 运行一次，`DEFAULT_ADJUST_TOLERANCE = 5 ms`；它计算 capture/playback 的 `diff_time`，超过阈值时向 source output 发 `SOURCE_OUTPUT_MESSAGE_APPLY_DIFF_TIME` 做块级重对齐，而不是调采样率。
- 原有的 `new_rate = base_rate ± ...` 代码在当前 master 被**注释掉**，只保留 `new_rate = base_rate`；即使有变化，也用 `new_rate > base_rate*1.1 || new_rate < base_rate*0.9` 直接放弃。也就是说，它没有可用的 ±100 ppm 级速率控制。
- `do_push_drift_comp()` 用 `drift = ((plen - sink_rem) - (rlen - source_rem)) / (rlen - source_rem)` 估计 AEC 的漂移，并调用 `u->ec->set_drift(u->ec, drift)`，把补偿交给 WebRTC/Speex AEC 引擎（`drift_compensation` 参数）。
- 结论：它证明了“漂移补偿可以下沉到 DSP/AEC 引擎”，但没有提供一个可借鉴的音频速率环；且默认处理格式是 32 kHz、单声道。

### 2.5 关于 `module-match`

`src/modules/module-match.c` 的模块描述是 “Playback stream expression matching module”，按 `match.table` 的键值把流路由到 sink；它**不是**速率匹配（【源码】）。PulseAudio 里真正涉及速率/漂移的是 `module-loopback` 与 `module-rtp-recv`；`module-echo-cancel` 把漂移交给 AEC 引擎；不要用 `module-match` 作为速率匹配的例子。

## 3. PipeWire

### 3.1 `SPA_IO_RateMatch` 与二阶 DLL

PipeWire 的通用机制是 `struct spa_io_rate_match`（`spa/include/spa/node/io.h`）：

- 节点在 `process()` 里设置 `SPA_IO_RATE_MATCH_FLAG_ACTIVE` 和 `rate`，表示期望的**重采样率修正**；`delay`/`delay_frac` 报告重采样器延迟；`size` 是重采样器需要的输入样本数（【源码】/注释）。
- `spa/plugins/audioconvert/audioconvert.c` 的 `resample_update_rate_match()` 在存在 active rate match 时把 `rate` 乘进重采样比（`rate *= io_rate_match->rate`），所以执行器是**小数比重的重采样器**，没有整数 Hz 量化（【源码】）。
- `spa/plugins/alsa/alsa-pcm.c` 的 `update_time()` / `spa_dll_update()` 是实际的控制器：
  - 误差 `err = delay - target`（播放方向），并把超过 `max_resync` 的误差钳到 `max_error`；
  - `spa_dll_update()`（`spa/include/spa/utils/dll.h`）是**二阶 DLL**：
    ```c
    z1 += w0 * (w1 * err - z1);
    z2 += w0 * (z1 - z2);
    z3 += w2 * z2;
    return 1.0 - (z2 + z3);
    ```
    `w0 = 1 - exp(-20w)`、`w1 = 1.5w/period`、`w2 = w/(1.5*period)`，`w = 2π*bw*period/rate`。
  - 带宽 `bw` 默认上限 `SPA_DLL_BW_MAX = 0.128 Hz`，头文件最小 `SPA_DLL_BW_MIN = 0.016 Hz`，ALSA 属性允许降到 `0.001 Hz`；对应时间常数约 `1/(2πf)`：0.128 Hz ≈ 1.24 s、0.016 Hz ≈ 9.95 s、0.001 Hz ≈ 159 s（【源码】/【推断】）。
  - `update_time()` 根据误差均值/方差自适应 `bw`：`bw = (|err_avg| + sqrt(err_var))/1000`，再 `SPA_CLAMPD(bw, min, dll_bw_max)`；`err_avg`/`err_var` 用 `err_wdw` 的滑动平均。
  - 对 ALSA 硬件 pitch 控件（`spa_alsa_update_rate_match`），`pitch` 用 micro-units；代码规定变化 `<10`（约 10 ppm）就不写控件，注释说规范允许 1 ppm 粒度但避免频繁写（【源码】）。
- `module-loopback.c` 在当前 master 上**不做速率匹配**：`recalculate_delay()` 通过移动 ringbuffer 读指针把延迟拉到 `target.delay.sec`（丢/补帧），不是调 rate（【源码】）。`module-echo-cancel.c` 也没有 DLL 速率环，只有 `buffer.play_delay` 和 AEC 的对齐/丢帧（【源码】）。
- 已知问题：只有支持 `SPA_IO_RateMatch` 的节点 + 重采样路径才生效；依赖 htimestamp/驱动时间戳；`max_resync` 触发时是跳变；重采样器本身引入延迟和相位；不是所有 sink 都能调。
- **可借鉴**：二阶 DLL 的结构、误差钳位、按误差统计自适应带宽、用小数重采样器做执行器。**不可直接照搬**：我们没有 PipeWire 的重采样图，只有 `AudioTrack` 的整数 Hz。

## 4. miniaudio / PortAudio

### miniaudio

- 没有内置的“估计 + 控制”漂移环。文档在 ring buffer 一节明确说：要修正读写指针漂移，用 `ma_pcm_rb_pointer_distance()` 观察距离，再用 `ma_pcm_rb_seek_read()` / `ma_pcm_rb_seek_write()` 移动指针；空间太多就前移读指针，太少就前移写指针（【源码】/文档注释，`miniaudio.h` 约 3588 行）。
- 提供 `ma_resampler`，并且 `ma_resampler_set_rate()` / `ma_resampler_set_rate_ratio()` 可在运行中改采样率/比例；但**没有**根据延迟或时钟误差自动调它的循环（【源码】）。
- 结论：只提供“丢/补帧”和“可变比重采样”两个原语，策略由应用实现。

### PortAudio

- 在 `include/`、`src/common/`、`doc/` 中搜索 `drift` / `rate match` / `resampl` 没有找到漂移控制实现；`pa_ringbuffer` 只是无锁环形缓冲，供应用在回调/主线程之间搬 PCM（【源码】，PortAudio master）。
- 结论：PortAudio 不提供采样率匹配；双设备时钟漂移完全由应用处理（通常是 ring buffer 水位 + 丢补帧或外部 SRC）。

**对第 1/2 问的对应**：这两个库告诉我们“生产级库不一定内置控制器”；如果要做，仍要自己实现估计/滤波/执行器。

## 5. Android `AudioTrack`：执行器与时间戳的真实语义

### 5.1 速率接口

- `setPlaybackRate(int sampleRateInHz)`：整数 Hz；Java 文档的有效范围是 1 Hz 到 2× native output rate（【文档】）；实现是 `native_set_playback_rate` → `AudioTrack::setSampleRate(uint32_t)`（【源码】，JNI `android_media_AudioTrack.cpp`，native `AudioTrack.cpp`）。
- `setPlaybackParams(PlaybackParams)`：`speed`、`pitch` 是 float 倍率（【文档】）。关键语义（`PlaybackParams.java` 类注释）：
  - `pitch == 1.0` 时，改变 speed 走**时间伸缩**（保持音高），不是重采样；
  - `pitch == speed` 时，走**重采样**，类似 `setPlaybackRate`。
  所以“用 float speed 绕过整数 Hz”并不可行：要重采样就得让 pitch 跟着 speed，而 native 的 `kFixPitch = true`、`adjustSampleRate(sampleRate, pitch) = round(sampleRate * pitch)` 又把有效采样率落回整数 Hz（【源码】）；`adjustSpeed(speed, pitch) = speed/pitch` 在 pitch=speed 时变成 1.0。`speed != pitch`（例如 pitch=1、speed≠1）走的是时间伸缩或混合，不是我们要的纯重采样。
- `AudioTrack::setSampleRate` 拒绝 offload/direct、`AUDIO_OUTPUT_FLAG_FAST`、haptic track（【源码】）。当前 app 没有请求 FAST，故现在可用；若以后为了低延迟启用 `PERFORMANCE_MODE_LOW_LATENCY`，必须换执行器或接受速率修正失效。
- 另外，native 还有 `AUDIO_RESAMPLER_DOWN_RATIO_MAX` / `AUDIO_RESAMPLER_UP_RATIO_MAX` 的重采样比检查（【源码】），但我们的 ±100 ppm 远在范围内，不构成限制。

### 5.2 `getTimestamp` 的语义

`AudioTrack.java#getTimestamp(AudioTimestamp)` 的 Javadoc（【文档】）：

- 返回 `framePosition`（帧）和 `nanoTime`（该帧被呈现或“承诺呈现”的估计时间）；数据可能已经播放，也可能尚未播放但已提交。
- 时钟稳定后，**每 10 s 到 1 分钟查询一次**；更频繁是低效的；**连续时间戳报告的短期差异没有意义**。
- 启动/路由变化后需要周期性重试；路由不支持时间戳时永久不可用，可退回 `getPlaybackHeadPosition()`。
- `getPlaybackHeadPosition()` 是 32 位连续计数，约 27 小时回绕（44.1 kHz），`flush()`/`stop()` 会清零（【文档】）。

native `AudioTrack::getTimestamp_l` 还会处理 `ExtendedTimestamp` 的最佳位置在 kernel/server 之间切换，并做位置补偿（【源码】）。这意味着时间戳的“位置”可能发生台阶，估计器必须把它当跳变/重置处理。

M5-11 的实测正好印证了文档：换成 `getTimestamp()` 硬件时钟后，逐窗噪声没有下降（同条件更差）。原因可能是时间戳的呈现/提交语义 + 10–20 ms 的平台节奏，而不是帧计数器分辨率；【推断】，需结合 host 模型和真机日志确认。

### 5.3 当前实现的边界问题

- 48 kHz 下 +100 ppm → 48,005 Hz = +104.2 ppm，越过 §8 上限（【源码】`playbackRateHz` + Android 整数 Hz）。
- 10 ppm/100 ms 的 slew 比执行器 LSB（约 21 ppm）还细；连续域里 slew 是平滑的，但量化后多数更新是空操作，且容易在相邻 Hz 之间来回。
- `DriftController` 的速率环没有死区，`target = clamp(estimate)` 直接跟随噪声；缓冲误差只用于 5 s 整帧干预，不参与速率。这就是 118/186 顶轨的直接原因（【源码】+【推断】）。

## 6. 时钟同步/漂移估计的经典做法

### 6.1 NTP（RFC 5905）

- `TOLERANCE / PHI = 15e-6`（15 ppm）：同步后失联时对系统时钟最坏漂移的假设（【文档】，RFC 5905 Figure 6）。
- poll 指数 `tau` 建议范围 6–10，即 **64 s–1024 s**；`T_c = 2^tau` 是环路时间常数（【文档】，RFC 5905 的 clock discipline 章节与 Figure 27/28）。
- 时钟纪律是 PLL/FLL 混合；`tau` 会根据 offset 与 jitter 的关系自适应，**温度突变导致频率跳变时会快速降低 tau**（【文档】）。这直接支持“频率环时间常数可以长到分钟级，但检测到突变要缩短”。
- NTP 用 clock filter（多样本选最小延迟）、hysteresis、step/slew 阈值等；它把“相位”和“频率”分开处理，而不是把瞬时斜率直接打到执行器（【文档】+【推断】）。

### 6.2 linuxptp

- PI servo：`servo.c`、`pi.c`；Kp/Ki 有软件/硬件时间戳两套 scale（`SWTS_KP_SCALE=0.1`、`SWTS_KI_SCALE=0.001`、`HWTS_KP_SCALE=0.7`、`HWTS_KI_SCALE=0.3`），并按 sync interval 做归一化（【源码】）。
- `max_frequency` 默认 **900,000,000 ppb = 0.9 s/s（90% 硬件上限）**；注意 1 ppm = 1,000 ppb，所以这不是 900 ppm。它是 PTP 网络时钟的允许范围，不是音频 codec 的精度目标（【文档】/【源码】）。
- `step_threshold` 默认 0（除启动外不 step），`first_step_threshold` 只在第一次更新生效（【文档】）。
- 对第 2 问的对应：PI + 输出限幅 + 条件积分/step 阈值是成熟结构；但 90% 硬件上限的范围和网络时钟的噪声模型不能直接搬到 60 ms 音频缓冲。

### 6.3 晶振漂移的量级与变化速率

- Epson FA-238（3.2×2.5 mm SMD，音频/消费设备常见级别）数据手册：标准初始容差 **±50 ppm @ +25 °C**（可选 ±15/±20/±25/±30 ppm），频率-温度特性 **±30 ppm over −20…+70 °C**，老化 **±5 ppm/year**（【文档】，FA-238/FA-238V/TSX-3225 datasheet）。
- 两台设备的最坏和：初始 ±50 + 温度 ±30 + 老化 ±5 ≈ **±85 ppm/台**，两台理论差可达 ±170 ppm；同一环境温度下温度项可能相关，实际差通常小于这个最坏和（【推断】）。这解释了为什么规范给 ±100 ppm 的修正范围是合理的，也解释了为什么真实漂移中位数可以接近 0（同批次/同温度）但不同设备可能接近 75–100 ppm。
- 32.768 kHz 音叉晶体的抛物线系数约 **0.04 ppm/°C²**，室温附近偏离 10 °C 约 4 ppm、20 °C 约 16 ppm（【文档】，ECS 应用说明；公式 `Δf/f = k(T−To)² + fo`）。AT-cut 高频晶体的曲线形状类似（【文档】Microchip/Vectron “Clock Oscillators” 应用说明给出不同切型的 Δf/f–温度曲线）；具体系数按切型变化，但“同量级”是【推断】。
- 宽温范围下消费级振荡器的总变化可达 **20–100 ppm**（【文档】，Rakon oscillator basics）。
- 变化速率：温度是主导因素，设备/房间的热时间常数通常是分钟到小时；在偏离转折点几 °C 时，室内 0.1–1 °C/min 的变化对应约 **0.1–1 ppm/min** 的频率变化，冷启动/握持/风道可能更快（【推断】，由抛物线系数和热时间常数估算；未实测）。这意味着 300–600 s 的频率环时间常数仍然能跟踪真实漂移；10 s 则主要是在跟踪量化噪声。

## 7. 方案对照表

“适配度”针对我们的 **±100 ppm 上限、10 s 原始窗口、60–200 ms 缓冲、±75 ppm 真实漂移**。

| 方案 | 估计 + 控制 | 关键参数 | ±100 ppm 适配 | 10 s 窗口适配 | 60–200 ms 缓冲适配 | 综合 |
|---|---|---|---|---|---|---|
| 当前：10 s LS + clamp + slew | 10 s LS，无死区；estimate 直接 clamp 到 ±100，10 ppm/100 ms；整数 Hz | 10 s / ±100 / 10 ppm | **低**：噪声直接进执行器；48 kHz 实际可达 +104 ppm | **低**：q/T ≈ 1,000–2,000 ppm | **中**：只有 20 ms/5 s 整帧保护 | **低**；保留为诊断 |
| NetEq | 无漂移速率估计；95 分位到达延迟 + 缓冲 IIR；15 ms 时间伸缩 | quantile .95、forget .983、20/100 ms、4× fast | **不适用**：执行器是时间伸缩，不是速率 | **不适用**：根本不估速率 | **强**：时间伸缩快，但 15 ms 粒度、语音导向 | **低/中**：学门限/滞回 |
| PulseAudio loopback | 延迟 snapshot + Kalman + 两级 0.125 低通 + P 控制 | 200 ms target、1 s、250 µs、±1%、2‰/步 | **低**：1% 范围、2‰ 步长太粗 | **中**：1 s 控制周期 + ~8–16 s 漂移低通 | **强**：200 ms 目标、underrun 反馈 | **中**：借结构，不借数值 |
| PulseAudio rtp-recv | 5 s 延迟差 + 自适应 α + a=7 渐进拉回 | 5 s、α 0.02–0.8（默认 0.02）、20 Hz 死区、2‰ | **低**：20 Hz ≈ 417–454 ppm 死区 | **低**：5 s 才更新一次 | **中**：大缓冲场景 | **低**：借 α/sanity |
| `pa_smoother` / `_2` | 回归 / Kalman + 二阶低通；只平滑时间戳/延迟 | 2–15 s 窗口、1 s adjust | **中**：本身不驱动速率 | **中**：ALSA 10–15 s，仍不足 | **强**：只做平滑，不引入相位风险 | **中**：做长基线低通的一部分 |
| PipeWire DLL | 缓冲误差 → 二阶 DLL；小数比重采样器执行 | bw 0.016–0.128 Hz（可到 0.001）、max_resync/max_error | **高**：小数比，无整数 Hz 限制 | **高**：误差环不依赖 10 s 斜率 | **强**：直接控延迟 | **高**（结构）；执行器需自建 |
| NTP / linuxptp | clock filter + PLL/FLL / PI；频率与相位分离 | PHI 15 ppm、T_c=2^tau（64–1024 s）、max_frequency 90% 硬件上限 | **高**：±100 ppm 很小 | **高**：长基线/长 τ | **中**：无音频相位保护，单独用太慢 | **中**：借“长 τ + 突变缩短 + 相位保护” |
| miniaudio / PortAudio | 无控制器；只有 ring 指针 seek / 可变比重采样原语 | — | **低**：无策略 | **低**：无估计 | **中**：丢补帧可用但无反馈 | **低/中**：只提供原语 |
| 建议：长基线 FF + 缓冲 PI | 10 s 仅诊断；300–600 s LS/EWMA/Kalman 做前馈；缓冲 PI + anti-windup | τ_ff 300–600 s、Ti 300–600 s、±100、1 LSB 滞回 | **高**：前馈 + 积分可跟 ±75 ppm | **高**：控制不依赖 10 s 单窗 | **高**：保留 20 ms/5 s 帧保护 | **高** |

> 表中的“适配”是对本项目约束的相对评价，不是对上游方案质量的评价。【推断】的评分依据见 §0.2、§8。

## 8. 对本项目的建议（按可信度排序）

### 建议 1【可信度：高】先固定执行器语义，再谈控制器

1. **保留 `drift_ppm` 原始值**用于诊断/证据，不钳位（符合 M5-12 要求）。
2. **修正 48 kHz 的 +104 ppm 问题**：`playbackRateHz()` 不要对 ppm 四舍五入到最近的 Hz 后越界。两种做法：
   - 在 Hz 域夹紧：先算 `max_delta = floor(100 * sampleRate / 1e6)`，再 `coerceIn(-max_delta, +max_delta)`；
   - 或向零取整（truncation）而不是 `roundToInt`：48 kHz 的 100 ppm → 4 Hz = 83.3 ppm，永远不会超过 100。
   二者都会带来“无法精确到 100 ppm”的量化，但这是执行器的物理限制。
   预期行为：48 kHz 下若改用 Hz 域夹紧/截断，最大只能到 4 Hz = 83.3 ppm；真实漂移在 83–100 ppm 之间的部分无法由速率完全抵消，会由 20 ms/5 s 的整帧干预承担。这是规范与执行器共同决定的预期行为，不是 `FAIL`；host 的 +100 ppm 边界测试应据此判定。
3. **量化 + 滞回**：控制器在连续域算 `r_desired`，输出层只在 `|r_desired - r_applied| > 0.75–1 LSB` 时改写整数 Hz；否则保持。否则会在相邻 Hz 之间极限环，听感上可能表现为缓慢的音高抖动。
4. **不要依赖 `setPlaybackParams(speed)` 绕过 1 Hz**：`pitch=speed` 的重采样最终仍走整数有效采样率；`pitch=1` 的 speed 是时间伸缩，不是连续速率修正（§5.1）。
5. **FAST track 风险**：如果未来启用低延迟 track，`setPlaybackRate` 会返回 `INVALID_OPERATION`；当前 `setPlaybackRateHz` 只记录日志并保留旧速率，需在诊断里暴露“速率修正未生效”，否则会误判为控制器无效。

### 建议 2【可信度：高】估计侧：10 s 只做诊断，控制用长基线

1. 保留现有 10 s 最小二乘，作为 `playback.drift` 的原始观测（覆盖完整、可复现），但**不要直接进控制**。
2. 控制路径用以下任一（按实现成本排序）：
   - **长窗口 LS**：300–600 s 窗口，1 Hz 抽样（或保留 10 Hz 但接受样本相关）；输出仍是斜率。量化误差量级从 ~1,000 ppm 降到 ~17–67 ppm（q/T，见 §0.2）；若只用 60–120 s，则约 80–170 ppm，仍偏噪。
   - **重叠窗中位数/鲁棒回归**：每 1 s 算一个 10 s 斜率，取最近 300–600 s 的中位数；对 re-anchor/trim 造成的离群更稳。注意：中位数降低的是离群敏感度，不消除阶梯方差；它应与长基线一起用。
   - **Kalman/一阶低通**：状态 `f`（ppm），过程噪声按晶振随机游走取很小（例如 0.1–1 ppm/min，【推断】），测量噪声用窗口长度和 q 估计；对创新做 3–4σ 门控，跳变/停顿时重置。
3. **死区 + 滞回**：先把控制路径的 σ_est 压到 ≤30–40 ppm（q=10 ms 约 300 s，q=20 ms 约 600 s，或多窗平均）；然后只在 `|f| > max(2σ_est, 1 LSB)` 时更新前馈，进入/退出用不同阈值（例如 30 ppm 进、15 ppm 出，或用软死区只保留超出部分）。死区不能大于 75 ppm，否则会吞掉真实漂移；建议硬死区 20–40 ppm、滞回 10–20 ppm（【推断】，需 host 模型标定）。如果 σ_est 压不到 40 ppm 以下，feed-forward 只做软去偏，主要靠缓冲积分，不要用会吞掉 75 ppm 的硬死区。
4. **不要用死区掩盖偏差**：如果长期真实漂移是 75 ppm，而死区是 ±50 ppm，前馈会停在 0，缓冲会持续漂移；这正是需要缓冲积分项的原因（建议 3）。
5. **状态重置**：source/sink 变化、sample rate 变化、`getTimestamp` 位置跳变、trim/re-anchor、暂停恢复，都要清空估计器和积分器；现有 `DriftEstimator::discontinuous` 的 60 ms 容差是好的起点。

### 建议 3【可信度：高/中】控制侧：缓冲误差 PI + 前馈，慢环 + 抗饱和

把主环从“估计 → clamp”改成“前馈 + 缓冲 PI”：

```text
e = buffer_current_ms - target_ms        // e > 0: 缓冲过多，需要加快播放
r_ff = hysteresis_deadband(lowpass(drift_ppm, tau_ff))
r_p  = Kp * e
I   += Ki * e * dt                       // 仅在允许积分时
r    = clamp(r_ff + r_p + I, -100, +100)
```

- **符号**：`e > 0`（缓冲高于 60 ms）→ 加快播放（正 ppm）；`e < 0`（缓冲低于 60 ms）→ 放慢（负 ppm）。这与 `drift.rs` 的 `correction_ppm` 正方向一致。
- **带宽/时间常数**：
  - 前馈低通 `tau_ff = 300–600 s`（对应 PipeWire DLL 的 0.001–0.016 Hz 下限和 NTP 的 64–1024 s 之间；对音频取中段）。
  - 积分时间 `Ti = 300–600 s`，`Ki = Kp / Ti`。建议 `Kp = 0.5–2 ppm/ms`（即 20 ms 误差 → 10–40 ppm），先用 host 模型扫参。这个增益下，纯 P 的闭环时间常数约 `1000/Kp = 500–2000 s`，正好是“分钟级”，不会追噪声。
  - 比例项可以加 **±5–10 ms 的死区**，避免在目标附近抖动；积分项不设死区（或只设 <1 ms），否则会留下稳态偏差。
- **抗饱和（anti-windup）**，必须有：
  - **条件积分**：当输出已在 +100 且 `e > 0`（还要继续加速）时，停止正向积分；在 −100 且 `e < 0` 时停止负向积分。
  - **积分限幅**：`I` 限制在 `±max(0, 100 - |r_ff + r_p|)` 或一个保守的 `±60 ppm`，防止退出饱和后长时间反向回调。
  - **back-calculation**（可选）：`I += (r_sat - r_unsat) * Kb * dt`，比条件积分更平滑。
  - **泄漏**：`I *= (1 - 1%/min)` 量级的慢泄漏，防止温度长期变化后残留偏差；泄漏时间常数应远大于 `Ti`。
  - **冻结**：启动预缓冲、`getTimestamp` 无效/过期、缓冲测量跳变、trim/re-anchor 期间冻结积分；现有 `seen_deadband` 可以用来 gate 积分，避免启动时把“从 0 填到 60 ms”当成漂移。
- **与 §8 的分工**：速率环只负责“长期频率偏差”，时间常数分钟级；20 ms/5 s 的整帧增删仍是**快速相位保护**，负责启动、路由切换、突发漂移。不要把速率环调快到能处理这些瞬态——那会重新引入噪声饱和。
- **执行器量化**：`r` 在连续域积分/限幅，输出前再量化到整数 Hz 并加 1 LSB 滞回；不要把量化后的值反馈进积分器（否则会因量化误差持续积分），可以用“量化前的连续值”做状态，或把量化误差作为 back-calculation 的一部分。

**一个需要先验证的前提**【可信度：中】：M5-11 显示 `buffer_current_ms` 长期在 0–28 ms，而目标是 60 ms。如果这个偏差是**结构性**的（ring 几何、`submitted_position` 口径、软修剪后的测量偏差），那么缓冲 PI 会一直认为“缓冲太少”，把速率推到 −100 ppm，形成另一种饱和。上线前必须用 host 噪声模型 + 真机日志确认 `buffer_current_ms` 在稳态下对 60 ms 是**无偏**的；若有固定偏差，应先修测量/目标口径，或把偏差作为可观测状态估计，而不是直接积分。

### 建议 4【可信度：高】保留整帧干预作为外层保护

- 现有 `>20 ms` 持续 `>5 s` 的整帧增删是规范的相位保护，不应被速率环取代。
- 当速率环因执行器 LSB、死区或 anti-windup 暂时跟不上时，整帧干预负责把缓冲拉回安全区；反过来，整帧干预造成的台阶必须从估计窗口中剔除（现有 `discontinuous`/`reset` 已做）。
- 建议在诊断里同时记录：`correction_ppm`（连续）、`applied_rate_hz`（实际写入 AudioTrack 的整数 Hz）、`actual_ppm = (applied_rate_hz/sample_rate - 1)*1e6`、`buffer_current_ms`、`drift_ppm_raw`、`drift_ppm_filtered`、积分器状态/是否饱和。没有这些，无法区分“控制器饱和”和“执行器没生效”。

### 建议 5【可信度：中】如果 21 ppm 的 LSB 仍不可接受

- 在 native 侧做**小数比重采样**（例如 48,000 → 48,000×(1±1e-4)），把 PCM 以略快/略慢的速率喂给 `AudioTrack`，让 `AudioTrack` 保持标称 48 kHz。这样执行器分辨率可以远细于 1 Hz，且不受 FAST track 的 `setSampleRate` 限制。
- 代价：多一层重采样（质量/延迟/CPU）、需要自己保证与 `getTimestamp` 的口径一致、并且仍然要遵守 ±100 ppm 的规范上限。
- 这是 PipeWire/miniaudio 的做法（小数比重采样 + DLL），但对当前 native ring 是一个结构性改动；建议作为“整数 Hz 确认不够用”后的备选，而不是首选。

### 建议 6【可信度：高】明确不要照搬的部分

- **NetEq 的时间伸缩**：15 ms 粒度、语音导向、听感风险；我们的规范要的是轻微速率修正，不是时长伸缩。
- **PulseAudio loopback 的 1%/2‰**：比 ±100 ppm 粗 10–100 倍；1 s 周期也太快。
- **PulseAudio rtp-recv 的 20 Hz 死区**：≈417–454 ppm，会直接吃掉我们整个预算。
- **PipeWire 的 `SPA_IO_RateMatch` 执行器**：它依赖图内重采样器；`AudioTrack` 没有等价物。
- **NTP 的 64–1024 s 纯频率环**：单独用太慢，必须配合我们的 20 ms/5 s 相位保护；但它的“长 τ + 突变时缩短 τ”值得借鉴。

### 建议 7【可信度：中】host 噪声模型的验收口径

M5-12 要求 host 先 RED 再 GREEN。建议至少覆盖：

1. **无噪声、零漂移**：`correction_ppm` 最终应接近 0（而不是在 ±100 之间摆），`applied_rate_hz` 不来回跳。
2. **真实漂移 ±75 ppm（阶跃 + 缓慢斜坡）叠加 q=10/20 ms 位置量化**：稳态 `correction_ppm` 应落在 50–100 ppm（或 −100…−50 ppm）内，不长期顶轨；缓冲应在 60 ms 附近有界，不持续触发 20 ms/5 s 整帧干预。
3. **真实漂移 +100 ppm 边界**：允许误差，但实际 `applied_rate_hz` 不得超过 ±100 ppm（先修 §0.3 的 +104 问题）。
4. **量化死区**：当 `|r_desired - r_applied| < 1 LSB` 时不应改写 Hz；统计每秒的 `applied_rate_hz` 变化次数。
5. **anti-windup 恢复**：先让输出饱和（例如注入 +200 ppm 持续 60 s），再回到 0 漂移；积分器应在合理时间内退出饱和，不产生长时间反向过冲。
6. **跳变/重置**：re-anchor、trim、`getTimestamp` 位置跳变、暂停恢复时，估计器和积分器必须清空；清空后不应把旧偏差带入。
7. **执行器未生效**：模拟 `setPlaybackRate` 返回失败，诊断必须暴露 `rate correction not applied`，控制器不能继续积分到饱和。

## 9. 来源与代码定位

### WebRTC NetEq

- `modules/audio_coding/neteq/timestamp_scaler.cc` — `TimestampScaler::ToInternal/ToExternal`；仅 codec clockrate 换算。
- `modules/audio_coding/neteq/delay_manager.h` / `.cc` — `DelayManager::Config`（quantile 0.95、forget 0.983、resample 500 ms）、`UnderrunOptimizer`。
- `modules/audio_coding/neteq/buffer_level_filter.cc` — `BufferLevelFilter::Update`、`SetTargetBufferLevel`（251–254）。
- `modules/audio_coding/neteq/decision_logic.cc` — `kTargetLevelWindowMs=100`、`kDelayAdjustmentGranularityMs=20`、`ExpectedPacketAvailable` 的 high/low limit 与 4× fast accelerate。
- `modules/audio_coding/neteq/accelerate.cc`、`time_stretch.cc`、`expand.cc` — 15 ms/30 ms 时间伸缩。
- 抓取自 GitHub 镜像 `webrtc-mirror/webrtc` 的 `main`（与 googlesource 同一棵树），未固定 commit。
- 入口：<https://webrtc.googlesource.com/src/+/refs/heads/main/modules/audio_coding/neteq/>

### PulseAudio

- `src/pulsecore/time-smoother.c` / `.h` — `pa_smoother`：`HISTORY_MAX=64`、`avg_gradient()` 线性回归、`pa_smoother_put()` 三次插值、`adjust_time`/`history_time`。
- `src/pulsecore/time-smoother_2.c` / `.h` — Kalman + 二阶低通，`smoother_window_time/(2π)` 与 `/ (16π)`。
- `src/modules/module-loopback.c` — `rate_controller()`、`adjust_rates()`、`FILTER_PARAMETER=0.125`、1%/2‰、200 ms/1 s/250 µs。
- `src/modules/rtp/module-rtp-recv.c` — `RATE_UPDATE_INTERVAL=5s`、α=0.02–0.8、20 Hz 死区、2‰、a=7。
- `src/modules/echo-cancel/module-echo-cancel.c` — `time_callback()`、`DEFAULT_ADJUST_TIME_USEC=1 s`、`DEFAULT_ADJUST_TOLERANCE=5 ms`、`set_drift()`；速率调整代码被注释，漂移交给 AEC 引擎。
- `src/modules/alsa/alsa-sink.c` — `SMOOTHER_WINDOW_USEC=10/15s`、`SMOOTHER_ADJUST_USEC=1s`、`update_smoother()`。
- `src/modules/module-match.c` — 按属性匹配路由，不是速率匹配。
- 入口：<https://gitlab.freedesktop.org/pulseaudio/pulseaudio/-/tree/master/src/pulsecore>、<https://gitlab.freedesktop.org/pulseaudio/pulseaudio/-/tree/master/src/modules>

### PipeWire

- `spa/include/spa/node/io.h` — `struct spa_io_rate_match`、`SPA_IO_RATE_MATCH_FLAG_ACTIVE`、rate matching 注释。
- `spa/include/spa/utils/dll.h` — `SPA_DLL_BW_MAX=0.128`、`SPA_DLL_BW_MIN=0.016`、`spa_dll_update()` 二阶 DLL。
- `spa/plugins/alsa/alsa-pcm.c` — `update_time()`、`spa_dll_set_bw`/`spa_dll_update`、`max_resync`/`max_error`、`spa_alsa_update_rate_match()`（pitch 控件，<10 ppm 不写）。
- `spa/plugins/audioconvert/audioconvert.c` — `resample_update_rate_match()`，`rate *= io_rate_match->rate`。
- `src/modules/module-loopback.c` — `recalculate_delay()`，当前 master 是 ring 指针修剪，不是速率匹配。
- `src/modules/module-echo-cancel.c` — `buffer.play_delay`、AEC 对齐，无 DLL 速率环。
- 入口：<https://gitlab.freedesktop.org/pipewire/pipewire/-/tree/master/spa/include/spa>、<https://gitlab.freedesktop.org/pipewire/pipewire/-/tree/master/spa/plugins/alsa>

### miniaudio / PortAudio

- `miniaudio.h` — ring buffer 漂移修正说明（约 3588 行）：`ma_pcm_rb_pointer_distance`、`ma_pcm_rb_seek_read`、`ma_pcm_rb_seek_write`；`ma_resampler_set_rate()` / `ma_resampler_set_rate_ratio()`；无自动控制器。
- <https://github.com/mackron/miniaudio/blob/master/miniaudio.h>
- PortAudio：`include/`、`src/common/pa_ringbuffer.*`；未发现 drift/rate matching 实现；<https://github.com/PortAudio/portaudio>

### Android

- `AudioTrack.java#setPlaybackRate(int)`、`#setPlaybackParams(PlaybackParams)`、`#getTimestamp(AudioTimestamp)`、`#getPlaybackHeadPosition()`；
  <https://developer.android.com/reference/android/media/AudioTrack>
- `PlaybackParams.java` — speed/pitch 语义（pitch=1 时间伸缩；pitch=speed 重采样）；
  <https://developer.android.com/reference/android/media/PlaybackParams>
- AOSP `frameworks/av/media/libaudioclient/AudioTrack.cpp` — `AudioTrack::setSampleRate`（FAST/offload/direct 拒绝）、`setPlaybackRate`、`getTimestamp_l`、`kFixPitch=true`、`adjustSampleRate`；
  <https://android.googlesource.com/platform/frameworks/av/+/refs/heads/main/media/libaudioclient/AudioTrack.cpp>
- AOSP `frameworks/base/core/jni/android_media_AudioTrack.cpp` — `native_set_playback_rate` → `AudioTrack::setSampleRate`。
- `frameworks/base/media/java/android/media/AudioTrack.java` — 上述 Javadoc。

### 时钟/晶振

- RFC 5905（NTPv4）：`TOLERANCE=15e-6`、`MINPOLL=4`、`MAXPOLL=17`、建议 6–10、`T_c=2^tau`、自适应 tau；<https://www.rfc-editor.org/rfc/rfc5905>
- linuxptp：`servo.c`、`pi.c`、`ptp4l.8`（`max_frequency` 默认 900,000,000 ppb = 90% 硬件上限、`step_threshold`、Kp/Ki scale）；<https://github.com/richardcochran/linuxptp>
- Epson FA-238/FA-238V/TSX-3225 datasheet：初始 ±50 ppm、温度 ±30 ppm（−20…+70 °C）、老化 ±5 ppm/year；
  <https://media.digikey.com/pdf/Data%20Sheets/Epson%20PDFs/FA-238(V),TSX-3225_Series_Rev_Dec_2015.pdf>
- ECS“Tuning Fork Crystal Frequency and Parabolic Temperature Curve”：`0.04 ppm/°C²`、室温 ±20 ppm、极端温度 >±150 ppm；
  <https://ecsxtal.com/tuning-fork-crystal-frequency-and-parabolic-temperature-curve>
- Rakon“Oscillator basics for telecommunications applications”：宽温 20–100 ppm；
  <https://www.rakon.com/guide-oscillator-basics-for-telecommunications-applications>
- Microchip/Vectron“Clock Oscillators”应用说明：不同切型的 Δf/f–温度曲线；
  <https://ww1.microchip.com/downloads/aemDocuments/documents/VOP/ApplicationNotes/ApplicationNotes/Clock+Oscillators.pdf>

## 10. 证据边界与待测项

- 本文的**源码结论**来自 2026-11-22 抓取的 `master`/`main`，不是固定版本；上游后续可能改动。
- **未实测**：本项目的 `AudioTrack` 实际生效速率（`applied_rate_hz` vs 请求）、`getTimestamp` 噪声是“呈现语义”还是“平台节奏”、`buffer_current_ms` 0–28 ms 的结构性偏差、真机温漂速率、`setPlaybackParams` 在小数 ratio 下的实际听感。均标 `NOT RUN`，不得写成 `PASS`。
- **【推断】** 的核心有两点：① 量化阶梯的 `q/T` 误差模型（由量纲、实测数量级和最小二乘公式推导）；② 60–300 s 时间常数足以跟踪晶振温漂（由 ECS 抛物线系数和“热过程分钟级”估算）。这两点应优先用 host 噪声模型验证，再上真机。
- 建议的下一步不是改协议或放宽判据，而是：
  1. host 噪声模型复现 118/186 顶轨；
  2. 对比“10 s 直接控制”“300–600 s 长基线 + 死区”“缓冲 PI + 前馈”三种控制器；
  3. 同时记录连续 `correction_ppm` 和实际 `applied_rate_hz`；
  4. 真机复测顶轨比例、缓冲分布、听感 A/B（人工门）。
