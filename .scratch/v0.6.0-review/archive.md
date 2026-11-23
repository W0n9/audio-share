# 归档候选与保留规则

本次只做导航和结果摘要，**没有移动、删除或批量纳入 Git**。盘点时 `.scratch/` 有 1,190 个未跟踪文件（Android 现代化 726 个、Wire Protocol v2 464 个）；未跟踪原件不会随审阅分支或标签交付，必须由保管原件的人另行备份。现有已跟踪摘要及历史记录原样保留；证据默认不入库，例外见[证据政策](../../docs/agents/evidence-policy.md)。

| 材料 | 本次怎么查 | 后续建议（待审阅，不执行） |
| --- | --- | --- |
| M5 的 L1、42 ms 播放缓冲、Windows A/B 与控制器复测 | 以[实验索引](experiments.md)里的版本化结果为准；原始日志、截图、脚本及 Windows 本地采集清单留原位。 | 如要归档，按工单、运行日期、两端版本和证据状态保存一份清单；暖机/判定窗、首测/复测和 RED/GREEN 不互相覆盖。配对记录、系统日志、设备标识、原始截图不随发布分发。 |
| Wire Protocol v2 工单 30 的 `final` / `merged` 和两份 `30-*` 工单 | 本地：`wire-protocol-v2/evidence/ticket-30/`、`wire-protocol-v2/issues/30-*.md`。入库部分是 [JVM 摘要目录](../wire-protocol-v2/evidence/ticket-30)。 | 先按构建与场景核对差异，再标推荐入口。两个 `30-*` 分别是实现和范围裁决，不因编号相同而合并、删除或重编号。 |
| Android 现代化的原型、证据、Windows handoff | 本地：`android-modernization/prototypes/`、`android-modernization/evidence/`、`android-modernization/windows-handoff/`。Git 上的合入状态见 [v2 map](../wire-protocol-v2/map.md#decisions-so-far)。 | 原型源代码、实验结果和安装包分存。先选出可脱敏的决策与验收摘要；APK、截图、Gradle/ADB 日志不上审阅分支。不要把改造完成或既有 `EMULATOR` 记录升格成 0.6.0 产物真机验收。 |
| Beta 打包与 Windows 交接 | 本地 `wire-protocol-v2/release-runbook-362d023.md`；已跟踪的 [Beta 发布说明](../release-notes-v0.5.0-beta.md)、[Windows handoff](../wire-protocol-v2/windows-handoff.md)、[真机结果](../wire-protocol-v2/evidence/ticket-25/ticket-25-result.md)。 | 保留为 `v0.5.0-beta` 历史；正式 0.6.0 runbook 从[放行清单](release-gate.md)重建，不覆盖旧步骤或用旧结果代替新产物验收。 |
| Server Rewrite、GUI 现代化、早期音频故障分析 | 见[历史索引](proposals.md)。 | 保留其原有版本、兼容性记录和 ADR；从 0.6.0 主阅读路径分离即可，不删除过去的结论。 |

若后续要迁移原件，先区分需要复现的输入、每次执行的输出与重复拷贝，记录来源/哈希和保管位置，再决定脱敏摘要是否入库。工作区的 IDE 项目文件不属于本次发布材料，也未处理。
