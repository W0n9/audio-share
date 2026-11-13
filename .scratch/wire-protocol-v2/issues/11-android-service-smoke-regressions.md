# 11: Android foreground Service smoke regressions

**What to build:** 修复 ticket 10 后真机 smoke 暴露的 Android 前台会话回归；仅修 Service/Activity 状态重放与生命周期可见性，保持 Wire Protocol 与 phase-3 控制语义，不扩展到媒体播放。

**Blocked by:** 10

**Status:** resolved

**Type:** task

## Evidence and public seams

- 设备：Xiaomi M2007J22C / Android 12 / API 31，实机安装 `83bfd55` 的 APK 后标题更新为 `audio-share v2 control session`，display 保持 IDLE；Server wire trace 已见完整 AuthChallenge→AuthProof→AuthResult→NegotiateResponse 四条流，但 Activity 无状态回调。真实 device screenshot/server log saved under `.scratch/wire-protocol-v2/evidence/ticket-10/`。
- 对照 `599edd0`（Service 之前）可进入 READY，因此不能只修 UI 表面而留下抛异常/过早退出。
- Service 在 dumpsys 中显示已前台且绑定，但轮询/状态回调未反映「后续新连接已经 READY」。
- TDD seam 是 `ClientSession` 状态/关闭的公共快照，以及 foreground Service→Activity 的重绑定；这不是 wire 级别测试，不 toy 写死 READY 绕过真实 native 状态。

## Definition of done

- 新公共快照在测试中被赋予预定状态，pre-fix 回归测试失败；Service 在已存在会话上 rebind 时能立即返回它。
- Activity attach 不再将现存 READY 会话初始覆盖为 IDLE；同时检查轮询 R8/native unwind 的真实根因，不能改名为 UI-only fix 掩盖子线程崩溃。
- 实机显示原生 READY，锁屏/重建不复发；显式 ACTION_STOP 触发 701 与 Service 清理。
- Kotlin/JVM tests、release 构建与 v0.3.4 回归通过，设备证据与最终 commit 回填本工单。

## Answer

2026-11-13 — 本轮 DoD 已完成，Status → `resolved`。

- `058a905` 实现公开快照与同步重放，真实 READY 会话的新 Binder 监听者在 setter 返回前收到 READY；恢复原始行为时两次断言得到 null 的 RED 已记录。
- `df27ac3` 修复最终 Spec review 的 STOP/迟到 native 状态竞态：来源、生命周期 token 与状态提交/回调原子化；STOP 及之后的新会话不能被旧 READY 覆盖，合法 native 重连仍可 READY。可控调度断言 RED 为 CLOSED→READY，修复后 GREEN。
- 集成代码 `424fd30` 及后续验收提交在 `phase-2/wire-protocol-v2`。真机无日志 R8 release 的 rebind/recreate/lock/STOP 都 PASS，生命周期场景没有新连接，Server 收到701且前台状态清理。
- 最终 workspace465 PASS（4既有ignored）、JVM36 PASS、native默认/test-support各102 PASS、默认不含fixture身份的release构建PASS；发布SHA匹配的frozen v0.3.4 APK实际AudioTrack/wire smoke PASS。
- 未将初次IDLE历史图一概解释为native崩溃：此次最终复验一次IDLE是手机休眠/Activity解绑，唤醒后同APK通过；工装已补初始屏幕准备并移除临时日志。Windows/长时间音质验收未重新运行，保留原待验说明。

证据：[初始RED/GREEN](../evidence/ticket-11-resume/README.md)、[最终审查修复](../evidence/final-review-1112.md)、[最终合入后验收](../evidence/post-fix-device-acceptance.md)。

## Comments

Diagnostic handoff is pushed to `fork/ticket/10-android-service-regression` @ `b56c01a`. This is **not a fix and not resolved**: Kotlin state replay is a small attempted change; the Robolectric tests reference provisional test-only accessor names and require proper red/green completion. A known-good control comparison at pre-Service `599edd0` reaches real READY on device, whereas new title `audio-share v2 control session` stays IDLE. The next agent must debug actual polling/native state, not hardcode READY.

Windows acceptance pending; do not use Linux job tmp paths or identity material. Create fresh throwaway identity/authorization under Windows temp; no API credentials/private fixture scalar need to be transferred.

### 2026-11-13 — real-device regression opened

当前处于 `ticket/10-android-service-regression` 隔离 worktree，基于集成 `83bfd55`。测试依赖新增 Robolectric/androidx.test core，首版 ROB 测试存在 scope API 引用错误，不作为行为 RED；改用可注入的公开状态快照 test seam 重新运行。

### 2026-11-13 — 最终复验通过并关闭

代码、可控调度 RED/GREEN 与真机证据已落地，详见上方 Answer 和最终验收记录。用户确认旧 Server PID4107018可停止且不恢复；旧APK地址已按用户授权真正持久化恢复，并确认停止播放。本轮仅收尾本工单的 phase-3 控制会话范围，没有引入媒体播放。
