# M5-12: 漂移修正的噪声饱和（quality）

**What to build:** 让漂移控制器在估计噪声下不再长期把播放速率修正顶在 ±100 ppm 轨上；判据与协议不变。

**Blocked by:** —（不阻塞 beta 合入判断，stable 前须处理）

**Status:** needs-triage

**Type:** task

## Evidence

- [48 kHz 复测](../evidence/m5-11/RESULT-WINDOWS-48K.md)：`correction_ppm` 始终在 ±100 内，但 **118/186** 个快照顶在 ±100；`buffer_current_ms` 0–28 ms（目标 60）。
- [M5-11 复测](../evidence/m5-11/RESULT-WINDOWS-M5-11.md)：50/72 顶轨。
- B3：35/67 顶轨。三轮一致：控制器被逐窗噪声驱动到最大速率修正。
- 长期漂移 ~0（48 kHz：快照中位数 −1 ppm、均值 −9.6 ppm），逐窗噪声约 1,000–1,800 ppm，属测量分辨率问题（见 [Windows 复测清单](../evidence/m5-09/windows-retest.md) 的裁决说明）。

## Requirements

- [ ] 若维持 10 s 窗口，噪声必然进入估计；控制器需要更强的平滑（更长积分）或对小幅估计设死区，避免把噪声当成速率差。
- [ ] 不得放宽 §8 的 ±100 ppm 上限、60/200 ms、re-anchor 触发或协议；不钳位原始 `drift_ppm`。
- [ ] 用 host 噪声模型（不同噪声幅度与真实漂移叠加）证明：真实 ±75 ppm 漂移仍能被跟踪到、不长期顶轨、修正有界；无噪声时修正接近 0。
- [ ] 真机复测记录顶轨比例与缓冲；若平滑带来持续偏差（例如真实漂移时修正跟不上），如实记 `FAIL`。
- [ ] 听感 A/B 属人工门；可听音高抖动与否由维护者判定，不写 `PASS` 除非实际听过。

## Notes

- 相关：`android-client-v2/native/src/drift.rs`（`DriftController`，`slew_ppm_per_update`、死区）。
- 本票不是 M5-09/B 的否决项；B 已按 2026-11-21 裁决口径判定。stable 前须关闭或明确 `WAIVED`。
