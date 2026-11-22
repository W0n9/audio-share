# 提案与历史索引

**协议怎么工作**看正式规范与 ADR；**这次发布能承诺什么**看维护者裁决；**工作怎么做和测到哪里**看具体工单和证据。按[本地工单约定](../../docs/agents/issue-tracker.md)，仍保留 `.scratch/<feature>/spec.md` 和逐工单文件，不迁移或重编号。下表的「本地」是指盘点时未跟踪的源文件，干净 checkout 中不保证存在。

| 主题 | 当前该读哪份 | 历史或本地来源 |
| --- | --- | --- |
| Wire Protocol v1 / v2 | v2：[正式规范](../../docs/wire-protocol-v2/spec.md)、[配对与密钥生命周期](../../docs/wire-protocol-v2/pairing.md)、[威胁模型](../../docs/wire-protocol-v2/threat-model.md)、[ADR-0004](../../docs/adr/0004-wire-protocol-v2-transport.md)、[控制消息 schema](../../protos/wire-protocol-v2/control.proto)。v1 另见[冻结规范](../../docs/protocol.md)。 | 本地 `wire-protocol-v2/spec.md` 是产品与验收设计基线，不定义线格式；其中待验证的原型立项状态不代表现状。本地 `prototype-brief.md` / `prototype-report.md` 是 M1 的验证和 GO 记录，不是协议规范。 |
| M3–M4 与 Customer Beta | [M3 跨端报告](../wire-protocol-v2/phase-3-report.md)、[路线图](../wire-protocol-v2/roadmap.md)、[Beta 发布说明](../release-notes-v0.5.0-beta.md)及[真机证据摘要](../wire-protocol-v2/evidence/ticket-25/ticket-25-result.md)。按各文档绑定的提交读。 | 本地 `wire-protocol-v2/beta-spec.md` 定义受控试用：在 M1–M4 上补生产配对、最小 UI、GUI 设备管理和打包，不代表 M5–M7 完成；`release-runbook-362d023.md` 只适用于 `v0.5.0-beta`。 |
| M5 提案与 v0.6.0 裁决 | [实施与裁决记录](../wire-protocol-v2/map.md#decisions-so-far)、[M5 实验索引](experiments.md)与[发布边界](release-gate.md)。当前发版口径以 2026-11-22 决策为准；路线图早期状态和旧 Frontier 只作快照。 | 本地 `wire-protocol-v2/m5-proposal.md` 是讨论草稿。Q1（60/200 ms）没有可听基线，规范值未改；Q2（AudioTrack 128 ms）以 `d03865b` 移除应用侧固定请求、改用平台最小值的裁决为准；Q3（MIUI underrun）以听测和其他计数交叉判断，平台计数不能单独作为听感证明。 |
| Android Client v2 现代化 | [ADR-0005：Session Outcome](../../docs/adr/0005-session-outcome-ownership.md)与 [v2 map 的合入记录](../wire-protocol-v2/map.md#decisions-so-far)。`89f84b0` 合入重构，保留独立的重构后证据边界。 | 本地 `android-modernization/spec.md` / `map.md`：01–12 为决策，13–27 为实现收尾，map 为 `complete`。26 收测试资产，27 记录验收与合入。按本地 `evidence/ticket-08-refactor/manifest.tsv` 对照提交和产物；重构前的 Beta/Windows/v1 验收不继承为重构后 `PASS`。 |
| Rust Server Rewrite | [Server 文档索引](../../docs/server-rewrite/README.md)，[ADR-0001](../../docs/adr/0001-server-rewrite-baseline.md) / [0002](../../docs/adr/0002-server-rewrite-stack.md) / [0003](../../docs/adr/0003-server-rewrite-deviations.md)。 | [Rewrite 提案](../server-rewrite/spec.md)与 [01–16 工单](../server-rewrite/issues)是 v0.4.0 Rust/Tauri 重写历史，不是 v2 发布门槛。 |
| Server GUI 现代化 | [现代化验收](../../docs/server-rewrite/gui-modernization-acceptance.md)是 Variant C 的视觉验收入口；未被替换的原生行为仍查[原清单](../../docs/server-rewrite/gui-acceptance-checklist.md)。 | [GUI 提案](../server-gui-modernization/spec.md)与 [01–05 工单](../server-gui-modernization/issues)保留 Variant C 决策，不代替 v2 配对规范。[早期音频故障诊断](../audio-glitching/diagnosis.md)另留历史，不纳入 M5 结果。 |

## 工单编号如何区分

- 用「功能目录 + 完整工单名」引用，不只写数字。Wire Protocol v2 的本地 `30-client-pairing-state-reset.md` 是实现票；`30-pairing-confirmation-scope-adjudication.md` 是范围裁决，后续落到 31 / 32。两者不是重复副本。
- 旧规划编号 16 / 17 由 M5-03 / M5-04、M5-06 取代；`M6-NN` / `M7-NN` 在该路线图中是范围占位，不是本次已完成工单。
- 若早期计划、handoff 和后续验证同述一件事，保留三者时间顺序，链接到最终裁决和对应构建证据，不把早期状态原地覆盖成新状态。
