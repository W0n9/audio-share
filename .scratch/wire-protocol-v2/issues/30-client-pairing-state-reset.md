# 30: Client pairing state reset and stale banners after revoke/reject

**What to build:** A new pairing attempt must start from a clean pairing/session state.
Today, after a revocation or a rejected pairing, tapping "Pair with code" reaches
`AUTHENTICATING` / `PairingReady`, but the SAS confirmation UI does not appear and the old
"This device was revoked" / "Pairing was rejected" banner remains. Clearing app data makes
the same flow work. The likely cause is that `PairingStateRelay` has no source ownership:
a stopped/old `ClientSession` can still publish a stale `PairingSnapshot` after the new
exchange has begun, overwriting the new `AWAITING_CONFIRMATION` state. Also clear the stale
"Connection lost" card after an explicit user Stop.

**Blocked by:** None

**Status:** resolved

**Evidence:** `evidence/ticket-25/ticket-25-result.md` (conditional GO; blocker reproduced
twice on Windows 11 + Xiaomi M2007J22C, Android 12/API 31). Non-blocking observations:
QR first-open countdown missing; first QR pairing ended by client 701 after 12 s; server.log
has no pairing/revoke/701 events.

- [x] Add pairing-source ownership to `PairingStateRelay` so only the current exchange can
      publish; a stopped session's late snapshot is rejected. Add a JVM test that fails on
      the old relay.
- [x] `startSession(pairing = true)` begins a fresh pairing source and clears the previous
      snapshot/error; revoke → re-pair shows the SAS confirmation UI.
- [x] Explicit user Stop clears the stale "Connection lost" error/banner.
- [x] Investigate the QR first-open countdown and the 12 s client `701 CLIENT_STOPPING`;
      fix if they share the stale-session race, otherwise record a separate finding.
- [x] Instrumentation/emulator coverage for revoke → re-pair and Stop banner; real-device
      rerun left to ticket 25.
- [x] Rebuild the signed APK and record the new hash; version stays `0.2.0-beta`
      (`versionCode 2`) unless the maintainer decides otherwise.
- [x] No accepted protocol, ADR, v1, fixtures, or frozen APK changes.

## Answer

已合入集成分支 `integration/wire-protocol-v2-beta`（merge commit `33276e8`，2026-11-17）。实现提交 `59e98e6`、`015bc64`。

- **根因**：`PairingStateRelay` 无 source ownership；停止后的旧 `ClientSession` 仍可能发布迟到的 `PairingSnapshot`，覆盖新交换的 `AWAITING_CONFIRMATION`。另外 `ClientSession` 的 pairing watcher 在 stop 后仍可能投递已读快照，且 Service 在 teardown 后才清 `lastErrorCode`，CLOSED 重放会把旧 204 横幅带回。
- **修复**：`PairingStateRelay.beginSession()/Source` 所有权，旧 source 的事件被忽略；watcher 以 `ClientSession` lifetime + `running` 为门；`startSession` 每次新建 pairing source 并清旧快照；teardown 前清除 `lastErrorCode`，READY/PLAYING 也清除；显式 Stop 清除 Home 的陈旧错误。
- **测试**：JVM **123 tests / 0 failed**（含新增 relay 所有权、reducer 陈旧横幅用例）；x86_64 release+androidTest 构建 PASS；默认 arm64 构建 PASS；`cargo fmt --check` PASS。
- **EMULATOR（API 35 / x86_64）**：撤销 204 → “Pair with code” 出现 SAS/fingerprint、旧横幅消失；通知 Stop 后 Home `CLOSED` 且无 “Connection lost”；601 后台重连成功后横幅消失。
- **签名 APK**：`c1d1b23634abb05bf760bcb975bd442e65abc674610dfac11140f6dd7992af58`（`0.2.0-beta`、versionCode 2、arm64+x86_64、v2+v3 签名、无 test-support）；旧 APK `6da98e4a…845141` 保留为 `.prev`。
- **非阻塞记录**：QR 首次倒计时属 Server GUI，未在本工单复现；12 s 701 未在模拟器复现，疑似 Activity 销毁触发 Stop 或旧 teardown 竞态，留给真机复跑；server.log 缺 701/配对/撤销事件另行跟进。
- **NOT RUN**：真机复跑（留给工单 25）。
