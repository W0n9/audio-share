# M5-01: End-to-end audible latency baseline (L1 phase 2)

**What to build:** 实现 L1 阶段 2 的声学回环测量工装，产出**端到端可听延迟**的
p50/p95/p99 基线，覆盖采集 → 网络 → 播放 → Audio HAL → 声学输出。这是 M5 所有优化
的对照基准，也是判定 60/200 ms 规范值（Q1）是否需调整的唯一依据。

L1 阶段 1 已完成（见 `.scratch/wire-protocol-v2/l1-handoff/l1-results.md`）：实测
「采集→提交 AudioTrack」p50 **96.8 ms**（ring ~46 ms + 上游 ~55 ms），提交速率 1.0x。
但 `submitted_position` **不是出声帧**，AudioTrack 之后的部分未测；端到端 ~225 ms 目前
只是推算。

**Blocked by:** —

**Status:** WAIVED（2026-11-22 维护者裁决：缺第二台录音设备，端到端可听延迟基线不测；这不是 PASS）

**Type:** task

> **人工门**：需要第二台录音设备（第二台手机或 PC 麦克风）与现场操作；agent 可准备脚本
> 与信号，但"同时录两只扬声器"这一步要人做。

> **前提**：L1 阶段 1 的埋点（`AUDIO_SHARE_CAPTURE_TIMING` / `AUDIO_SHARE_L1_TIMING`）
> 已提交到 `experiment/m5-audiotrack-buffer` @ `f073e07`（7 个文件，含 `Cargo.lock`；
> 实验分支，未 push、未合并）。维护者已裁决保留在实验分支，不转正。M5-01 可引用该
> 提交中的 L1 埋点；转正仍需清理与 Q2 裁决。

## Requirements

- [ ] Windows 播放已知参考信号（短脉冲串或 chirp），复用 `scripts/gen-tone-wav.py`、
      `ffmpeg`/`ffplay`；信号必须能被 WASAPI loopback 采集到。
- [ ] 一台外部录音设备**同时**录到 PC 扬声器与 Android 手机扬声器；对两路做互相关 /
      matched filter（算法起点：`tools/protocol-harness/src/probe.rs:764-870` 的 Goertzel）。
- [ ] 记录并扣除声学路径差（录音设备到两只扬声器的距离需一致或实测修正），或在报告中
      明确标注未扣除。
- [ ] 采集 ≥10 分钟，输出 p50/p95/p99 端到端可听延迟；用 L1 阶段 1 埋点做分项拆解
      （采集 / ring / AudioTrack / HAL / 声学）。
- [ ] 把 L1 阶段 1 的 capture 日志间隔从 1 s 提升到 100 ms（插值误差从 ±24 ms 降到
      ~±2–4 ms），作为工装的一部分。
- [ ] 冷启动、断流重连、锁屏三种状态分别标注；不得混进稳态 p50。
- [ ] 真机证据（M2007J22C / API 31）；模拟器不算。每行按 `PASS`/`FAIL`/`NOT RUN` 记录。
- [ ] 基线数据与脚本存入 `.scratch/wire-protocol-v2/evidence/m5-l1/`，不入库。

## Notes

- 不做任何协议、规范数值或生产默认改动。
- 产出的基线是 M5-02..M5-07 的验收对照；没有它，后续"延迟降低 X ms"无法被验证。

## Comments

- **2026-11-22 维护者裁决：`WAIVED`**。本轮没有第二台录音设备，声学回环无法执行；按证据政策记为 `WAIVED`（接受未验证风险），不记为 `PASS`。
- 因此**仍无**端到端可听延迟基线：现有 ~225 ms 只是 L1 阶段 1 的推算（采集→提交 p50 96.8 ms），AudioTrack 之后与声学路径未测。
- 受影响项：M5-07 的 p95/p99 时延预算无法用本工装测量；Q1（60/200 ms 规范值）失去实测依据，只能按推算裁决。
- 若将来补测，requirements 与判定标准原样适用，需新证据后才能把本票从 `WAIVED` 改为 `PASS`。
