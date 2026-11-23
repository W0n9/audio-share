# 05: 文档、测试与证据收口

**What to build:** 确认术语与实现一致，补齐 Rust host、JVM、GUI 与模拟器证据，按证据政策记录结论并关闭 spec。

**Blocked by:** 02, 04。

**Status:** ready-for-human

## 验收

- [x] `GLOSSARY.md` 的 Revocation / Restore / Delete / Forget Server / Re-pairing 与最终实现一致。
- [x] ADR-0006 状态为 accepted；若实现偏离，补一条修订说明而不是静默改写。
- [x] Rust host、JVM、`svelte-check`、生产构建结果按 `docs/agents/evidence-policy.md` 记录。
- [ ] 模拟器走通：首次配对 → 撤销 → QR 重配恢复 → 撤销 → SAS 重配恢复 → Forget；记录 `EMULATOR`。（当前仅 SAS 路径为 `EMULATOR`；`pairQr`、QR 重配、Forget 与完整清单为 `NOT RUN`）
- [x] 真机仅在排到会话时记录 `PASS`，否则 `NOT RUN`；不得用模拟器结果冒充真机验收。
- [x] `spec.md` 状态与剩余项一致；未做的项明确列出。

## 事实来源

- `docs/agents/evidence-policy.md`
- `docs/adr/0006-revocation-and-repairing.md`
- `.scratch/pairing-ux/spec.md`

## Comments

- Review fixes merged at `6242ec7`. Automated verification PASS: Rust 357 (349 + 8
  `pairing_live`), GUI 114, Android JVM 332; fmt/clippy/GUI build PASS.
- The full emulator UI checklist (`pairQr`, QR re-pair, Forget) and the real device remain
  `NOT RUN`; the ticket stays `ready-for-human` until a human records that gate.
