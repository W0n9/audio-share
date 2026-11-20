# M5-02 真机验证（2026-11-19，部分）

设备：Redmi Note 9 / M2007J22C / cannon，MIUI / Android 12 / API 31
adb：tailscale `<REDACTED>:5555`；LAN 媒体 `<REDACTED>:65531` ← `<REDACTED>:65531`
APK：`.scratch/wire-protocol-v2/evidence/m5-02/app-release-m5-02.apk`（sha256 `82f58aacc044c5f88fb17668bb17dc4d082966403c8c6abc94919d445eca872c`）

## 已验证

- prop 已清：`track_buffer_ms=0`、`track_frames=0`、`low_latency=0`。
- `adb install -r` 成功，冷启动后 app 自动恢复播放态（配对/会话保留），UI 显示「正在播放，服务器 <REDACTED>:65531」→ LAN QUIC 路径通。
- `AudioSink.open` 日志：`minBuffer=16416 lowLatency=false overrideMs=null requestedBytes=16416`；
  `actual bufferSizeInFrames=2052 capacity=2052 actualMs=42`。
- `dumpsys media.audio_flinger` active track（pid 9649）：
  `FrmCnt=2052 FrmRdy=2052 Underruns=0 Latency=125.43`。
  → 默认缓冲确实是平台最小 42 ms，不是旧 6150 fr / 128 ms。
- 注意：`audio_flinger.Latency` 列 125.43 ms 与旧默认 127 ms 接近，**不能当端到端可听延迟收益**（spec §8 明确网络/平台指标 ≠ 可听延迟）。收益证据要靠阶段 1 埋点复测 + M5-01 声学基线。

## 待验证

- 用户听测 ≥30 分钟（连续、无咔哒、无断续）——进行中/待用户回传。
- 阶段 1 埋点复测：需 Windows Server 带 `AUDIO_SHARE_CAPTURE_TIMING=1` / `AUDIO_SHARE_L1_TIMING=1` 运行，回传 `server.log` 后用 `l1-handoff/analyze.py` 与 128 ms 时期数据对比。
- M5-01 可听基线对照 —— NOT RUN。

证据等级：真机（非 EMULATOR）。MIUI `underrunCount` 只作参考，不作通过标准。

## 阶段 1 埋点复测（server-m5-02.log，2026-11-19 12:55）

日志：265 条 capture / 65 条 playback；跨 3 段连接（session 1 ×58 + session 2 ×7），
中间有一次 ~45 s 播放停顿（t≈204–261 s，`submitted_position` 不动、`underruns=85`）。

**不能与旧 128 ms 基线直接比较**：analyze.py 全局统计被停顿/重连污染
（`submitted_rate=36523 f/s`，延迟 min=-351/max=355 ms，全是失配插值产物）。

按连续段重算后：
- 段 1（t=13–204 s，session 1，rate=47825 f/s ≈ 实时）：延迟从 t≈100 s 起单调爬升
  126→355 ms，t≈204 s 骤变为 -351 ms 后线性回补 → **capture 帧计数与 playback 提交位置
  失配/重锚定**（归因未定：42 ms 余量饥饿 或 上游问题，见下方结论）。
- 段 2（t=261–298 s）：rate=46776 f/s，ring p50=99 ms，延迟 196.9 ms（重连后较高）。
- 段 3（t=313–338 s，session 2）：rate=38894 f/s，ring p50=84 ms，延迟 152 ms。
- 前 100 s 干净窗口（t=13–95 s）：延迟 p50≈93 ms、p95≈148 ms —— 与旧 128 ms 基线
  p50 96.8 / p95 135.1 量级相近（±24 ms 插值误差内，两次非受控对照，不能下定论）。

**结论（2026-11-19 修订，撤回早先的“不退化 PASS”归因）**：42 ms 下观察到持续爬升
（~3.6 ms/s，100 s 积累约 360 ms）+ 一次 45 s 停顿，两种解释都站得住：
① 42 ms 缓冲余量不足导致客户端饥饿（ring p50 仅 10 ms、停顿段 underruns 0→85 时间吻合）；
② 上游 pacing/会话问题。旧 128 ms 基线只有 ~100 s、未覆盖爬升区间，无法作为对照。
归因**未定**，不能甩给 M5-04/M5-05，也不能宣称 M5-02 不退化 PASS。
下一步：查停顿日志；做背靠背 128 ms 对照（同 Server ≥5 min）后再判定。
端到端可听收益仍需 M5-01 声学基线；听测 ≥30 min 仍待用户回传。

## 停顿/爬升成因排查（2026-11-19）

- `server-m5-02.log` 时间轴是 UTC，t0=04:49:16（本地 12:49）。全日志只有两次
  `session.closed code=701`（04:54:34 / 04:55:11）和两次 `session.accepted`
  （session 1 @04:49:34、session 2 @04:54:39）——**t≈204 s 的停顿并非会话关闭**。
- 手机 `logcat` 显示 12:58–12:59 反复 `resumed from suspend` / `suspend aborted`；
  当前 `dumpsys power` 为 `mWakefulness=Asleep`。设备在测试期间**反复休眠/唤醒**，
  采集帧计数跳变与 `submitted_position` 停顿与此高度相关。
- 结论：本轮日志受设备休眠 + 多次会话污染，**不能作为 42 ms 好坏的判据**
  （既不能判 FAIL，也不能判不退化 PASS）。需要受控 A/B：保持屏幕常亮，
  同 Server 背靠背跑 42 ms 与 128 ms 各 ≥5 min，再比较延迟爬升斜率与停顿。
- 用户听测（≥30 min）结果仍待回传。

## 受控 A/B 对照（2026-11-19 13:01–13:06）

用户确认 12:49–12:55 那轮**锁屏/切后台了**，故 42 ms 首轮数据作废（休眠污染）。
按用户同意做背靠背对照：

- **128 ms 臂**：`setprop track_buffer_ms=100` → 冷启动，open 日志
  `overrideMs=100 requestedBytes=38400`、`bufferSizeInFrames=6150 / 128 ms`；
  屏幕常亮，跑 ~170 s。日志 `server-m5-02-128ms.log`（累积日志，含旧 42 ms 段；
  A/B 窗口取 UTC 05:01:30 起，分析见 `.scratch/wire-protocol-v2/evidence/m5-02/analyze-ab.py`）。
  - 单会话窗口 qpc-derived capture rate = **48000 f/s**（计数口径正常）。
  - `capture_frame − submitted_position` 从 ~0 单调涨到 297 ms（~1.8 ms/s），
    ring p50=10 ms（空），underruns 0→83，服务端**无 drop/overflow 事件埋点**。
  - 用户听感（初次反馈「有断续/静音/爆音」，2026-11-19 进一步修正为**「断续爆音很少」**，按后者为准；该问题用户决定**暂缓不追**）。
  - 跑完已恢复 `track_buffer_ms=0`，冷启动确认 `overrideMs=null`、FrmCnt=2052（42 ms）。

**收敛结论**：爬升在旧默认 **128 ms 下同样复现**，且屏幕常亮 —— 这是**既有问题，
不是 M5-02 引入的回归**。但按 advisor 审查，不写“已归因上游 pacing”：
`capture_frame − submitted_position` 增长 + ring 空 + underruns 上升，既可能是上游
丢帧/迟到（MediaPump 丢最新、pacer、网络），也可能是计数口径问题；服务端目前无
drop/overflow 埋点，无法区分。可听层面对照只证明“128 ms 也不干净”。

**M5-02 判定**：
- 默认值改为平台 `getMinBufferSize`（42 ms 档）在实验分支实现完成，JVM 251/251 PASS，
  beta 签名 APK，真机 open 日志 + dumpsys 确认 FrmCnt=2052。
- 30 分钟听测 —— **NOT RUN**（两轮都不足 30 min，首轮还锁屏）。
- 可听延迟收益 —— **NOT RUN**（需 M5-01 声学基线）。
- 保持 `ready-for-agent`，不 resolve；生产默认变更仍待 Q2 裁决。

**既有问题移交**（不写进 M5-02 结论）：两份日志与窗口说明附在 M5-05 Notes，供
上游 pacing / 丢帧调查；不写成“已定位”。

## 重要修正：与 11-18 历史听测矛盾（2026-11-19）

11-18 缓冲实验里，用户对 **128 ms 档**的听测原话是「连续，无咔哒」。今天同样 128 ms
仅有「很少」的断续爆音。因此**不能说旧默认一直有问题**——今天坏掉的更可能是测试
环境本身：

- 今天的 Server 是**带 L1 埋点的构建**（`AUDIO_SHARE_CAPTURE_TIMING=1` /
  `AUDIO_SHARE_L1_TIMING=1`，capture 循环每秒一次 tracing），与 11-18 的听测条件不同。
- 05:01:29 Server **重启过**，两臂不在同一 Server 进程；10:38 那份干净基线（同 exe）
  只跑了约 100 s，未覆盖爬升区间。
- 屏幕常亮的 42 ms 臂**从未跑过**：首轮 42 ms 锁屏作废，A/B 只完成 128 ms 一侧。

**下一步建议**：用**不带埋点的正式 v0.5.0-beta Server** 跑 128 ms 与 42 ms 各 ≥3 min，
只听、不看日志。若断续消失，问题在埋点构建；若仍在，再按上游丢帧/计数口径排查。
在此之前，M5-02 的 42 ms 一侧一律记为 `NOT RUN`。
