# M5-11 Windows + Android 真机用例 B 复测结果（APK 71e0155 / Server 431997b）

生成时间：2026-11-21 09:30（本地 UTC+8）
执行者：Claude（部署、采集、日志分析）+ 用户（连接、配对、播放、音源）
本文件为脱敏版，可直接回传；原始日志、截图与配对内容仅保留在本地
`evidence-2026-11-21-m5-11-B\`。地址、client_id、序列号与配对内容均未写入本文件。

---

## 环境 / 产物校验

- Windows：11 专业工作站版 10.0.26300，时区 UTC+8。
- Server：`AudioShareServer.exe` 0.5.0-beta（GUI 便携版），
  SHA-256 `ee7a35a4bccad6bc09344fd7672c703e6e5466f60859ffb4ff6911fb69bd94d8`，
  与清单一致。运行目录为本地可写目录（`config.ini`、`server.log` 在此生成），
  本轮未改 `config.ini`（`WhenAppStart=0` 保持默认）。
- Android APK：`app-debug-arm64.apk`，SHA-256
  `123e04fbd8c5e9ea15556bdfd38357a3ad4c8e93f0fc7275e615537a29d223e7`（19,723,829 B），
  与清单一致。`adb install -r` 后重新拉取已安装 base APK 复算哈希，仍为
  `123e04fb…`，`lastUpdateTime=2026-11-21 09:19:33` 本地 —— 确认本轮跑的是
  M5-11 构建，不是 `cb28095`（安装前已确认旧包为 `a6838e3b…`）或 `4d23f89`。
  包：`dev.audioshare.v2` 0.2.0-beta（versionCode 2），minSdk 26 / targetSdk 36。
- 手机：M2007J22C（cannon），Android 12 / API 31，arm64-v8a，
  MIUI V14.0.6.0.SJECNXM；adb 无线连接。
  输出路由 `AUDIO_DEVICE_OUT_SPEAKER`，48 kHz；无蓝牙/耳机。
- 网络：与 Windows 同一 LAN，未注入网络损伤或时钟偏移。
- 屏幕常亮：采集开始时设备 `screen_off_timeout` 已为最大值
  （2147483647），另置 `svc power stayon true`；测后已恢复原值并置
  `stayon false`。
- 音源：用户在 Windows 上播放的媒体；窗口内无服务端断流迹象
  （`consumed_frames` 持续增长、`pending_frames=0`），但该计数不能证明音源
  始终有声。具体音源由用户提供，本记录未固定其来源类型。

### 与上一轮（B3）可比性相关的观察

- 服务端 `media.stats` 的 `consumed_frames` 增量折算为 **191,944.7 帧计数/s**
  （由帧计数增量得出，不是已确认的声卡采样率；`pacer_calls_per_sec`
  1,197–1,202），而 B3 为 **47,997 帧/s**（pacer 299–300/s），正好约 **4×**。
  `server.log` 未记录采集格式，因此无法仅凭日志区分「192 kHz 单/立体声」与
  「48 kHz 4 声道」；可确定的是采集侧帧计数率相对 B3 变为 4 倍。
- 这是日志可见的主要差异（现有采集不能证明它是唯一差异）；是否影响漂移估计
  不在本记录判定范围内。若需要与 B3 严格同条件的结论，建议把 Windows 默认
  播放设备格式设为 48 kHz / 2 声道后重跑一轮（本轮未执行该重跑）。

## 会话与窗口

- 服务端会话：
  - session 1：配对后立即 `session.closed code=701`（开放 238 ms，无
    `media.stats`，播放前噪声，不纳入判定）。
  - **session 2：判定会话**，`instance=1`，accepted 于 09:21:18.166 本地。
- 整窗：**09:21:19 – 09:27:40 本地（6 分 21 秒）**，用户报告连续播放满 5 分钟。
- 稳态分析窗（剔除首次 10 s 暖机）：**09:21:35.243 – 09:27:35.993 本地**，
  `start_ms=10,497` → `end_ms=376,247`，**365.75 s（6 分 05.75 秒）**。
- 暖机区间（单列，不计入判据）：09:21:19.992（2 报告/0 有效）、
  09:21:24.993（49/0）、09:21:30.243（52/3）。

## 客户端观测

### `playback.drift_clock`（本轮硬性检查）

- 窗口内 74 行，**`source` 全程 `hardware`**，无任何 `accepted` 行。
- `hardware_reports` 增量 **3,722**；`hardware_missing_reports=0`、
  `accepted_fallback_reports=0`、`head_reanchor_events=0`、
  `head_give_up_events=0`。
- 结论：该设备 `getTimestamp()`/播放头本轮可用，未触发回退；因此本次
  不适用清单中「`source=accepted` 只能记 NOT RUN/FAIL」的例外，但硬件时钟
  本身的精度未达标（见下）。

### `playback.drift_interval`（稳态窗）

- 区间数 **71**；`reports` 合计 **3,621**，`valid_estimates` 合计 **3,621**
  （完全相等）；无 0 有效区间；相邻区间 `start_ms == end_ms` 连续。
- `max_report_gap_ms` 最大 **106**；`discontinuous_intervals` 全程 **0**。
- 全区间极值：**min_ppm = −1,390，max_ppm = +1,529**。
- **至少一端越出 ±100 的区间：71 / 71（100%）**，且不是个别尖峰：
  每个 5 s 区间都出现数百至上千 ppm 的单报告极值。
- 无 `drift_window` 重建事件。

### `playback.drift`（每 5 s 快照，仅诊断）

- 72 个快照；`drift_ppm` 范围 −1,294 … +1,209；**中位数 +66，均值 +26**。
- 快照本身落在 ±100 内的仅 **12 / 72**。
- `correction_ppm` 始终在 ±100 内，但有 **50 / 72** 个快照顶在 ±100 轨上。
- `buffer_current_ms`：**23 … 61**，中位 **40**（`buffer_target_ms=60`）——
  仍低于目标，但比 B3（0–26，中位 10）高。
- `discontinuous_intervals` 最大 **0**。

### 缓冲 / 干预

- **re-anchor：0 次、0 帧**；session 2 内没有任何 `playback.trim` 行。
- **soft trim：0**。
- concealment：窗口开始后到 09:22:00 之间出现 **1 次 drift concealment
  （1,920 帧，约 40 ms）**；此后计数不再增长，`gap=0`、`underrun=0`、
  `reanchor=0`。该事件发生在前 41 s 内，保留记录、不事后剔除。
- 无 `drift_window` 重建。

## 服务端观测

- `media.stats` 76 条（09:21:23 – 09:27:38 本地），`session_id=2` / `instance=1`
  唯一。
- `pending_frames` 全程 **0**（min/max/avg 均 0）。
- `pacer_calls_per_sec` **1,197 – 1,202**。
- `consumed_frames` 增量 72,038,400 / 375.3 s ≈ **191,944.7 帧计数/s**
  （帧计数率，不是已确认的声卡采样率；见上方可比性说明）。
- `media.drops` **0 条**。

## 判定

| 判据（windows-retest.md） | 观测 | 结论 |
|---|---|---|
| 必须记录 `source`；`hardware` 才可判漂移 | 74 行全 `hardware`；missing/fallback/give_up 均 0 | **PASS（前置条件）** |
| 稳态原始估计在 ±100 ppm 内（全区间极值） | −1,390 … +1,529；71/71 区间越界 | **FAIL** |
| 覆盖：`reports>0`、`valid_estimates==reports`、无异常 gap | 3,621/3,621；gap≤106 ms；continuity OK；`discontinuous=0` | PASS |
| 单 5 s 周期无 +7 次级 re-anchor 突跳 | re-anchor 0 次 / 0 帧；无 trim 行 | PASS |
| `correction_ppm` 始终 ±100 内 | 是，但 50/72 顶轨 | PASS（附注） |
| 记录缓冲 / Server pending / drops | buffer 23–61 ms（目标 60，中位 40）；pending 0；drops 0 | 记录，buffer 偏低 |

**用例 B 结论：FAIL** —— 唯一不达标项是稳态原始漂移估计超出 ±100 ppm，且是
持续性的（71 个区间无一完全落在 ±100 内），不是个别尖峰。本轮
`source=hardware`，所以这不是「设备播放头不可用」的回退情形；按现有证据，
M5-11 的硬件播放时钟在该真机上没有把逐窗精度带到 ±100 ppm 以内，极值
（−1,390…+1,529）反而比 B3 的 accepted 路径（−645…+532）更大。

## 与清单 / 上一轮的偏差（如实记录）

1. **采集帧计数率约为 B3 的 4 倍**（191,944.7 vs 47,997 帧计数/s；pacer
   约 1,200 vs 300/s）。该值由 `consumed_frames` 增量折算，不是已确认的
   声卡采样率；`server.log` 未记录采集格式，无法区分 192 kHz 与 4 声道
   48 kHz。这是本轮最重要的可比性注记；本轮未改 Windows 音频格式，也未
   据此改判。
2. 判定窗口为用户报告结束时刻截断（09:27:40），实际有效播放 6 分 21 秒，
   超过要求的 5 分钟；稳态窗 6 分 05.75 秒。
3. session 1 在播放前 701 关闭（238 ms），无 `media.stats`，未纳入判定。
4. Server 运行目录为本地可写目录（`<工作目录>\m5-windows-gui-431997b\`），
   带匹配的防火墙入站规则；未做 UAC 提权。
5. 配对为本次新做（QR），手机端旧凭据被新记录取代；配对内容不入库。
6. 服务端 `server.log` 时间戳为 **UTC**；本文件未标注时区的时间均为本地
   UTC+8，换算已核对（+8 h）。
7. 采集冻结后服务端与手机播放仍在运行（与 B3 相同），09:27:40 之后的追加
   数据不在本次判定内。
8. 本轮未记录具体音源名称；`media.stats` 持续增长、`pending=0`，只能说明
   无服务端断流迹象，不能证明音源始终有声。

## 建议（回传后由 Linux 侧决定）

1. 若认为 4× 帧计数率构成不可比因素：把 Windows 默认播放设备格式改为
   48 kHz / 2 声道后重跑一轮；该重跑只是可选验证，不能豁免本轮 FAIL，也
   不据此改判。
2. 若 48 kHz 重跑仍复现：按 `diagnosing-bugs` 先建可复现的 RED，重点看
   真机 `getTimestamp()`/播放头的逐报告年龄与量化步长，不调阈值、不改判据。
3. M5-11 保持 `ready-for-human`，不写 `resolved`；M5-09 不关闭；不合 beta。

## NOT RUN（不得写成 PASS）

- 真机 ±100 ppm 注入矩阵
- 真实网络损伤 / 用例 C
- 双真机并发、API 26 真机、M5-07 的 60 分钟 soak
- 本轮未做主观听测判定（用例 B 不作主观项）
- 48 kHz 同条件重跑（本轮未执行，见上）

## 证据文件（本地 `evidence-2026-11-21-m5-11-B\`，未脱敏，仅留本地）

- `session-raw-logcat.txt`：完整未过滤 logcat（2.4 MB，09:20:04 起）
- `server-full.log`：服务端完整日志（09:27:40 冻结时副本）
- `audio_flinger-before.txt` / `audio_flinger-mid.txt` / `audio_flinger-after.txt`
- `phone-mid.png` / `phone-end.png` / `screen-01-server-launched.png`
- `device-info.txt` / `artifact-hashes.txt` / `device-settings-before.txt` /
  `timeline.txt`
- `analysis.json` / `analyze_m5_11.py`：本轮解析脚本与输出

## 收尾状态

- logcat 抓取已停止；`screen_off_timeout` 已恢复原值，`svc power stayon=false`。
- Server 进程与手机播放未由本会话停止（采集冻结时仍在运行）。
- 未改任何代码、未执行任何 git 操作；`integration/wire-protocol-v2-beta`
  与 `v0.5.0-beta` 均未动。
