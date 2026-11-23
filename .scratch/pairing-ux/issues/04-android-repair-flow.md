# 04: Android 专用重配入口与配对失败阻断态

**What to build:** 重配复用 PAIRING 屏并携带 `PairingIntent`，保留失败原因与“先让服务端开始配对”的提示；SAS 在地址命中已存记录时复用其 `keyAlias`；配对成功清理同地址旧记录；201/206 改为阻断等待态。

**Blocked by:** 01, 03。

**Status:** ready-for-agent

## 验收

- [ ] `AppState` 增加 `PairingIntent`（`First` | `RePair(serverPin)`）；PAIRING 屏据此渲染重配提示，不新增第六个屏。
- [ ] 204 错误卡与 `Needs re-pairing` 卡片的入口都进入 `RePair` 上下文；重配完成前 Home 的 `Connect` 禁用。
- [ ] 重配提示：“Ask the Server operator to start pairing. Then scan the new QR code or use the pairing code.”；地址从记录预填；QR 为主、SAS 次之。
- [ ] SAS 重配：输入地址命中已存记录时复用该记录的 `keyAlias`；未命中时按现有逻辑生成新身份。
- [ ] 配对成功：删除同 `host:port` 的其他记录；清除 `needsRepairing`；进入 SESSION。
- [ ] 201/206 阻断态：显示“邀请已失效，请服务端刷新后再扫新码”，按钮 `Scan new code`；不再显示裸 `Scan again`。
- [ ] JVM/UI 测试覆盖：RePair 上下文、SAS 密钥复用、同地址清理、失败态文案与按钮。

## 事实来源

- `android-client-v2/app/src/main/java/dev/audioshare/v2/AppState.kt:616-631`（`Retry` 直接落入通用 PAIRING）
- `android-client-v2/app/src/main/java/dev/audioshare/v2/AudioShareApp.kt:651-693`（`Scan again` 主按钮）
- `android-client-v2/app/src/main/java/dev/audioshare/v2/ControlSessionService.kt:243,271-275`（QR/SAS 别名策略）
- `android-client-v2/app/src/main/java/dev/audioshare/v2/AppState.kt:131-135`（`selectedPin` 按地址优先、旧记录在前）

## Comments

- Merge/tip: `fe38989` (with `b57d1c9`).
- Closeout snapshot (`d28e099`): **PASS** for JVM and build evidence.
- JVM: `./gradlew testReleaseUnitTest -PnoNative=true` (JDK 21) → 325 tests, 0 failures,
  0 errors, 0 skipped; `BUILD SUCCESSFUL in 25s`.
- Emulator (`emulator:audio-share-api35`, API 35, x86_64, `EMULATOR`): a SAS re-pair after
  revocation reused the stored identity (same `client_id`), triggered the Server's
  explicit Restore prompt, and restored the same record (`created_at` preserved, one
  active row); the stored credential then authenticated (`RELOAD_OK`). Real device
  `NOT RUN`.
- The legacy device contract's `pairSas` case still fails its `awaitPairingPhase("COMPLETE")`
  assertion because the session handoff clears the pairing snapshot. The clearing path is
  unchanged between `v0.6.0` and `d28e099` (diff over `SessionOrchestrator.kt` /
  `Pairing.kt`), but it was not re-run against a `v0.6.0` build; pairing and Restore were
  verified independently. `pairQr`, the manual Re-pair/Restore/Forget UI checklist, and
  the real device are `NOT RUN`.
- Details: `.scratch/pairing-ux/evidence/README.md`.
