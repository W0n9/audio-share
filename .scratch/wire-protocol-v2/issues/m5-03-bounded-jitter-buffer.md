# M5-03: Bounded jitter buffer, late drop, and concealment

**What to build:** 实现 spec §8 的有界 jitter buffer 与损伤计数，替换 §9 诊断里当前
**硬编码 0** 的字段（`jitter_microseconds`、`late_drops`、`concealments`）。这是 M5 的
核心媒体机制。

**ring 就是 jitter buffer 的落点**：本工单同时负责客户端 ring 的目标深度与修剪（L1 实测
ring 稳态积压 ~46 ms 且不主动修剪），以及播放截止（late drop）判定。不要与 M5-02 的
`AudioTrack` 缓冲重复——两者是不同层：`AudioTrack` 是平台缓冲，ring 是应用层抖动缓冲。

L1 实测上游延迟 ~55 ms、波动 2–105 ms（`l1-handoff/l1-results.md`），说明 LAN 上仍有
显著抖动，jitter buffer 是必要的；但 target 是否就用规范值 60 ms，需 M5-01 基线 + 维护者
裁决（Q1）。

**Blocked by:** M5-02

**Status:** ready-for-human

**Type:** task

## Requirements

- [ ] 有界 jitter buffer：target **60 ms**、hard limit **200 ms**（规范值；若 M5-01
      数据支持调整，须维护者裁决 + 新证据，不得私自改）。
- [ ] **ring 目标深度与修剪**：积压超过目标时丢**最旧**帧并计数，避免启动填充永久滞留
      （L1 实测 ~46 ms 稳定积压）；必须与 late-drop 计数区分，且不得产生可听咔哒。
- [ ] 到达晚于其播放截止 → 丢弃并计 `late_dropped`，**与网络丢包分开计数**。
- [ ] gap/underrun → 静音 concealment（all-zero PCM，活动 Transfer Format），计
      `concealments` / `concealed_frames`。
- [ ] concealment 恰好覆盖四类干预：gap fill、underrun fill、drift insert/delete、
      hard-limit re-anchor deletion（spec §8）。
- [ ] §9 诊断字段改为真实测量值；客户端不得再把未实现的计数器报 0
      （`native/src/media_session.rs:331-343` 当前是占位）。
- [ ] **QUIC ACK 或 socket 收包不得当成播放证据**；播放证据只来自平台实际接受的字节。
- [ ] host/JVM/真机分层测试；RED→GREEN 切片；真机听测确认无新增可听咔哒。

## Notes

- 规范：`docs/wire-protocol-v2/spec.md` §8（Live playback）、§9（Playback diagnostics）。
- 与 M5-02 的顺序：先降平台缓冲，再在应用层定义抖动缓冲的 target/hard-limit 与
  re-anchor，避免两套缓冲策略打架。

### 2026-11-19 只读分析提示（来自 M5-05）

M5-05 的只读代码分析发现：`MediaPump.accept` 在 pending 超上界时丢的是**尚未 packetize**
的 chunk，而 packetizer 只按实际发送的帧推进 `timestamp`（`crates/v2-transport/src/media.rs:191-205`，
无 `chunk.start_frame` 重锚）。因此服务端丢帧在 wire 上不留 sequence 跳空，
但 header `timestamp` 会与真实 capture frame 永久错开一次。本工单依赖 `timestamp`
（jitter buffer 目标 / 漂移窗口），实现与验收时须考虑该偏移，不能假设 timestamp 与采集位置
严格一致。详见 `m5-05-upstream-pacing-tuning.md` 的「只读代码分析」段。

## Answer

**实现（`integration/m5`；提交 `491d159` + `bb08808`，review 修复 `f799599`）**

- `android-client-v2/native/src/jitter.rs`：按扩展 sequence 重排、按 wire `timestamp` 计量等待时间的有界 jitter buffer；`JitterConfig { target_ms: 60, hard_limit_ms: 200 }` 为可配置结构，默认值即规范值并由测试钉住（Q1 未裁决，未私改）。
- late drop：到达晚于其播放截止（deadline，基于注入会话时钟）→ 丢弃并计 `late_dropped`，与网络 `cumulative_lost` 分开计数。
- concealment：gap fill 与 underrun fill 均写 all-zero PCM 并计数；`ConcealmentKind` 四类（Gap/Underrun/Drift/ReAnchor）共享 §9 聚合计数并保留分项；drift/re-anchor 两类由 M5-04 落地。
- ring：积压超过 **target + 20 ms**（滞回）时软修剪最旧帧至 target（帧对齐），作为 buffer management **独立计数**（`soft_trimmed_frames/events`）并写 `playback.trim` 日志，**不计 concealment**；超过 **200 ms** hard limit 时 re-anchor 并计 `ReAnchor`。Kotlin 播放端不再硬编码 60/200，改为读取 native `JitterConfig`。
  - 说明：软修剪是 **M5-03 工单对 §8「concealment 恰好四类干预」的刻意偏离**——§8 约束的是 concealment 计数，buffer management 另行记录；2026-11-19 维护者裁决保留软修剪+滞回。
- underrun fill：仅在**平台 underrun 计数出现增量**且读端为空时补一个播放周期的静音，播放开始前先按 target 预缓冲；平台 `underruns` 字段与 concealment 分开，不重复计数。
- §9：`late_drops`、`concealments`、`concealed_frames`、`jitter_microseconds` 改为真实测量；held-media 释放与 timestamp re-base 不再污染 RFC 3550 A.8 估计。
- 服务器丢帧造成的 contiguous timestamp 跳空按 re-base 处理，不误插静音（与 M5-05 协同）。

**验证（host/JVM，PASS）**
- native `cargo test`（默认与 `--features test-support`）：各 **227 passed, 0 failed**（含 `jitter_buffer_contract`、`impairment_matrix`、`parser_fuzz`）；native `clippy --all-targets -D warnings` 干净。
- JVM `testReleaseUnitTest -PnoNative=true`：**268 tests, 0 failed, 0 skipped**（JDK 21 toolchain）。
- 两轮 code-review（含对 `f799599` 的第二轮）修复后重跑全绿；第二轮修复了 deadline 卡死回归、underrun 误报、软修剪滞回、漂移窗口被软修剪污染、config 变更未重建 jitter buffer、native clippy。

**NOT RUN（不写成 PASS）**
- 真机听测「无新增可听咔哒」：NOT RUN（无设备）。
- 设备/模拟器 Gradle device tests：NOT RUN。
- 真机 underrun/concealment 节奏调优：NOT RUN。
- 证据：`evidence/m5-03/`（未跟踪）；code-review 修复见 `evidence/m5-review-fixes.md`。

## Comments

### 2026-11-20 Windows 真机验收（用例 A）

PASS（主观）：用户「没有卡顿，爆音不明显，可以接受」，窗口 5m06s。客观计数异常：
`concealments total=5601`（gap 22 / underrun 5576 / drift 0 / reanchor 3），
`playback.trim soft_frames=63263 soft_events=28`，`reanchor_frames=43207 reanchor_events=3`。
计数与可听结果不一致，根因交 [M5-08](m5-08-pump-drop-trim-underrun-investigation.md)。
证据 `.scratch/wire-protocol-v2/evidence/m5-windows-evidence-2026-11-20/`。

### 2026-11-20 复测（evidence-2，431997b）

用例 A PASS：用户「连续无新增异常」；underrun 事件 5576 → 窗口内 +125；新增
`concealed_frames`/`underrun_frames` 可测（5m25s 内约 1.2 s 静音填充）。服务端 pending 全程 0、
`media.drops` 0。trim/reanchor 仍偏高（soft +86.9k、reanchor 1 次），记录待第 4 项。
证据 `evidence/m5-windows-evidence-2026-11-20/evidence-2/`。
