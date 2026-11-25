# 20: Android native pairing and credential storage

**What to build:** The Android Client pairs via a QR payload or SAS input, stores the paired credential in Android Keystore, reloads it across process restarts, and reaches READY→PLAYING using only that credential — no adb-injected pin or key. Revoked and unknown identities are rejected with the specified codes.

**Blocked by:** 19

**Status:** resolved

- [ ] Pairing commands and pairing-state events are exposed through the existing Android session/pairing interface.
- [ ] Both the QR payload and SAS input complete pairing against a real Server.
- [ ] The paired credential and Server pin persist; private key material uses Android Keystore where available and is never stored in plaintext app storage.
- [ ] A paired connection reaches READY→PLAYING without adb extras; revoked and unknown identities fail with the specified codes.
- [ ] Reconnect and bounded backoff use the stored credential; a v2 failure never falls back to v1 automatically.
- [ ] Native contract tests, cross-end live-QUIC interop, and emulator instrumentation pass.
- [ ] Production builds contain no test identity or test-only addressing path; test-support remains behind the explicit build flag.

## Answer

已合入集成分支 `integration/wire-protocol-v2-beta`（merge commit `65be014`，2026-11-16）。实现提交：`deb789b`（native 配对状态机/signer seam/跨端）、`1bfd873`（Kotlin 配对接口/Keystore/凭据）、`12f3e9e`（结果捕获与生产边界）、`ae0012e`（reload 用例显式 pin）。

- **配对接口**：`SessionController` 暴露 `beginQrPairing(uri)`、`beginSasPairing(address)`、`confirmPairing()`、`cancelPairing()`、`pairingSnapshot()`/监听、`credentials()`、`connectStored()`、`disconnect()`、`lastErrorGuidance()`；Service 重绑可重放 `COMPLETE` 配对状态。
- **Native 配对**：QR/SAS `PairingRequest`、TLV 1–9 binding、TLS-Exporter 校验、SAS 派生/格式化、`PairingConfirm` proof、`PairingResult` 后续 Negotiate；QR 解析拒绝未知/重复字段与坏编码。
- **Keystore**：P-256 私钥不可导出，native 通过 JNI upcall 调 Kotlin `ClientSigner`（AndroidKeyStore secp256r1、SHA256withECDSA、DER→P1363；StrongBox 可用则用，否则普通 Keystore），tokio worker 正确 attach；仅在 Keystore 不可用时软件回退并标记 `software_backed`。
- **凭据**：仅成功后持久化；重载后自动连接并达到 READY→PLAYING；204/202/205 分别映射为撤销/未知身份/pin 不匹配；v2 失败不回退 v1。
- **生产边界**：adb-extras harness 移入 `src/testSupport`，默认 arm64 release 的 dex 中无 fixture scalar、`pinnedSpki`、`serverKey`、`smoke-client` 或 harness 路径。
- **测试**：native 154（两种 feature 配置）、interop 18、JVM 77、as-cmd v2_pairing 4、v2-runtime 9；fmt/clippy 干净。
- **post-merge EMULATOR（API 35, x86_64）**：`keystore`、`pairSas`、`pairQr`、`reload`（SAS+QR）、`revoked`(204)、`unknown`(202)、`identityChange`(205)、`harness` 全部 PASS；证据见 [`evidence/ticket-20/`](../evidence/ticket-20/README.md)。
- **NOT RUN**：真机、Windows、StrongBox/TEE 硬件后端（模拟器 `hardware_backed=false`）、30 分钟播放/锁屏/断网、Windows 与真机配对。
