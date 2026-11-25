# L1 端到端可听延迟测量方案（定稿）

Status: 提案 — 待维护者裁决是否执行；不含排期承诺，不改动已 accepted 的规范数值
Date: 2026-11-19
相关: `m5-proposal.md` §1/§4、`evidence/m5-buffer-experiment/results.md`（低延迟档补充见同目录）
调研: 音频路径与诊断设施(full path survey) 2026-11-19

## 0. 为什么需要 L1

m5-buffer-experiment（真机，2026-11-18/19）结果：
- 平台缓冲 128 → 42 ms 后，用户**仍感觉有延迟**（只觉得 20ms 档"小一点"），听不出大差别
- `audio_flinger.Latency` 从 ~200 → 126 → 85 ms（`PERFORMANCE_MODE_LOW_LATENCY`），但那**不是**端到端可听延迟
- 20ms 档 `FrmRdy` 常满，说明积压可能从 AudioTrack 挪到了上游
- 后续补测（2026-11-19，见 `evidence/m5-buffer-experiment/low-buffer-and-ring.md`）：
  - Kotlin ring 积压稳定 0–20 ms，**排除**了"延迟藏在 ring"的假设
  - `setBufferSizeInFrames` 可把有效缓冲压到 21/10 ms，但 21 ms 档用户听测 **FAIL**
    （"延迟高了，有爆音"）——硬压播放缓冲不可行
  - `PERFORMANCE_MODE_LOW_LATENCY` 走 fast path，`Latency` 降到 58–80 ms，听测尚未做

结论：**播放端已到安全下限（42 ms），可感知延迟的大头在上游**（Windows WASAPI 1 s
缓冲 / 3 ms poll、Server send-ahead ≤100 ms）。必须先测出端到端可听延迟的分项，才能谈
优化。本方案不改变任何规范数值（60/200 ms、p95/p99）或生产默认，除非拿到数据 + 维护者裁决。

## 1. 音频路径与可观测点（调研结论）

```
[Windows 渲染端点] WASAPI loopback，1 s 硬编码缓冲，3 ms 轮询（backend.rs:35,372,402）
   └ BufferInfo.timestamp (QPC 100ns) 现成但被丢弃（backend.rs:436-450）★采集锚点
[FrameSink mpsc 64] → [MediaTap cap 8] → [v2-runtime cap 8]
[MediaPump.pending ≤500ms] 10ms tick / ≤64 datagram（media_pump.rs:44,driver.rs:81）
[Pacer 100ms token bucket]（send_queue.rs:18,106-156）
[SendQueue 256 dg, drop-newest] → QUIC DATAGRAM
[Android read_datagram] now_ms = media_clock.elapsed()（session.rs:590-613）
[MediaSession reorder/dedup] → [native ring 256 KiB 覆盖写]（ring.rs）
[Kotlin ring 读端，1-10ms 轮询，无 jitter buffer]（MediaPlayer.kt:317-329）
   └ buffer_current_ms 只量这里；recordPlayback 每 100ms 上报（MediaPlayer.kt:219,334-347）
[AudioTrack.write NON_BLOCKING]（MediaPlayer.kt:159-176）
   └ AudioTrack.getTimestamp() (API 26+) 可用但完全没用 ★出声锚点
[AudioTrack 缓冲] 默认 128ms / 实验 42-64ms / 低延迟 fast-path ~85ms Latency
[Audio HAL → 扬声器 → 人耳]
```

§9 PlaybackDiagnostics 关键字段（每 5 s 上报，**目前不写 server.log，只在内存**，唯一读者是测试）：
- `submitted_position`（field 24）= 最近提交给 AudioTrack 的帧位置，**在采集时钟坐标系**
- `buffer_current_ms`（22）= Kotlin ring 深度（不是端到端）
- `jitter_microseconds` / `late_drops` / `concealments` 目前**硬编码 0**（`media_session.rs:331-343`，M5 owns the estimator）

现有可复用资产：
- Goertzel 音检测 `tools/protocol-harness/src/probe.rs:764-870`（v1 算法，可复用）
- 音源 `scripts/gen-tone-wav.py`、`.scratch/audio-glitching/{gen-tone,play-tone}.ps1`
- `--fake-capture` / `SyntheticCapture::with_frames` 可注入精确 PCM（绕过真实 WASAPI）
- 桌面 v2 receiver `tools/rust-android-interop/tests/media_plane.rs:117-177`（无手机的接收点）
- WASAPI spike 记录 `timestamp_error`（可做采集侧校验）

## 2. 两个时间锚点（本方案的核心）

1. **采集侧**：`BufferInfo.timestamp`（QPC 100 ns，Windows 单调时钟）——现在被丢弃，接上即可
2. **播放侧**：`AudioTrack.getTimestamp()`（API 26+）把 frame position 映射到 `nanoTime`——现在完全没用

配合 header 的 `timestamp`（采集时钟 sample-frame 位置，两端一致），可以把"采集第 N 帧"映射到"真实时刻 t1"和"出声时刻 t2"。**不需要时钟同步**（如果只用采集侧锚点 + 播放侧同一时钟域的测量），或用 QUIC RTT（LAN 2–9 ms）估算少量偏差。

## 3. 分阶段执行

### 阶段 1 — 软件分段拆解（推荐先做，2 处小埋点）

Status: **DONE（2026-11-19，真机 + 真 Windows Server）** — 结果见
[`l1-handoff/l1-results.md`](l1-handoff/l1-results.md)。

实测（p50）：采集→提交 **96.8 ms**，其中 ring ~46 ms、上游 ~55 ms；提交速率 1.0x 实时；
端到端可听延迟推算 ≈ 225 ms（97 + AudioTrack 128 ms，未实测出声）。埋点通过
`AUDIO_SHARE_CAPTURE_TIMING=1` / `AUDIO_SHARE_L1_TIMING=1` 开启，日志走
`v2_transport::session_log` target 进入 GUI `server.log`。

### 阶段 2 — 端到端可听总量（gold standard，零协议改动）

目标：给出一个真实"可听延迟"数字（p50/p95/p99），满足 m5-proposal L1 的 DoD。

方法（声学回环 + 外部录音）：
1. Windows 侧用 `scripts/gen-tone-wav.py` / ffmpeg 生成脉冲串或 chirp（如 1 kHz 短脉冲每隔 2 s），用 `play-tone.ps1` / ffplay 循环播放——它会被 WASAPI loopback 采集
2. 一台外部录音设备（第二台手机或电脑麦克风）**同时**录到 PC 扬声器输出与 Android 手机扬声器输出
3. 对两路录音做互相关 / Goertzel（复用 `probe.rs:764-870`）得到延迟 = 两路信号时间差
4. 跑 10 分钟，统计 p50/p95/p99

优点：覆盖完整链路（含 HAL + 声学），不需要时钟同步、不改协议、不依赖 RECORD_AUDIO。
注意：录音设备到两只扬声器的距离要一致（或测出并扣除声学路径差）；冷启动/重连单独标注。

### 阶段 3 — in-band marker（可选，自动化分段）

用 `SyntheticCapture::with_frames` 注入带唯一脉冲的 PCM，在 Kotlin `playOnce` 前用 Goertzel 打点，测"注入→提交"延迟，可自动化、可给 p50/p95/p99。缺点：不含 AudioTrack/HAL/声学，**不能单独声称端到端**，必须与阶段 2 组合。

## 4. 证据与边界

- 真机：M2007J22C / MIUI / API 31，LAN `<lan-ip> ↔ <lan-ip>:65531`（RTT 2–9 ms）
- 状态词：见 `docs/agents/evidence-policy.md`；模拟器证据不算数；MIUI underrun 计数不可信，只作参考
- `audio_flinger.Latency` ≠ 可听延迟；`buffer_current_ms` ≠ 端到端
- 修改放在实验分支，不提交/推送，除非用户要求；`.scratch/` 不入库
- 三问裁决（60/200 数值、128ms 归属、underrun 可信度）在拿到 L1 数据后由 `/grill-with-docs` 处理；underrun 一问已有结论：不可信

## 5. 前置决策

1. 是否执行阶段 1 的 3 处小埋点（只加日志/观测，不改协议与音频行为）？
2. 阶段 2 的外部录音设备：用第二台手机、还是 Windows 另一台电脑的麦克风？
3. 低延迟模式是否纳入测量矩阵（`debug.audioshare.low_latency=1`，实测 Latency 85ms 但 underrun 计数暴涨，需听测确认）？
