# 23: M3 lifecycle test fixes (lock and recreate)

**What to build:** The `lock` and `recreate` instrumentation cases reflect M4+ behavior: the lock case accepts a live `PLAYING` session, and the recreate case no longer races a duplicate `ACTION_START` into `AUTHENTICATING`. Both pass repeatedly on the emulator and are ready to be re-run on the real device during beta acceptance.

**Blocked by:** None (can start immediately)

**Status:** resolved

- [ ] The `lock` case accepts `READY` or `PLAYING` and separately asserts that the foreground media Service and the session remain alive across sleep and wake.
- [ ] The `recreate` case's duplicate `ACTION_START` race is investigated; it is either fixed or documented with a stable regression test.
- [ ] Both cases pass repeatedly on the emulator, including the API 26 image where applicable.
- [ ] No production behavior changes beyond the race fix; v1 and the frozen APK remain untouched.
- [ ] The real-device rerun is deferred to ticket 25 and recorded there.

## Answer

已合入集成分支 `integration/wire-protocol-v2-beta`（merge commit `fe8da1c`，2026-11-16）。

- **lock**：锁屏期间与唤醒后接受 `READY` 或 `PLAYING`，并分别断言前台媒体 Service 存活、Service 进程 PID 不变、同一 Service/session 未重建、状态序列中没有 `IDLE`/`AUTHENTICATING`/`CLOSING`/`CLOSED`。`ready` 用例保持严格 `READY` 口径，未放宽。
- **recreate**：logcat 证据否定了“重复 `ACTION_START`”推断。真实竞态是 `onAvailable` 在 QUIC endpoint 写入前触发 rebind，失败后 teardown 并以 code 0 重连，重建恰好落在新会话的 `AUTHENTICATING` 窗口。修复只改 `ClientSession.onAvailable`：`handle == 0` 不再额外 `scheduleReconnect`；endpoint 未建立时跳过 rebind；已建立时保留原迁移路径。新增 `SessionStartupPolicyTest`（3 个 JVM 用例）。
- **EMULATOR 结果**：API 35 上 `ready` 15/15、`rebind` 3/3、`recreate` 3/3、`recreate-loop` 20 次连续重建 PASS、`lock` 3/3、`stop` 2/2；API 26 代表性各 1 次 PASS。JVM 51 tests / 0 failures；x86_64 release + androidTest 构建成功。
- **证据**：[`evidence/ticket-23/`](../evidence/ticket-23/README.md)，含修复前 RED 时间线与修复后日志。
- **NOT RUN**：真机 lock/recreate 复跑（留给工单 25）；可听音质、端到端时延、underrun、MIUI/厂商网络、真实网络切换、IPv6/动态 MTU；API 26 未做与 API 35 同等的重复矩阵。
