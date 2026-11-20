# M5-10：调查单窗口内的 re-anchor 突跳

**What to build:** 先确认新构建是否仍出现集中 re-anchor。如果复现，区分上游补发、客户端交付/读端节奏和积压触发频率，再决定是否改代码。此票不预设修法。

**Blocked by:** —；现场证据优先复用 [M5-09 用例 B](../evidence/m5-09/windows-retest.md) 的新采样。

**Status:** needs-triage

**Type:** task

## Evidence

- [历史用例 B](../evidence/m5-windows-evidence-2026-11-20/evidence-2/RESULT2.txt) 在一个约 5 s 区间内新增 7 次 re-anchor，删 64,672 帧；该区间的逐次触发时序没有记录。
- [M5-09 host 对照](../evidence/m5-09/host-drift.md) 中，去掉 trim 的探针仍能复现同类漂移估计离群；修复估计窗后，保留的同一合成输入仍触发 7 次 hard trim。因此不能靠估计器修复宣称突跳已解决，也不能把合成结果当真机根因。

## 边界

- 不为减少计数削弱规范 §8 的硬限保护（>200 ms 丢最旧到 target，并如实记账），不改 60/200 ms 默认值。
- 用例 B 仍同时看有效漂移样本与 re-anchor；两者任何一项未过都不能放行 beta。M5-10 本轮不实施，不因开票改变 B 的下一步复测。
- 新构建上的 re-anchor 复测：**NOT RUN**。若复现，记录同一会话的逐次触发前后缓冲深度、删帧数、平台接受进度和服务端 `media.stats`，再评估是否做针对性 A/B。

## Comments

- 2026-11-21 [B3 复测](../evidence/m5-09/RESULT-B3.md)：`cb28095` 构建下 5 分 47 秒稳态窗 **re-anchor 0 次 / 0 帧**，soft trim 0，Server pending 0、零丢弃——历史 B2 的单周期 +7 未复现。只跑了一次，且当时 Server 干净；不能据此关闭本票，等更多样本或冒烟复现再 triage。
