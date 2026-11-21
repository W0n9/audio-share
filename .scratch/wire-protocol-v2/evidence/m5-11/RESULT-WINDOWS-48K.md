# M5-11 Windows 48 kHz 同条件复测（用例 B）结果（APK 71e0155 / Server 431997b）

生成时间：2026-11-21 23:40（本地 UTC+8）
执行者：Claude（部署、采集、日志分析、记录）+ 用户（设置确认、配对、播放、音源）
本文件为脱敏版，可直接回传；原始日志、截图与配对内容仅保留在本地
`evidence-2026-11-21-m5-11-48k\`。地址、client_id、序列号、指纹与配对内容均未写入本文件。

---

## 环境 / 产物校验

- Windows：11 专业工作站版 10.0.26300.0，时区 UTC+8。
- Server：`AudioShareServer.exe` 0.5.0-beta（GUI 便携版），
  SHA-256 `ee7a35a4bccad6bc09344fd7672c703e6e5466f60859ffb4ff6911fb69bd94d8`，
  与清单一致；运行目录为可写目录，`config.ini` 未改（`endpoint=default`、
  `WhenAppStart=0`）。
- Android APK：`app-debug-arm64.apk`，SHA-256
  `123e04fbd8c5e9ea15556bdfd38357a3ad4c8e93f0fc7275e615537a29d223e7`（19,723,829 B），
  与清单一致。安装态 base APK 已拉回复算，哈希相同；`lastUpdateTime=2026-11-21
  09:19:33` 本地，确认本轮跑的是 `71e0155` 构建。
  包：`dev.audioshare.v2` 0.2.0-beta（versionCode 2），minSdk 26 / targetSdk 36。
- 手机：M2007J22C（cannon），Android 12 / API 31，arm64-v8a，
  MIUI V14.0.6.0.SJECNXM；adb 无线连接。
- 网络：与 Windows 同一 LAN，未注入网络损伤或时钟偏移。
- 屏幕常亮：`screen_off_timeout` 开测前即为 2147483647；采集期间另置
  `svc power stayon true`，结束后恢复（`stay_on_while_plugged_in=0`）。
- 音源（用户报告）：Windows 上播放的**视频**；用户确认视频已播完，
  结束时刻由日志推定为 **23:10:36**。

### 采集格式（本轮目标条件）

- Windows 默认播放端点：**扬声器 (Steam Streaming Speakers)**，注册表
  `MMDevices\Audio\Render` DeviceFormat 为 **48,000 Hz / 2 声道 / 24-bit**
  （WAVE_FORMAT_EXTENSIBLE）。GUI「高级」页显示「24 位, 48000 Hz (录音室音质)」，
  截图本地留存（`windows-default-format.png`）。
- 开测自检（`media.stats`，22:54:39.897 – 22:54:54.907 本地）：
  `consumed_frames` 240,000 → 960,480，增量 720,480 / 15.010 s ≈ **47,999.9
  帧计数/s**；`pacer_calls_per_sec` **299–300**；`pending_frames=0`。
  与历史 B3（47,997 帧/s、pacer 299–300）同量级，**≈192,000/s（4×）
  现象未复现**。

## 会话与窗口

- 服务端会话：
  - session 1：配对后立即 `session.closed code=701`（约 200 ms，播放前噪声，
    不纳入判定）。
  - **session 2：判定会话**，`instance=2`，accepted 于 22:54:34.657 本地，
    closed code=701 于 23:28:19.960 本地（用户在音源结束后手动停止）。
- **有声判据窗口（判定用）**：
  - 暖机（单列，不计入）：22:54:35.481（reports=1/valid=0）、
    22:54:40.481（49/0）、22:54:45.731（51/3）。
  - **稳态窗：22:54:50.732 – 23:10:35.993 本地**，
    `start_ms=10,496` → `end_ms=960,758`，**950.26 s（15 分 50.26 秒）**，
    `instance=2` 唯一，区间连续。
- **排除段（单列，不拼入判据）**：23:10:37 – 23:28:19 本地。用户确认视频
  已播完（日志推定结束于 23:10:36）；此后客户端持续写零数据
  （`AudioTrack: isLongTimeZeroData` 由 1 s 计数至 59 s，间有短时非零）。
  该段内估计器出现两段重建：23:11:11 – 23:14:42（其中 23:11:16–23:14:32
  共 39 个 5 s 区间 `valid_estimates=0`）与 23:26:20 – 23:27:06（其中
  23:26:25–23:27:01 共 8 个区间 `valid_estimates=0`），会话末尾
  23:28:08–23:28:18 再次中断（2 个区间 `valid_estimates=0`）；
  `drift_window status=rebuilding` 共 52 行。累计 underrun 3 → 7,194，
  concealed_frames 2,746 → 3,456,875（约 72 s），`drift_window` 的
  `discontinuous_intervals` 由 45 增至 2,400。
  这一段不是有效有声播放，按清单不作为测量覆盖；其中的重建与欠载
  只作诊断记录，不据此改判。

## 客户端观测

### `playback.drift_clock`（本轮硬性检查）

- 窗口内 188 行，**`source` 全程 `hardware`**，无 `accepted` 行。
- `hardware_reports` 增量 **9,152**；`hardware_missing_reports=0`、
  `accepted_fallback_reports=0`、`head_reanchor_events=0`、
  `head_give_up_events=0`。
- 结论：本轮测的确是硬件播放时钟，未触发回退。

### `playback.drift_interval`（稳态窗）

- 区间数 **185**；`reports` 合计 **9,052**，`valid_estimates` 合计 **9,052**
  （完全相等）；无 0 有效区间；相邻区间 `start_ms == end_ms` 连续。
- `max_report_gap_ms` 最大 **122**；`discontinuous_intervals` 全程 **0**；
  窗内无 `drift_window` 重建事件。
- 全区间极值：**min_ppm = −1,807，max_ppm = +723**。
- **至少一端越出 ±100 的区间：184 / 185（99.5%）**；不是个别尖峰。
  - 最大负值：23:01:15.994，`start_ms=395,508–400,758`，min −1,807 / max +84；
  - 最大正值：22:59:59.237，`start_ms=319,001–324,002`，min −305 / max +723。

### `playback.drift`（每 5 s 快照，仅诊断）

- 186 个快照；`drift_ppm` 范围 −1,758 … +603；**中位数 −1，均值 −9.6**。
- 快照本身落在 ±100 内的仅 **47 / 186**。
- `correction_ppm` 始终在 ±100 内（−100 … +100），但有 **118 / 186** 个快照
  顶在 ±100 轨上。
- `buffer_current_ms`：**0 … 28**，中位 **10**（`buffer_target_ms=60`）——
  低于目标，也低于上一轮（23–61，中位 40）。
- `discontinuous_intervals` 最大 **0**。

### 缓冲 / 干预

- **re-anchor：0 次、0 帧**；整个会话没有任何 `playback.trim` 行。
- **soft trim：0**。
- concealment（窗内累计；会话开始时计数为 0）：gap **1**、underrun **3**、
  drift 0、reanchor 0、total **4**，concealed_frames **2,746**（约 57 ms），
  underrun_frames **1,440**（约 30 ms）。
  - 首次非零 underrun：22:57:08.987 出现（480 帧）；23:01:05.743 增至 3 次。
  - 不剔除，如实记录。
  - 注：解析脚本按「窗内首末行差值」计算会漏掉第一个非零事件（480 帧），
    此处已按会话基线 0 改为绝对值；上一轮 RESULT-WINDOWS-M5-11 的同类
    数字可能受同一偏差影响，此处只标注，不改旧记录。

## 服务端观测

- `media.stats` 192 条（22:54:39.897 – 23:10:35.678 本地），
  `session_id=2` / `instance=2` 唯一。
- `pending_frames` 全程 **0**（min/max/avg 均 0）。
- `pacer_calls_per_sec` **299 – 300**。
- `consumed_frames` 增量 45,877,440 / 955.78 s ≈ **47,999.9 帧计数/s**
  （与 B3 同量级）。
- `media.drops` **0 条**。

## 判定

| 判据（windows-retest.md） | 观测 | 结论 |
|---|---|---|
| 采集格式 48 kHz / 2 声道，自检 ≈48,000/s、pacer≈300/s | 48,000 Hz / 2 ch / 24-bit；47,999.9/s；pacer 299–300 | **PASS（前置条件）** |
| 必须记录 `source`；`hardware` 才可判漂移 | 188 行全 `hardware`；missing/fallback/give_up 均 0 | **PASS（前置条件）** |
| 稳态原始估计在 ±100 ppm 内（全区间极值） | −1,807 … +723；184/185 区间越界 | **FAIL** |
| 覆盖：`reports>0`、`valid_estimates==reports`、无异常 gap | 9,052/9,052；gap≤122 ms；连续；无重建 | PASS |
| 单 5 s 周期无 +7 次级 re-anchor 突跳 | re-anchor 0 次 / 0 帧；无 trim 行 | PASS |
| `correction_ppm` 始终 ±100 内 | 是，但 118/186 顶轨 | PASS（附注） |
| 记录缓冲 / Server pending / drops | buffer 0–28 ms（目标 60，中位 10）；pending 0；drops 0 | 记录，buffer 偏低 |

**用例 B 结论：FAIL** —— 48 kHz 同条件已成立，采集帧率与 B3 对齐；唯一不达标项
仍是稳态原始漂移估计超出 ±100 ppm，且是持续性的（184/185 个区间越界），
不是个别尖峰，也不是「播放头不可用」的回退情形（`source=hardware` 全程有效）。
本轮极值（−1,807 … +723）与上一轮（−1,390 … +1,529）同属一个失效模式，
量级未收敛。

## 与上一轮（RESULT-WINDOWS-M5-11）的偏差（如实记录）

1. **4× 采集帧计数率已排除**：上一轮 191,944.7 帧计数/s、pacer 1,197–1,202；
   本轮 47,999.9 帧计数/s、pacer 299–300，与 B3 同量级。上一轮未记录采集格式；
   本轮记录为 48 kHz / 2 ch / 24-bit（默认端点 Steam Streaming Speakers）。
   不能据此断言上一轮的 4× 由何产生，只能确认本条件不再复现。
2. **漂移极值**：上一轮 −1,390 … +1,529（71/71 区间越界）；本轮 −1,807 … +723
   （184/185 区间越界）。失效模式相同，极值未收敛。
3. **快照中位数**：上一轮 +66；本轮 −1（均值 −9.6）。快照接近 0 但区间极值仍大，
   两轮都呈现「快照中位数与逐报告极值不一致」的现象；本记录不据此下结论。
4. **缓冲**：上一轮 23–61 ms（中位 40）；本轮 0–28 ms（中位 10），更低。
   按现有设计不触发插帧，仅记录。
5. 窗口长度：本轮有声稳态 15 分 50 秒（上一轮 6 分 05 秒），均超过 5 分钟要求。
6. 本轮音源为用户播放的视频，23:10:36 播完；23:10:37 之后为静音/重建段，
   已单列排除，不参与判据。上一轮未固定音源类型。
7. 服务端 `server.log` 时间戳为 **UTC**；本文件未标注时区的时间均为本地 UTC+8
   （换算已核对，+8 h）。
8. 配对为本次新做（QR）；配对内容不入库。
9. 采集冻结后会话已关闭（session 2 closed code=701，用户手动停止）；
   Server 进程（GUI）未由本会话停止，仍可手动关闭。未改任何代码，
   未执行任何 git 操作；`integration/wire-protocol-v2-beta` 与 `v0.5.0-beta` 均未动。

## 建议（回传后由 Linux 侧决定）

1. 维持 `FAIL`，不关闭 M5-09、不判 B 通过、不合 beta。
2. 按 `diagnosing-bugs` 为「逐报告极值 ≫ 快照中位数」建立可复现 RED，重点看
   真机 `getTimestamp()`/播放头的逐报告年龄与量化步长；不调阈值、不改判据。
3. 若维护者要区分「估计器问题」与「播放头采样问题」，可在同一构建上增加
   逐报告原始值落盘（本轮日志只有区间 min/max），但这需要改代码，不在本轮范围。

## NOT RUN（不得写成 PASS）

- 真机 ±100 ppm 注入矩阵
- 真实网络损伤 / 用例 C
- 双真机并发、API 26 真机、M5-07 的 60 分钟 soak
- 主观听测（用例 B 不作主观项）
- 23:10:37 之后的静音/重建段（音源已结束，非有效有声播放）

## 证据文件（本地 `evidence-2026-11-21-m5-11-48k\`，未脱敏，仅留本地）

- `session-raw-logcat.txt`：完整未过滤 logcat（约 8 MB，22:53:26 起）
- `server-full.log`：服务端完整日志（冻结时副本）
- `windows-default-format.png`：默认格式 GUI 截图
- `windows-audio-formats-before.txt`：全部播放端点格式快照
- `desktop-before.png` / `desktop-end.png` / `phone-end.png`
- `device-info.txt` / `artifact-hashes.txt` / `device-settings-before.txt` /
  `timeline.txt`
- `analysis-audio-window.json` / `analysis-valid-window.json` /
  `analyze_m5_11_48k.py`：本轮解析脚本与输出
