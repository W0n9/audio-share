# 27: Authorization store drift — non-atomic replacement can clear live authorizations

**What to build:** Remove the transient missing-file window in
`v2-transport::identity::atomic_write_with_backup` (rename target to `.bak`, then write the
new file) that lets a concurrent `AuthorizationStore::reload_if_changed` observe `Missing`
and clear the in-memory authorization table. Add a regression test that fails on the old
sequence and passes on the fix. Keep one backup generation, 0600/0700 permissions, and the
version-1 `clients.json` schema unchanged.

**Observed:** Ticket 21 emulator evidence: after many single-Client session replacements
(303) and a cross-emulator teardown, the Server returned `202 AUTHORIZATION_UNKNOWN` while
`clients.json` still held the active record; restarting the process fixed it. Ticket 19/22
evidence did not exercise this interleaving.

**Blocked by:** None

**Status:** resolved

- [ ] Reproduce the clear-on-transient-missing race with a deterministic test.
- [ ] On Unix, replace the target atomically (write temp, rename over the target) while
      preserving `clients.json.bak`; do not remove the target first.
- [ ] Windows behaviour documented honestly (best-effort if a true replace is unavailable).
- [ ] Permissions and existing auth-store tests remain green.
- [ ] Evidence records the RED/GREEN and the residual risk.

## Answer

已合入集成分支 `integration/wire-protocol-v2-beta`（merge commit `fdc8b19`，2026-11-16）。实现提交 `c338d7a`。

- **根因**：旧 `atomic_write_with_backup` 先 `rename(target → target.bak)` 再写 temp、rename temp→target；两次 rename 之间存在目标缺失窗口。并发 `reload_if_changed` 观察到 `Missing` 会清空内存授权表，导致 `202 AUTHORIZATION_UNKNOWN`，直到下次成功 reload 或重启。工单 21 的模拟器观察路径与此一致。
- **修复**（`crates/v2-transport/src/identity.rs`）：先写 temp（0600）并 fsync；用 `hard_link`（失败回退 `copy`）保留上一代 `.bak`，不再移走活动目标；最后 `fs::rename(temp, target)` 在 Unix 上原子覆盖。真实删除仍会在 reload 时撤销；identity 私钥轮换与 `.bak` 命名/权限不回归。
- **RED/GREEN**：`failed_replacement_never_hides_or_clears_live_authorizations` 与 `reload_during_a_replacement_never_observes_a_missing_store`（cfg(unix)）在旧实现上 FAILED，修复后 PASS。
- **回归**：`cargo test -p v2-transport` 251 passed；`cargo test --workspace` 586 passed / 4 ignored；interop 18 passed；fmt/clippy 干净。
- **证据**：[`evidence/ticket-27/`](../evidence/ticket-27/README.md)。
- **NOT RUN / 残余风险**：Windows `MoveFileExW` 语义未在 Windows 实操；无硬链接时的 copy 备份可被中断；绕过 `clients.json.lock` 的外部并发 writer 不在 §10.3 契约内。
- **附注**：一次 `protocol-harness` 时序测试在并行负载下 flake（`heartbeat_count=1`、`max_datagram_gap_ms=1265`），单独重跑 3/3 通过，第二次全量 workspace 通过；与本次改动无关，建议后续跟进。
