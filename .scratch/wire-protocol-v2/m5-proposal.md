# M5 提案：抖动、漂移与损伤恢复（草案，未 accepted）

Status: 提案草案 — 不构成排期承诺，不新增授权，不改动任何已 accepted 的规范数值
来源: M4 真机验收（2026-11-15）暴露的播放侧数据 + `roadmap.md` §2 的 M5 DoD
相关: `issues/15-*.md`、`evidence/ticket-15/device-acceptance/`

## 0. 这份提案要解决什么问题

M4 把「单 Client PCM 从 capture 到真机出声」打通了，但**端到端可听时延从未被测量过**，
而 M5 的 DoD 里写着 p95 ≤ 80 ms / p99 ≤ 120 ms（normal-LAN）。在开 M5 工单之前必须先回答：
这个预算里，钱花在哪一项上。

真机验收的 dumpsys 已经给了一个不舒服的答案。

## 1. 延迟分解（LAN 场景）

| 组成 | 量级 | 来源 |
|---|---|---|
| LAN 网络 RTT | **< 10 ms** | 维护者 2026-11-15 说明；偶有抖动 |
| native ring 常驻 | 几 ms | 客户端 ring 只在有数据时前进（实测 `available` 常在 3.5 KB ≈ 7 ms） |
| **AudioTrack 平台缓冲** | **≈ 128 ms** | 实测 `frameCount = 6150`（48 kHz f32 stereo） |
| 合计 | **≈ 140 ms** | — |

`AudioSink.open` 当前的算法是：

```kotlin
val bufferBytes = maxOf(minBuffer, sampleRate / 10 * bytesPerSample * channels)
```

即**至少 100 ms**，平台再向上取整到 6150 帧 = 128 ms。也就是说：

> **在 LAN 上，延迟的主要来源不是网络，是播放端自己申请的 128 ms 平台缓冲。**

这一项单独就把 p95 ≤ 80 ms 撑破了。任何在传输侧做的优化都无法弥补。

## 2. 由此得出的排序修正

原计划把 M5 当成「抗损伤工程」（丢包、重排、复制、漂移对抗网络）。LAN 现实下顺序应当改写：

### L1 — 先能测量，再谈调度

目前**没有任何端到端可听时延的观测手段**。`PlaybackDiagnostics` 里的
`buffer_target_ms` / `buffer_current_ms` 是应用侧 ring 的深度，不含 AudioTrack 队列，
也不含采集侧。没有这个数，后面所有阈值都是拍脑袋。

交付：一个可复现的测量（采集参考信号 → 真机 → 回环测量或时间戳法），
产出 p50/p95/p99 的可听时延基线，以及分项拆解（网络 / ring / 平台缓冲）。

### L2 — 平台缓冲尺寸 + playout 调度

- 把 `AudioTrack` 缓冲从 ~128 ms 降到 20–40 ms 量级，把平滑责任移交给应用层 jitter buffer。
- 应用层 jitter buffer 按 LAN 实测抖动取值（预期 20–40 ms，**远低于规范的 60 ms target**，
  也远在 200 ms hard limit 内）。
- 关键约束：不能为了降延迟把 underrun 换成可听咔哒。L1 的基线是判定依据。

### L3 — 损伤处理

丢包（2%）、重排（25%/50%）、重复（5%）、±100 ppm 漂移、MTU 1280、300 kbit/queue50 拥塞、
25 s idle、connection-kill 重连 —— §11.2 矩阵逐行 PASS/FAIL/NOT RUN，外加 parser fuzz
与 60 分钟无死锁/无假活/无延迟增长。

在 LAN 上这一层是「保证不崩」，不是「保证延迟」；但它仍是 M5 的 DoD，不能省。

## 3. 待决问题（需要维护者裁决）

1. **60 ms target / 200 ms hard limit 是否要改？**
   LAN 实测可能证明 30 ms 就够。但这是 **accepted 的规范数值**，按 `roadmap.md` §7.4
   只能由维护者裁决 + 新证据修改。本提案不预设结论，只要求 L1 先给出数据。
2. **降 AudioTrack 缓冲算 M5 还是算 M4 缺陷？**
   倾向算 M5：M4 明确把时延预算排除在 DoD 之外（`roadmap.md` M5 行）。但如果 L1 测出
   现有实现根本无法达到 80 ms，需要在 M5 开票前补一条记录说明「M4 交付时已知此限制」。
3. **`underruns` 计数在 MIUI 上的语义未验证。**
   一次 5 分钟会话里 `dumpsys` 报 23 028 次，与主观「连续无咔哒」不一致。在把它当验收指标前
   必须先确认这个计数器在目标设备上可信，否则会得出错误结论。

## 4. 延后的实验（下一次真机会话）

**目标**：判断 L2 的第一步（平台缓冲 100 ms → 20–40 ms）是否可行。

步骤：

1. 在 `AudioSink.open` 里把 buffer 目标从 `sampleRate/10` 改为可配置（默认保持现值，
   实验时用 20 ms / 30 ms / 40 ms 三档）。
2. 同一个 server、同一条路径，各跑 2 分钟，记录：`FrmRdy` 稳态、`underruns` 增量、
   主观听感（连续 / 咔哒 / 断续）。
3. 判定：若 20–40 ms 下主观无咔哒且 underrun 不显著增长，则 L2 成立，M5 主体工作量
   从「抗损伤矩阵」前移到「调度器 + 小幅缓冲」；否则保留大缓冲，把 jitter buffer 放在应用层
   重新取值。

这个实验不需要 LAN 联通（缓冲区是本地行为），可以随时做。

## 5. 非目标（本提案不覆盖）

- 多 Client 隔离、生产 pairing / Keystore / GUI —— 属 M6。
- 双真机 60 分钟、Windows/API 26 真机、release/migration —— 属 M7。
- 改变已 accepted 的规范数值（60/200、p95/p99）—— 需维护者裁决。
- 音频编解码、FEC、PLC/Opus —— v2.0 非目标。

## 6. 下一步

本提案**不创建工单**。等 §4 的实验跑完、拿到数据后，再按 L1/L2/L3 拆成
`issues/16-*`（jitter buffer 与调度）和 `issues/17-*`（损伤矩阵与 fuzz），
并同步 `roadmap.md` 的 M5 行与 `map.md` 的 Fog。
