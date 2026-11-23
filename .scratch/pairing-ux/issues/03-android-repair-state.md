# 03: Android 的 needs-repairing 状态、Forget 与自动连接闸门

**What to build:** 在凭据上持久化“需要重新配对”，Home/Settings 显示；提供 `Forget this Server`（凭据 + 身份密钥 + 匹配地址）；`AutoConnectPolicy` 据此不再自动连接；修正首页与 Settings 的主语文案。

**Blocked by:** 无（可与 01 并行）。

**Status:** resolved

## 验收

- [x] `CredentialRecord` 增加 `needsRepairing`（默认 false）；`CredentialCodec` 兼容缺字段的旧记录。
- [x] Service 在 `Ended(Surfaced(code))` 且 `ErrorGuidance.requiresRepairing` 时置位；认证成功或配对成功时清除并持久化。
- [x] `AutoConnectPolicy`：`needsRepairing=true` 时 `shouldAutoConnect` 返回 false；进程重启后仍生效（不依赖内存 outcome）。
- [x] `CredentialStore.delete` 有真实调用方；Forget 同时删除 Keystore alias 或软件密钥文件，失败不掩盖凭据删除结果。
- [x] Home 的 Server 卡片显示 `Needs re-pairing`；Settings 显示同一状态。
- [x] `Forget this Server`：二次确认；删除凭据与密钥；若地址字段与该记录一致则清空；停止该会话；回到未配对 Home。
- [x] 文案：Home 已配对次要动作 `Re-pair`（移除 `Pair a new phone` 与独立 `Scan QR instead`）；Settings 空态 `Pair a Server`。
- [x] JVM 测试覆盖：置位/清除与持久化、冷启动闸门、Forget 删除凭据与密钥、旧记录解码。

## 事实来源

- `android-client-v2/app/src/main/java/dev/audioshare/v2/CredentialStore.kt:12-33,99-133`（记录与 `delete`，当前无调用方）
- `android-client-v2/app/src/main/java/dev/audioshare/v2/AppState.kt:203-219`（`AutoConnectPolicy` 把 null outcome 当可连接）
- `android-client-v2/app/src/main/java/dev/audioshare/v2/AudioShareApp.kt:244-268,972-1040`（Home/Settings 文案与只读设置）
- `android-client-v2/app/src/main/res/values/strings.xml:24,103`（`Pair a new phone` / `Pair a phone`）

## Comments

- Merge: `afa4942` (commits `4e48ad8` … `0c7cd69`).
- Closeout snapshot (`d28e099`): **PASS**.
- JVM: `./gradlew testReleaseUnitTest -PnoNative=true` (JDK 21) → 325 tests, 0 failures,
  0 errors, 0 skipped; `BUILD SUCCESSFUL in 25s`.
- Emulator (`emulator:audio-share-api35`, API 35, x86_64, `EMULATOR`): a revoked stored
  credential surfaces the typed 204 guidance; real device `NOT RUN`.
- Known gaps: `pairingIntent` is not saved by `UiStatePersistence`, and SESSION's
  "Connect again" silently no-ops for a needs-repairing record. See the evidence README.
- Details: `.scratch/pairing-ux/evidence/README.md`.
- Review fixes merged at `6242ec7`: the re-pair intent persists across rotation, Forget
  stops only the owning session, and the repair-state rule plus address identity are
  deduplicated; Android JVM 332 tests PASS. Status: resolved.
