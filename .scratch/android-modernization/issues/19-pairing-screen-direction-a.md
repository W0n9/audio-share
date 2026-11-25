# 19: 配对屏 Direction A

**What to build:** 配对流程按方向 A 重做：QR 一步确认、SAS 输入与指纹、失败/过期原地重扫、COMPLETE 状态，全部可读、可本地化、可访问。

**Blocked by:** 17, 18。

**Status:** resolved

- [x] 覆盖未开始（QR/粘贴链接/SAS 入口）、扫码中、STARTING、AWAITING_CONFIRMATION、FAILED、COMPLETE 六种状态。
- [x] QR 模式在 AWAITING_CONFIRMATION 时自动确认恰好一次；显示“已确认，等待 Server 接受”；SAS 模式保留输入与显式确认。
- [x] 201/206 显示阻塞态与显式 Scan again；绝不自动重开相机。
- [x] 失败显示错误码、指引与 Try again；COMPLETE 显示保存/交接反馈。
- [x] SAS 逐位朗读、错误/状态 liveRegion、48dp、200% 字体、zh-CN 文案齐备。
- [x] Robolectric 测试覆盖上述状态与自动确认恰好一次；不重写 [工单 30](../../wire-protocol-v2/issues/30-client-pairing-state-reset.md) / [工单 32](../../wire-protocol-v2/issues/32-android-qr-one-action-recovery.md) 已覆盖的原生行为。

## Comments

2026-11-17 — 完成并合入集成分支（merge `d0af689`，feature `eab5a35`，已同步 ticket 20 tip）。
六种状态、QR 恰好一次自动确认（Service `PairingConfirmGate` 未动）、201/206 显式 Scan again、
失败码/指引/Try again、COMPLETE 交接反馈、逐位 SAS、liveRegion、48dp、en+zh 资源。
JVM 244 tests/variant PASS、shrinkReleaseResources PASS。证据：`evidence/ticket-19/`。
模拟器/相机/视觉项 NOT RUN（26/27 承接）。
