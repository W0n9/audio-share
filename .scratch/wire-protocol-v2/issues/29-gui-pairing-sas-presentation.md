# 29: GUI pairing SAS presentation deadlock (Windows GUI beta blocker)

**What to build:** The Server must present the live pairing code (SAS), display name,
`client_id`, address, mode, and expiry as soon as it has sent `PairingResponse` — before
the Client sends `PairingConfirm`. The Client needs the Server's SAS to perform its own
explicit confirmation; today the Server only exposes SAS inside the post-`PairingConfirm`
approval request, so the GUI waits for the Client while the Client waits for the GUI.

**Blocked by:** None

**Status:** resolved

**Evidence:** `evidence/ticket-25/ticket-25-writeback.md` and `notes.md`. Windows 11 +
Xiaomi M2007J22C (Android 12/API 31) reached `PairingReady / AwaitingConfirmation` in QR
(pasted link and human camera scan) and SAS modes, but the GUI never showed a pending
card; 5 reproductions. Non-GUI protocol, 30-minute playback, lock, network drop, revoke,
identity reset, and v1 coexistence passed.

- [ ] State machine exposes a one-shot `PairingPresented` view after `PairingResponse`
      (mode, SAS, fingerprint/identity, display name, client_id, address, expiry) without
      changing the wire protocol or the post-confirm approval semantics.
- [ ] `PairingCore` / `PairingAdmin` carry the live presented pairing; it is cleared on
      approval, decline, cancel, deadline, or connection loss.
- [ ] Server GUI shows `Pairing code: 123 456` (and display name/client_id/address) under
      the QR/invitation immediately when the phone connects, and keeps the dialog usable
      while a live pairing or pending approval exists.
- [ ] The approval card still appears only after `PairingConfirm`; accept/decline behavior
      is unchanged.
- [ ] `as-cmd pair` prints the pairing code when the request is presented (flushed), not
      only after the confirmation.
- [ ] TDD coverage at public seams: state-machine presented view before confirm; controller
      `pairing_status.live` lifecycle; as-cmd CLI live output; GUI store/controller state.
- [ ] Handoff docs correct the portable-store directory assumption: `as-cmd clients` /
      `identity reset` must run from the GUI's exe directory or with the same
      `AUDIO_SHARE_CONFIG`; otherwise they see a different `pairing/` store.
- [ ] Linux GUI manual check with a client that waits at `AwaitingConfirmation`; Windows
      GUI rerun is left for ticket 25 (record `NOT RUN` here).
- [ ] No accepted protocol, ADR, v1, fixtures, or frozen APK changes.

## Answer

已合入集成分支 `integration/wire-protocol-v2-beta`（merge commit `a380ee0`，2026-11-17）。实现提交 `88a6517`。

- **根因**：SAS 只在 `PendingPairing` 中，GUI 的 `ApprovalRequest` 要等 `PairingConfirm` 后才生成；Android 在 `AwaitingConfirmation` 需要 Server 的 SAS 才能确认，双方互等。
- **修复**：状态机在 `PairingResponse` 成功写出后一次性交付 `PairingPresented`（mode/SAS/display name/client_id/address/expiry）；`PairingCore`/`PairingAdmin` 保存 live 列表，在 approve/decline/cancel/deadline/transport loss 时清除；GUI 在 QR/邀请下立即显示 `Pairing code: 123 456` 与设备信息，审批卡片仍在 confirm 后出现；`as-cmd pair` 在 confirm 前打印并 flush SAS。
- **测试**：新增状态机/ core / controller / as-cmd 进程级用例，RED→GREEN；`cargo test --workspace` 603 passed / 4 ignored；interop 18；GUI check 0 errors、build PASS；Android JVM 107。
- **Linux GUI（Xvfb）**：客户端停在 `AwaitingConfirmation` 时 GUI 立即显示 live 卡片（设备名、client_id、地址、配对码），对话框不退回初始视图；confirm 后审批卡片出现。Xvfb 下 Accept 按钮点击未可靠完成，approve/decline 由 controller/core 测试覆盖。
- **文档**：handoff 修正 portable store 按 exe 目录隔离，`as-cmd clients/identity` 需在 GUI 的 `dist-windows` 同目录运行或共用 `AUDIO_SHARE_CONFIG`。
- **证据**：[`evidence/ticket-29/`](../evidence/ticket-29/README.md)。
- **NOT RUN**：Windows GUI 复跑（留给工单 25）；真机复跑。
