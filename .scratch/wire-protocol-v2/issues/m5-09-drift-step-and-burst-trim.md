# M5-09: Drift step-offset correction and burst-aware trim/re-anchor

**What to build:** 先在 Rust host 上复现用例 B 的 `drift_ppm` 离群，然后只修能通过证据归因的代码问题。re-anchor 突跳是关联信号，不预先认定为因果。

**Blocked by:** —

**Status:** claimed

**Type:** task

## Evidence

- `evidence/m5-windows-evidence-2026-11-20/evidence-2/RESULT2.txt`
- 复测 B（5m21s）：reanchor 0→8（15:07:53 同一 5 s 内 1→8，删除 64,672 帧 ≈1.35 s），
  同刻 `drift_ppm=-38082`，随后 `+336157`；span≥10 s、sample≥98 均满足。
- A（5m25s）：trim soft +86,941 帧，reanchor 1 次，drift |ppm|>10000 有 6 点。
- 服务端 pending 0 / ≤66 ms、零丢弃——问题在客户端与估计器，不在服务端排空。

## Hypotheses

1. `drift_*_step` 偏移补偿只作用于新样本或符号/量纲有误，导致阶跃后窗口仍含跳变。
2. reanchor 触发过密（B 5 min 8 次）源于 catch-up/突发交付使 ring backlog 瞬间 >200 ms；
   hard-limit 阈值不应在单次突发上反复触发。

## Requirements

- [ ] 从 `client-B2.log` 中截取离群附近约 60 s 的观测。仅在有实际输入时做原样回放；若只有汇总，明确标记用观测参数生成的合成输入。在真实 native 路径做稳定的 RED→GREEN；若不能复现同类症状，不盲改。
- [ ] 只修有实验支持的离群根因；不钳位原始 `drift_ppm`，不改协议或 60/200 ms。若突发 trim/re-anchor 频率是独立问题，另立工单，不顺手改。§7.4 继续用 `media.stats` 作守卫，不改 Server pacing/catch-up。
- [ ] 检查修正量依然满足规范 §8 的 ±100 ppm；这个限值约束施加的修正，不是物理时钟差上限，也不是对原始估计的限幅。
- [ ] 新产物用 Windows GUI Server + 单台 Android 真机重跑用例 B（有效播放 ≥5 min，不加网络损伤）。现场裁决要求：满窗后稳态原始 `drift_ppm` 在 ±100 ppm 内；若 re-anchor 使窗口失效，记录事件及重建窗口期间的样本，不把异常尖峰静默排除；没有类似单个 5 s 记录周期 +7 次 re-anchor 的突跳。记稳态中位数（仅诊断）、样本数、极值、最大每 5 s re-anchor 增量、Server pending/drops。没有新真机证据不关闭工单、不合入 beta；±100 ppm 注入矩阵仍是独立验收。

## Notes

- 相关代码：`android-client-v2/native/src/{drift.rs,media_session.rs}`、
  `android-client-v2/app/src/main/java/dev/audioshare/v2/{MediaPlayer.kt,MediaRingBuffer.kt}`。
- 第 7 项（jitter hole-hold）仍单独待裁决。
