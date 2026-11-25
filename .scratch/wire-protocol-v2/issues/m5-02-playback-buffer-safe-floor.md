# M5-02: AudioTrack platform buffer down to the safe floor (42 ms)

**What to build:** 把 Android `AudioTrack` 平台缓冲从默认 `sampleRate/10`（请求 100 ms，
平台取整 **6150 帧 = 128 ms**）降到已验证的安全下限 **42 ms**（`getMinBufferSize` 平台下限，
2052 帧）。这是 L1 实测中单项收益最大（~86 ms）、且真机听感已验证的一步。

L1 实测（`l1-handoff/l1-results.md`）：128→42 ms 真机听测**连续、无爆音**；硬压到
**21 ms 会爆音**（`setBufferSizeInFrames` 实验确认）。

**Blocked by:** —

**Status:** resolved

**Type:** task

> 前后对比不依赖 M5-01：用阶段 1 埋点（采集→提交）+ 真机听测即可；M5-01 可听基线出来
> 后再补正式对照。

## Requirements

- [ ] `AudioSink.open` 默认目标从 `sampleRate/10` 改为 42 ms（或直接使用
      `getMinBufferSize` 返回值），记录实际 `bufferSizeInFrames`。不得降到 21 ms 量级
      （已证爆音）。
- [x] 真机听测：连续、无咔哒、无断续 —— **PASS**（<host>，人耳 ≥5 min，用户原话
      「连续，无咔哒/爆音/断续」）。≥30 min 门槛 **WAIVED**（用户 2026-11-19 豁免）。
      MIUI `underrunCount` 30 min 内 58857→321653 但已知不可信，只作参考；以听测为准。
- [ ] 用阶段 1 埋点复测采集→提交延迟，确认不退化；M5-01 基线出来后补可听延迟对照。
- [ ] 生产默认值变更需维护者裁决（m5-proposal Q2）；未裁决前只保留在实验分支。
- [ ] 不改协议、不改 v1、不改 `getMinBufferSize` 的平台语义。

## Notes

- 依据：`l1-handoff/l1-results.md`；`evidence/m5-buffer-experiment/results.md` 与
  `low-buffer-and-ring.md`。
- 若 42 ms 在 30 分钟听测中出现咔哒，回退到平台允许的次低档并记录数据，不得为了数字
  牺牲可听质量。
- **ring 目标深度与修剪属 M5-03**（ring 就是 jitter buffer 的落点），不在本工单重复。

## Comments

### 2026-11-19 — 实验分支实现完成，真机验证待用户参与

**代码改动（已提交 `experiment/m5-audiotrack-buffer` @ `f073e07`，实验分支，未 push、未合并；涉及 `MediaPlayer.kt` + 新增测试 + 5 个 Rust/Cargo 文件）：**

- `AudioSink.open` 默认缓冲从 100 ms 请求（平台取整 6150 帧 / 128 ms）改为平台 `getMinBufferSize` 返回值，不写死 42 ms —— 实测 minBuffer 在 16416 B（42.8 ms）与 24624 B（64 ms）间波动，一律用实际返回值。
- 新增纯函数 `AudioSink.requestedBufferBytes(minBuffer, frameBytes, sampleRate, overrideMs)`，默认用 minBuffer；adb 覆盖 `debug.audioshare.track_buffer_ms <N>` 只能增大、不能低于平台下限（`maxOf` clamp）；超大 override 上限 1000 ms，避免溢出成负数后静默回退。
- 删掉 `System.getProperty` 死代码（与反射读取重复）；删掉 `debug.audioshare.track_frames` 旋钮（`setBufferSizeInFrames` 21 ms 已证爆音+感知延迟升高，不再保留入口）；删掉每 100 ms 打一次的 `RingBacklog` 日志（长跑会刷屏）。
- `debug.audioshare.low_latency` 保留但默认关闭（听测 NOT RUN，不进默认路径）。
- 保留 `AudioSink.open` 的实际 `bufferSizeInFrames` / capacity / ms 日志供 dumpsys 对照。

**验证（已跑）：**

- RED→GREEN：`MediaPlaybackTest` 新增 `audio_sink_default_buffer_is_the_platform_minimum_and_override_only_grows`，RED 因缺函数失败、GREEN PASS。
- 全量 JVM 单测：**debug 251/251 PASS、release 251/251 PASS**（0 failures/errors/skipped；JDK 21 at `~/.cache/audio-share-refactor-worktrees/toolchain/jdk-21.0.13+9`，SDK `/home/w0n9/Android/Sdk`）。构建用 `local.properties`（已被 `.gitignore` 忽略，含本机 SDK 路径）。
- `assembleRelease` 成功，APK 用官方 beta key 签名（CN=Audio Share Beta，证书 SHA-256 `90bedcd1…a13c`），可 `adb install -r` 保留配对。
- APK 证据副本：`.scratch/wire-protocol-v2/evidence/m5-02/app-release-m5-02.apk`（不入库），文件 SHA-256 `82f58aacc044c5f88fb17668bb17dc4d082966403c8c6abc94919d445eca872c`（同目录 `SHA256SUMS`）。

**NOT RUN（需要用户/设备）：**

- 真机听测 ≥30 分钟（tailscale `<device>` / `<tailnet-ip>` 显示 `offline, last seen 15m ago`；LAN `<lan-ip>` 100% packet loss）。
- 阶段 1 埋点复测采集→提交延迟（Windows Server `<lan-ip>` 当前也 100% packet loss）。
- M5-01 可听基线对照（工单独立，仍 `ready-for-human`）。

**状态：** 保持 `ready-for-agent`；生产默认值变更仍待 m5-proposal Q2 裁决，不 resolve、不提交、不 push。

### 2026-11-19 — 真机 A/B 结果（追加）

- 实现已在实验分支完成，默认取平台 `getMinBufferSize`（本次冷启动 16416 B → FrmCnt=2052 / 42 ms）；
  prop 已恢复 `track_buffer_ms=0` / `track_frames=0` / `low_latency=0`，app 冷启动确认
  `overrideMs=null`、FrmCnt=2052。
- 首轮 42 ms 播放（12:49–12:55）：用户确认**期间锁屏/切后台**，日志受休眠污染，**作废**。
- 受控 A/B：128 ms 臂（`overrideMs=100`，FrmCnt=6150）屏幕常亮跑 ~170 s，用户听感
  **「断续爆音很少」**（用户 2026-11-19 修正，问题暂缓不追）；`capture_frame − submitted_position` 单调增长（~1.8 ms/s，
  单会话 qpc rate=48000 正常），ring 空、underruns 0→83。该爬升在旧默认 128 ms 下复现，
  **不是 M5-02 引入**，但按 advisor 审查不写「已归因上游」——可能是上游丢帧/迟到或计数口径，
  服务端无 drop/overflow 埋点无法区分。日志与窗口见 `evidence/m5-02/device-verification.md`，
  已附 M5-05 Notes + map 待裁决。
- **判定**：实现完成 + 真机平台缓冲确认（FrmCnt=2052）；但 **≥30 min 听测 NOT RUN**
  （两轮都不足 30 min）、可听延迟收益 NOT RUN（需 M5-01）。状态保持 `ready-for-agent`，
  Q2 生产默认裁决未做，不 resolve、不提交、不 push。

### 2026-11-19 — 补充：A/B 对照的环境局限

128 ms 臂今天「断续爆音很少」（用户修正后，非持续性问题）与 11-18 同档「连续，无咔哒」接近；今天 Server 是
**带 L1 埋点的构建**且两臂之间有 Server 重启（05:01:29），环境因素未排除。屏幕常亮的
**42 ms 干净臂从未跑过**（首轮锁屏作废）。建议先用**不带埋点的正式 v0.5.0-beta Server**
只听不测跑 128/42 各 ≥3 min，再决定 42 ms 的听测判定。在此之前 42 ms 一侧 `NOT RUN`。


### 2026-11-19 — 听测 PASS，状态转 ready-for-human

- 环境：Server 换为 <host> `<lan-ip>:65531`（<host> 忙），手机 `<lan-ip>`；
  Server 构建是否为正式版未确认；prop 全 0，屏幕常亮。
- 听测：≥5 min，用户「连续，无咔哒/爆音/断续」；30 min 门槛用户豁免（WAIVED）。
- 30 min 平台采样 7 点：FrmCnt 恒 2052（42 ms），屏幕全程 Awake。
  MIUI Underruns 58857→321653（不可信，参考），Flushed 全 0。
  证据：`evidence/m5-02/soak-<host>/`。
- 状态转 `ready-for-human`：实现与听测已完成，剩余为人工裁决——
  ① Q2 生产默认值是否改；② 阶段 1 埋点复测「不退化」记 `NOT RUN` 还是 `WAIVED`。
- 端到端可听延迟收益仍 `NOT RUN`（M5-01）。不 commit、不 push（实验分支 `f073e07` 已提交）。


### 2026-11-19 — Q2 裁决与生产提交

- **Q2 裁决：改**（用户 2026-11-19）。生产提交 `m5/02-playback-buffer-floor @ d03865b`
  （只含 `MediaPlayer.kt` + `MediaPlaybackTest.kt`；L1 埋点留在实验分支 `f073e07`，未转正）。
  生产版只保留 `bufferBytes = minBuffer` + 纯函数 + 一次 open 日志；所有 adb 旋钮与反射调用已删。
- 验证：RED→GREEN；JVM debug/release 各 251/251 PASS；beta 签名 APK
  `evidence/m5-02/app-release-m5-02-prod.apk`，SHA-256 `f574770a…a755`。
- **阶段 1 埋点复测「不退化」：`NOT RUN`**（用户 2026-11-19 选择；断续问题暂缓）。
- 听测证据与生产等价性见 `evidence/m5-02/soak-<host>/result.md`；Server 为正式 v0.5.0-beta。
- 状态保持 `ready-for-human`：**待合并裁决**（生产提交尚未合入 `integration/wire-protocol-v2-beta`，
  未 push）。合入后再写 `## Answer` 并标 `resolved`。
- 端到端可听延迟收益仍 `NOT RUN`（M5-01），不阻塞本工单。


## Answer

M5-02 完成。Q2 于 2026-11-19 裁决为「改」：生产默认缓冲从 100 ms 请求（平台取整
128 ms）改为 `getMinBufferSize` 返回值（真机 42 ms）。

- 生产提交 `d03865b`，已 fast-forward 合入 `integration/wire-protocol-v2-beta`
  （仅 `MediaPlayer.kt` + `MediaPlaybackTest.kt`；无 adb 旋钮、无反射调用）。
  L1 埋点未转正，仍在 `experiment/m5-audiotrack-buffer @ f073e07`。
- 验证：RED→GREEN；JVM debug/release 各 251/251 PASS；beta 签名 APK
  `evidence/m5-02/app-release-m5-02-prod.apk`（sha256 `f574770a…a755`）。
- 真机听测 PASS（<host>，正式 v0.5.0-beta Server，≥5 min，用户原话
  「连续，无咔哒/爆音/断续」）；≥30 min 门槛用户 WAIVED。生产版与被测路径行为等价
  （均走 `minBuffer`、未设 low-latency），不重测。证据 `evidence/m5-02/soak-<host>/`。
- 阶段 1 埋点复测「不退化」：`NOT RUN`（用户选择）。
- 端到端可听延迟收益：`NOT RUN`（M5-01），不阻塞本工单。
