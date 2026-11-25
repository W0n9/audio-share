# 15: M4 stream lifecycle, explicit close, and integration acceptance

**What to build:** 将工单 13/14 接成单 Android Client 端到端 tracer bullet，验证真实 stream 生命周期、诊断与显式关闭，并从固定基线 `a5de2d9` 做 scoped Standards/Spec review 与回归。

**Blocked by:** 13, 14

**Status:** resolved

**Type:** task

## Requirements

- 验证 READY + configuration → Subscribe → StreamStarted(真实 id/generation/anchor) → PCM/AudioTrack → PLAYING → StreamEnded/Unsubscribe/Close 的完整顺序；media/control 无 cross-channel ordering 时靠 instance/generation 防护，而不是假设推送先到。
- `StreamEnded` 501/502、capture restart 新 instance、Unsubscribe 先 StreamEnded(502, `unsubscribed`) 后 response、Close 单向无 echo、transport close registry code、CLOSING 5 s grace 与 CLOSED 资源释放均有可观察断言。停止 Android Client 不停止共享 capture；不自动恢复 capture；不自动降级 v1。
- Server 收到的 `PlaybackDiagnostics` 来自 Android 实际播放提交与接收统计，不是 fixture literal、socket receive 或 QUIC ACK。周期 5 s、closing best-effort、pending cap、统计失败不影响媒体要有实证；字段完整且 interval reset/epoch reset 正确。
- 用独立 golden media/control vectors 做 Rust↔Android 互测，不能只测同一端 encode/decode。复用真实认证 live-QUIC 与隔离临时身份；不要从旧 tmp/worktree 路径复用私钥，不提交测试私钥。
- 真实 `AudioTrack`、前台 Service、默认 release 构建与 minSdk 26 检查；设备连接、安装测试 APK、恢复用户配置、停止已发现进程、锁屏操作均先重新确认。设备未运行时标 NOT RUN，不以 host/Java 编译代替。
- 从 `a5de2d9` 对本 M4 diff 调 `code-review` Standards/Spec 两轴；修复经确认的问题并复验。完成 workspace、native 默认/test-support、JVM 与 v1 fixture/frozen v0.3.4 既有回归，记录数字和失败原因。
- 每票 resolve 前更新 `map.md` Decisions-so-far/Fog 与本 roadmap 对应状态/证据。保持 `.scratch` specs、原型、roadmap、release notes、evidence 未跟踪，不全局 clean/add。

## Explicitly not M4 completion

完整 jitter-buffer target/hard-limit scheduling、late-drop/concealment、drift correction、完整 impairment/fuzz/60 min 矩阵属 M5；多 Client 隔离/soak、生产 pairing/Keystore/revocation UX、GUI/`as-cmd` 诊断属 M6；Windows/API26 真机、双真机长测、stable release/migration 属 M7。不要把 M4 单项通过写成这些项目 PASS。

## Answer

2026-11-14 — M4 集成切片完成（host/JVM/跨端 host harness 证据；**真机播放与长时间音质 NOT RUN**）。

- 合并工单 13/14 两侧改动；新增独立 `tools/rust-android-interop/`，用 golden vectors 与真实认证 live-QUIC（每次新建隔离身份）验证 Server 真实编码器/状态机 ↔ Android 真实接收器/解码器/状态机，共 12 tests，含 §9 诊断被 Server 观测与 Close(701) 单向不 echo。
- 完整生命周期与关闭顺序在 `state_machine_playing.rs`、跨端 `control_plane.rs`、live `v2_media.rs`/`live_quic.rs` 可观察：Subscribe→StreamStarted(真实 anchor)→PLAYING、StreamEnded 订阅保持、capture restart 新 instance、Unsubscribe 先 StreamEnded(502) 后响应、Close 不 echo。
- scoped 两轴 review 从 `a5de2d9` 完成：Standards 与 Spec 两轴报告均返回，逐条裁决与修复见 Comments。
- 回归：workspace 517 passed / 4 既有 ignored；native 默认与 `test-support` 各 127 passed；interop 12 passed；JVM 46 passed；默认 release 成功；六套 v1 fixture replay 通过；全部改动 crate `clippy -D warnings` 干净。v1 / `android-app/` / 冻结 APK / fixtures 未改动。
- 明确未验：真机 `AudioTrack` 出声、锁屏、长时间音质、API 26、双真机、60 min、pacing 时延预算；本会话未接触设备，设备操作须重新确认。冻结 `media.sequence-wrap` timestamp 内部矛盾已记录，未改资产。完整抖动/漂移/损伤、多 Client、生产 pairing/GUI、Windows、release 属 M5–M7。

**状态口径**：2026-11-14 经维护者授权「提交，本工作树收尾」，M4 代码已提交于隔离分支 `phase-4/single-client-pcm` 的 HEAD（47 files，无 Co-Authored-By）；`.scratch/` 的 specs/issues/evidence/roadmap 按项目约定保持未跟踪。无 push / PR。

2026-11-15 维护者授权「收口」并在普通终端完成 `git merge --ff-only phase-4/single-client-pcm`：`phase-2/wire-protocol-v2` 已快进到 `0377b59`，与 `phase-4/single-client-pcm` 一致（0 ahead / 0 behind、纯 fast-forward、无冲突）。满足 `triage-labels.md` 的 `resolved = merged and verified`，本票标 `resolved`。无 push / PR。真机验收仍为 NOT RUN。

## Comments

2026-11-14 — M4 范围与三处公共 seam 已确认，尚待 Server/Android 两个纵向切片。基线最终验收只证明 M3 READY/701 控制会话，不是本票的媒体证据。

### 2026-11-14 — 集成与跨端证据（工单 13/14 已 resolved）

Server 补丁从 `m4-server-pcm` 合入本工作树后：workspace 516 passed / 4 既有 ignored，native 默认与 `test-support` 各 122 passed，JVM 46 passed，默认 release 成功，六套 v1 frozen fixtures replay 通过，全部改动 crate 的 `clippy -D warnings` 干净。

新增 `tools/rust-android-interop/` 独立 harness（12 tests）：Server 真实 packetizer/pump/状态机与 Android 真实 `MediaSession`/`ClientMachine`/`FrameDecoder` 通过 golden vectors 与真实认证 live-QUIC 互测；live-QUIC 使用每次新建的隔离身份，不复用旧 tmp/worktree 私钥。详见 [evidence/ticket-15/README.md](../evidence/ticket-15/README.md)。

真机播放/锁屏/长时间音质/API 26/双真机/60 分钟仍为 **NOT RUN**；本会话未接触设备，任何设备操作须重新确认。冻结 fixture `media.sequence-wrap` 的 timestamp 内部矛盾已记录，未改资产。

### 2026-11-14 — scoped 两轴 review 与修复

`/code-review` 从 `a5de2d9` 用 `evidence/ticket-15/review-diff.patch`（52 files，含未跟踪新增文件）并行跑 Standards / Spec。Standards 轴的硬违规「工单 13/14/15 状态与 map/roadmap/frontier 不同步」已在审查读取旧快照期间修正：13/14 `resolved`、15 由 `ready-for-agent` 改 `claimed`（审查当时的状态，现已 `resolved`），map Fog 与 roadmap M4 行已同步；「改动未提交，不合 merged 口径」在审查时属实；其后经维护者授权已提交于隔离分支 `phase-4/single-client-pcm` 的 HEAD；2026-11-15「收口」在普通终端完成 fast-forward 合入，`phase-2/wire-protocol-v2` 现为 `0377b59`，满足 "merged"，工单标 `resolved`。另一条硬违规（工单 14 有真机 NOT RUN 却标 `resolved`）不采纳：该票的 human-only gate 明确归工单 15，票内 Answer 已标 NOT RUN；状态口径在 Comments 留痕。

审查发现并已修复的 M4 范围内真实缺陷：
- **§9 CLOSING best-effort 最终诊断缺失**：Android `stop()` 只发 Close(701)。新增 `MediaControl` 的 CLOSING 一次性报告 + `session.rs` shutdown 分支 best-effort 写诊断再 Close；`media_control_closing.rs` RED→GREEN 2 tests，并更新既有 `states_other_than_playing_never_report` 期望（改名 `_periodically`）。
- **§7.4/D10 的 256 datagram 队列未接生产路径**：`MediaSendQueue` 原来只有测试调用。现 `MediaTask` 持队列，pump 产出先 enqueue（溢出丢 newest）再非等待 pop 发送；`TooLarge` 用 `push_front` 保留等待更小限额，不静默丢媒体。新增 `a_too_large_send_is_requeued_in_order` RED→GREEN。
- **§9 pending 双份状态**：删除 `MediaSession` 的死 `pending_diagnostics`（只被测试用），单一真相保留在 `MediaControl`；对应重复测试改为由 `MediaControl` 覆盖。
- **命名/注释**：`MAX_PENDING_MS = 500` 的错误「100 ms」注释修正，driver 的 `64` 提为 `MAX_DATAGRAMS_PER_TICK`。

未采纳的判断项：跨端 harness 与 `as-cmd` live 测试各自持有 `start_server`（刻意复制，避免生产 crate 依赖测试）；`MediaChunk`/geometry 的三份类型属 crate 分层边界（`audio-share-core` / `v2-transport` / Android native 各自独立 workspace，adapter 显式逐字段转换），不引入共享 DTO。

复验：workspace 517 passed / 0 failed / 4 既有 ignored；native 默认与 `test-support` 各 106+2+5+10；interop 12；JVM 46；默认 release；全部改动 crate `clippy -D warnings` 干净；六套 v1 fixtures replay 通过。

### 2026-11-14 — Spec 轴报告（子代理延迟返回）与逐条修复

Spec 轴子代理在首轮汇总后延迟返回完整报告，逐条核实如下（不盲信，均查源码确认）：

**采纳并修复（M4 范围内真实缺陷）**
- **§9 5 s 诊断发不出（最严重）**：`session.rs` 的 `media_tick` 分支 `continue`，`send_diagnostics.take()` 只在收到 Server 控制帧后执行；PLAYING 期无控制帧 → 永不发送且 `diagnostics_pending` 不复位（`docs/wire-protocol-v2/spec.md:545`、工单 14:22）。修复：把发送移到产生诊断的同一 tick，抽出可测 `write_diagnostics<W: AsyncWrite>` helper，CLOSING 与 5 s tick 共用；新增 `diagnostics_are_written_without_a_server_control_frame` / `no_active_media_writes_no_diagnostics` 真实 async stream 测试。
- **§7.6 2 s 403 时钟不触发**：`media.tick()` 只置 flag，session 未检查；且释放 hold 后 sticky flag 会误杀后续包。修复：`MediaSession::tick` 返回「本 tick 是否跨过 give-up 且 hold 仍活跃」，`check_hold_give_up` 只在 hold 活跃时置位，`apply_configuration` 清 `held_since_ms`/flag；session 的 media_tick 分支收到 true 即发 403；新增 `a_released_hold_never_later_triggers_give_up`。
- **S24LE 静默停播**：`MediaSession::start` 接受 S24LE，但 `AudioSink.open` 返回 null 后仅 Log.w+sleep，无 403。修复：`start` 用既有 `configuration_decodable` 拒绝不可解码 geometry，走既有 403 路径；新增 `an_undecodable_geometry_is_refused_not_silently_played`。
- **`buffer_current_ms` 编数**：`MediaPlayer.kt` 造了 `TARGET_FILL_MS = 40` 下限。修复：只报实际 ring 缓冲毫秒，删掉该常量。
- **scope creep `mediaFormatJson` 第 7 键**：工单 14 JNI 契约固定 6 键。修复：去掉 `delivered_frames`，新增 6 键断言。
- **`jitter_microseconds` 恒 0**：不改为编造值。工单 14:21 允许「真实值或显式零值语义」，M4 未实现 RFC 3550 A.8 估算器（属 M5）；在 `diagnostics()` 注释中明确该零值的规范依据与 M5 归属。

**核实为旧快照 / 不采纳**
- 报告称「CLOSING 最终诊断与 Close(701) 缺失」：agent 读的是修复前的 `review-diff.patch` 快照；本轮已先修（见上节），源码有 `closing_report_sent` 与 shutdown 分支。
- `RequestConfiguration` 未发送：§7.6 step 2 是 SHOULD；M4 的 `ConfigurationPublished` 由 Server 主动推送且互测已验证，不新增未确认的请求流。
- 24-byte BE 头、send-time `max_datagram_size()`、§7.4 队列/pacer、§7.6 有界 hold、§5.2 Unsubscribe 顺序与订阅保持、Close 无 echo、`submitted_position` 取 AudioTrack 实收字节：报告确认核查通过。
- `media.sequence-wrap.json` timestamp 矛盾：冻结资产内部不一致，已记录，不用作实现 oracle。

**scope creep 总体**：未引入 M5 jitter/drift/concealment、多 Client、生产 pairing/Keystore/GUI、外部 telemetry、原始 PCM 日志；`tools/rust-android-interop/` 是独立测试 workspace，未加入根 workspace、未改产品分层。

复验（Spec 修复后）：workspace 517 / 0 failed / 4 ignored；native 默认与 `test-support` 各 127；interop 12；JVM 46；默认 release；六套 v1 fixture replay；全部改动 crate `clippy -D warnings` 干净。

### 2026-11-15 — 真机验收（NOT RUN → PASS），暴露并修复三个端到端缺陷

此前 M4 的 human-only gate 全部 NOT RUN。真机（Redmi Note 9 / API 31 / MIUI，adb over tailscale）
接上后按 M3 的既有工装流程跑：全新隔离 server 身份（指纹 `E090EA31 …`，临时目录，仅授权 fixture
测试身份）、`--fake-capture`、`-PtestSupport=true -PdeviceTests=true` 构建安装。

**第一次真机运行完全无声**。逐层取证后定位到三个只有端到端才能暴露的缺陷，全部 RED→GREEN：

1. **Server 丢弃唤醒媒体任务的 chunk**（`v2-transport/src/driver.rs`）：`select!` 的 `recv` 分支
   只判 `is_none()`，把收到的 chunk 丢掉，只有 `try_recv` 的积压进 pump。环回实测只有 51% 实时，
   真机 0.01x。新增端到端测试 `subscribed_client_receives_media_in_real_time_not_a_trickle`
   ——此前没有任何测试驱动这条循环（跨端 harness 直接调 `MediaPump`）。
2. **F32 轨道拒绝 `write(byte[])`**（`MediaPlayer.kt`）：`ENCODING_PCM_FLOAT` 的 `AudioTrack.write`
   恒返回 −3（阻塞/非阻塞皆然），被 `if (accepted > 0)` 吞为 0，播放头恒 0。改为 float 样点路径，
   接受数按 ×4 回算字节；新增 2 个 JVM 测试。
3. **去重位图 lap 换标不清位**（`native/src/media.rs`）：8192 个 datagram（约 22 s）后，
   64 位字换 lap 时未清旧位，导致 ~98% 新包被误判 duplicate。真机表现正是"先连续、后断续"。
   lap 变化时清字；新增 `dedup_survives_the_reorder_window_lapping`（RED 时 24 640 包中 16 191 个假重复）。

**验收结果**：真机 `AudioTrack` 出声与锁屏连续播放由用户听感确认；`audio_flinger` track
`Active: yes`、增益 −21 dB、播放头以实时速率推进；§9 诊断以 5.00–5.25 s 节奏到达 Server；
`Stop` 触发服务端 `ClientClosed(701)`；339 s 窗口送达 124 894 个 DATAGRAM = **0.98x 实时**。

复验：workspace 518 / 0 failed / 4 ignored；native 默认与 `test-support` 各 128；JVM 48；
interop 12；clippy 干净。证据见 [evidence/ticket-15/device-acceptance/](../evidence/ticket-15/device-acceptance/README.md)。

仍未验：60 分钟 soak、双真机、API 26、Windows、p95/p99 时延预算、无 underrun 播放（M5 §8）、
专业听测。
