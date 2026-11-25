# 35: Beta tag placement — ship before or after the Android modernization?

**Type:** grilling
**Status:** resolved
**Blocked by:** None
**Blocks:** 24

## Question to answer

`v0.5.0-beta` (annotated tag) points at `8858c91`. The integration branch
`integration/wire-protocol-v2-beta` is at `89f84b0`, **41 commits later**, including the
entire `android-modernization` refactor (`89f84b0 merge(refactor): android-client-v2 现代化`).
Every artifact and every acceptance record for ticket 24/25 was built from code at or before
`8858c91`.

Ticket 24 (formal GitHub Release) cannot close until this is decided: the tag, the built
APK/Server artifacts, the SHA-256 manifest, and the release notes must all describe the same
commit.

## Options

### A — ship the pre-refactor build (tag stays at `8858c91`)

- Existing APK `c1d1b236…2af58`, Server `0a7dd9ce…4ca7`, the post-fix artifact manifest, and
  all ticket 24/25 evidence remain valid; nothing is rebuilt.
- The released client is the pre-modernization UI/architecture (single form, no zh-CN, no
  Direction A screens). The refactor then lands after the beta release.
- Risk: pilot users get the old UI; the modernized client reaches them only in a later beta.

### B — move the tag to `89f84b0` and ship the modernized client

- The tag moves (delete/recreate locally or a new tag such as `v0.5.0-beta.2`), and the APK
  and Server artifacts are rebuilt from `89f84b0`.
- Requires re-running the evidence that the refactor itself did **not** re-collect as
  post-refactor proof:
  - real-device QR + SAS pairing, 5–10 minute audible playback, lock-screen continuity,
    network-drop recovery + Stop (evidence/ticket-27 retest covers the fix at `6ac3efe`;
    it must be re-bound to the new artifact hash);
  - one full 30-minute playback, revoke → 204 → re-pair, identity reset → 205, v1
    coexistence (ticket 25 prior runs are pre-refactor evidence and cannot be inherited);
  - Windows GUI pairing + 5-minute playback with the rebuilt exe;
  - JVM/`EMULATOR` matrices from `android-modernization` evidence are already post-refactor
    but must be bound to the same commit.
- Risk: a second real-device/Windows window is required before release.

## Constraints that do not change

- `VERSION` / `versionName` / `versionCode` were not changed by the refactor; either option
  keeps `0.5.0-beta` / `0.2.0-beta` / versionCode 2.
- The beta-acceptance scope and known limitations are unchanged (tickets 33/34).
- v1 APK and frozen v0.3.4 evidence are untouched.

## Definition of done

- Maintainer picks A or B and records it here.
- If A: ticket 24 proceeds with the existing `8858c91` tag and artifact manifest; the
  modernization ships in the next beta.
- If B: the new tag/commit and the exact re-verification list above are recorded, ticket 24's
  artifact manifest is regenerated, and the release notes cite the new hashes.
- The chosen tag is pushed together with the release (still a human action).

## Answer

2026-11-18 — 维护者裁决：**选 B**，发布包含 Android 现代化的构建。

- **tag 目标**：`362d023`（`integration/wire-protocol-v2-beta` 当前 tip），而不是工单写作时的
  `89f84b0`。两者是超集关系；`362d023` 含 `89f84b0` 的现代化合入与随后合入的工单 33
  （`server.log` 会话事件，`de83686`）。选 `89f84b0` 会刻意发一个已知缺日志诊断的 Server，
  与 33 的裁决冲突。
- **tag 形式**：本地删除并重建 annotated `v0.5.0-beta`（该 tag 从未 push，也没有下游消费者）。
  不新增 `v0.5.0-beta.2`，因为不存在已发布的 `v0.5.0-beta`。
- **产物与证据**：按 B 的要求重建 APK 与 Server 产物，重新生成 SHA-256 manifest，并更新
  release notes 引用新哈希。
- **仍需人工**：真机 QR/SAS、5–10 分钟播放、锁屏、断网恢复 + Stop、30 分钟、撤销后重配对、
  身份重置、v1 共存、Windows GUI 配对 + 5 分钟播放。`EMULATOR`/JVM 可在新 tip 上重跑。
- 工单 24 解除「tag 未定」阻塞，但在新证据齐备前仍保持 `ready-for-human`。

- 2026-11-18 追加：B 要求的「真机/Windows/EMULATOR 绑定新哈希复跑」由维护者按代码等价性
  `WAIVED`（记录在工单 24）。`WAIVED` 不是 `PASS`；JVM 250/250 已在 `362d023` 重跑。
- 2026-11-18 执行：tag 已推送至 fork `W0n9/audio-share`；对应 draft Release 已创建，未发布。
