# M5-11: 漂移估计改用硬件播放位置作为播放侧时钟

**What to build:** 先用 host RED 证明「以 AudioTrack 接受量作为播放时钟」在粗粒度写入下产生数百 ppm 噪声，再改为读取硬件播放进度（`getPlaybackHeadPosition()` 或 `getTimestamp()`），最后用同一判据复测用例 B。

**Blocked by:** —

**Status:** ready-for-agent

**Type:** task

## Evidence

- [B3 复测](../evidence/m5-09/RESULT-B3.md)：稳态原始估计 −645…+532 ppm，67/67 区间至少一端越出 ±100；均值 +0.3、中位数 −40，无灾难性离群；`discontinuous_intervals=0`、re-anchor 0；Server pending 0、drops 0。`correction_ppm` 35/67 顶在 ±100。
- 代码事实：`media_session.rs:663` 的 `submitted_position()` = `anchor + submitted_bytes/frame_bytes`，`submitted_bytes` 来自 Kotlin `recordPlayback(..., submittedBytes, ...)`，即 **`AudioTrack.write` 已接受**的字节，不是硬件播放进度。
- 量级：10 s 窗口下单个 2052 帧块约合 4,275 ppm，一次回归后残留在数百 ppm，与观测同量级（见 [host 证据](../evidence/m5-09/host-drift.md) 的批提交模型）。
- 以上是**待验证假设**，不是已确认根因；B3 没有逐报告输入与写入粒度，不能回放。

## Requirements

- [ ] **先 RED**：host 上用双 48 kHz、按 2052 帧（另加更细/更粗一档）批量接受、100 ms 报告、注入已知漂移，复现「逐窗残差超 ±100、长期均值正确」；新时钟输入的同场景必须转绿。不能复现就停在报告，不盲改。
- [ ] 播放侧时钟改取真实硬件进度：Kotlin 端读 `AudioTrack.getPlaybackHeadPosition()`（32 位无符号帧计数，会回绕）或 `getTimestamp()`（framePosition + nanoTime）。
- [ ] JNI/报告契约扩展要向后兼容、单位明确（帧 vs 字节）、失败时退回现路径并记录诊断；不得静默把「读不到」当成 0。
- [ ] 位置记账要一致：已接受的 trim/re-anchor 删除、插入的静音、应用 ring 的软修剪，都会让硬件进度与接受量分离；`drift_playback_step` / `anchor` 语义必须一并考虑，不能只换取值而留下双重补偿。
- [ ] 不改协议、60/200 ms、±100 ppm 上限；不钳位原始 `drift_ppm`；不改变 re-anchor 触发条件与计数；§7.4 不动。
- [ ] **修正饱和纳入本票**：修复后验证稳态 `correction_ppm` 不再被噪声长期顶在 ±100。若需调平滑步长/死区，用不同噪声水平的 host 测试证明稳定性，不能靠放宽判据。
- [ ] 真机 B 复测用同一判据（原始估计 ±100、诊断覆盖、re-anchor、缓冲/服务端）。没有真机 PASS 不写 `resolved`，不合 beta。

## Notes

- 相关代码：`android-client-v2/native/src/{drift.rs,media_session.rs,jni.rs,session.rs}`、`android-client-v2/app/src/main/java/dev/audioshare/v2/MediaPlayer.kt`。
- 首次报告前播放头为 0；AudioTrack 重建/换轨后需要重新锚定；跨 32 位回绕要显式处理。
- 硬件播放头含平台自身 42 ms 缓冲，这是可接受的延迟，不影响 10 s 斜率；但**不要**因此去改 `buffer_current_ms` 的口径。
- 若 host RED 证明噪声另有来源（例如采集侧时间戳量化），在同一票内改记证据并重新评估，不要套用本票的修法。
