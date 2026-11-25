# M5-05: Upstream pacing / send-ahead tuning

**What to build:** 定位并降低 L1 实测的上游延迟（~55 ms，波动 2–105 ms）。这一段的组成
是 **Windows WASAPI 采集缓冲 + `MediaPump` pending + pacer send-ahead + 网络 + 客户端
read**，目前只有总账，没有分项归属。

**Blocked by:** —

**Status:** ready-for-human

**Type:** task

> **前提**：本工单依赖 L1 阶段 1 的埋点（`AUDIO_SHARE_CAPTURE_TIMING` /
> `AUDIO_SHARE_L1_TIMING`），已提交到 `experiment/m5-audiotrack-buffer` @ `f073e07`
> （实验分支，未 push、未合并）。维护者裁决：保留在实验分支，不转正。本工单写代码时，
> 若需改这些埋点，应从 `main` 另开分支或先与实验分支协调，避免混入。
> 不依赖 M5-01：阶段 1 埋点已足够做分项。

## Requirements

- [ ] 用 L1 埋点（`AUDIO_SHARE_CAPTURE_TIMING` / `AUDIO_SHARE_L1_TIMING`）把上游拆成：
      WASAPI 采集（`BufferInfo.timestamp` 与 `MediaTap` 帧位置差）、`MediaPump.pending`、
      pacer 令牌桶积压、网络。
- [ ] 核实 WASAPI 1 s 缓冲请求（`BUFFER_DURATION_HNS`，`backend.rs:35`）在稳态下的
      **实际**贡献——1 s 是总缓冲，不是稳态延迟；用数据说话。
- [ ] 在 spec §7.4 约束内（send-ahead **≤ 100 ms**）降低稳态上游延迟；不得增加晚到
      （`late_dropped`）或网络丢包。
- [ ] 与 M5-03 的 jitter buffer 协同：上游越低、抖动越大，缓冲策略要一起评估，不能各自
      优化到互相打架。
- [ ] 用 M5-01 基线复测，给出每项改动的实际收益；无收益的改动不保留。
- [ ] 不改协议；不得在 capture 线程等待网络（保持现有非阻塞/丢最新语义）。

## Notes

- 依据：`l1-handoff/l1-results.md` 上游 p50 54.6 ms / p95 104 ms。
- `MediaPump.pending` 上限 500 ms（`media_pump.rs:44`）、pacer 100 ms 令牌桶
  （`send_queue.rs:18`）——先测清哪一项在稳态真正贡献延迟。
- **2026-11-19 新增待查（M5-02 A/B 中发现，未归因）**：两份日志
  `l1-handoff/server-m5-02.log`（42 ms 臂，首轮锁屏污染）与
  `l1-handoff/server-m5-02-128ms.log`（128 ms 臂，屏幕常亮，窗口 UTC 05:01:30 起）
  均出现 `capture_frame − submitted_position` 单调增长（128 ms 臂 ~1.8 ms/s，
  单会话 qpc rate=48000 正常），同时 ring p50≈10 ms（空）、underruns 0→83，
  用户听感修正为「断续爆音很少」（问题暂缓不追）。爬升在 128 ms 旧默认下同样复现，故**不是
  M5-02 引入**。可能是上游丢帧/迟到或计数口径问题；服务端目前**无 drop/overflow
  埋点**，无法区分。本工单先补埋点区分「真丢帧」与「迟到」，再谈调优；
  不要直接写成「已定位为 pacing」。详见 `evidence/m5-02/device-verification.md`。

### 2026-11-19 只读代码分析：L1 指标在有丢帧时的可信度

**结论：`capture_frame − submitted_position` 在丢帧时必然单调增长，它不是可听延迟指标。**

证据链（全部为源码，未改代码）：
1. `submitted_position = anchor + 实际提交字节 / frame_bytes`
   （`android-client-v2/native/src/media_session.rs:354-360`）；`submitted_bytes`
   是 AudioTrack **实际接受的字节**（`MediaPlayer.kt:350-352` / `ClientNative.kt:145-152`），
   不是 wire header 的 `timestamp`。`anchor` 来自 `StreamStarted` 的 `initial_timestamp`（:198）。
2. **若存在永久丢失的帧**：它们在采集侧 `capture_frame` 里计数，但永远到不了
   `submitted_position`，差值就会单调增长（迟到的帧最终仍会被提交，只表现为延迟，
   不会让差值永久增长）。观测 ~1.8 ms/s ÷ 0.02083 ms/帧 ≈ 86 帧/s 的永久缺失速率。
3. 客户端收到 sequence 跳空时（`:288-292`）：`interval.lost += gap` 后**直接
   `deliver_payload`**，不插静音、不补洞、不改 timestamp；M4 无 jitter buffer /
   concealment（:335-343 明示 0）。ring 因此保持浅（实测 p50≈10 ms），来一帧播一帧。
4. 客户端 ring 套圈丢弃路径（`MediaPlayer.kt:340-344`：`readPosition = writePosition`）
   在本场景可排除：实测 ring 空（p50≈10 ms），没有积压可被套圈。
5. 推论：ring 空 + 差值单调增长 + underruns 上升，符合"永久丢帧导致饥饿"的形态，
   **不是"可听延迟在增长"**。当天用户修正听感为「断续爆音很少」，与"少量丢帧"一致。

**更关键（同一只读分析）**：服务端丢帧在客户端完全不可见。
`MediaPump.accept` 在 pending 超上界时丢掉的是**尚未 packetize** 的 chunk；
`packetize` 只按**实际发送**的帧推进 `sequence` 与 `timestamp`
（`crates/v2-transport/src/media.rs:191-205`：`sequence += 1`、`timestamp += frames`，
没有按 `chunk.start_frame` 重新 seek）。因此服务端丢的帧既不占 sequence 也不占 timestamp：
- `MediaTap.dropped_chunks`（`capture/media.rs:184/210`）与 `MediaPump.dropped_chunks`
  的丢失在客户端**不产生 sequence 跳空**，`receiver.cumulative_lost` 永远发现不了；
- `cumulative_lost` 只统计 packetize **之后**的网络/队列丢失（经 §9 诊断已上报到 Server，
  只是 `l1_playback_timing` 没打印）；
- 服务端每丢一次帧，wire header 的 `timestamp` 就与真实 capture frame 永久错开一次。
  **M5-03 的 jitter buffer 与 M5-04 的漂移估计都依赖 timestamp，这一点必须让它们知道。**

**对 M5-05 的意义**：现有 L1 `capture→submit` 指标混淆了"延迟"与"丢帧"。
做 pacing 调优前应先：① 分开两类丢失——网络丢失看客户端 §9 `cumulative_lost`（已上报，
只是 L1 埋点没打印），服务端丢失看 `dropped_chunks` / `dropped_newest`（内存计数，
**未上报**）；② 先把服务端计数补进 `server.log`，③ 再决定是调 pacing 还是改 L1 口径。**不写成"已定位为 pacing"**。
注意：用户已决定断续问题暂缓，本条仅为只读分析，不改代码、不追查。

## Answer

**实现（`integration/m5`；提交 `51ccc56` + `e6f48e3`，review 修复 `f799599`）**

- **packetizer timestamp re-anchor（spec §7.2）**：`MediaPump::PendingChunk` 携带 `start_frame`，`take_sendable` 在 packetize 前按 `chunk.start_frame` 调用 `packetizer.seek`，使 wire `timestamp` 始终是该 payload 首帧的真实采集位置；`sequence` 仍每 datagram +1。服务端预 packetize 丢帧因此在 wire 上表现为 timestamp 跳空（客户端 jitter buffer/drift 已按 re-base 处理）。
- **服务端丢弃埋点**：`DropReporter`（注入时钟、5 s 节奏、仅变化时写）+ `session_log::media_drops` 事件（`media.drops`），把 `pump_dropped_chunks` / `queue_dropped_newest` / `pending_frames` 写入 `server.log`（经 tracing bridge，不改 GUI），从而把「服务端丢帧」与网络 `cumulative_lost` 区分开。
- host 测试：丢 chunk 后 timestamp 跳空、send-ahead ≤ 100 ms（§7.4）、reporter 节奏、日志红线。

**验证（host，PASS）**
- `cargo test -p v2-transport`：**289 passed, 0 failed**。
- `cargo test -p audio-share-core`：**118 passed, 0 failed, 1 ignored**（既有 ignore）。
- `cargo fmt` / `clippy -D warnings`（v2-transport）干净。

**NOT RUN / 未保留**
- WASAPI 采集 / `MediaPump.pending` / pacer / 网络的分项归属与调优：**NOT RUN**。L1 阶段 1 埋点仅存在于 `experiment/m5-audiotrack-buffer @ f073e07`，维护者裁决保留实验分支、不转正、不合入；本分支未复制这些埋点，也未做无数据支撑的调参。**不写成「已定位为 pacing」。**
- 真机/真实网络复测上游延迟：NOT RUN（无设备）。
- 已知缺口：task 退出前最后一个 <5 s 窗口的丢弃未 flush。
- 证据：`evidence/m5-05/`（未跟踪）。

## Comments

### 2026-11-20 Windows 真机验收

`media.drops` 事件在 `server.log` 正常出现（`pump_dropped_chunks` / `queue_dropped_newest` /
`pending_frames`），timestamp 重锚版本已实际运行。观察到 `pending_frames`≈480 ms 且
`pump_dropped_chunks` 持续增长、`queue_dropped_newest=0`，归因交
[M5-08](m5-08-pump-drop-trim-underrun-investigation.md)。L1 分项与上游调优仍 NOT RUN。
