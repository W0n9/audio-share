# 02: Server GUI 的设备行、重配入口、失败态与成功反馈

**What to build:** revoked 行提供“重新配对”和“删除”，active 行只有撤销；approve 命中 revoked 身份时显示恢复确认卡而不是普通接受卡；配对失败态把“刷新二维码”置前并保留“使用配对码”；配对成功显示一行反馈。

**Blocked by:** 01。

**Status:** ready-for-agent

## 验收

- [ ] revoked 行：`Re-pair`（打开新的配对邀请，并记住目标 `client_id` 用于提示）+ `Delete`（二次确认；仅 revoked 行可见）。
- [ ] active 行：只有 `Revoke`；不提供 `Delete`。
- [ ] live/pending 卡命中 `would_restore`：标题“此设备已被撤销”，正文说明“恢复后不会生成新密钥”，按钮“恢复设备”；普通设备仍显示“接受设备”。
- [ ] 配对失败态（consumed / expired）：`Refresh QR code` 置前；同屏保留“使用配对码”入口，无需先刷新。
- [ ] 配对成功：设备列表更新，并显示一行“{name} 已配对。”
- [ ] 文案按 `spec.md` 的 en/zh 定稿录入两个 locale。
- [ ] `svelte-check` 与生产构建 PASS。

## 事实来源

- `gui/src/views/PairingDialog.svelte:53-239`（现有对话框与设备列表）
- `gui/src-tauri/src/controller.rs:526-598`（begin/cancel/approve/decline/revoke）
- `gui/src-tauri/src/controller.rs:471-520`（失败态只剩刷新）
- `gui/src/lib/store.svelte.ts:334-452`（poll 与 clients 刷新）

## Comments

- Merge: `d28e099` (rows `3e6f050`, `cba8a91`, `f64294c`, `d28e099`).
- Closeout snapshot (`d28e099`): **PASS**.
- GUI: `npm ci && npm run check && npm run build` → `svelte-check` 101 files, 0 errors,
  0 warnings; Vite production build 140 modules, built in 642 ms.
- GUI Rust crate: `cargo test -p audio-share-server-gui` → 114 passed, 0 failed,
  1 ignored (the ignored case needs GitHub network access).
- Limitation: Svelte/store behaviour has no test runner; it is covered by `svelte-check`
  plus the production build only. Real device: not applicable / `NOT RUN`.
- Details: `.scratch/pairing-ux/evidence/README.md`.
