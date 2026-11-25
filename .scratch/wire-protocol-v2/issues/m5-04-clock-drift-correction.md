# M5-04: Capture/playback clock drift measurement and correction

**What to build:** 实现 spec §8 的时钟漂移测量与修正（±100 ppm），以及 hard-limit
re-anchor。两端时钟不同源，长时间播放会漂移；不修正会导致缓冲持续增长或耗尽。

**Blocked by:** M5-03

**Status:** ready-for-human

**Type:** task

## Requirements

- [ ] 漂移测量：capture clock vs playback clock，**10 s 滚动窗口**。
- [ ] 漂移修正：播放速率调整 **≤ ±100 ppm**，平滑，不按帧计数。
- [ ] 当缓冲偏离 target **> 20 ms 持续 > 5 s**（即使速率匹配后）→ 插入或删除整帧，
      计入 `concealment`。
- [ ] hard-limit re-anchor：缓冲 **> 200 ms** → 丢最旧到 target 并前跳提交位置；删除帧
      计入 concealment；不得让缓冲无限累积延迟。
- [ ] 真机注入 ±100 ppm 漂移验证（可先用合成时钟/`SyntheticCapture` 在 host 验证，
      再真机确认）。
- [ ] §9 诊断暴露漂移量，便于 L1 工装复核。

## Notes

- 规范：spec §8「Drift measurement」「Drift correction」「Hard-limit re-anchor」。
- 4 类 concealment 中 drift insert/delete 与 re-anchor deletion 由本工单落地，另两类
  在 M5-03。

### 2026-11-19 只读分析提示（来自 M5-05）

M5-05 的只读代码分析发现：`MediaPump.accept` 在 pending 超上界时丢的是**尚未 packetize**
的 chunk，而 packetizer 只按实际发送的帧推进 `timestamp`（`crates/v2-transport/src/media.rs:191-205`，
无 `chunk.start_frame` 重锚）。因此服务端丢帧在 wire 上不留 sequence 跳空，
但 header `timestamp` 会与真实 capture frame 永久错开一次。本工单依赖 `timestamp`
（jitter buffer 目标 / 漂移窗口），实现与验收时须考虑该偏移，不能假设 timestamp 与采集位置
严格一致。详见 `m5-05-upstream-pacing-tuning.md` 的「只读代码分析」段。

## Answer

**实现（`integration/m5`；提交 `0a19a06` + `356a38a`，review 修复 `f799599`）**

- `android-client-v2/native/src/drift.rs`：纯逻辑、注入时钟的 `DriftEstimator`（capture 时钟 vs playback 时钟，**10 s 滚动窗口**最小二乘斜率 → ppm）与 `DriftController`（**±100 ppm** 上限，10 ppm/update 平滑，不按帧计数）。review 修复了窗口淘汰逻辑——现在保留一个早于 cutoff 的样本，10 s 内有估计。
- 持续偏差：缓冲偏离 target **> 20 ms 且 > 5 s**（速率匹配后）→ 插入/删除整帧，经 `record_concealment(Drift, ±frames)` 计入 concealment。
- hard-limit re-anchor：缓冲 **> 200 ms** → drop-oldest 到 target 并前跳提交位置，删除计入 `ReAnchor`。
- 应用路径：Kotlin `AudioTrack.setPlaybackRate(playbackRateHz(...))`（48 kHz 下约 20.8 ppm/Hz）；控制器本身纯逻辑、host 可测。
- 暴露：`driftPpm` / `driftCorrectionPpm` JNI + `playback.drift` 日志。**§9 `PlaybackDiagnostics` 无 drift 字段，未改 proto**；新增字段属协议变更，需维护者裁决，记为 deferral。

**验证（host/JVM，PASS）**
- native 默认与 `--features test-support`：各 **227 passed, 0 failed**（含 `clock_drift_contract`、`clock_drift_session_contract`）。
- JVM：**268 tests, 0 failed, 0 skipped**。
- 第二轮 review 修复了软修剪污染漂移窗口（`record_soft_trim` 重置窗口）与 config 变更后 jitter 几何未重建的问题。

**NOT RUN（不写成 PASS）**
- 真机注入 ±100 ppm 漂移与可听验证：NOT RUN（无设备）。
- 目标设备音频路由是否接受 `AudioTrack.setPlaybackRate`：NOT RUN。
- §9 drift 字段：deferred（协议变更需裁决）。
- 残余风险：deadline 锚点按已接受 datagram 滚动刷新，长时间网络 stall 且 capture timestamp 前进时可能接受而非判 late；真机行为 NOT RUN。
- 证据：`evidence/m5-04/`（未跟踪）。

## Comments

### 2026-11-20 Windows 真机验收（用例 B）

FAIL（用户裁决）。buffer 60/56 ms 有界；drift concealment 0；`drift_ppm` 36 / 499（单次离群），
correction 限幅 -80/100；窗口末尾起服务端 `pump_dropped_chunks` 持续增长（pending≈483 ms）。
±100 ppm 注入 NOT RUN。修复与复测待 [M5-08](m5-08-pump-drop-trim-underrun-investigation.md)。
证据 `.scratch/wire-protocol-v2/evidence/m5-windows-evidence-2026-11-20/`。

### 2026-11-20 复测（evidence-2，431997b）

用例 B **FAIL（Linux 侧裁决，open）**：buffer 0–62 ms 有界、零丢弃，但 drift ppm 仍有极端离群
（−38,082 / +336,157），与 reanchor 1→8 突跳同刻；发布门槛（span≥10 s、sample≥10）已满足，
说明是阶跃偏移补偿问题，不是窗口长短。drift concealment +2。待第 3 项残留修复后复测。
