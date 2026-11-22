# M5-12: 漂移修正的噪声饱和（quality）

**What to build:** 让漂移控制器在估计噪声下不再长期把播放速率修正顶在 ±100 ppm 轨上；判据与协议不变。

**Blocked by:** —（不阻塞 beta 合入判断，stable 前须处理）

**Status:** resolved（2026-11-22 真机判定窗 PASS；听感 A/B 仍 `NOT RUN`，未作为通过依据）

**Type:** task

## Evidence

- [48 kHz 复测](../evidence/m5-11/RESULT-WINDOWS-48K.md)：`correction_ppm` 始终在 ±100 内，但 **118/186** 个快照顶在 ±100；`buffer_current_ms` 0–28 ms（目标 60）。
- [M5-11 复测](../evidence/m5-11/RESULT-WINDOWS-M5-11.md)：50/72 顶轨。
- B3：35/67 顶轨。三轮一致：控制器被逐窗噪声驱动到最大速率修正。
- 长期漂移 ~0（48 kHz：快照中位数 −1 ppm、均值 −9.6 ppm），逐窗噪声约 1,000–1,800 ppm，属测量分辨率问题（见 [Windows 复测清单](../evidence/m5-09/windows-retest.md) 的裁决说明）。

## Requirements

- [x] 若维持 10 s 窗口，噪声必然进入估计；控制器需要更强的平滑（更长积分）或对小幅估计设死区，避免把噪声当成速率差。（选用 180 s 长基线残差 + 20 s 阻尼；死区因会吞掉 ±75 ppm 被否决）
- [x] 不得放宽 §8 的 ±100 ppm 上限、60/200 ms、re-anchor 触发或协议；不钳位原始 `drift_ppm`。（已核对：`ppm()` 语义未变，见证据 §6）
- [x] 用 host 噪声模型（不同噪声幅度与真实漂移叠加）证明：真实 ±75 ppm 漂移仍能被跟踪到、不长期顶轨、修正有界；无噪声时修正接近 0。（host PASS，见证据 §4）
- [x] 真机复测记录顶轨比例与缓冲；若平滑带来持续偏差（例如真实漂移时修正跟不上），如实记 `FAIL`。（2026-11-22 真机 PASS，见 [RESULT-WINDOWS](../evidence/m5-12/RESULT-WINDOWS.md)）
- [ ] 听感 A/B 属人工门；可听音高抖动与否由维护者判定，不写 `PASS` 除非实际听过。（`NOT RUN`）

## Comments

- **2026-11-22 维护者裁决：agent 并行处理，host 先行**。状态 `needs-triage` → `ready-for-agent`；范围是 host 侧噪声模型复现 → 控制器方案对比 → host GREEN，另开后台调研（生产音频栈的速率匹配实践）并行参考。真机复测、听感 A/B 仍是人工门，未执行前一律 `NOT RUN`，不得写 `PASS`。
- 先确认 `rate_ppm` 是否真的作用到 `AudioTrack`（`setPlaybackParams`）。若根本没生效，顶轨只是日志现象，本票结论要重写。

- **2026-11-22 host 侧完成**（分支 `fix/m5-12-drift-saturation`，基于 `51a7aae`，本地未合入）。RED/GREEN、命令与完整数字见 [host 证据](../evidence/m5-12/host-drift-saturation.md)。链路核对结果：`rate_ppm` 确实经 `playbackRateHz` → `AudioTrack.setPlaybackRate` 生效，符号与单位正确。
  - **闭环残差（首要发现）**：播放头按已施加速率前进，估计量是 `d − applied`。旧控制器把它当绝对目标，稳态 `applied = d/2`，真实漂移长期只被修正一半。已改为 `d = residual + mean(实际施加)` 重建。
  - **整数 Hz 执行器**：48 kHz 下 1 Hz ≈ 20.83 ppm；旧 `playbackRateHz` 把 +100 ppm 映射成 5 Hz = **+104.2 ppm，越过 §8 上限**。已改为 Hz 域夹紧（48 kHz 最多 4 Hz = 83.3 ppm），native 侧 `actuator_ppm` 同步，命令值也不再报平台达不到的速率。
  - **SRC 被拒风险**：`setPlaybackRate` 返回非 SUCCESS 时速率修正整体失效，而新的重建会正反馈顶到执行器上限（旧控制器只收敛到 `d`）。该路由是否接受 SRC 本轮 **`NOT RUN`**；真机复测必须先确认日志无 `setPlaybackRate(...) rejected`。
  - host 验证：RED 顶轨 53–70%（12 + 24 个单元），GREEN 顶轨 0%，±75 ppm 稳态误差 −7.7…+12.8 ppm，无噪声修正 0；`cargo test` 260 / `--features test-support` 260 / clippy / fmt 干净。两轴 code-review 的 13 项整改已落地（状态词、actuator 配置耦合、控制窗单一来源、暖机残差语义、命名、测试公共模块等）。
- **适用域**：真机「顶轨比例」判据只在 |d| ≤ 83.3 ppm 有意义；84–100 ppm 时会停在执行器上限、留下残差，由 §8 整帧干预兜底（host 用例 `d = +90 ppm` 已钉住，残差 +6.9 ppm）。
- **2026-11-22 真机复测（APK `4120cdd` / Server `431997b`）：按裁决口径 PASS。** 完整脱敏结果见 [RESULT-WINDOWS](../evidence/m5-12/RESULT-WINDOWS.md)。
  - 前置：两个产物哈希与清单一致；48 kHz / 2 声道（执行器上限 ±83.3 ppm）；`drift_clock source=hardware` 215/215；`setPlaybackRate rejected` 0 次；覆盖 8,817/8,817；`discontinuous_intervals` 0、重建 0。
  - 判定窗 **10:27:06–10:42:25（919.5 s，180 个快照）**：顶轨 **0/180（0.0 %）**；`residual_ppm` 中位 **1**、均值 **0.92**（范围 −3…+5）；`correction_ppm` 最大绝对值 **5**。原始 10 s `drift_ppm` 仍为 −225…+361、170/180 区间越出 ±100——噪声存在但**未传导到执行器**。
  - 缓冲 0–20 ms、中位 10，5 分钟桶无上升趋势；Server `pending_frames` 0、`media.drops` 0；re-anchor / soft trim / concealment 均 0。
  - 暖机段（`residual_ppm=none`）**17/33 顶轨（51.5 %）**，按裁决单列、不进判据；本轮重建 0 次，所以每个会话只付一次 180 s 暖机成本。
  - 诚实边界：本轮设备自身长期漂移约 1 ppm（`residual` 中位 1），因此真机证明的是「噪声不再传导」，**不是**「真机上跟踪 ±75 ppm 真实漂移」——后者只有 host 噪声模型证据，真机 ppm 注入仍 `NOT RUN`。
  - **听感 A/B：`NOT RUN`**，未写成 `PASS`；修正最大 5 ppm（≈0.009 音分），如维护者要求仍需实际听测。
- **2026-11-22 维护者裁决：两项均接受。**
  1. **暖机期验收口径**：真机顶轨比例只统计 `residual_ppm != none` 的快照；整段会话（含暖机与每次窗口重建）的比例单列、不混入判据。复测会话至少 **8–10 分钟**，保证 180 s 控制基线有足够的稳态窗口。这不改变 §8 判据，只是明确新控制器的适用域。
  2. **`playbackRateHz` 5 Hz → 4 Hz** 作为 M5-12 的一部分**追认**：它修的是 +104.2 ppm 越过 §8 ±100 ppm 的事实缺陷，且新的残差重建依赖执行器的真实映射。§8 的 ±100 ppm 上界本身未变。

## Notes

- 真机复测交接：[handoff-windows-retest](../evidence/m5-12/handoff-windows-retest.md)（产物哈希、前置检查、判据与回传格式）。
- 相关：`android-client-v2/native/src/drift.rs`（`DriftController`，`slew_ppm_per_update`、死区）。
- 本票不是 M5-09/B 的否决项；B 已按 2026-11-21 裁决口径判定。stable 前须关闭或明确 `WAIVED`。
