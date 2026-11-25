# 19: Production pairing end-to-end (Pairing Core + CLI)

**What to build:** A real Protocol Client can pair with the Server using QR or SAS; the Server persists the authorization; the operator can list, revoke, and reset from `as-cmd`. This is the server-side tracer bullet for pairing: it cuts through the pairing state machine, authorization store, identity, CLI, and cross-end tests. Production code must not depend on the test-only insertion path.

**Blocked by:** None (can start immediately)

**Status:** resolved

- [ ] `PairingRequest` admission follows `pairing.md`: QR and SAS modes, single-use short-lived nonce, transcript binding, no-detail pre-authentication errors, and pairing-failure rate limits.
- [ ] A completed pairing persists an authorized client with canonical `client_id` and a 33-byte SEC1-compressed P-256 public key; directory 0700, file 0600, atomic write with one backup generation.
- [ ] `as-cmd` exposes begin-pairing (QR output where practical, SAS/fingerprint always), `clients list`, `clients revoke`, and `identity reset` with a confirmation step.
- [ ] Identity reset invalidates all old pairings; revocation affects new connections immediately and terminates an affected active session promptly.
- [ ] No pairing secret, private key, or raw audio appears in logs or error payloads.
- [ ] Host golden-vector and live-QUIC tests cover happy path, replay, expiry, malformed frames, unknown identity, and revoked identity.
- [ ] The production binary has no test-only authorization path; fixture material remains behind the explicit test-support flag.
- [ ] v1 framing, commands, UDP behavior, fixtures, and the frozen v0.3.4 APK remain unchanged.
- [ ] `as-cmd --help` states that v1 is unencrypted and trusted-network only.

## Answer

已合入集成分支 `integration/wire-protocol-v2-beta`（merge commit `9e5f916`，2026-11-16）。实现提交：`e9b63fd`（Pairing Core/状态机）、`115008f`（as-cmd 管理命令）、`b27a579`（建立会话时清理 pending pairing）。

- **准入与协议**：QR nonce/SAS window、16B CSPRNG nonce（10 分钟、单次、最多 8 个、重启失效）、TLV 1–9 binding + SHA-256（与两条 golden vector 一致）、SAS `900067`、配对失败 5 次/60s/地址、10 分钟 206、无 detail 的 201/206/302/401、`PairingConfirm` 校验 binding 与 proof 后才落盘。
- **本地批准**：已验证待批准状态由 driver 注入 approve/decline；批准后持久化并进入 Negotiating，拒绝则 201 + Close 且不落盘。
- **持久化**：只用 `AuthorizationStore::upsert`；canonical `client_id`、33B SEC1、0700/0600、原子写 + `.bak`、`created_at`/`last_seen_at`；旧 version-1 记录可加载。
- **管理面**：`as-cmd pair [--sas] [--accept|--yes] [--addr] [--timeout]`、`clients list|revoke`、`identity reset [--yes]`；离线管理使用 `clients.json.lock` 排他锁（`std::fs::File::lock`）。
- **身份重置/撤销**：重置先清空授权再轮换身份（崩溃时 fail-closed）；撤销对新连接立即生效，live 测试中活动会话在 100 ms 轮询内以 204 终止。
- **日志**：配对模块不记录 secret；driver 只记录错误码与脱敏地址；Debug 不暴露 nonce/SAS/binding/proof/私钥。
- **回归**：`cargo fmt --check`、`cargo clippy --workspace --all-targets -D warnings`、`cargo test --workspace`（564 passed / 0 failed / 4 ignored）、interop（12 passed）、Android JVM（51 passed）全部 PASS。
- **证据**：[`evidence/ticket-19/`](../evidence/ticket-19/README.md)。
- **偏差**：`as-cmd pair` 为前台模式并占用 v2 UDP 端口，运行中的 Server 需先停止；SAS 由连接派生，故在配对请求到达/批准时显示。已在 `--help` 记录。
- **NOT RUN**：Windows 锁/CLI 行为；Android 模拟器/真机配对与 Android↔Rust 配对互操作（工单 20/24）；真机验收（工单 25）。
