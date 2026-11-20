# M5-09：Windows + Android 真机用例 B

状态：**NOT RUN**。这是工装说明，不是复测结果。历史 `431997b` 的结论仍为 [FAIL](../m5-windows-evidence-2026-11-20/evidence-2/RESULT2.txt)。

## 产物与环境

1. 写下 `integration/m5` 的构建提交、Windows GUI Server 和 arm64 APK 的版本及 SHA-256。不要把 `431997b` 的 APK 当成修复版。若仅 Android 端变更且复用原 Server，应单独写 Server 的真实构建提交与哈希。
2. 使用 Windows GUI Server、单台 Android 真机、与历史 B 可比的正常 LAN、屏幕常亮。本轮不注入网络损伤或时钟偏移。记录 OS、Android API、设备型号（不记录序列号）、ABI、构建模式和采样率。
3. 开始记录日志后进入一个新播放会话，连续播放至少 5 分钟。以该会话的 Server 与客户端日志对齐窗口；重连或误触导致的废弃段不拼入窗口。

## 采集与判定

- 客户端收集 `playback.drift`、`playback.trim` 和缓冲/损失快照；Server 收集 `media.stats` / `media.drops`。`playback.drift` 的空白期不当作 0 ppm。
- 记录稳态原始 `drift_ppm` 的样本数、极值、中位数，以及所有窗失效/重建区间；中位数只用于诊断。满窗后的稳态原始估计应在 ±100 ppm 内，这是**本次基线试验**的判据，不是协议对物理时钟差的限幅。另验实际 `correction_ppm` 始终在 ±100 ppm 内。
- 用同一会话的相邻 5 s 快照求 `reanchor_events` 与 `reanchor_frames` 增量，记录最大值及对应原始估计和缓冲数据。不得出现类似历史单周期 `+7` 次的突跳；发现超差或缺样，保留上下文，不事后剔除来判为通过。
- 记录缓冲、Server pending 和丢弃。有异常就保留原始时间窗；不要据单个累计计数推断实际播放或单向延迟。
- 记录结论 `PASS` / `FAIL` / `NOT RUN` 及原因；如果没有有效窗口，不能记作通过。原始日志仅留本地，回传时先移除地址、设备/身份标识和配对内容。

**不由本次覆盖**：真机 ±100 ppm 注入、真实网络损伤、双真机并发、API 26 真机。M5-01 声学基线可独立安排；用例 B 获得真机 `PASS` 后才排 M5-07，并仍需按其工单做 60 分钟 soak 和时延预算验收。
