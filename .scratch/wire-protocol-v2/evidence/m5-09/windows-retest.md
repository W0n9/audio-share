# M5-09：Windows + Android 真机用例 B

状态：**NOT RUN**。这是工装说明，不是复测结果。历史 `431997b` 的结论仍为 [FAIL](../m5-windows-evidence-2026-11-20/evidence-2/RESULT2.txt)。请使用下方 `71e0155` APK（M5-11 硬件播放时钟）；`cb28095` 与 `4d23f89` 均不适用于本次验收。

## 产物与环境

1. 候选产物如下。下载或转移后核验 SHA-256，不要把 `431997b` 的 APK 当成修复版。以下文件仅在本地 `.build-artifacts/`，未发布。若候选代码再改，更新对应构建与哈希。

   | 产物 | 构建提交与版本 | 本地路径 | SHA-256 |
   |---|---|---|---|
   | Windows GUI Server（复用，本轮未改 Server） | `431997b` / `0.5.0-beta` | `.build-artifacts/m5-windows-gui-431997b/AudioShareServer.exe` | `ee7a35a4bccad6bc09344fd7672c703e6e5466f60859ffb4ff6911fb69bd94d8` |
   | Android arm64 debug APK | `71e0155`（M5-11 合入后）/ `0.2.0-beta`（code 2） | `.build-artifacts/m5-11-71e0155/app-debug-arm64.apk` | `123e04fbd8c5e9ea15556bdfd38357a3ad4c8e93f0fc7275e615537a29d223e7` |

   APK 构建 **PASS**：`assembleDebug -PandroidAbis=arm64-v8a`（JDK 21、`--offline`）重新编译了 native 库，`apksigner verify` 确认签名，native 库含 `playback.drift_interval` 与 `playback.drift_clock` 标记。不是安装/真机 `PASS`。`cb28095` 的 APK 已被取代，不再使用。
2. 使用 Windows GUI Server、单台 Android 真机、与历史 B 可比的正常 LAN、屏幕常亮。本轮不注入网络损伤或时钟偏移。记录 OS、Android API、设备型号（不记录序列号）、ABI、构建模式和采样率。
   **采集格式必须先固定为 48 kHz / 2 声道**：Windows「声音设置 → 更多声音设置 → 播放 → 默认设备 → 属性 → 高级 → 默认格式」。开测前记录该设置（截图或文字），并在窗口内核对 `consumed_frames` 折算率约 48,000 计数/s、`pacer_calls_per_sec` 约 300/s。若仍出现约 4 倍（≈192,000/s、pacer≈1,200/s），说明格式未生效，本轮按 `NOT RUN` 处理，不与历史 B 比较。
3. 开始记录日志后进入一个新播放会话，连续播放至少 5 分钟。以该会话的 Server 与客户端日志对齐窗口；重连或误触导致的废弃段不拼入窗口。

## 采集与判定

- 客户端收集 `playback.drift_clock`、`playback.drift_interval`、`playback.drift`、`playback.drift_window`、`playback.trim` 和缓冲/损失快照；Server 收集 `media.stats` / `media.drops`。**必须记录 `source=`**：`hardware` 才代表本次测的是硬件播放时钟；若为 `accepted`（`head_give_up_events > 0` 或启动回退），说明 `getTimestamp`/播放头在该设备不可用，本轮**不能**用来证明漂移精度，按该原因记 `NOT RUN`/`FAIL` 并保留诊断，不能当通过。按同一会话和 stream `instance` 核对区间连续性，记录 `start_ms/end_ms`、`reports/valid_estimates`、`min_ppm/max_ppm` 和 `max_report_gap_ms`。`discontinuous_intervals` 计被拒绝的相邻采样区间，不计独立停滞或 re-anchor 事件。
- 区分首次 10 s 窗口预热和后续测量缺口。检查从首次有效估计到窗口结束的诊断覆盖：除了明确记载的暖机/重建区间，`reports` 应非零、`valid_estimates` 应等于 `reports`，且不得有缺失区间或未解释的异常 `max_report_gap_ms`。明确记录所有重建与时长，不把 0 个估计、`none` 或缺日志当作 0 ppm；没有足够稳态覆盖或重建导致无法完成有效验收，不判 PASS。
- **判据（2026-11-21 维护者裁决，取代原「每个区间极值都在 ±100 ppm」）**，以规范 §8/§11.2 为准：
  - 长期快照的**中位数与均值**在 ±100 ppm 内；
  - **无灾难性离群**：任何有效估计不得出现历史失效量级（|ppm| ≥ 10,000）；
  - 覆盖完整（上一条）；缓冲有界；re-anchor 触发与计数正常（下一条）；
  - `correction_ppm` 始终在 ±100 ppm 内；**顶轨比例只作质量指标**，由 [M5-12](../../issues/m5-12-drift-correction-saturation.md) 跟踪，不单独否决 B。
  - 逐窗 `min_ppm/max_ppm` 仍全量记录用于诊断与回归，但单窗极值不作否决项：10 s 窗口在本平台的量化误差约 10–20 ms（≈1,000–2,000 ppm），±100 ppm 需要小于 1 ms 的位置分辨率。
- 用同一会话的相邻 5 s 快照求 `reanchor_events` 与 `reanchor_frames` 增量，记录最大值及对应原始估计和缓冲数据。不得出现类似历史单周期 `+7` 次的突跳；发现超差或缺样，保留上下文，不事后剔除来判为通过。
- 记录缓冲、Server pending 和丢弃。有异常就保留原始时间窗；不要据单个累计计数推断实际播放或单向延迟。
- 记录结论 `PASS` / `FAIL` / `NOT RUN` 及原因；如果没有有效窗口，不能记作通过。原始日志仅留本地，回传时先移除地址、设备/身份标识和配对内容。

**不由本次覆盖**：真机 ±100 ppm 注入、真实网络损伤、双真机并发、API 26 真机。M5-01 声学基线可独立安排；用例 B 获得真机 `PASS` 后才排 M5-07，并仍需按其工单做 60 分钟 soak 和时延预算验收。
