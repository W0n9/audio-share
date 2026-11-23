# 05: 文档、测试与证据收口

**What to build:** 确认术语与实现一致，补齐 Rust host、JVM、GUI 与模拟器证据，按证据政策记录结论并关闭 spec。

**Blocked by:** 02, 04。

**Status:** ready-for-agent

## 验收

- [ ] `GLOSSARY.md` 的 Revocation / Restore / Delete / Forget Server / Re-pairing 与最终实现一致。
- [ ] ADR-0006 状态为 accepted；若实现偏离，补一条修订说明而不是静默改写。
- [ ] Rust host、JVM、`svelte-check`、生产构建结果按 `docs/agents/evidence-policy.md` 记录。
- [ ] 模拟器走通：首次配对 → 撤销 → QR 重配恢复 → 撤销 → SAS 重配恢复 → Forget；记录 `EMULATOR`。
- [ ] 真机仅在排到会话时记录 `PASS`，否则 `NOT RUN`；不得用模拟器结果冒充真机验收。
- [ ] `spec.md` 状态与剩余项一致；未做的项明确列出。

## 事实来源

- `docs/agents/evidence-policy.md`
- `docs/adr/0006-revocation-and-repairing.md`
- `.scratch/pairing-ux/spec.md`
