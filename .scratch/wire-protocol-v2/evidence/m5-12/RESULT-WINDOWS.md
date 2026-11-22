# M5-12 Windows 真机复测结果（APK 4120cdd / Server 431997b）

生成时间：2026-11-22 10:47（本地 UTC+8）
执行者：Claude（部署、采集、日志分析、记录）+ 用户（GUI 操作、配对、播放、停止）
本文件为脱敏版，可直接回传；原始日志、截图与配对内容仅保留在本地
`evidence-2026-11-22-m5-12-4120cdd\`。地址、client_id、序列号、指纹与配对内容均未写入本文件。

---

## 环境 / 产物校验

- Windows：11 专业工作站版 10.0.26300，时区 UTC+8。
- Server：`AudioShareServer.exe` 0.5.0-beta（GUI 便携版），
  SHA-256 `ee7a35a4bccad6bc09344fd7672c703e6e5466f60859ffb4ff6911fb69bd94d8`，
  与清单一致；运行目录可写，`config.ini` 未改（`endpoint=default`、`WhenAppStart=0`）。
- Android APK：`app-debug-arm64.apk`，SHA-256
  `9a039a042e2f934e4b17334150273e72a4a01b5361ce17ea494f903ab25b8fdf`（22,230,471 B），
  与清单一致；安装后拉回 base APK 复算，哈希相同；`lastUpdateTime=2026-11-22
  10:20:27` 本地，确认本轮跑的是 `4120cdd` 构建。
  包：`dev.audioshare.v2` 0.2.0-beta（versionCode 2），minSdk 26 / targetSdk 36。
- 手机：M2007J22C（cannon），Android 12 / API 31，arm64-v8a，MIUI V140；adb 无线连接。
- 网络：与 Windows 同一 LAN，未注入网络损伤或时钟偏移。
- 屏幕常亮：开测时 `stay_on_while_plugged_in` 由 **0 置为 7**（`svc power stayon true`），
  结束后执行 `svc power stayon false`，回读恢复为 **0**。`screen_off_timeout`
  开测前为 `null`，未改动。
- 音源（用户报告）：Windows 上播放的**视频**。日志未显示长静音段：判定窗内仅
  10:37:19 一处 `isLongTimeZeroData`（1 s），重建 0 次、concealment 0。

### 采集格式与执行器上限（本轮前置）

- Windows 默认播放端点：**扬声器 (Steam Streaming Speakers)**，注册表
  `MMDevices\Audio\Render` DeviceFormat 为 **48,000 Hz / 2 声道 / 24-bit**；
  该端点是快照中唯一 `state=1`（active）的渲染端点。与上一轮同条件。
- 客户端实际 AudioTrack：**48,000 Hz / 2 声道 / PCM_FLOAT**
  （`AudioFlinger` 与 `AudioTrack` 日志一致），不是仅凭默认值推定。
- 整数 Hz 执行器上限：48 kHz → **±4 Hz = ±83.3 ppm**。本轮顶轨阈值取
  **`|correction_ppm| ≥ 83`**。
- 开测自检（`media.stats`，10:27:09 – 10:42:25 本地）：`consumed_frames`
  43,955,040 / 919.5 s ≈ **48,000.1 帧计数/s**；`pacer_calls_per_sec` **299–300**；
  `pending_frames` 全程 **0**。

## 会话与窗口

- 服务端会话：
  - session 1：配对后立即 `session.closed code=701`（约 0.2 s，播放前噪声，不纳入判定）。
  - **session 2：判定会话**，`instance=1`，accepted 于 10:24:03.651 本地，
    closed code=701 于 10:42:28.003 本地（用户手动停止）。
- 客户端 `AudioTrack` 建立于 10:24:04.974；首个 `playback.drift` 快照 10:24:16.077。
- **暖机段（单列，不计入判据）**：10:24:16.077 – 10:27:01.078 本地，
  共 **33** 个 `playback.drift` 快照，全部 `residual_ppm=none`（180 s 长基线未满）。
- **判定窗（新定义）**：从首条 `residual_ppm != none` 的快照起，
  **10:27:06.328 – 10:42:25.827 本地**，共 **180** 个快照，
  **919.5 s（15 分 19.5 秒）**，满足「≥8–10 分钟、建议 15 分钟」。
  区间终点取客户端日志的最后一个快照；服务端 session 在 10:42:28.003 关闭
  （用户在 10:37:25 收到停止提示后于此时停止，实际多播约 5 分钟，不影响判据）。

## 客户端观测

### `playback.drift_clock`（硬性前置）

- 窗口内 **215** 行，`source` **全程 `hardware`**，无 `accepted` 行。
- `hardware_reports` 增量 **10,483**（末值 10,484）。
- `hardware_missing_reports=0`、`accepted_fallback_reports=0`、
  `head_reanchor_events=0`、`head_give_up_events=0`。
- 结论：本轮测的是硬件播放时钟，未触发回退。

### `setPlaybackRate` 被拒检查（第一道闸）

- 全量 logcat grep `setPlaybackRate(.*) rejected`：**0 次**。
- 结论：未观测到平台拒绝 SRC。按 handoff 口径，速率修正链路可进入后续判据；
  本检查只能证明「没有被拒日志」，不能单独证明平台一定接受了 SRC。本轮顶轨
  为 0 且残差接近 0，与「修正已生效」一致，未出现 §0.4 所述正反馈顶到上限的现象。

### `playback.drift` 快照（新顶轨定义：`|correction_ppm| ≥ 83`）

| 分段 | 快照数 | 顶轨数 | 顶轨比例 | `residual_ppm` 中位/均值 | `correction_ppm` 最大绝对值 |
|---|---|---|---|---|---|
| **判定窗**（`residual_ppm != none`） | **180** | **0** | **0.0 %** | **1 / 0.92** | **5** |
| 暖机段（`residual_ppm=none`，单列） | 33 | 17 | 51.5 % | — | 83 |
| 整段会话（含暖机） | 213 | 17 | 7.98 % | 1 / 0.92（180 条） | 83 |

- 判定窗 `residual_ppm`：范围 **−3 … +5**，中位 **1**，均值 **0.92**，
  p10/p90 = −1/+3。
- 判定窗 `correction_ppm`：范围 **−2 … +5**，中位 1，均值 0.9；最大绝对值 **5**。
  执行器上限 ±83 从未在判定窗内触及。
- 判定窗 `drift_ppm`（§8 的 10 s 原始诊断）：范围 **−225 … +361**，中位 0.5，
  均值 2.42；**仍越出 ±100，但不传导致执行器**（同上，`correction` 最大 5）。
- 整段会话的 17 个顶轨快照**全部**落在暖机段，判定窗内为 0。

### `playback.drift_interval`（判定窗）

- 区间数 **180**；`reports` 合计 **8,817**，`valid_estimates` 合计 **8,817**
  （完全相等）；无 0 有效区间；相邻区间 `start_ms == end_ms` 连续。
- `max_report_gap_ms` 最大 **117**；`discontinuous_intervals` 全程 **0**。
- 逐窗极值：**min_ppm = −361，max_ppm = +411**。
- 至少一端越出 ±100 的区间 **170 / 180（94.4 %）**。这与判据 2 的预期一致：
  逐窗原始估计仍含量化噪声，但**没有被传导致执行器**。

### 缓冲 / 干预

- 判定窗 `buffer_current_ms`：**0 … 20**，中位 **10**，均值 10.18，
  p10/p90 = 2/20（`buffer_target_ms=60`）。按 5 分钟桶统计：
  0–5 min 均值 10.7、5–10 min 9.6、10–15 min 9.9（末桶仅 4 条、16.5），
  **无持续增长**。
- **re-anchor：0 次、0 帧**；**soft trim：0**；无任何 `playback.trim` 行。
- `playback.concealments`：会话内 **没有任何行**（该事件仅在 `total>0` 时输出），
  即 gap / underrun / drift / reanchor 计数全程为 0。
- `playback.drift_window status=rebuilding`：**0 行**。
- 备注：10:37:19 出现 1 条 `AudioTrack: isLongTimeZeroData ... 1 Seconds`
  （音源短暂静音 1 s），未产生不连续区间、未产生 concealment、未触发重建。

## 服务端观测

- `media.stats` 判定窗内 **184** 条（10:27:09.261 – 10:42:24.989 本地），
  `session_id=2` / `instance=1` 唯一。
- `pending_frames` 全程 **0**（min/max/avg 均 0）。
- `pacer_calls_per_sec` **299 – 300**。
- `consumed_frames` 增量 43,955,040 / 919.5 s ≈ **48,000.1 帧计数/s**。
- `media.drops` **0 条**；服务端无 re-anchor/trim 事件。
- 会话生命周期无异常：仅 session 1 的播放前 0.2 s `code=701` 噪声，
  session 2 正常关闭（用户停止，`code=701`）。

## 判定

| 判据（按 2026-11-22 维护者裁决） | 观测 | 结论 |
|---|---|---|
| 顶轨比例（仅 `residual_ppm != none`；48 kHz：`\|correction_ppm\| ≥ 83`）**< 10 %** | **0 / 180 = 0.0 %** | **PASS** |
| `residual_ppm` 长期接近 0，原始逐窗噪声不传导致执行器 | 中位 1 / 均值 0.92（范围 −3…+5）；原始 −361…+411，`correction` 最大 5 | **PASS** |
| 缓冲 `buffer_current_ms` 不持续增长（目标 60） | 0–20 ms，中位 10，5 分钟桶趋势平稳 | **PASS**（记录；仍低于目标 60） |
| 覆盖与重建：`reports>0`、`valid_estimates==reports`、无异常 gap | 8,817/8,817；gap ≤117 ms；连续；`discontinuous_intervals=0`；重建 0 | **PASS** |
| Server：`pending_frames`、`media.drops`、re-anchor/trim | pending 0；drops 0；re-anchor 0 次/0 帧；soft trim 0 | **PASS** |
| 前置：`drift_clock source=hardware` 全程 | 215 行全 `hardware`；missing/fallback/give_up 均 0 | **PASS（前置）** |
| 前置：`setPlaybackRate rejected` 不出现 | 0 次 | **PASS（前置）** |
| 前置：采样率与执行器上限 | 48 kHz / 2 ch；上限 ±4 Hz = ±83.3 ppm | **PASS（前置）** |

**本轮结论：判定窗内顶轨 0/180（0.0 %），`residual_ppm` 中位 1、均值 0.92，
`correction_ppm` 最大绝对值 5 —— 按 2026-11-22 裁决口径，真机顶轨判据 PASS。**
暖机段顶轨 51.5% 按裁决单列；整段会话含暖机为 7.98%，均不进入判据。

## 适用域说明（避免误判）

- 本轮 48 kHz 的执行器可达上限为 ±83.3 ppm；判定窗内 `correction_ppm` 最大 5，
  从未接近上限，因此**不是**「命令停在物理上限」的 84–100 ppm 情形。
- 真实长期漂移以 `residual_ppm`（闭环残差，180 s 基线）体现，中位 1 ppm、
  范围 −3…+5 ppm，落在速率修正的适用域内。
- 10 s 原始 `drift_ppm` 仍会越出 ±100（本轮 −361…+411），这是被 180 s 基线与
  阻尼过滤掉的量化噪声，**不是**本轮判据的输入；报告它只是为了证明噪声仍存在、
  且没有被传导。

## 与上一轮 48K（RESULT-WINDOWS-48K）的偏差

1. **顶轨比例**：上一轮 **118/186（63 %）** 顶在 ±100（旧实现、旧定义、命令可达
   100 ppm）；本轮 **0/180（0.0 %）** 顶在 ±83（新实现、新执行器上限）。两者定义
   不同，**不能直接相减**，只作量级对照。
2. **原始 10 s 噪声**：上一轮区间极值 −1,807 … +723；本轮 −361 … +411，
   量级更小。本轮仍 170/180 个区间越出 ±100，但 `correction` 最大仅 5，
   噪声未传导。
3. **快照中位数**：上一轮 `drift_ppm` 中位 −1、均值 −9.6（旧实现未平滑）；
   本轮判定窗 `drift_ppm` 中位 0.5、均值 2.42；`residual_ppm` 中位 1、均值 0.92。
4. **缓冲**：上一轮 0–28 ms（中位 10）；本轮判定窗 0–20 ms（中位 10），
   仍低于目标 60 ms，趋势平稳。
5. **窗口长度**：上一轮有声稳态 15 分 50 秒；本轮判定窗 15 分 19.5 秒（暖机另计）。
6. **音源**：本轮为连续播放的视频，停止前未进入长静音段；上一轮视频播完后
   的静音段曾引发 52 次重建，本轮未复现（重建 0 次、concealment 0）。
7. **时间基准**：服务端 `server.log` 为 UTC，本地为 UTC+8；换算已核对。
8. 配对为本次新做（QR）；配对内容不入库。

## NOT RUN（不得写成 PASS）

- 听感 A/B（人工门，维护者判定）。
- 84–100 ppm 适用域外的真机会话（host 用例已有，真机未做）。
- 8 kHz 路由（整数 Hz 下无速率修正步长，依赖整帧干预）。
- 60 分钟 soak、真实网络损伤矩阵 / 用例 C、双真机并发、API 26 真机。
- `setPlaybackRate` 的「实际接受速率」上行（本票范围外，未新增计数器）。
  本轮只能证明没有 `rejected` 日志，不能证明平台回读的实际速率。

## 建议（回传后由 Linux 侧决定）

1. 按上表判定真机顶轨判据 **PASS**，写进工单 Comments 与 `map.md`；
   听感 A/B 继续 `NOT RUN`，不替维护者写 PASS。
2. 不因本轮结果改动 §8 的 ±100 ppm、60/200 ms、re-anchor 触发或协议。
3. 若维护者要区分「估计器问题」与「播放头采样问题」，可在同一构建上增加
   逐报告原始值落盘；本轮日志只有区间 min/max，不需要为此改生产代码。
4. beta 合入与 tag 仍需维护者另行授权；本轮未做任何 git 操作。

## 证据文件（本地 `evidence-2026-11-22-m5-12-4120cdd\`，未脱敏，仅留本地）

- `session-raw-logcat.txt`：完整未过滤 logcat（约 4.5 MB，10:20:52 起）
- `server-full.log`：服务端完整日志（冻结时副本）
- `analysis.json` / `analysis-partial.json` / `analyze_m5_12.py`：解析脚本与输出
- `monitor.txt`：每分钟运行监控（rejected / residual / 重建 / 静音 / 服务端）
- `windows-audio-formats-before.txt`：全部渲染端点格式快照
- `artifact-hashes.txt` / `device-info.txt` / `device-settings-before.txt` /
  `timeline.txt` / `installed-base.apk`
- `desktop-server-launch.png` / `desktop-end.png` / `phone-precheck.png` / `phone-end.png`