# M5-09 Windows + Android 真机用例 B 复测结果（构建 cb28095 / Server 431997b）

生成时间：2026-11-21 00:24（本地 UTC+8）
执行者：Claude（部署、采集、日志分析）+ 用户（手机连接、配对、播放、音源）
本文件为脱敏版，可直接回传；原始日志、截图与配对内容仅保留在本地
`C:\AudioShareBeta\evidence-m5-09\`。

---

## 环境 / 产物校验

- Windows：11 专业工作站版 10.0.26300（Build 26300），时区 UTC+8。
- Server：`AudioShareServer.exe` 0.5.0-beta（GUI 便携版），
  SHA-256 `ee7a35a4bccad6bc09344fd7672c703e6e5466f60859ffb4ff6911fb69bd94d8`，
  复制后重算一致；`--smoke-test` 退出码 0。
  运行目录：`C:\Users\<user>\Desktop\audioshare\`（`config.ini`、`server.log`、
  `identity/`、`pairing/` 均在此）。
- Android APK：`app-debug-arm64.apk`，SHA-256
  `a6838e3b1ec21203a3f975c29d4229195681b3c2edeecbcc4bd5d3b95c7c194c`（22,221,975 B），
  安装时间 2026-11-20 23:47:53 本地（`lastUpdateTime` 与安装时刻一致），
  `lib/arm64-v8a/libaudio_share_client_v2_native.so` 含 `playback.drift_interval` 标记，
  跑测中该日志行确实持续输出 —— 确认为修复版构建，不是 `4d23f89` 首版候选。
  包：`dev.audioshare.v2` 0.2.0-beta（versionCode 2），minSdk 26 / targetSdk 36。
- 手机：M2007J22C（cannon），Android 12 / API 31，arm64-v8a，
  MIUI V14.0.6.0.SJECNXM。adb 无线连接。
  输出路由 `AUDIO_DEVICE_OUT_SPEAKER`，48 kHz；无蓝牙。
- 网络：与 Windows 同一 LAN（地址已脱敏），未注入网络损伤或时钟偏移。
- 屏幕常亮：`svc power stayon true`（测后已恢复 `false`）。
- 音源：Windows 上播放视频（与上一次复测同类来源），窗口内连续不断流；
  服务端 WASAPI loopback 采集 48 kHz，`media.stats` 每 5 s 稳定 240,000 帧。

## 会话与窗口

- 本次从配对完成起共 4 个会话：
  - session 1（15:55:57 UTC，配对后立即关闭，701）；
  - session 2（15:55:58 UTC 起）：**无效段**，电脑上无音源，
    `media.stats` 全程 `consumed_frames=0 / pacer_calls=0`，
    客户端 `valid_estimates=0`；期间 00:02–00:03 本地有一次 35 s 测试音，
    也在本段内。**不纳入判定。**
  - session 3→4：00:06:00–00:06:01 本地同秒内 701 关闭后重连。
- **判定会话：session 4 / instance 1，accepted 16:06:01.152 UTC = 00:06:01.152 本地。**
- 整窗：00:06:01 – 00:12:01 本地（6 分 00 秒）。
- 稳态分析窗（剔除暖机）：**00:06:15 – 00:12:02 本地（5 分 47 秒）**，
  首区间 00:06:17.509，末区间 00:11:58.259，`start_ms` 10,747 → `end_ms` 356,498。
- 暖机区间（单列，不计入判据）：00:06:02（1 报告/0 有效）、
  00:06:07（50/0）、00:06:12（49/5）。

## 客户端观测（稳态窗）

`playback.drift_interval`：

- 区间数 **67**；`reports` 合计 **3,277**，`valid_estimates` 合计 **3,277**（完全相等）。
- 全区间极值：**min_ppm = −645，max_ppm = +532**。
- 至少一端越出 ±100 的区间：**67 / 67（100%）**。
- `max_report_gap_ms` 最大 **113**；`discontinuous_intervals` 全程 **0**；
  无 `drift_window status=rebuilding` 事件。

`playback.drift`（每 5 s 快照，仅诊断）：

- 67 个快照；`drift_ppm` 范围 −467 … +447；
  **中位数 −40，均值 +0.3**（快照中位数，仅作诊断，不能替代区间极值判据）。
- `correction_ppm` 始终在 ±100 内，但 **35 / 67** 个快照顶在 ±100 轨上。
- `buffer_current_ms`：0 … 26，中位 **10**（`buffer_target_ms=60`）——
  全程低于目标，记录为异常观测。

缓冲/干预计数：

- **re-anchor：0 次、0 帧**。session 4 内没有任何 `playback.trim` 行
  （该行仅在 `soft_trimmed_frames>0 || reanchor_frames>0` 时输出），
  `playback.concealments` 中 `reanchor=0`。历史 B2 是单个 5 s 周期 +7 次，本次未复现。
- soft trim：0 帧。
- concealment：窗口内 **0**；00:12:03 本地出现会话内第 1 次 underrun
  （480 帧，≈10 ms），已在窗口之外。

## 服务端观测（稳态窗）

- `media.stats` 70 条（00:06:16 – 00:12:01 本地），`session_id=4` 唯一。
- `pending_frames` 全程 **0**（min/max/avg 均 0）。
- `pacer_calls_per_sec` **299–300**。
- `consumed_frames` 增量 16,569,600 / 345 s ≈ **47,997 帧/s**≈实时。
- `media.drops` **0 条**。

对比历史 B2（431997b）：当时服务端全程 catch-up（pending 均值约 2,924 帧、
pacer 峰值 496–500/s、窗口内 drift 到 ±3.8 万/+33.6 万 ppm、单周期 re-anchor +7）。
本轮服务端完全干净、无 re-anchor 突跳，**但稳态原始漂移估计仍越界**。

## 判定

| 判据（windows-retest.md） | 观测 | 结论 |
|---|---|---|
| 稳态原始 `drift_ppm` 在 ±100 ppm 内（用全区间极值） | −645 … +532；67/67 区间越界 | **FAIL** |
| 覆盖：`reports>0`、`valid_estimates==reports`、无异常 gap | 3,277/3,277；gap≤113 ms；discontinuous 0 | PASS |
| 单 5 s 周期无 +7 次级 re-anchor 突跳 | re-anchor 0 次 / 0 帧 | PASS |
| `correction_ppm` 始终 ±100 内 | 是，但 35/67 顶轨 | PASS（附注） |
| 记录缓冲 / Server pending / drops | buffer 0–26 ms（目标 60）；pending 0；drops 0 | 记录，buffer 偏低 |

**用例 B 结论：FAIL** —— 唯一不达标项是稳态原始漂移估计超出 ±100 ppm，
且是持续性的（67 个区间无一完全落在 ±100 内），不是个别尖峰。

### 仅作相似性描述，不作归因

这种"无停滞、无 discontinuity、无 re-anchor，但逐窗残差在数百 ppm 摆动"的形态，
与 host 证据中"批提交压力模型 905/1,083 个窗口越出 ±100 ppm、均值仍正确"的
已知残差相似。**相似不等于已确认根因**；现场缺少逐报告
`(now_ms, capture_frames, playback_frames)` 与 `AudioTrack.write` 接受粒度，
无法回放，需按 `diagnosing-bugs` 另建 RED。

## 与清单的偏差（如实记录）

1. Server 运行在 `C:\Users\<user>\Desktop\audioshare\`，不是 `C:\AudioShareBeta\`：
   前者已有匹配程序路径的防火墙入站规则，后者没有；为避免提权（UAC）改用前者。
2. 部署时把 `config.ini` 的 `WhenAppStart=0` 改为 `1`，使 GUI 启动即拉起服务器
   （controller 随即将 `[App] Running` 写成 1）；`WhenClose` 等其余项未动。
3. 配对为本次新做（QR），旧的 `pairing/clients.json` 与旧 identity 已不存在；
   手机端旧凭据被新记录取代。
4. session 2 为无音源无效段；测试音（35 s）只出现在该段，均已排除。
5. session 3→4 在 00:06:01 同秒内完成 701 关闭与重连；判定窗口对齐 session 4。
6. 服务端 `server.log` 时间戳为 **UTC**；本文件未标注时区的时间均为本地 UTC+8，
   换算已核对（+8 h）。
7. 窗口结束后会话未立即停止：采集冻结时（00:23 本地）手机仍显示「正在播放」、
   服务端仍在输出 `media.stats`。00:12:01 之后的追加数据不在本次判定内。

## NOT RUN（不得写成 PASS）

- ±100 ppm 注入矩阵（服务端无速率偏移开关）
- 真实网络损伤 / 用例 C
- 双真机并发、API 26 真机、M5-07 的 60 分钟 soak
- 本轮未做主观听测判定（用例 B 不作主观项）

## 证据文件（本地 `evidence-m5-09\`，未脱敏，仅留本地）

- `client-all.log`：完整 logcat（16 MB 缓冲，23:55 起）
- `client-B3.log`：切窗客户端日志（00:05:55 – 00:12:xx）
- `server-full.log` / `server-B3.log`：服务端完整日志与切窗
- `config.ini`：本次运行配置
- `aflinger-session4-mid.txt` / `aflinger-session4-end.txt`：
  `dumpsys media.audio_flinger` 窗口内两次快照
- `phone-app-launch.png` / `phone-after-pair.png` / `phone-session4-mid.png` /
  `phone-session4-end.png` / `phone-current.png`：手机端状态截图
- `gui-started.png` / `gui-pairing-dialog.png` / `gui-pairing-click.png` /
  `gui-now.png`：GUI 状态截图
- `desktop-server-running.png` / `desktop-after-pairclick.png`：桌面截图
- `analyze_m5.py` / `analyze_extra.py`：本次使用的解析脚本

## 收尾状态

- logcat 抓取已停止；`svc power stayon` 已恢复 `false`。
- Server 进程与手机播放**未由本会话停止**（采集冻结时仍在运行）。
- 未改任何代码、未执行任何 git 操作；`integration/wire-protocol-v2-beta` 与
  `v0.5.0-beta` 均未动。
