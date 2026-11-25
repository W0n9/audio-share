# M5-08: Diagnose server pump drops, client trim/re-anchor and underrun counters

**What to build:** 诊断 2026-11-20 Windows 真机验收暴露的三个耦合信号，定位根因并给出修复方案；
本工单先做**只读诊断**，修复另开或经维护者裁决后在同一分支追加。

**Blocked by:** —

**Status:** ready-for-human

**Type:** task

## Evidence (2026-11-20, integration/m5 @ 800c40c)

- 记录与原始日志：`.scratch/wire-protocol-v2/evidence/m5-windows-evidence-2026-11-20/`
  （`README.md`、`RESULT.txt`、`server-C-extended.log`、`client-A/B.log`）。
- 用例 B 用户裁决 FAIL；用例 A 主观 PASS 但计数异常。

## 观察到的信号

1. **Server pending 钉在 ~480 ms 并持续丢帧**
   - `media.drops` 每 ~5 s 一条：`pump_dropped_chunks` 1→176，`queue_dropped_newest=0`，
     `pending_frames` 22.5k–23.7k（≈470–495 ms，逼近 `MAX_PENDING_MS=500`）。
   - 说明 capture 生产速率略快于 pacer 排空，或 pacer/loop 的排空节奏与采集时钟不一致。
   - 若 `capture_position − last_sent_position` 按最新采集位置计，≈480 ms 超出 spec §7.4
     的 100 ms send-ahead；先核实 `MediaPacer` 的口径，不要预设违规或合规。
2. **客户端 underrun 事件 5576 / 5 min**
   - `underrunGate` 以平台 `underrunCount` 增量为门控；MIUI 该计数已知不可信（Q3：
     teardown 瞬间数千次）。需确认 5576 次 fill 各自实际写入的帧数（`concealed_frames`
     未在报告快照中），并评估改用「持续空读 N 个周期」等更可靠信号。
3. **软修剪 63k 帧/28 次、reanchor 43k 帧/3 次（A）；扩展期 trim 619k / reanchor 156k**
   - 滞回 `target+20 ms` 可能对突发交付过紧，频繁丢最旧帧；需区分「瞬时突发」与
     「持续速率差」，避免可听丢帧。
4. **drift_ppm 36 / 499（单次），correction 限幅 -80/100**
   - 软修剪会重置 10 s 漂移窗口；频繁重置可能使估计只覆盖短窗口。
   - 观测到的缓冲/修剪量级（~0.4%）远超 ±100 ppm；需确认是真实速率差、突发修剪还是口径问题。

## Requirements

- [ ] 只读定位：把上述信号分别归因到可证伪的机制（pacer 令牌率 / 采集时钟 / pump 上界 /
      trim 滞回 / underrun 门控 / 漂移窗口重置），用日志或最小复现支持。
- [ ] 明确 Server pending 与 §7.4 send-ahead 的关系：给出当前口径与是否违规的判断。
- [ ] 给出最小修复方案与优先级，标明是否触及已 accepted 数值（60/200、±100 ppm、100 ms）。
- [ ] 不修改 spec 默认值；如需改，先列维护者裁决项。
- [ ] 输出 `evidence/m5-08/` 诊断记录（未跟踪），并回写本工单 Answer。

## Notes

- 复现路径：Windows Server + 真机正常播放，约 10 min 后出现持续 `media.drops` 与客户端 trim。
- 相关代码：`crates/v2-transport/src/{media_pump.rs,send_queue.rs,driver.rs}`、
  `android-client-v2/native/src/{jitter.rs,drift.rs,media_session.rs}`、
  `android-client-v2/app/src/main/java/dev/audioshare/v2/MediaPlayer.kt`。
- 相关工单：M5-03（jitter/trim/concealment）、M5-04（drift）、M5-05（pacing/埋点）。

## Comments

### 2026-11-20 只读根因诊断（完成）

完整笔记：`evidence/m5-08/README.md`。结论（有日志/代码引用与定量拟合）：

- **A（High）**：服务端 pending 钉在 500 ms 的主因是 `MediaPacer` 令牌桶每次 `try_consume`
  截断小数帧并前移 `last_refill`，在 ~200 次调用/秒下少发 ~2000 ppm；模拟与观测丢弃率
  （0.206/s ≈ 2060 ppm）吻合。次因：无 catch-up，瞬态积压永久化。§7.4 字面
  `capture_position − last_sent_position ≤ 100 ms` 实际被违反（≈480 ms），且代码从未检查。
- **B（High）**：5576 次 underrun 是平台 `AudioTrack.underrunCount` 伪计数驱动；同窗口
  AudioFlinger `partial=0 empty=0`、`FrmRdy/FrmCnt=2015/2052`（98% 满），per-track
  Underruns 3 508 242 不可信。应改用「持续空读 20–30 ms」或 `getPlaybackHeadPosition` 停滞。
- **C（High）**：trim 是突发/滞回驱动，不是 ~0.4% 物理速率差；FastMixer `localSR` 实测
  −65…+42 ppm，在 ±100 ppm 内。499 ppm 是窗口被频繁重置后的短窗口伪值（50 min 仅 11 条
  drift 日志）。
- **修复排序（1–5 不改 accepted 数值）**：1 小数帧进位；2 重做 underrun 门控并打印
  `concealed_frames`；3 漂移窗口鲁棒化；4 突发感知 trim；5 服务端周期埋点。
  **6 §7.4 字面口径（catch-up 或 drop-oldest 到 100 ms）与 7 jitter hole-hold 策略需维护者裁决。**

### 2026-11-20 修复 1+2+3+5 已合入 `431997b`

- pacer 小数帧进位；underrun 门控改为持续空读 ~30 ms 并打印 `concealed_frames`/`underrun_frames`；
  漂移窗口扣除已知偏移并打印 `span_ms`/`sample_count`；新增 `media.stats`（每 5 s）。
- 验证：v2-transport 292、core 118(+1 ignored)、native 231×2、JVM 271；v2/native clippy 干净。
- 已重建 Windows GUI/CLI 与 arm64 APK（见复测 handoff）。**真机复测 NOT RUN，待用户执行。**
- 未做：第 4/6/7 项（等复测数据裁决）。

### 2026-11-20 复测结论（evidence-2，431997b）

- 第 1 项（pacer）：A pending=0、consumed≈48k/s；B pending ≤66 ms、零丢弃 → **有效**；
  §7.4 字面 100 ms 在两个窗口内满足。
- 第 2 项（underrun 门控）：事件 5576→125，`concealed_frames` 可测 → **有效**。
- 第 3 项（漂移窗口）：发布门槛生效，但极端 ppm 与 reanchor 突跳同刻 → **部分有效**。
- 第 4 项（trim/reanchor）：soft trim 仍高、reanchor 8 次/5 min → **待处理**。
- 第 5 项（media.stats）：每 5 s 出现，含 pending/consumed/pacer_calls → **有效**。
- 第 6 项（§7.4）：观测满足（pending 0 与 ≤66 ms），建议以 `media.stats` 为守卫关闭，不做 catch-up/drop-oldest。
- 第 7 项：仍待裁决。

### 2026-11-20 收尾裁决

- §7.4（第 6 项）：以 `media.stats`（每 5 s pending/consumed/pacer_calls）为守卫**关闭**，不做 catch-up/drop-oldest。
- 第 3 项残留与第 4 项转 [M5-09](m5-09-drift-step-and-burst-trim.md)（`needs-triage`）；第 7 项仍待裁决。
- 用户决定暂不改代码，先收尾；真机 B 仍 FAIL（open），不 FF beta。
