# 09: Phase-3 cross-validation and acceptance — Rust ↔ Android authenticated control channel

**What to build:** 第 3 阶段的交叉验收：真机 Android Client ↔ 生产 Rust Server 完成「连接 → 认证 → 协商 → READY」的完整控制通道，正/负/限额/撤销全部有设备证据；v1 回归确认。交付 phase-3 报告。

**Blocked by:** 06, 07, 08

**Status:** resolved

**Type:** task

**Environment note:** 设备与通路已验证可用（见 memory `prototype-device-validation-setup.md` 与 `prototype/wire-protocol-v2-transport/evidence/device-validation.log`）：MIUI 真机 adb over tailscale、USB 安装需用户侧开关、uiautomator 不可用（用 screencap + input）、`--ez` 传布尔 extra。

## Requirements

- E2E 正向：真机连接生产 Server（65531）→ AuthInit/Challenge/Proof/Result → Negotiate/NegotiateResponse → READY 稳态 → 用户主动停止（Close 正常码）；全程 Server/Client 日志 + 状态截图入 `evidence/`（沿用带命令头的记录惯例）。
- 负向（每项都有捕获结果 + 日志引用）：
  - 错 SPKI pin（握手层拒绝，无应用数据）；
  - 错签名 auth_context / 错公钥身份（202 AUTHORIZATION_UNKNOWN）；
  - 撤销授权：连接中撤销 → 活跃会话 ≤5 s 终止 + 新连接 204；
  - 状态违规：READY 前杂散帧 → 可判定 401/402（按 §5.2）；
  - malformed：坏长度/未知 oneof case → 401 + Close；
  - 限流：连续失败触发 TM-5 上限的可判定行为。
- 空闲与 keep-alive：READY 稳态静置 > 60 s（D3：30 s timeout + 10 s keep-alive）连接不假死；锁屏/亮屏（前台 Service）会话存活；飞行模式/断网恢复 → 有界退避重连成功。
- 资源基线：设备端 CPU/RSS + Server 端会话计数（对照 prototype 的桌面基线）。
- **v1 回归（红线）**：`protocol-harness replay` 全绿；v1 集成测试全绿；v0.3.4 兼容线（`android-app/`）构建不受影响；服务器同时跑 v1+v2 互不干扰（v1 客户端 fixture 连 v1 端口、v2 客户端连 v2 端口并发）。
- Rust↔Android「对拍 fixtures 而非互为编码器」的证据在两侧各留一份（复用 06/08 的回放工装在真实构建上重跑）。
- 报告：`.scratch/wire-protocol-v2/phase-3-report.md` —— 逐项 PASS/FAIL/NOT RUN、版本表、证据链接、遗留事项；报告不宣称媒体/统计能力（那是第 4 步）。

## Definition of done

- 上述每项有 PASS/FAIL/NOT RUN + 证据；无 FAIL（有 FAIL 按裁决修或升格工单）。
- phase-3 报告落档；本 ticket resolved 即第 3 阶段（Tracer-Bullet 第 3 步）完成，第 4 步（单 Client PCM 媒体 + 统计 + 显式关闭）解锁。

## Comments

**2026-11-12 — phase-3 device cross-validation executed; Status stays `claimed` due to one FAIL (live revocation).**

- Environment: `ticket/09-phase3-cross-validation` @ `9c706dd`; server `as-cmd --v2-port 65531` from this worktree; APK rebuilt from this worktree (sha256 `59b4b97a9e5d51ec3062569de19cd99e88e5cdf22c6136ebbadb461749b64bf6`), Xiaomi M2007J22C / Android 12 / API 31 / arm64, adb over tailscale. Full report: [`phase-3-report.md`](../phase-3-report.md); evidence: [`evidence/ticket-09/`](../evidence/ticket-09/).
- Positive E2E PASS with the documented temporary bypass for the thread-local auth-context defect (`android-client-v2/native/src/session.rs:89-95`, 2 → 1 runtime workers; real fix owned by `review/wire-protocol-v2-fixes`). READY reached; idle >80 s PASS; lock/unlock PASS; network loss → bounded backoff → READY PASS; clean Close 701 captured on the server via Activity recreation (UI has no stop button; BACK only backgrounds under MIUI).
- Negative: wrong SPKI pin PASS (handshake rejection, no app data); unknown identity PASS (device rejection; exact 202 proven by `state_machine.rs:213` + replay fixture `control/error-202-authorization-unknown.json` because `driver.rs:314-317` logs a hardcoded `code=203`); TM-5 per-source 20/60 s PASS on device (first 20 failures then 302/budget exhausted), per-connection 5-failure PASS on host live-QUIC; stray-frame 402 / malformed 401 PASS on host live-QUIC (NOT RUN on device — no injection surface).
- **FAIL — live revocation ≤5 s:** READY session survived >12 s after removing the client record from `pairing/clients.json`; server never terminated it. Root cause: `AuthorizationStore` loads the file once, no reload or runtime revoke surface in `as-cmd`, and the driver's 2 s revocation poll only acts on `Revoked`, not removed/`Unknown` identities (`state_machine.rs:712`). New-connection-after-revocation 204 PASS via server restart (exact 204 covered by `session_driver.rs:454`), but the live-teardown requirement is unmet → `pairing.md` §9.2 violation.
- v1 regression PASS: 6/6 `protocol-harness replay` suites; `cargo test --workspace` 435 passed / 0 failed / 3 ignored; live v1 replay on 65530 while v2 READY on 65531; `android-app/` zero diff and `assembleDebug` BUILD SUCCESSFUL with a temporary local debug-signing `keystore.properties` (removed afterwards). Rust replay 22 tests + Android native 83 tests PASS.
- Adjudication needed: implement store reload/revoke handling (reload on revocation poll; treat now-unknown authenticated identity as revocation; and/or production revoke surface) and re-run the ≤5 s + 204 device checks. This is not among the six known review-branch defects, so it needs an owner; ticket cannot be resolved until it passes. Secondary: server auth-failure log should print the real registry code instead of hardcoded 203.

**2026-11-13 — post-fix rerun: all items PASS; Status → `resolved`.**

- Integration `8c219b0` (review fix branch) merged into the ticket branch; the temporary `worker_threads(1)` bypass was reverted by that merge and is absent from the final APK. Final APK sha256 `7f15a2fc8332c493c85ea21fc20474326b6784ecd1e26b1e6a1b8accf2aad6d6` (2,478,374 bytes); `as-cmd` sha256 `0fd79b00ca8cc53d8f08ade3ec6ebd7dd7655d56df35c043847a5d0dec6e542d`. Multi-worker E2E reaches READY with no bypass (`09-post-fix/ready-multithread.png`).
- Live revocation PASS: record deleted while READY; server `Close(204)` 0.800 s later (deleted `15:51:38.694Z`, close `15:51:39.494`); UI `CLOSED`, `close_code=204`, no reconnect. New connection with `state:"revoked"` PASS: server logs real `code=204`, UI `close_code=204`, no reconnect. Unknown identity PASS: server real `code=202`, UI `close_code=202`, no reconnect. A14 real-code logging confirmed for 202/204.
- One remaining client bug found during the rerun and fixed on this branch in `7a42315 fix(v2-client): surface remote Close code to reconnect policy`: remote `CloseInfo` was not stored, so Kotlin saw `close_code=null` and reconnected once after 204. Added `record_close` + `remote_close_code_is_surfaced`; native tests 97 passed.
- Regression/idle/network PASS post-fix: READY idle 64 s with six v1 fixture replays running; Wi-Fi off 15 s → reconnects at 1037/2183/3471/7954 ms → READY (old session 303); lock/unlock 20 s session 40879 survived; clean stop Close 701 at `16:01:25.419`; resource baseline 0% idle CPU / VmRSS 154,892 kB / 37 threads, server RSS 9,960 kB.
- v1 PASS post-fix: 6/6 `protocol-harness replay` exit 0; `cargo test --workspace` 445 passed / 0 failed / 4 ignored / 39 suites; live v1 replay on 65530 while v2 READY; `android-app/` zero diff. Host live-QUIC 9/9 incl. `deleting_the_record_ends_the_live_session_and_rejects_reconnects`; state machine 30/30; Rust replay 22; Android native 97.
- Device-injection-impossible items remain NOT RUN on device with host coverage: stray frame 402, malformed 401, wrong-signature 203, per-connection 5-failure limit. Leftover: fixture server PKCS#8 key is v1 and ring rejects it (fresh server identity used); `7a42315` must be carried into integration if this branch is not merged whole. Full result table and evidence links: [`phase-3-report.md`](../phase-3-report.md); post-fix evidence: [`evidence/ticket-09/09-post-fix/`](../evidence/ticket-09/09-post-fix/).

