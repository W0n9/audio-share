# M5-07: 60-minute single-client soak and stability

**What to build:** 单 Client 连续播放 60 分钟，验证无死锁、无假活会话、无不可恢复乱序、
延迟不持续增长。这是 M5 DoD 的稳定性门槛（双真机并发 60 分钟属 M7，不在此）。

**Blocked by:** M5-06；另须等 [M5-09](m5-09-drift-step-and-burst-trim.md) 用例 B 的真机复测 `PASS` 才排期

**Status:** ready-for-human

> **人工门**：60 分钟真机连续播放需现场值守、屏幕常亮与期间人为注入抖动/重连；
> agent 可准备脚本与采集工装，但长时间真机运行与听测由人执行。

**Type:** task

## Requirements

- [ ] 单 Client 连续播放 **60 分钟**，同一 Server、同一路径、屏幕常亮。
- [ ] 延迟不持续增长：用 L1 工装在整个窗口内采样 p50/p95/p99，末段与首段无系统性上升。
- [ ] **时延预算（spec §3 固定验收数值，normal-LAN，媒体路径额外时延）**：
      **p95 ≤ 80 ms、p99 ≤ 120 ms**。用 M5-01 工装测量并逐项判定；达不到就记 `FAIL`
      并作为 Q1 规范值裁决的输入，**不得静默放宽**。当前推算端到端约 225 ms，距 80 ms
      仍有很大差距，本项很可能触发 Q1 裁决。
- [ ] 无死锁、无假活会话（会话状态与实际播放一致）、无不可恢复乱序。
- [ ] Server/Client 无内存/句柄/会话累积泄漏（长跑前后对比）。
- [ ] `concealments`、`late_dropped`、`underruns` 分开统计并报告；MIUI `underrunCount`
      只作参考，不作为通过标准。
- [ ] 期间人为制造一次短暂网络抖动/重连，确认恢复且延迟回到基线。
- [ ] 结果写进 `.scratch/wire-protocol-v2/evidence/m5-soak/`，不入库。

## Notes

- 规范：spec §8/§11 的 60 分钟稳定性要求；`roadmap.md` M5 DoD。
- 双真机、API 26 真机、Windows 长跑仍属 M7 汇总。
