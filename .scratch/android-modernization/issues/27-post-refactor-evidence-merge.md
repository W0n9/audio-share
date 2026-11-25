# 27: Post-refactor 证据与合入

**What to build:** 对即将合入的 refactor tip 采集 08 的证据矩阵，写入带 SHA/哈希的 manifest；通过后以 `--no-ff` 合入 `integration/wire-protocol-v2-beta`。

**Blocked by:** 26，以及 [wire-protocol-v2 工单 25](../../wire-protocol-v2/issues/25-beta-acceptance-real-device-windows-v1.md) GO / `v0.5.0-beta` 冻结（外部条件）。

**Status:** resolved

- [x] JVM 全绿；`EMULATOR` 矩阵（签名安装、配对、播放/锁屏、旋转、进程死亡、rebind/recreate、API 26、Light/Dark/zh 截图）全部 PASS 或维护者 WAIVED。
- [x] 真机四项：QR+SAS 配对、5–10 分钟可听播放、锁屏 2–5 分钟、断网恢复 + Stop；缺项仅维护者 WAIVED。
- [x] `evidence/ticket-08-refactor/manifest.tsv` 每行绑定 tier/status/SHA/APK+Server 哈希/设备与 build flags/日期/命令/路径；无未关闭 FAIL。
- [x] 重构前证据（Windows、Server、v1、30 分钟真机音频）仅作历史引用，未作为 post-refactor PASS。
- [x] 以 `--no-ff` merge commit 合入集成分支；记录合入 SHA 与时间线。
- [x] 未改 `VERSION`/`versionName`/`versionCode`，未倒灌 beta。

## Comments

2026-11-18 — 证据已采集并写入 `evidence/ticket-08-refactor/`（`manifest.tsv`、`timeline.md`、
`jvm/`、`emulator/`、`device/`），绑定 refactor tip `5ff8936` 与 APK/Server 哈希。
JVM 249 tests/variant PASS；`EMULATOR` 16 KB 生命周期矩阵五项 PASS；API 26 安装/smoke PASS；
Light/Dark/zh-CN/200% 截图 8 张。真机四项与 TalkBack 为 `NOT RUN`。
**未合入 beta**：合入门槛要求真机四项 PASS 或维护者显式 `WAIVED`。真机执行结果：QR+SAS 配对 PASS、5m09s 可听播放 PASS、锁屏 3m14s PASS、断网恢复 + Stop FAIL（UI 未显示 `Reconnecting (attempt N)`，根因 `SessionLifecycleDriver.kt:137 Live(CONNECTING)` 覆盖 `:307 Reconnecting(N)`，维护者裁定 FAIL 且不放宽断言）。TalkBack 审计 WAIVED（维护者签署）。真机四项未全 PASS，本票保持 `ready-for-human`，未合入 beta。

## Comments

2026-11-18 — Windows 真机验收返回 `evidence/ticket-27/`（设备 Xiaomi M2007J22C / Android 12 / API 31，
APK `d0d446…`，commit `5ff8936`）：

| 项 | 结果 |
|---|---|
| QR + SAS 配对 | **PASS**（两条路径均完成，Server 接受后自动进入 SESSION） |
| 5–10 分钟可听播放 | **PASS**（用户确认有声音、无中断/爆音/卡顿；约 5 分 09 秒） |
| 锁屏 2–5 分钟 | **PASS**（约 3 分 14 秒，AudioTrack 持续 started，唤醒后仍 PLAYING） |
| 断网恢复 + Stop | **FAIL** — 断网期间 UI 未持续显示 `Reconnecting (attempt N)`；仅退避窗口短暂出现，connect 期间被 `Live(CONNECTING)` 覆盖 |

根因与修复：`SessionLifecycleDriver.publishLive()` 在重试连接期间把 `Reconnecting(attempt)` 覆盖为
`Live(CONNECTING)`。已修复（`6ac3efe` / feature `f93ba15`）：重连周期内保持 attempt 可见，
AUTHENTICATING 起恢复 Live。新增 JVM 闭包测试；JVM 250 tests/variant PASS。
需用新 APK 在真机重测第 4 项（或维护者单独 WAIVED）后才能合入 beta。

### 2026-11-18 第 4 项修复后重测

证据：`evidence/ticket-27/results-template-1.md` 与 `evidence/ticket-27/evidence-retest-item4/`。
APK `30fc763c…f667` / commit `6ac3efe`；设备 Redmi M2007J22C / Android 12 / API 31。

- A 轮：断网 20.5 秒，+1s→+20s 共 23 张截图均显示「正在重连（第 N 次）」（N=1→2→3，含 connect 窗口）；
  恢复后自动 QUIC 重连并回到播放。
- B 轮：Reconnecting 期间实际 Stop 成功（`stopped (User)`），此后无重连、前台清理。
- C 轮：恢复后自动播放，维护者确认出声；应用内 Stop → `701 CLIENT_STOPPING` + `stopped (User)` + 前台清理。
- **第 4 项判定：PASS**。

**未决观察（交维护者裁定）**：Server「接受设备」后手机停在「正在保存配对」，未自动进入 SESSION；
返回主页后可见「正在播放」，会话实际已建立。与用户故事 10 / 工单 17 的一次性 `PairingCompleted`
交接不符。第 1 项按维护者决定沿用上一轮 PASS，未重跑。

本票在偏差裁定并完成 beta 合入前保持 `ready-for-human`。

### 合入

2026-11-18 15:31 +0800，在 beta 工作区以 `--no-ff` 合入：
`89f84b0`（parents `8858c91` + `86b2635`）。合入前已恢复 `GLOSSARY.md` 并移除过期的未跟踪
ADR 副本，合并后 `GLOSSARY.md` 与 ADR-0005 为重构分支版本。合入后 JVM 250 tests/variant PASS；
`VERSION`/`versionName`/`versionCode` 未变；未 push。记录：`evidence/ticket-27/merge.md`。
本票 resolved。
