# L1 阶段 1 结果 — 采集↔播放提交延迟（真机 + 真 Windows Server）

Date: 2026-11-19
Device: Redmi Note 9 / MIUI / API 31 · Server: Windows `测试主机 B`, LAN `<REDACTED>:65531`, RTT 2–9 ms
Server build: `experiment/m5-audiotrack-buffer` + L1 埋点，cargo-xwin 交叉编译
`AudioShareServer.exe` `sha256 dd6ba948…1257fc`，环境变量 `AUDIO_SHARE_CAPTURE_TIMING=1`
`AUDIO_SHARE_L1_TIMING=1`
Log: `server.log`（96 条 capture / 20 条 playback，记录时长约 104 s，播放约 97 s）
Analysis: `analyze.py`（uv + numpy）

## 方法

两端用同一个**采集帧坐标系**：
- `l1.capture_timing.frame` = 服务端已采集的总帧数（与媒体 header `timestamp`、`device_index` 同步）
- `l1.playback_timing.submitted_position` = 客户端最后交给 `AudioTrack` 的帧位置（§9）

在任一时刻：`采集→提交延迟 = capture_frame(T) − submitted_position(T)`，帧数 ÷ 48 = ms。
`capture_frame(T)` 由每秒一条的 capture 日志线性插值得到。

## 结果

采样时钟校验：qpc 推算 **47 999.9 帧/秒**（期望 48 000）——采集时钟稳定。
稳态提交速率：**48 022 帧/秒 ≈ 1.0x 实时**——没有持续积压/漏送。

| 指标 | p50 | p95 | p99 | min | max |
|---|---|---|---|---|---|
| **采集→提交延迟** | **96.8 ms** | 135.1 ms | 144.7 ms | 48.4 | 147.0 |
| ring 深度（`buffer_current_ms`） | 46 ms | — | — | 9 | 69 |
| 上游（延迟 − ring，近似） | **54.6 ms** | 104.0 ms | — | 1.9 | 105.0 |

Underruns 全程 0；但 MIUI 该计数已被前序实验判定不可信，仅作参考。

## 关键结论

1. **采集→提交就有 p50 ≈ 97 ms**，而且只是"交给 AudioTrack"，**不含**实际出声。
2. 拆分（近似）：
   - **ring 积压 ≈ 46 ms** — M4 没有目标深度控制；ring 里剩多少就一直延迟多少，启动填充后不主动修剪。
   - **上游 ≈ 55 ms** — WASAPI 采集缓冲 + Server send-ahead（§7.4 上限 100 ms）+ 网络 + 客户端 read。波动大（2–105 ms），是抖动的主要来源。
3. **端到端可听延迟（推算，非实测）** ≈ 采集→提交 97 ms + AudioTrack 缓冲 **128 ms**（手机当前 `track_buffer_ms=100`→平台取整 6150 帧）**≈ 225 ms**。与 `audio_flinger.Latency` ~200 ms 量级一致，也解释了用户的延迟感。
4. 前序缓冲实验已证明 AudioTrack 128→42 ms 听感连续、21 ms 会爆音；即播放端有 **~86 ms** 的安全可削减空间。

## 证据边界（必须如实标注）

- `submitted_position` 是"交到播放 API 的帧"，**不是出声帧**；AudioTrack 之后到扬声器（缓冲 + HAL + 声学）未计。
- 端到端 225 ms 是**推算**，不是声学实测（阶段 2 才做）。
- 插值精度受 capture 日志 1 s 间隔限制，单点误差可达 **±24 ms**；稳态样本仅 18 个，p95/p99 参考价值有限。
- 本阶段所有延迟数字只覆盖"服务端采集 → 客户端提交"，模拟器不参与。

## 对 M5 三问的含义

- **Q1（60/200 ms 规范值）**：仍不能改。本实验没测可听端到端，只测了提交前链路；但已显示上游抖动需要 jitter buffer，60 ms target 仍有意义。
- **Q2（128 ms 归 M4 还是 M5）**：证据支持"可修"——AudioTrack 42 ms 听感连续，ring 与上游也有优化空间。
- **Q3（MIUI underrun 可信否）**：不可信（前序已定，本实验 underruns 恒 0 与听感无冲突但不足以反证）。

## 优化排序（按预期收益，均需验证）

1. **AudioTrack 128→42 ms**：省 ~86 ms，已真机听测连续（20ms 档）。
2. **ring 目标深度**：当前稳定积压 ~46 ms；按目标修剪（spec §8 hard-limit re-anchor 的方向）可省数十 ms，需避免爆音。
3. **上游 send-ahead / WASAPI**：~55 ms，涉及抖动预算，需与 jitter buffer 一起设计。
4. 合计理论空间：225 ms → 约 100 ms 量级，但需逐项真机验证（先 L1 阶段 2 声学回环校准）。

## 复现

```bash
# Windows: 设环境变量后启动带埋点的 AudioShareServer.exe，播放 ≥2 分钟
# Linux:
uv run --with numpy python analyze.py server.log
```
提高精度：把 capture 日志改为每 100 ms 一条（当前 1 s），插值误差可降到 ~±2–4 ms。
