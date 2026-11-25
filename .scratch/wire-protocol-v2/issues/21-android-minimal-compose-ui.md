# 21: Minimal Android Compose UI

**What to build:** A pilot customer can enter the Server address, pair by scanning a QR code or entering an SAS code / comparing a fingerprint, see session states, play audio, stop, and read the beta limitations — without adb or a developer harness.

**Blocked by:** 20

**Status:** resolved

- [ ] Home shows Server address entry, pairing/connect entry, current state, and clear error and retry guidance.
- [ ] Pairing offers QR scanning, SAS entry, fingerprint display, and cancel.
- [ ] Playing shows state and a stop action.
- [ ] About shows the beta version, known limitations, the v1 unencrypted warning, and manual update instructions.
- [ ] The last Server address is persisted and the app auto-connects when a pairing exists; it does not start on boot.
- [ ] The foreground media notification identifies the session and offers stop; playback continues while the screen is locked.
- [ ] The UI consumes only the session/pairing interface; it does not call JNI, QUIC, or Keystore directly.
- [ ] UI state-reducer unit tests and key Compose screen tests pass; the emulator manual path (pair, play, stop) passes.
- [ ] `applicationId` is `dev.audioshare.v2`, `versionName` is `0.2.0-beta`, and `versionCode` is 2, so the app coexists with v1.

## Answer

已合入集成分支 `integration/wire-protocol-v2-beta`（merge commit `d01f3e3`，2026-11-16）。实现提交：`7b885fe`、`6ec57e9`、`6823f42`。

- **UI**：`MainActivity : ComponentActivity`；Home（地址输入/校验、上次地址持久化、配对/连接、状态映射、错误与重试指引）、Pairing（CameraX+ZXing 扫码、QR URI 粘贴回退、SAS `123 456` 输入比对、fingerprint、确认/取消）、Playing（状态+Stop）、About（beta 版本、已知限制、v1 明文警告、手动更新/SHA-256、无 telemetry/无原始音频）。UI 只依赖 `SessionController`/Binder，不直接调用 JNI/QUIC/Keystore（契约测试断言）。
- **自动连接**：存在已配对凭据时启动自动连接；无 boot receiver。前台通知标识媒体会话并提供 Stop；锁屏继续 `PLAYING`。
- **版本**：`dev.audioshare.v2`、`versionCode 2`、`versionName 0.2.0-beta`。
- **测试**：JVM 107（baseline 77 + 30，含 reducer、SAS/地址、命令执行、Robolectric Compose 屏幕测试）；默认 arm64 `assembleRelease` 成功，dex 中无 test-support/fixture/adb-extras 路径。
- **EMULATOR（API 35 / x86_64）**：真实 `as-cmd pair --sas` 完成配对与凭据持久化、SAS/fingerprint 一致；force-stop 重启自动连接并 `PLAYING`；锁屏 4 s 前台 Service/media session 存活；通知 Stop → `CLOSED`、Server 701；`service-device-test.py` 的 ready/rebind/recreate/lock/stop 5/5。API 26 的 5 个生命周期用例代表性通过。
- **证据**：[`evidence/ticket-21/`](../evidence/ticket-21/README.md)。
- **偏差/NOT RUN**：相机实扫 QR 未演练（SAS 与 QR URI 粘贴路径已验）；`POST_NOTIFICATIONS` 运行时授权对话框未演练；真机、Windows、StrongBox/TEE、可听音质/时延/underrun 均 NOT RUN。
- **附带发现**：一次 202 `AUTHORIZATION_UNKNOWN` 漂移（`clients.json` 仍为 active，重启恢复）已分诊为工单 27 的原子写缺失窗口竞态并修复，未归因于 UI。
