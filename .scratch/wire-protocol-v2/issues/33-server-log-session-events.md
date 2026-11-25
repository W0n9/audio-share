# 33: Server log lacks v2 session events (pairing, connect, close 701, revoke)

**What to build:** `server.log` currently contains only startup/stop lines. Add structured,
secret-free info-level events for the v2 lifecycle so support can verify pairing, session
establishment, `ClientClosed(701)`, revocation, and identity reset from the Server side:
pairing presented/accepted/declined, session accepted with `client_id` (public), session
closed with the §6 close code, client revoked, identity reset. Never log SAS, nonce,
binding, proofs, private keys, or raw audio.

**Blocked by:** None

**Status:** ready-for-human

**Evidence:** `evidence/ticket-30/evidence/notes.md` §8 — Windows acceptance could not
verify `ClientClosed(701)` from `server.log`; only client logs had it. Also noted in the
earlier ticket-25 run as a non-blocking observation.

- [x] Pairing presented/accepted/declined logged without SAS or secrets.
- [x] Session accepted and closed logged with `client_id` and close code.
- [x] Revoke and identity reset logged.
- [x] Redaction test: no SAS/nonce/binding/proof/private key in emitted lines.
- [ ] Windows host rerun of the 701 path after rebuilding Server artifacts. — **NOT RUN**

## Comments

2026-11-18 — 实现完成，证据见 `evidence/ticket-33/README.md`。分支
`ticket/33-server-log-events`，提交 `233c1a7` + `2695443`，以 `--no-ff` 合入
`integration/wire-protocol-v2-beta` @ `de83686`。未 push。

根因：GUI 进程从未安装 `tracing` subscriber，库里的事件全部被丢弃；Windows 验收用的正是
GUI。`as-cmd` 装了 subscriber，所以历史无头日志能看到 701，GUI 的 `server.log` 只有启动/停止行。

实现：
- 新模块 `v2_transport::session_log`：`pairing.presented/accepted/declined`、
  `session.accepted`、`session.closed`（含 §6 `code`）、`client.revoked`、`identity.reset`；
  只接收公开 `client_id`、mode、code、fingerprint。
- driver 在配对呈现、首次注册会话、统一 `terminate()` 处发事件；PairingCore 在
  approve/decline/revoke 与 identity reset 处发事件。
- GUI 新增 `tracing_bridge`，只转发 `v2_transport::session_log` 的 INFO 事件到
  `server.log`；安装失败会写 error。
- 离线 `as-cmd clients revoke` / `identity reset` 以 append 方式写 `server.log`
  （不调用会截断文件的 `init_logging`）。

验证：`cargo test -p v2-transport --test session_log` 4 PASS（RED 阶段 0/4）；
workspace 621 PASS / 4 ignored；GUI lib 112 PASS / 1 ignored；clippy 与 fmt 通过。
脱敏测试覆盖 SAS、nonce、binding、proof、私钥标量与二进制字节。

剩余人工门：在 Windows 上用重建后的 Server 跑一次 GUI 会话并 Stop，确认
`server.log` 出现 `event=session.closed … code=701`。完成前保持 `ready-for-human`。

- 2026-11-18 追加 `362d023`：脱敏断言改为检查本次随机私钥的标量（原断言用的是未参与该测试的
  fixture 标量，恒真）。重新跑 `session_log` 4 PASS、clippy 通过。
