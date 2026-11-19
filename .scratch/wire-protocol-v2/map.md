# Wire Protocol v2 — implementation map

## Notes

集成分支：`integration/wire-protocol-v2-beta`（产物与 tag 对应 `362d023`，2026-11-18；前序 M4 集成分支为 `phase-2/wire-protocol-v2` @ `1cb4e80`）。本地 annotated tag `v0.5.0-beta` 指向 `362d023`（工单 35 裁决 B 后重建，未 push）；其后的文档提交不属于发布内容。
本轮授权范围：2026-11-14 维护者确认 M4（Tracer-Bullet 第 4 步）——单 Android Client PCM、播放统计与显式关闭；固定基线 `a5de2d9`，隔离分支 `phase-4/single-client-pcm`。
原始设计 baseline 与 normative 协议不同；M5 抖动/漂移/损伤恢复、M6 生产 pairing/GUI/多 Client、M7 release 等后续阶段没有因本轮批准而自动宣称实现。
已确认公共测试 seam：① Server capture 输入到认证 QUIC 媒体/控制输出；② native 媒体交付、实际播放提交与统计 Interface；③ Android 播放 Interface 与真实 Service/Binder。逐切片 RED→GREEN，预期字节来自 normative spec/golden vectors，不把收包当播放证据。

## Decisions-so-far

- 工单 01–10 已在本轮前 resolved；对应历史裁决和证据保留在各自工单，不重新改写。
- [11 — Android Service 回归](issues/11-android-service-smoke-regressions.md#answer)：公开状态/同步重放、STOP迟到事件与新旧来源隔离已实现；实际 minified release 的 READY/rebind/recreate/lock/STOP701 通过。核心提交 `058a905` / `df27ac3`，集成 `424fd30`及后续验收提交。
- [12 — 实时 capture 门控与配置](issues/12-server-capture-gating-config-publish.md#answer)：只读实际capture线程、每请求刷新共享状态、不可用Subscribe501且不订阅、真实配置/stream状态字段完成；不自动恢复或更改v1。核心提交 `78c6981` / `df27ac3`。
- [13 — Server PCM 媒体路径](issues/13-server-pcm-media.md#answer)：`resolved`。共享 capture 有界观察 `MediaTap`、24-byte BE header/逐发送限额重切、§7.4 队列与 pacer、认证 QUIC DATAGRAM、§5.2 PLAYING/生命周期、诊断观察；workspace 516 passed。证据 [evidence/ticket-13/README.md](../evidence/ticket-13/README.md)。
- [14 — Android PCM 播放与统计](issues/14-android-pcm-playback-diagnostics.md#answer)：`resolved`（host/JVM 证据）。native `MediaSession`/`MediaControl`、JNI 低频契约、direct ByteBuffer ring→`AudioTrack`、§9 仅计平台实际接受字节；真机播放 NOT RUN。证据 [evidence/ticket-14/README.md](../evidence/ticket-14/README.md)。
- [15 — M4 生命周期与验收](issues/15-m4-stream-lifecycle-close-acceptance.md)：`resolved`（集成/跨端/两轴 review 完成并修复；2026-11-15 已 fast-forward 合入 `phase-2/wire-protocol-v2` @ `0377b59`；2026-11-15 真机验收：AudioTrack 出声与锁屏连续播放由用户确认，§9 诊断 5 s 节奏到达 Server，Close(701) 由 Server 观测，339 s 窗口 0.98x 实时；验收中修复三个端到端缺陷）；新增跨端 harness 12 tests（含真实认证 live-QUIC）、workspace/native/JVM/release/v1 fixture 回归通过。证据 [evidence/ticket-15/README.md](../evidence/ticket-15/README.md)、[device-acceptance](../evidence/ticket-15/device-acceptance/README.md)。
- [最终两轴审查与真正断言RED/GREEN](evidence/final-review-1112.md)：0 Standards硬违规，3维护问题和2Spec缺陷全部闭环。
- [最终合入后验收](evidence/post-fix-device-acceptance.md)：465 workspace PASS/4既有ignored、36JVM、native102×2、默认release和真机通过；frozen v034 SHA匹配、真实AudioTrack/wire smoke通过；用户原地址已真正持久化恢复。
- [32 — Android QR 一步配对与失效邀请恢复](issues/32-android-qr-one-action-recovery.md#answer)：QR 模式在 binding 校验后自动确认一次（Service gate 防重建重复入队），UI 以过渡态替代 Confirm；201/206 阻断卡片 + 显式重扫，相机不自动重开。提交 `f9347ff`，JVM/Robolectric 118 PASS，模拟器 PASS（一步配对、拒绝后阻断），真机与 206 路径 NOT RUN。证据 [evidence/ticket-32/](evidence/ticket-32/README.md)。
- [25 — Beta 真机/Windows 验收](issues/25-beta-acceptance-real-device-windows-v1.md#answer)：`resolved`。2026-11-17 最终裁决 **GO（fresh-install pilot）**，接受两条已知限制：`server.log` 缺 v2 会话事件（→ 33）、旧 v2 beta `install -r` 升级可能 205（→ 34）。30 分钟/锁屏/断网/撤销后重配对/v1 实际播放按维护者决定沿用此前 PASS、未复跑；工单 30 的 revoke/reject 修复以模拟器 RED→GREEN 为准，真机未复跑，不得写成真机 PASS。证据 [evidence/ticket-25/](evidence/ticket-25/ticket-25-result.md)。
- [33 — Server 日志补 v2 会话事件](issues/33-server-log-session-events.md#comments)：`ready-for-human`。根因是 GUI 从未安装 `tracing` subscriber（Windows 验收用 GUI）；新增 `v2_transport::session_log` + driver/PairingCore 事件 + GUI tracing bridge + 离线 admin append 审计。`--no-ff` 合入 `de83686`，session_log 测试 4 PASS、workspace 621 PASS，两轴审查修复完成；仅剩 Windows 重建产物后复跑 701 的人工门。证据 [evidence/ticket-33/](evidence/ticket-33/README.md)。
- [34 — 旧 beta 升级 205](issues/34-upgrade-path-205.md#answer)：`resolved`。维护者 2026-11-18 选择出口 (b)——不做凭据迁移修复，把「先卸载旧 v2 beta 再安装」写入发布说明与 Windows handoff；修复与回归测试 `WAIVED`。文档已落地：[release notes](../release-notes-v0.5.0-beta.md)、[windows-handoff](../../docs/beta-acceptance/windows-handoff.md)。
- Android 现代化（`.scratch/android-modernization/`）：map `complete`，13–27 全 `resolved`；2026-11-18 15:31 以 `--no-ff` 合入 beta 集成分支 `89f84b0`（JVM 250 tests/variant PASS，`VERSION`/versionName/versionCode 未变，未 push）。该合入位于原 `v0.5.0-beta` tag（`8858c91`）之后 41 个提交。
- [35 — Beta tag 位置裁决](issues/35-beta-tag-placement-decision.md#answer)：`resolved`。维护者 2026-11-18 选 **B**（发布含现代化的构建）；tag 目标取当前 tip `362d023`（含 33），本地删除并重建 annotated `v0.5.0-beta`，不新增 `.2`。产物重建与证据重跑见工单 24。
- [24 — Beta 发布](issues/24-beta-release-packaging-signing.md)：`resolved`。产物从 `362d023` 重建，JVM 250/250；最终产物真机/Windows/EMULATOR 复验按代码等价性 `WAIVED`（不是 `PASS`）。2026-11-18 已发布 pre-release：`https://github.com/W0n9/audio-share/releases/tag/v0.5.0-beta`（7 资产，digest 全匹配）。

## Fog

**Frontier（2026-11-19 重算；规则：open + unblocked + unclaimed，编号小者优先）**

| 顺序 | 工单 | Status | Blocked by | 下一步 | 执行者 |
|---|---|---|---|---|---|
| 1 | [M5-03 有界 jitter buffer](issues/m5-03-bounded-jitter-buffer.md) | `ready-for-agent` | — | M5-02 已 resolved，依赖解除；实现 jitter buffer + ring 修剪 + late drop + concealment | agent |
| 2 | [M5-05 上游 pacing 调优](issues/m5-05-upstream-pacing-tuning.md) | `ready-for-agent` | — | 用阶段 1 埋点拆采集 / pending / pacer | agent |
| 3 | [M5-01 端到端可听延迟基线](issues/m5-01-end-to-end-latency-baseline.md) | `ready-for-human` | — | 声学回环工装 + 外部录音，产出可听延迟 p50/p95/p99 | 人工（需录音设备） |
| 4 | [33 — Server 日志补 v2 会话事件](issues/33-server-log-session-events.md) | `ready-for-human` | — | 在 Windows 用重建产物复跑 GUI 701 路径 | 人工（代码已合入 `de83686`） |

M5 其余工单被依赖阻塞：M5-04（漂移）← M5-03；M5-06（损伤矩阵 + fuzz）← M5-03,04；
M5-07（60 min soak + **p95/p99 预算**）← M5-06。M5-02 已 `resolved`（生产提交 `d03865b`
已合入集成分支；听测 PASS；埋点复测 NOT RUN）。34 已 `resolved`；24 已发布
（`v0.5.0-beta` pre-release）。

**M5 开票（2026-11-19）**：基于 L1 阶段 1 实测开 7 张工单 `m5-01`…`m5-07`，用 `M5-NN`
编号避开 16–21 冲突。L1 阶段 1 结果见 [l1-handoff/l1-results.md](l1-handoff/l1-results.md)：
采集→提交 p50 96.8 ms（ring ~46 + 上游 ~55），AudioTrack 128→42 ms 真机听测连续、
21 ms 爆音，端到端可听延迟推算 ≈225 ms（未实测出声）。

**待裁决（尚未开票）**

1. **`v0.5.0-beta` tag**：已由 [35](issues/35-beta-tag-placement-decision.md) 裁决为 B；tag 已推送至 fork，Release 已发布（pre-release）。
2. **M5 提案三问**（L1 阶段 1 后更新）：Q1「60/200 ms 是否改」——**仍待 M5-01 可听基线**，本阶段只测提交前链路；Q2「AudioTrack 128 ms 归属」——已于 2026-11-19 裁决为**改**，生产提交 `d03865b` 已合入集成分支；Q3「MIUI `underruns` 可信否」——**不可信**（teardown 瞬间数千次、听感连续时计数暴涨）。见 [m5-proposal.md](m5-proposal.md)、[l1-results](l1-handoff/l1-results.md)。
3. ~~**M5–M7 编号冲突**~~：M5 已改用 `M5-NN` 规避；M6/M7 的 18–21 与 Beta 19–32 仍撞号，开票前须重排或改用 `M6-NN`/`M7-NN`。
4. **L1 阶段 1 埋点归属**：已由维护者裁决「保留在实验分支，不转正」。代码提交于
   `experiment/m5-audiotrack-buffer` @ `f073e07`（7 个文件，含 `Cargo.lock`；未 push、
   未合入集成分支）。M5-01、M5-05 可引用；若需改这些埋点，应从 `main` 另开分支或先协调，
   避免混入实验分支。
5. **工单文件名形式**：M5 用 `m5-NN-*.md`，与约定的 `<NN>-<slug>` 不同；若将来有脚本按
   数字编号计算 frontier，`m5-*` 会排在数字工单之后。当前无自动化脚本，人工/agent 计算
   不受影响。
6. **M5-02 A/B 发现的既有问题（未归因）**：`capture_frame − submitted_position` 在 42 ms
   与 128 ms 两档都单调增长（128 ms 臂屏幕常亮也复现，~1.8 ms/s；qpc rate=48000 正常），
   ring 空、underruns 上升，用户听感「断续爆音很少」（问题暂缓不追）。不是 M5-02 回归；但与 11-18
   同档「连续，无咔哒」矛盾——今天 Server 是**带 L1 埋点的构建**且两臂间重启过（05:01:29），
   环境因素当时未排除。后续 dell-ws 已用**正式 v0.5.0-beta Server**、屏幕常亮完成
   42 ms 听测 **PASS**（≥5 min，30 min WAIVED），证据 `evidence/m5-02/soak-dell-ws/`，
   已排除埋点构建干扰。**爬升/断续仍未归因，用户决定暂缓不追**；不写成已定位。
   早期分析见 `evidence/m5-02/device-verification.md`，已附 M5-05 Notes。

**不在本 map**：`.scratch/server-rewrite/` 仍有 4 张 `ready-for-human`（05/13/15/16），属 v0.4.x Server 重写线，需补验或标 `wontfix`。

**M4 历史（2026-11-15，保留）**：13、14、15 均已 resolved；M4 全部工单关闭。维护者在普通终端完成 `git merge --ff-only phase-4/single-client-pcm`，集成分支 `phase-2/wire-protocol-v2` 快进到 `0377b59`（无冲突、无 push/PR）。同日补齐真机验收：真机 AudioTrack 出声、锁屏连续播放、§9 诊断节奏与 Close(701) 均取得证据，并修复三个端到端缺陷（Server 丢弃唤醒 chunk、F32 轨道 `write(byte[])` 恒 −3、去重位图 lap 换标不清位）。

**仍如实记为 NOT RUN**：60 分钟 soak、双真机并发、API 26 真机、p95/p99 时延预算、无 underrun 播放（M5 §8）、专业听测、冻结 `media.sequence-wrap` timestamp 矛盾（待裁决）；工单 30 的 revoke/reject 后重新配对真机复跑。M5–M7 的 stable-gate 工作不因 Beta GO 而完成。

- M5-02 更新：42 ms 真机听测 **PASS**（dell-ws，≥5 min，用户原话「连续，无咔哒/爆音/断续」）；≥30 min 门槛用户 **`WAIVED`**。30 min 平台采样 FrmCnt 恒 2052、屏幕常亮；MIUI Underruns 58857→321653 不可信只作参考。证据 `evidence/m5-02/soak-dell-ws/`。端到端可听收益仍 NOT RUN（M5-01）。

- 2026-11-18 发布证据裁决：工单 24 的最终产物复验（真机/Windows/EMULATOR on `362d023`）
  由维护者按代码等价性 **`WAIVED`**；JVM 250/250 已在 `362d023` 重跑。`WAIVED` 不是 `PASS`，
  release notes 与 manifest 已按此标注；工单 33 的 Windows 701 验证仍单独保持 `ready-for-human`。
