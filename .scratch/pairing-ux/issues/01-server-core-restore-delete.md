# 01: Server Core 的显式 Restore 与设备记录 Delete

**What to build:** approve 前识别“同身份记录为 revoked”，把待确认状态暴露给本地管理面；确认后原 `client_id` 原地恢复为 active，拒绝则保持 revoked 且不新增记录。新增“删除 revoked 记录”的管理操作，并让 `as-cmd clients delete <id>` 走同一 Core。

**Blocked by:** 无。

**Status:** ready-for-agent

## 验收

- [ ] 配对完成路径按 `public_key` 命中既有 revoked 记录时，不直接 `upsert` 覆盖；`PendingRequestView` / `PresentedRequestView` 暴露 `would_restore`（含既有 `display_name`）。
- [ ] 恢复确认后：原记录 `state=active`、`client_id` 不变、`created_at` 保留；列表中只有这一条。
- [ ] 拒绝恢复：记录保持 revoked，不写 `PairingResult(OK)`，不广播 paired。
- [ ] 无既有记录的新配对行为不变；active 记录再次配对的行为不变。
- [ ] `PairingAdmin` 增加 `delete(client_id)`：仅允许 revoked；active 返回错误并保持原状；删除后列表与 `clients.json` 都不再有该记录。
- [ ] `as-cmd clients delete <client_id>` 使用同一 Core 状态；删除 active 设备时提示先撤销。
- [ ] Rust host 测试覆盖：revoked→恢复、revoked→拒绝、active 不得删除、删除后认证被拒。

## 事实来源

- `crates/v2-transport/src/pairing.rs:1059-1096`（`persist_client` 现在的覆盖行为）
- `crates/v2-transport/src/auth_store.rs:528-551`（`upsert` 按公钥替换）
- `crates/v2-runtime/src/admin.rs:118-146`（`PairingAdmin` trait）
- `crates/as-cmd/src/main.rs:179-189`（现有 `clients` 只有 list/revoke）
- `docs/wire-protocol-v2/pairing.md:310-316`（§8.3 替换必须是显式管理员动作）

## Comments

- Merge: `34f3857` (implementation `64e6d97`).
- Closeout snapshot (`d28e099`): **PASS**.
- Rust host: `cargo test -p v2-transport -p v2-runtime -p as-cmd` → 355 passed, 0 failed,
  1 ignored (the ignored case needs real PipeWire). `pairing_live` 8/8, no hang.
- Emulator (`emulator:audio-share-api35`, API 35, x86_64, `EMULATOR`): a revoked identity
  was refused with typed 204; SAS re-pairing of the same identity required the explicit
  Restore prompt and restored the same `client_id` and `created_at` with one active
  record. Real device `NOT RUN`.
- `as-cmd clients delete`: manual smoke PASS — active refused with "revoke it before
  deleting", then revoke → delete → `no paired clients`. Store/admin tests cover the same
  paths; there is no dedicated CLI test for `delete`.
- Details: `.scratch/pairing-ux/evidence/README.md`.
