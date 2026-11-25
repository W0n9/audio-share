# 26: Evidence policy and roadmap alignment

**What to build:** `EMULATOR` is documented as a formal evidence status alongside `PASS`/`FAIL`/`NOT RUN`/`WAIVED`, and the roadmap's M5–M7 dependency view reflects the beta path. This is a governance change only: no protocol, verification-number, or accepted-ADR change.

**Blocked by:** None (can start immediately)

**Status:** resolved

- [ ] `EMULATOR` is defined in the project's evidence vocabulary, including the gates it can and cannot satisfy.
- [ ] The roadmap shows the beta path (pairing Core → Android native pairing / GUI pairing → Compose UI → release → acceptance) and keeps M5/M6 multi-Client work as stable-gate items.
- [ ] The beta spec, roadmap, and issue tracker agree on scope, exclusions, and dependencies.
- [ ] No accepted protocol document, ADR-0001/0004, or verification number is modified.
- [ ] Maintainer sign-off is recorded.

## Comments

- 2026-11-16：维护者（用户）明确授权更新 roadmap 与证据词汇，并要求记录其签核。本工单据此落盘。
- 完成内容：
  - 新增 `docs/agents/evidence-policy.md`，定义 `PASS` / `FAIL` / `NOT RUN` / `WAIVED` / `EMULATOR`，并列出 `EMULATOR` 可以满足与永远不能替代的门槛。
  - `AGENTS.md` 增加 Evidence policy 指针。
  - `roadmap.md` §0 增加 `EMULATOR` 与证据策略指针；§2 后新增 Customer Beta 路径表；§4 更新 production pairing 与 v1 标注两行；§7 维护规则纳入 `EMULATOR`。
- 未修改任何 accepted 协议文档、ADR-0001/0004 或验证数字。roadmap 位于 `.scratch/`，按约定保持未跟踪。
