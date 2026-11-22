# M5 实验结果与证据索引

这些是历史测量和判据，不是 v0.6.0 正式产物的复测。先区分**声学输出**、**采集到提交**、**Android 平台播放缓冲**和 **native PCM ring / jitter buffer**：它们不是同一个指标。以下链接是 Git 已跟踪的摘要；完整采集文件未打包，不能只凭仓库重算所有结果。

## 延迟、缓冲和稳定性

| 实验 | 结果及能支持的结论 |
| --- | --- |
| [L1 阶段 1：采集→提交](../wire-protocol-v2/l1-handoff/l1-results.md)（11-19，Windows + Android 真机 API 31，48 kHz，实验埋点 `f073e07`） | 到 `AudioTrack` 提交的 p50/p95/p99 为 **96.8 / 135.1 / 144.7 ms**；ring p50 46 ms（9–69 ms），上游 p50 约 54.6 ms。仅 18 个稳态样本，采集日志按秒插值，单点误差可达 ±24 ms。**约 225 ms 是推算，不是出声时延**；不能据此判定规范的 p95/p99 预算。 |
| [AudioTrack 缓冲档位](../wire-protocol-v2/evidence/m5-buffer-experiment/results.md)与[低缓冲 / ring 追测](../wire-protocol-v2/evidence/m5-buffer-experiment/low-buffer-and-ring.md)（11-18/19，同一 MIUI/API 31 真机） | 请求 **100 / 20 / 30 / 40 ms**，实际约 **128 / 42 / 64 / 64 ms**；每档短听连续，但未听出 128→42 ms 的延迟收益，不是严格 A/B。强制 21 ms 有爆音，10 ms 听测 `NOT RUN`；fast path 仍是 42 ms，AudioFlinger 的 58–80 ms 仅为平台诊断。另一轮 PCM ring 采样 0–20 ms，不能与 L1 的 46 ms 合并成固定积压。 |
| M5-02 [42 ms 听测](../wire-protocol-v2/evidence/m5-02/soak-dell-ws/result.md)和[早期 A/B 边界](../wire-protocol-v2/evidence/m5-02/device-verification.md)（v0.5.0-beta Server + 实验 APK `f073e07`） | 真机 ≥5 分钟主观听测 `PASS`；30 分钟连续**听测** `WAIVED`，仅留 7 个平台采样点。生产提交 `d03865b` 改用平台最小值（证据记录了与听测路径的等价性），不是把所有设备的缓冲固定为 42 ms。早期 A/B 条件不同且 42 ms 埋点臂因锁屏作废；MIUI underrun 计数不可靠。 |
| M5-03 / M5-05 [实现及两轮审查](../wire-protocol-v2/map.md#decisions-so-far) | M5-03 规范目标 **60 ms / 硬限 200 ms** 未改，Windows 用例 A 的短时主观听感 `PASS` 不能代表 60 分钟。M5-05 合入 packetizer 时间戳重锚和 `media.drops` 计数；WASAPI、pacer、网络分项归因与调优 `NOT RUN`。 |
| M5-01 / M5-07 | [声学回环基线](../wire-protocol-v2/issues/m5-01-end-to-end-latency-baseline.md)与[正式 60 分钟 soak 及 p95/p99 预算验证](../wire-protocol-v2/issues/m5-07-60min-soak.md)为 `WAIVED`，不是 `PASS`。 |

## 漂移：按时间读，不把不同口径混算

1. **M5-04 / M5-08，11-20。** [Windows 首测](../wire-protocol-v2/evidence/m5-windows-evidence-2026-11-20/README.md)和 [`431997b` 复测](../wire-protocol-v2/evidence/m5-windows-evidence-2026-11-20/evidence-2/RESULT2.txt)中，A 主观 `PASS`，B 当时为 `FAIL`。[M5-08 诊断](../wire-protocol-v2/evidence/m5-08/README.md)修了 pacer 小数帧进位、underrun 门控和漂移窗口，并增加 `media.stats`。别把累计 5,576 与复测增量 125 相除来算欠载降幅：计数门控也变了。B 历史单周期 **+7 次 re-anchor、删 64,672 帧**；[M5-10](../wire-protocol-v2/issues/m5-10-reanchor-burst-investigation.md)仍 `needs-triage`，后续未复现不等于根因已修。
2. **M5-09 / M5-11，11-21。** [B3](../wire-protocol-v2/evidence/m5-09/RESULT-B3.md)（`cb28095` / `431997b`）未再见突跳，但按旧逐窗口径 `FAIL`。[M5-11 首轮](../wire-protocol-v2/evidence/m5-11/RESULT-WINDOWS-M5-11.md)有约 4× 采集帧计数率且格式未记录，不能直接与 B3 对比；[48 kHz 对齐复测](../wire-protocol-v2/evidence/m5-11/RESULT-WINDOWS-48K.md)仍见 **−1,807…+723 ppm** 逐窗极值，原「硬件播放时钟可降低噪声」假设未证实。48 kHz 报告原判是 `FAIL`；按[维护者后续裁决](../wire-protocol-v2/evidence/m5-09/windows-retest.md)，以长期快照中位数/均值（−1 / −9.6 ppm）、无灾难离群、覆盖与有界等条件评价，M5-09 用例 B 才是 `PASS`。10 秒原始诊断仍保留。
3. **M5-12，11-22。** [host 模型](../wire-protocol-v2/evidence/m5-12/host-drift-saturation.md)显示，180 秒残差基线 + 20 秒阻尼后，噪声下 ±75 ppm 跟踪 `PASS`，误差绝对值不超过 12.8 ppm。Windows 11 Server `431997b` + Android API 31/arm64 debug APK `4120cdd` 的[真机判定](../wire-protocol-v2/evidence/m5-12/RESULT-WINDOWS.md)为 `PASS`：48 kHz 判定窗 919.5 秒、顶轨 **0/180**，缓冲 **0–20 ms** 无持续增长，残差中位 **1 ppm** / 均值 **0.92 ppm**，修正最大 **5 ppm**，Server 无待发积压和丢弃。暖机 **17/33** 顶轨单列；旧实验 **118/186** 看的是 ±100 ppm，新判据看 48 kHz 可达上限 **±4 Hz（约 ±83.3 ppm）**，不能直接相减。真机只证明这一路由、低残差条件下噪声未持续传给执行器；听感 A/B、真机注入已知 ppm、实际接受速率回读仍为 `NOT RUN`，没有 `rejected` 日志不等于回读验证。

## 网络损伤和解析验证

[M5-06 矩阵](../wire-protocol-v2/evidence/m5-matrix/matrix.md)逐行区分应用层模拟、进程内队列/pacer、状态机和真实网络。可模拟的丢包、重排、去重、抖动、拥塞、MTU、漂移、空闲与断连行只在各自 host 层 `PASS`；真实网络、移动设备时钟注入、传输层去重/动态 MTU、真实重连等不能由这些结果替代，仍 `NOT RUN`。[解析器验证](../wire-protocol-v2/evidence/m5-matrix/fuzz.md)共 **70,000** 个带固定种子的 host 用例，`PASS` 只覆盖记录中的解析和状态机断言。

查原件保管边界见[归档候选](archive.md)，查本次允许的承诺见[放行清单](release-gate.md)。
