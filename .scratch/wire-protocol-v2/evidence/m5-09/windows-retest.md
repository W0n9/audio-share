# M5-09：Windows + Android 真机用例 B

状态：**NOT RUN**。这是工装说明，不是复测结果。历史 `431997b` 的结论仍为 [FAIL](../m5-windows-evidence-2026-11-20/evidence-2/RESULT2.txt)。下方 `4d23f89` APK 是首版候选，不含 `playback.drift_interval`，**不能用于下方的最终验收**，等待审查修复合入后重建。

## 产物与环境

1. 候选产物如下。下载或转移后核验 SHA-256，不要把 `431997b` 的 APK 当成修复版。以下文件仅在本地 `.build-artifacts/`，未发布。若候选代码再改，更新对应构建与哈希。

   | 产物 | 构建提交与版本 | 本地路径 | SHA-256 |
   |---|---|---|---|
   | Windows GUI Server（复用，本轮未改 Server） | `431997b` / `0.5.0-beta` | `.build-artifacts/m5-windows-gui-431997b/AudioShareServer.exe` | `ee7a35a4bccad6bc09344fd7672c703e6e5466f60859ffb4ff6911fb69bd94d8` |
   | Android arm64 debug APK | `4d23f89` / `0.2.0-beta`（code 2） | `.build-artifacts/m5-09-4d23f89/app-debug-arm64.apk` | `1a61d62bc71560d9c6b3f06ca48f96f0e32b9bfc7a3ac00f07a3b14e0efa8926` |

   APK 构建 **PASS**：`assembleDebug -PandroidAbis=arm64-v8a`（JDK 21、`--offline`）重新编译了 native 库，`apksigner verify` 及 `aapt dump badging` 确认签名和 ABI。不是安装/真机 `PASS`。
2. 使用 Windows GUI Server、单台 Android 真机、与历史 B 可比的正常 LAN、屏幕常亮。本轮不注入网络损伤或时钟偏移。记录 OS、Android API、设备型号（不记录序列号）、ABI、构建模式和采样率。
3. 开始记录日志后进入一个新播放会话，连续播放至少 5 分钟。以该会话的 Server 与客户端日志对齐窗口；重连或误触导致的废弃段不拼入窗口。

## 采集与判定

- 客户端收集 `playback.drift_interval`、`playback.drift`、`playback.drift_window`、`playback.trim` 和缓冲/损失快照；Server 收集 `media.stats` / `media.drops`。按同一会话和 stream `instance` 核对区间连续性，记录 `start_ms/end_ms`、`reports/valid_estimates`、`min_ppm/max_ppm` 和 `max_report_gap_ms`。`discontinuous_intervals` 计被拒绝的相邻采样区间，不计独立停滞或 re-anchor 事件。
- 区分首次 10 s 窗口预热和后续测量缺口。检查从首次有效估计到窗口结束的诊断覆盖：除了明确记载的暖机/重建区间，`reports` 应非零、`valid_estimates` 应等于 `reports`，且不得有缺失区间或未解释的异常 `max_report_gap_ms`。明确记录所有重建与时长，不把 0 个估计、`none` 或缺日志当作 0 ppm；没有足够稳态覆盖或重建导致无法完成有效验收，不判 PASS。
- 用**所有区间内产生的有效估计**的 `min_ppm/max_ppm` 判定 ±100 ppm，包括部分有效的重建区间；不能只看每 5 s `playback.drift` 的末值。记录样本数和极值；若只从 `playback.drift` 快照算中位数，明确标为快照中位数，仅作诊断。±100 ppm 是**本次基线试验**的原始估计判据，不是协议对物理时钟差的限幅。另验实际 `correction_ppm` 始终在 ±100 ppm 内。
- 用同一会话的相邻 5 s 快照求 `reanchor_events` 与 `reanchor_frames` 增量，记录最大值及对应原始估计和缓冲数据。不得出现类似历史单周期 `+7` 次的突跳；发现超差或缺样，保留上下文，不事后剔除来判为通过。
- 记录缓冲、Server pending 和丢弃。有异常就保留原始时间窗；不要据单个累计计数推断实际播放或单向延迟。
- 记录结论 `PASS` / `FAIL` / `NOT RUN` 及原因；如果没有有效窗口，不能记作通过。原始日志仅留本地，回传时先移除地址、设备/身份标识和配对内容。

**不由本次覆盖**：真机 ±100 ppm 注入、真实网络损伤、双真机并发、API 26 真机。M5-01 声学基线可独立安排；用例 B 获得真机 `PASS` 后才排 M5-07，并仍需按其工单做 60 分钟 soak 和时延预算验收。
