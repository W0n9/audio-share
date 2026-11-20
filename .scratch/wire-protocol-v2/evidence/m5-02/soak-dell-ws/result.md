# M5-02 听测结果（2026-11-19，测试主机 A）

## 环境

- Server：测试主机 A，LAN `<REDACTED>:65531`（此前是 测试主机 B `<REDACTED>:65531`，
  本轮 测试主机 B 忙）。手机 `<REDACTED>/24`，网段与之前不同。
- Server 构建：**正式 v0.5.0-beta**（用户 2026-11-19 确认）；
  本轮听测不带 L1 埋点，已排除埋点构建干扰。
- Android Client：本实验分支 commit `f073e07` 的 beta 签名 APK（已安装）。
- prop：`track_buffer_ms=0` / `track_frames=0` / `low_latency=0`。
- 屏幕：`svc power stayon true` 保持常亮；测完已恢复 `false`。

## 起点现象（记录，不定性）

- 用户手动连接 `<REDACTED>:65531` 后正常播放（UI「正在播放」）。
- 我 force-stop 冷启动后，app 自动重连用了旧地址 `<REDACTED>`，
  5 s handshake timeout → 601，用户随后再次手动连接恢复。原因是配对里存的
  旧 Server 地址；此处只记现象，不定性为 bug。

## 听测判定

- **听测 PASS**：用户原话「连续，无咔哒/爆音/断续」，人耳听测 ≥5 分钟。
- **30 分钟听测门槛：WAIVED**（用户 2026-11-19 明确「5 分钟听测足矣」，豁免 M5-02 的 ≥30 min 要求）。
- 平台采样：7 个点覆盖 30 分钟（16:50–17:20），屏幕全程 `mWakefulness=Awake`，
  **FrmCnt 恒为 2052（42 ms）**，无档位漂移。`samples.txt` + 7 份 dumpsys 在同目录。

## underrun 计数（列位已修正）

dumpsys 列序 `Server FrmCnt FrmRdy F Underruns Flushed Latency`。
MIUI `Underruns` 从 58857 涨到 321653（30 min）；`Flushed` 全程 0。
按 m5-proposal Q3，MIUI underrun 计数**不可信**，只作参考；以听测为准。
（早先口头说「Underruns 全程 0」是把 Flushed 列当成 Underruns，已更正。）

## 未覆盖 / 边界

- 阶段 1 埋点复测「不退化」：此前数据未定论，用户决定断续问题暂缓；本轮为只听不测，
  无新埋点日志。记 `NOT RUN` 或 `WAIVED` 待用户选。
- 端到端可听延迟收益：仍 `NOT RUN`（M5-01 声学基线未做）。
- Q2 生产默认值裁决：未做。


## 与生产提交的等价性（2026-11-19）

Q2 生产提交 `m5/02-playback-buffer-floor @ d03865b` 与本次被测路径行为等价：

- 被测实验版（prop 全 0，`overrideMs=null`）：`bufferBytes = maxOf(minBuffer, minBuffer) = minBuffer`，
  `lowLatency=false`，未设置 performance mode。
- 生产版：`bufferBytes = minBuffer`，无 adb 旋钮，未设置 performance mode。
- 两者唯一差别是日志字符串；缓冲字节数、track 配置与播放路径一致。
- 因此 测试主机 A 的听测 PASS 可直接作为生产提交的证据，无需重测。
  生产 APK SHA-256 `f574770ab41a0c27740c9ded4e8826e144982943e89cbf94a2baea94ef26a755`。
