# M5-11: 漂移估计改用硬件播放位置作为播放侧时钟

**What to build:** 先用 host RED 证明「以 AudioTrack 接受量作为播放时钟」在粗粒度写入下产生数百 ppm 噪声，再改为读取硬件播放进度（`getPlaybackHeadPosition()` 或 `getTimestamp()`），最后用同一判据复测用例 B。

**Blocked by:** —

**Status:** ready-for-human（实现与两轴审查修复合入 `71e0155`；真机 B 复测 FAIL，待裁决重跑条件或判据）

**Type:** task

## Evidence

- [B3 复测](../evidence/m5-09/RESULT-B3.md)：稳态原始估计 −645…+532 ppm，67/67 区间至少一端越出 ±100；均值 +0.3、中位数 −40，无灾难性离群；`discontinuous_intervals=0`、re-anchor 0；Server pending 0、drops 0。`correction_ppm` 35/67 顶在 ±100。
- 代码事实：`media_session.rs:663` 的 `submitted_position()` = `anchor + submitted_bytes/frame_bytes`，`submitted_bytes` 来自 Kotlin `recordPlayback(..., submittedBytes, ...)`，即 **`AudioTrack.write` 已接受**的字节，不是硬件播放进度。
- 量级：10 s 窗口下单个 2052 帧块约合 4,275 ppm，一次回归后残留在数百 ppm，与观测同量级（见 [host 证据](../evidence/m5-09/host-drift.md) 的批提交模型）。
- 以上是**待验证假设**，不是已确认根因；B3 没有逐报告输入与写入粒度，不能回放。

## Requirements

- [x] **先 RED**：host 上用双 48 kHz、按 2052 帧（另加更细/更粗一档）批量接受、100 ms 报告、注入已知漂移，复现「逐窗残差超 ±100、长期均值正确」；新时钟输入的同场景必须转绿。不能复现就停在报告，不盲改。
- [x] 播放侧时钟改取真实硬件进度：Kotlin 端读 `AudioTrack.getPlaybackHeadPosition()`（32 位无符号帧计数，会回绕）或 `getTimestamp()`（framePosition + nanoTime）。
- [x] JNI/报告契约扩展要向后兼容、单位明确（帧 vs 字节）、失败时退回现路径并记录诊断；不得静默把「读不到」当成 0。
- [x] 位置记账要一致：已接受的 trim/re-anchor 删除、插入的静音、应用 ring 的软修剪，都会让硬件进度与接受量分离；`drift_playback_step` / `anchor` 语义必须一并考虑，不能只换取值而留下双重补偿。
- [x] 不改协议、60/200 ms、±100 ppm 上限；不钳位原始 `drift_ppm`；不改变 re-anchor 触发条件与计数；§7.4 不动。
- [x] **修正饱和纳入本票**：修复后验证稳态 `correction_ppm` 不再被噪声长期顶在 ±100。若需调平滑步长/死区，用不同噪声水平的 host 测试证明稳定性，不能靠放宽判据。
- [ ] 真机 B 复测用同一判据（原始估计 ±100、诊断覆盖、re-anchor、缓冲/服务端）。没有真机 PASS 不写 `resolved`，不合 beta。

## Notes

- 相关代码：`android-client-v2/native/src/{drift.rs,media_session.rs,jni.rs,session.rs}`、`android-client-v2/app/src/main/java/dev/audioshare/v2/MediaPlayer.kt`。
- 首次报告前播放头为 0；AudioTrack 重建/换轨后需要重新锚定；跨 32 位回绕要显式处理。
- 硬件播放头含平台自身 42 ms 缓冲，这是可接受的延迟，不影响 10 s 斜率；但**不要**因此去改 `buffer_current_ms` 的口径。
- 若 host RED 证明噪声另有来源（例如采集侧时间戳量化），在同一票内改记证据并重新评估，不要套用本票的修法。

## Comments

### 2026-11-21：host 实现与 RED/GREEN

- 基线 `integration/m5 @ 99263a8`；`git merge integration/m5` 为「已经是最新的」，
  fork 远端无该分支。
- Host RED 复现 accepted 字节时钟：2052 帧批量接受、120 s、+75 ppm 时
  901/1090 窗越出 ±100，长程均值 73.33（精确 accepted 对照 75.00）；4104 帧
  批量接受直接饿死窗口。硬件头同 fixture 逐窗 ≤ ±100，`correction_ppm` 不再
  顶在 ±100。详见
  [`../evidence/m5-11/host-clock-source.md`](../evidence/m5-11/host-clock-source.md)。
- Native 默认/test-support 各 248 项通过；Clippy/ fmt 通过。
  JVM debug/release 各 278 项通过；`-PtestSupport=true` 编译通过。
- 实现采用 `anchor + head`，`position_adjust_frames` / `drift_playback_step`
  只保留在 accepted 路径与 §9 `submitted_position`，避免双重补偿。
- 保留风险：5 ms 混音周期量化的裸 head 在抖动报告时刻下仍可能越界（240 帧量化
  host 实测 43/491），因此生产实现优先 `getTimestamp()` 的 frame+time 插值，
  裸 32 位计数器仅作启动回退，需真机确认其精度。
- **真机用例 B：NOT RUN**。状态保持 `ready-for-agent`，不写 `resolved`，不合 beta。
- 另：未验证 arm64/APK 构建与 MIUI 路由上的时间戳行为。

### 2026-11-21：两轴审查修复（基于 `d60dc9e`）

五处修复均先 RED 后改：

1. **32 位扩展单一所有者**：Kotlin 已输出单调值，native 删除了 `last > 0xF0000000`
   的 2^32 分支；契约明确「输入已扩展」，native 的任何回退只按重建重锚。RED：
   `an_extended_long_run_head_then_rebuild_reanchors` 在修复前
   `head_reanchor_events` 为 0（误判回绕），修复后为 1。
2. **时钟源不乒乓**：单次缺失只跳过该次观测（不发布过期值、不清窗），计
   `hardware_missing_reports`；距最后硬件报告达到 `HARDWARE_HEAD_GIVE_UP_MS = 500`
   （5 个 100 ms 报告间隔，>8× 60 ms 连续性预算）才切 accepted、清窗并计
   `head_give_up_events`。RED：每 5 s 缺一次时旧实现
   `accepted_fallback_reports = 7`、`hardware_missing_reports = 0`；持续缺失时
   `head_give_up_events = 0`。
3. **年龄回跳**：Kotlin 端超过 5 ms 容差的一次回退先保持上值，**连续两次**才认定
   重建；5 ms 容差按 sink 采样率换算（48 kHz = 240 帧，8 kHz = 40 帧）。RED：
   单次回跳与采样率容差用例在修复前失败。
4. **命名一致**：`playback.drift_clock` 的键与 `PlaybackClockStats` 字段一一对应
   （`hardware_missing_reports` / `accepted_fallback_reports` /
   `head_reanchor_events` / `head_give_up_events`）。
5. **证据**：`.scratch/wire-protocol-v2/evidence/m5-11/host-clock-source.md`
   已改为完整可复制命令、实际输出片段，并补入 500 ms 回退阈值与 200 ms
   时间戳新鲜度的依据。

真机 B 仍为 **NOT RUN**，状态不变。

### 2026-11-21：真机 B 复测（`71e0155`）— FAIL

- 见 [RESULT-WINDOWS-M5-11](./../evidence/m5-11/RESULT-WINDOWS-M5-11.md)。
  `playback.drift_clock source=hardware` 全程有效（missing/fallback/give_up 均 0），
  说明该机 `getTimestamp()` 可用；但逐窗极值 **−1,390…+1,529 ppm**，71/71 区间越界，
  比 B3 的 accepted 路径（−645…+532）更大。覆盖、re-anchor、Server pending/drops 均通过；
  `correction_ppm` 50/72 顶在 ±100。
- **可比性注记**：本轮 `consumed_frames` 折算约 191,944.7 帧计数/s、pacer 约 1,200/s，
  约为 B3 的 4 倍；`server.log` 未记采集格式，无法区分 192 kHz 与 4×48 kHz。
  该差异未经控制，本轮与 B3 不是严格同条件。
- 真机结论维持 **FAIL**；不写 `resolved`、不合 beta。下一步待维护者选择：
  48 kHz 同条件重跑、裁决判据口径，或继续改估计器。
