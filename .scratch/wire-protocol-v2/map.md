# Wire Protocol v2 — implementation map

## Notes

集成分支：`phase-2/wire-protocol-v2`。
本轮授权范围：工单 11 的 Android 前台控制会话回归，以及工单 12 的 capture/配置/StreamStarted 修复；继续保持 Tracer-Bullet 第 3 步 READY 范围。
原始设计 baseline 与 normative 协议不同；媒体/漂移/pairing/GUI 等后续阶段没有因本轮关闭而自动宣称实现。

## Decisions-so-far

- 工单 01–10 已在本轮前 resolved；对应历史裁决和证据保留在各自工单，不重新改写。
- [11 — Android Service 回归](issues/11-android-service-smoke-regressions.md#answer)：公开状态/同步重放、STOP迟到事件与新旧来源隔离已实现；实际 minified release 的 READY/rebind/recreate/lock/STOP701 通过。核心提交 `058a905` / `df27ac3`，集成 `424fd30`及后续验收提交。
- [12 — 实时 capture 门控与配置](issues/12-server-capture-gating-config-publish.md#answer)：只读实际capture线程、每请求刷新共享状态、不可用Subscribe501且不订阅、真实配置/stream状态字段完成；不自动恢复或更改v1。核心提交 `78c6981` / `df27ac3`。
- [最终两轴审查与真正断言RED/GREEN](evidence/final-review-1112.md)：0 Standards硬违规，3维护问题和2Spec缺陷全部闭环。
- [最终合入后验收](evidence/post-fix-device-acceptance.md)：465 workspace PASS/4既有ignored、36JVM、native102×2、默认release和真机通过；frozen v034 SHA匹配、真实AudioTrack/wire smoke通过；用户原地址已真正持久化恢复。

## Fog

本轮工单 frontier 为空。后续 Tracer-Bullet 第 4–7 步、生产 pairing/Keystore/GUI、Windows/API26真机与长时间音质矩阵仍需各自后续工单和验收，不由本 map 替代。
