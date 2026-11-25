# 16: 导航与状态恢复

**What to build:** 屏幕选择只由 reducer 持有；旋转/Activity 重建保留当前屏与播放，进程死亡落 HOME 且不自动连接；Back 只做导航；SETTINGS 成为第五个目的地。

**Blocked by:** 15。

**Status:** resolved

- [x] outcome→屏幕转移表落地：Connect/PairingCompleted → SESSION；Live/Reconnecting/Ended 留 SESSION；配对失败留 PAIRING；Surfaced close 留在 SESSION 出错误卡。
- [x] 配置变更只保存 screen/address/sasInput；outcome、配对、凭据、错误从 Service 回放；相机不自动重开。
- [x] 进程死亡落 HOME、不自动连接、配对不续接、outcome 不落盘；冷启动沿用现有 auto-connect。
- [x] 恢复出的 PAIRING 无快照但 outcome 为 Live/Reconnecting 时落 SESSION。
- [x] Back：PAIRING 取消后 HOME；SETTINGS/ABOUT→HOME；SESSION→HOME 继续播放；HOME finish；Activity finish 仍发 ACTION_STOP。
- [x] `AppScreen` 增加 SETTINGS；JVM/Robolectric 覆盖旋转、进程死亡、Back 与恢复规则。

## Comments

2026-11-17 — 完成并合入集成分支（merge `2025443`，feature `aabc572`，已同步 ticket 21）。
`NavigationRestoration.reconcile` 实现 06 的恢复表；`UiStatePersistence` 只存
screen/address/sasInput；`BackHandler` 按屏导航；SETTINGS 成为第五目的地（内容由 18 完成）。
JVM 170 tests/variant PASS，androidTest Java 编译 PASS。
证据：`evidence/ticket-16/`。真机旋转/进程死亡为 NOT RUN。
