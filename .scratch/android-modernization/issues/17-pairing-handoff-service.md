# 17: 配对成功交接归 Service

**What to build:** 配对成功后由 Service 持久化凭据、启动新的认证 Client Session，并向 UI 推送一次完成事件；UI 只导航和更新 Server 列表，不再轮询凭据。

**Blocked by:** 15。

**Status:** resolved

- [x] 凭据持久化成功后，Service 启动新的认证会话，并异步派发一次性 `PairingCompleted`；不在 pairing watcher 回调内同步 stop/teardown。
- [x] UI 收到后进入 SESSION 并更新 Server 列表；删除 100 ms / 5 秒凭据轮询与 `CredentialStored` 触发 `ConnectStored`。
- [x] 配对交换不进入 session outcome store；只有交接后的会话发布 outcome。
- [x] 持久化失败保持 COMPLETE、记录日志、不自动连接。
- [x] QR 一步配对（恰好一次自动确认）与 SAS 显式确认行为不变。
- [x] JVM 测试覆盖交接成功/失败、只推送一次、配对期间无 outcome、恢复规则。

## Comments

2026-11-17 — 完成并合入集成分支（merge `c79e720`，feature `f65d7be`，已同步 ticket 16 tip）。
`setPairingCompletedListener` 为一次性、不重放；持久化成功后经主线程异步启动新认证会话并推送
`PairingCompleted(record)`；`MainActivity` 的 100 ms/5 s 轮询已删除；`CredentialStored` 只更新列表。
无凭据/密钥缺失的 stored-connect 失败改为 `Ended(Surfaced(205))`。
JVM 177 tests/variant PASS，androidTest Java 编译 PASS。证据：`evidence/ticket-17/`。
