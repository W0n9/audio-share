# Wire Protocol v2 — implementation map

## Notes

集成分支：`phase-2/wire-protocol-v2`。
本轮授权范围：2026-11-14 维护者确认 M4（Tracer-Bullet 第 4 步）——单 Android Client PCM、播放统计与显式关闭；固定基线 `a5de2d9`，隔离分支 `phase-4/single-client-pcm`。
原始设计 baseline 与 normative 协议不同；M5 抖动/漂移/损伤恢复、M6 生产 pairing/GUI/多 Client、M7 release 等后续阶段没有因本轮批准而自动宣称实现。
已确认公共测试 seam：① Server capture 输入到认证 QUIC 媒体/控制输出；② native 媒体交付、实际播放提交与统计 Interface；③ Android 播放 Interface 与真实 Service/Binder。逐切片 RED→GREEN，预期字节来自 normative spec/golden vectors，不把收包当播放证据。

## Decisions-so-far

- 工单 01–10 已在本轮前 resolved；对应历史裁决和证据保留在各自工单，不重新改写。
- [11 — Android Service 回归](issues/11-android-service-smoke-regressions.md#answer)：公开状态/同步重放、STOP迟到事件与新旧来源隔离已实现；实际 minified release 的 READY/rebind/recreate/lock/STOP701 通过。核心提交 `058a905` / `df27ac3`，集成 `424fd30`及后续验收提交。
- [12 — 实时 capture 门控与配置](issues/12-server-capture-gating-config-publish.md#answer)：只读实际capture线程、每请求刷新共享状态、不可用Subscribe501且不订阅、真实配置/stream状态字段完成；不自动恢复或更改v1。核心提交 `78c6981` / `df27ac3`。
- [13 — Server PCM 媒体路径](issues/13-server-pcm-media.md#answer)：`resolved`。共享 capture 有界观察 `MediaTap`、24-byte BE header/逐发送限额重切、§7.4 队列与 pacer、认证 QUIC DATAGRAM、§5.2 PLAYING/生命周期、诊断观察；workspace 516 passed。证据 [evidence/ticket-13/README.md](../evidence/ticket-13/README.md)。
- [14 — Android PCM 播放与统计](issues/14-android-pcm-playback-diagnostics.md#answer)：`resolved`（host/JVM 证据）。native `MediaSession`/`MediaControl`、JNI 低频契约、direct ByteBuffer ring→`AudioTrack`、§9 仅计平台实际接受字节；真机播放 NOT RUN。证据 [evidence/ticket-14/README.md](../evidence/ticket-14/README.md)。
- [15 — M4 生命周期与验收](issues/15-m4-stream-lifecycle-close-acceptance.md)：`resolved`（集成/跨端/两轴 review 完成并修复；2026-11-15 已 fast-forward 合入 `phase-2/wire-protocol-v2` @ `0377b59`）；新增跨端 harness 12 tests（含真实认证 live-QUIC）、workspace/native/JVM/release/v1 fixture 回归通过；设备项 NOT RUN。证据 [evidence/ticket-15/README.md](../evidence/ticket-15/README.md)。
- [最终两轴审查与真正断言RED/GREEN](evidence/final-review-1112.md)：0 Standards硬违规，3维护问题和2Spec缺陷全部闭环。
- [最终合入后验收](evidence/post-fix-device-acceptance.md)：465 workspace PASS/4既有ignored、36JVM、native102×2、默认release和真机通过；frozen v034 SHA匹配、真实AudioTrack/wire smoke通过；用户原地址已真正持久化恢复。

## Fog

13、14、15 均已 resolved；M4 全部工单关闭，frontier 为空。2026-11-15 维护者在普通终端完成 `git merge --ff-only phase-4/single-client-pcm`，集成分支 `phase-2/wire-protocol-v2` 快进到 `0377b59`（无冲突、无 push/PR）。真机播放/锁屏/长时间音质/API 26、pacing 时延、冻结 `media.sequence-wrap` timestamp 矛盾均如实记为 NOT RUN / 待裁决，不由本轮写成 PASS。后续 Tracer-Bullet 第 5–7 步、生产 pairing/Keystore/GUI、Windows/API26 真机与长时间音质矩阵仍需各自后续工单和验收，不由本 map 替代。
