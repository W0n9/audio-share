# v0.6.0 发布审阅包

**这是维护者的审阅入口，不是对外发布公告。** 审阅分支从 `integration/m5` 的 `9fece0d` 分出；本地 annotated tag `v0.6.0` 指向 `675f290`，不包含本次整理。仓库已有记录是「仅本地打标，未发 Release、未重建产物」；本次未向远端复查，也未执行推送、构建、签名或发布。

| 想审阅什么 | 从这里开始 |
| --- | --- |
| 本次能承诺什么、哪些没有验证、现有发版流程能否直接用 | [发布边界与放行清单](release-gate.md) |
| 用户会看到哪些变化与限制 | [v0.6.0 发布说明草稿](../release-notes-v0.6.0.md) |
| 延迟、缓冲、漂移、网络损伤分别测了什么 | [实验结果与证据索引](experiments.md) |
| 各代提案、规格和工单应以哪份为准 | [提案与历史索引](proposals.md) |
| 哪些材料可以后续归档，哪些不能当重复文件删 | [归档候选与保留规则](archive.md) |
| GitHub Actions 版本、Windows 构建耗时与挂起问题的调研 | [Windows CI 与 Action 调研](windows-ci-research.md) |

**先看两条结论：**

- 2026-11-22 的维护者裁决只覆盖 **单 Client 的 Wire Protocol v2 PCM（M4 + 已裁决的 M5 子集）**。可听时延基线、60 分钟 soak 和真实网络损伤等未验证项，仍必须按证据记录披露。
- **本地 stable 标签不是可发布产物。** 标签源码的 `VERSION` 仍是 `0.5.0-beta`；本分支已升到 `0.6.0`（Android `0.6.0` / `versionCode 3`，按 ABI 分别出包），但标签仍指向旧提交。修正后的 `Make Release` 会拒绝目标版本不匹配的构建；创建 draft 前，远端标签还必须指向实际构建提交。按[放行清单](release-gate.md)操作。

审阅材料不搬迁原始证据、不修改协议或工单状态。Markdown 链接指向该分支已跟踪的资料；标为「本地」的路径仅供有原始工作区的人查找，干净 checkout 不依赖它们。证据术语见[证据政策](../../docs/agents/evidence-policy.md)。
