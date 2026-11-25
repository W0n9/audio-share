# 34: Upgrade path from an older beta reports 205 on saved-server reconnect

**What to build:** Reproduce and fix (or explicitly document as a release limitation) the
upgrade path where an older v2 beta is upgraded with `adb install -r` (data retained) and
"Connect to saved Server" fails with 205 `SERVER_IDENTITY_MISMATCH` even though the saved
pin matches the running Server. A clean uninstall/install of the same APK does not
reproduce it. Determine whether the old credential record or key alias survives the
upgrade incorrectly, and either migrate it or require an uninstall-first upgrade in the
beta release notes.

**Blocked by:** None

**Status:** resolved

**Evidence:** `evidence/ticket-30/evidence/notes.md` lines 93–97 (clean install PASS, old
data `install -r` → 205; new client_id after clean install). Ticket 24/25 upgrade/rollback
documentation must reflect the outcome.

- [x] Reproduce with the previous beta APK's data (`.prev`).
- [ ] Identify the stale persisted field/key alias and fix or fail-closed with clear guidance.
      — **WAIVED**：维护者 2026-11-18 选择出口 (b)，不做凭据迁移修复。
- [x] If not fixed for beta, document "uninstall the previous v2 beta before installing"
      in the release notes and handoff.
- [ ] Regression test for the credential reload path. — **WAIVED**：随修复路径一并裁掉。

## Answer

2026-11-18 — 维护者裁决：**选出口 (b)，不做代码修复**，把「升级前卸载旧 v2 beta」作为
已知限制写进发布说明与 Windows handoff。理由：干净安装不复现，beta 目标为 fresh-install
pilot；凭据迁移需要额外真机验证窗口，不及 block 发布。

已落地（文档，PASS）：
- `.scratch/release-notes-v0.5.0-beta.md`「Known beta limitations」与「Upgrade → Android」
  两处写明 `adb install -r` 覆盖旧 beta 可能保留陈旧凭据并触发 205，要求先卸载、
  重装后重新配对。
- `.scratch/wire-protocol-v2/windows-handoff.md` 的 adb 安装步骤与升级说明含 `adb uninstall
  dev.audioshare.v2` 与 205 提示。

复现证据：`evidence/ticket-30/evidence/notes.md` 第 93–97 行（干净安装 PASS，保留旧数据
`install -r` → 205）。

未做（WAIVED）：陈旧字段/key alias 定位、迁移或失败关闭、凭据重载回归测试。若后续 beta
需要支持原地升级，重新开票并在真机上验证 `install -r` 路径。
