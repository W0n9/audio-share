# 22: Server GUI pairing and device management

**What to build:** A Server operator can begin pairing from the GUI, show a QR code and SAS/fingerprint with an expiry and cancel action, see the paired devices, and revoke one. Revocation terminates the affected active session promptly. The GUI does not gain a live v2 session or diagnostic view in beta.

**Blocked by:** 19

**Status:** resolved

- [ ] The pairing dialog shows the QR code, SAS code, fingerprint, expiry, and cancel.
- [ ] The dialog delegates to the shared Pairing Core; no pairing logic is duplicated in the GUI.
- [ ] The paired-device list shows display name and `client_id`, and supports revoke.
- [ ] Revoking a device terminates its active session promptly.
- [ ] Controller tests use a fake Pairing Core; `svelte-check` and the production frontend build pass.
- [ ] Pairing and revoke are verified manually on Linux; Windows is smoke-tested or explicitly recorded as NOT RUN.
- [ ] No live v2 session/diagnostic view or diagnostics export is added.

## Answer

已合入集成分支 `integration/wire-protocol-v2-beta`（merge commit `2ce53ab`，2026-11-16）。实现提交：`5780313`（共享 v2 runtime、GUI 配对对话框与设备管理）、`1d218b2`（修正确认/通知顺序）。

- **共享 runtime**：新增 `crates/v2-runtime`，统一持有 ServerIdentity、AuthorizationStore、PairingCore、QUIC listener、accept loop 与 capture/media 适配器；`as-cmd` 与 GUI 共用，GUI 无配对逻辑复制。
- **Controller**：同时管理 v1 与 v2；v2 启动失败保留 v1 Running 并暴露独立 `v2.lastError`；stop 先关 v2 再关 v1。
- **Tauri commands**：`pairing_begin`、`pairing_cancel`、`pairing_status`、`pairing_approve`、`pairing_decline`、`clients_list`、`clients_revoke`。
- **前端**：QR（Rust qrcodegen SVG）、URI、fingerprint、倒计时、取消；待批准卡片含设备名/`client_id`/来源/SAS/接受/拒绝；设备列表支持撤销；en-US/zh-CN 完整。无 live v2 会话或诊断视图。
- **竞态修复**：`PairingCore::persist_client` 不再提前广播；`Outcome` 携带 `paired_client`，driver 在 `PairingResult` 帧写成功后调用 `notify_paired`；一次性 `as-cmd pair` 用 `wait_for_idle` 事件驱动等待，删除 150 ms sleep。新增 20 次连续真实配对回归（修复前首轮复现 503，修复后 4×20 次全通过）。
- **验证**：`cargo test --workspace` 584 passed / 4 ignored；`v2-runtime` 9；GUI 103（1 ignored）；as-cmd 40（1 ignored）；interop 12；`npm run check` 0 errors/0 warnings；`npm run build` PASS；Android JVM 51 PASS。
- **Linux 手动 GUI**：Xvfb + dbus + `--fake-capture` PASS——QR/SAS/fingerprint/倒计时/取消、待批准、接受、设备列表、撤销（客户端 `Close(204)`）、停止顺序均有截图与日志。
- **证据**：[`evidence/ticket-22/`](../evidence/ticket-22/README.md)。
- **NOT RUN**：Windows GUI smoke 与 Windows 编译（新增 ring/quinn 依赖）；Android/模拟器对 GUI 的配对；手机侧手动验证使用临时 Rust QUIC 客户端，不记为 Android 验收。
