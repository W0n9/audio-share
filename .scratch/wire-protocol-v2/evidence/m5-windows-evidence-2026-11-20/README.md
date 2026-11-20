# M5 Windows 真机验收记录 — 2026-11-20

本仓库收录脱敏摘要；原始日志和桌面截图仅保留在本地。

原始结果与日志见同目录 `RESULT.txt` 及 `aflinger-*` / `client-*` / `server-*`。
代码基线：`integration/m5` @ `800c40c`；Server `AudioShareServer.exe` 0.5.0-beta
(SHA-256 `321b747a…81ea`)，Client `dev.audioshare.v2` 0.2.0-beta (code 2，
APK SHA-256 `56e27380…2878`)。两哈希与交接文档一致。

环境：Xiaomi M2007J22C / Android 12 (SDK 31)，Wi-Fi <REDACTED>（Windows 热点）；
Windows 11 10.0.26300；v2 `[::]:65531`。QR 配对，SAS `<REDACTED>`。

## 判定

| 项 | 结果 | 依据 |
|---|---|---|
| 用例 A — M5-03 听测（5m06s） | **PASS（主观）** | 用户：「没有卡顿，爆音不明显，可以接受」 |
| 用例 B — M5-04 漂移基线（5 min） | **FAIL（用户裁决）** | ppm 未接近 0；单次 499 ppm 离群；窗口末尾起服务端持续 pump 丢弃；用户「A 好一点」 |
| 用例 C — M5-06 真实网络损伤 | **NOT RUN** | Windows 无 tc netem，本轮按决定跳过 |
| M5-04 ±100 ppm 注入 | **NOT RUN** | Server 无 capture 速率偏移开关 |
| 双真机 / 60 min soak | **NOT RUN** | 留 stable/M7 |
| M5-05 L1 分项/调优、M5-01 p50/p95/p99 | **NOT RUN** | 实验分支不转正；需录音设备 |

## 用例 A 计数（窗口末，客观记录）

- `concealments total=5601`（gap 22 / **underrun 5576** / drift 0 / reanchor 3）
- `playback.trim soft_frames=63263 soft_events=28`；`reanchor_frames=43207 reanchor_events=3`
- drift 采样：60 ppm（buffer 45 ms）/ 16 ppm（buffer 33 ms）
- 3 次 re-anchor 合计 43207 帧 ≈ 900 ms；用户未听出异常。

## 用例 B 计数

- `buffer_current_ms` 60 / 56（target 60），有界；drift concealment 0。
- `drift_ppm` 36（correction -80）/ 499（correction 限幅 100）——仅两个采样点，499 为单次离群。
- gap 22→49；underrun 5576→5600（基本停涨）。
- 服务端 `media.drops` 于 10:58:21（本地）首次出现：`pump_dropped_chunks=1`，
  `queue_dropped_newest=0`，`pending_frames=23174`（≈483 ms）。

## 扩展观察（10:58:50–11:12:10，非验收窗口）

- 服务端：`pump_dropped_chunks` 1→176（≈1 条/5 s），`queue_dropped_newest=0`，
  `pending_frames` 稳定 22.5k–23.7k（≈470–495 ms，逼近 `MAX_PENDING_MS=500`）。
- 客户端：reanchor 14 次 / 155650 帧（≈3.2 s）；trim 619387 帧（≈12.9 s）；gap 298。
- 用户：「A 好一点」。本机空闲对照 **NOT RUN**（桌面有浏览器/VS Code，负载未测）。

## 需要诊断的信号（已开 M5-08）

1. **Server pending 钉在 ~480 ms 并持续丢帧**：`pump_dropped_chunks` 每 5 s +1，
   `queue_dropped_newest=0`。意味着 capture 生产速率略快于 pacer 排空；
   且若 `capture_position − last_sent_position` 按最新采集位置计，≈480 ms 已超出
   §7.4 的 100 ms send-ahead。需核实 `MediaPacer` 语义与捕获设备实际速率。
2. **Underrun 事件 5576 / 5 min**：`underrunGate` 仍以平台 `underrunCount` 增量为门控，
   而 MIUI 的该计数已知不可信（Q3）。需改用更可靠的信号（持续空读 N 个周期）或忽略平台计数。
3. **软修剪 28 次 / 5 min、reanchor 3 次**：滞回（target+20 ms）可能过紧，对突发交付过度修剪；
   需区分「突发」与「持续速率差」，避免可听的丢帧。
4. **漂移 499 ppm 离群**：软修剪会重置 10 s 窗口，频繁重置可能使估计只覆盖短窗口；
   同时 controller 限幅 ±100 ppm 无法吸收观测到的 ~0.4% 量级差异，需确认口径。
5. 服务端 `media.drops` 与客户端 `late_dropped`/`cumulative_lost` 已分开记录（符合 §8）。

## 结论

- M5-03 可听验收 **PASS（主观）**，但客观计数异常，不能据此宣称机制健康。
- M5-04 基线 **FAIL**（用户裁决），需修复后复测。
- 其余真机/网络项 **NOT RUN**。
- 已开 `issues/m5-08-pump-drop-trim-underrun-investigation.md`（`ready-for-agent`）承接根因诊断。

---

# 复测（evidence-2，基线 `integration/m5` @ `431997b`）

原始数据：`evidence-2/RESULT2.txt` 与同目录日志。产物哈希与 handoff 一致。

## 用例 A（5m25s）

- 用户：「PASS — 连续无新增异常」。
- underrun 事件窗口内 +125（上一轮会话起累计 5576）——下降约 97.6%。
- 新增埋点生效：`concealed_frames` +57,108（≈1.19 s），`underrun_frames` +60,000（≈1.25 s）。
- 服务端：`pending_frames` 全程 0，`consumed_frames` ≈48,000/s，`pacer_calls_per_sec` 299–300，
  `media.drops` 0（上一轮 5 min 时已持续 +1）。
- drift：63 样本，span 10,000–10,082 ms，sample 99–101；ppm −1865…28252，±100 内 4 点，
  |ppm|>10000 有 6 点。
- **判定：PASS（主观听感 + 客观改善）。**

## 用例 B（5m21s，中间误触停止的 6 s 段已作废）

- 用户明确不做主观判定，交 Linux 侧裁决。
- underrun +178；`concealed_frames` +30,356；`underrun_frames` +85,359；gap +37；
  drift concealment +2；reanchor 0→8（15:07:53 同一 5 s 内 1→8，删除 64,672 帧 ≈1.35 s）；
  soft trim +143,216。
- drift：62 样本，span ≥10,000 ms、sample ≥98；ppm −38,082…336,157，±100 内 12 点，
  |ppm|>10000 有 4 点。极端值与 reanchor 突跳同刻（15:07:53 / 15:07:58）。
- 服务端：`pending_frames` 0–3187（均值 ≈2924 ≈61 ms），`pacer_calls_per_sec` 300→500（catch-up），
  `media.drops` **0**。
- **裁决（Linux 侧）：FAIL（open）**——PASS 条款要求 ppm 接近 0；本轮仍有极端离群，
  且与 reanchor 突发耦合。buffer 有界、零丢弃，说明第 1/2/5 项有效，问题收敛到第 3 项残留与第 4 项 trim/reanchor。

## 扩展观察

- 15:09–15:11 catch-up 突发（pending 2880–3187、pacer 496–500/s），**零丢弃**；
  15:12:43 起 pending 回落 0、pacer 回 299–300/s，自愈。
- 与上一轮「10 分钟后 pending 钉在 ~480 ms 并持续丢帧」不同。

## 结论

- 第 1 项（pacer 小数帧）：A 窗口 pending=0、consumed ≈48k/s；B 窗口 pending ≤66 ms、零丢弃
  → **有效**。§7.4 字面 `capture_position − last_sent_position ≤100 ms` 在本轮两个窗口内**满足**
  （0 与 ≤66 ms），第 6 项可由现有 `media.stats` 作为观测守卫，无需 catch-up/drop-oldest 改动。
- 第 2 项（underrun 门控）：事件 5576 → 125，且 `concealed_frames` 可测 → **有效**。
- 第 3 项（漂移窗口）：发布门槛生效，但 **reanchor/trim 阶跃的偏移补偿不正确**（极端 ppm 与
  reanchor 1→8 同刻）→ **部分有效，需继续修**。
- 第 4 项（trim/reanchor 突发）：soft trim 仍高（A +72.9k、B +143.2k），reanchor 8 次/5 min → 需处理。
- 第 7 项（jitter hole-hold）：仍待裁决。

## 本轮裁决（2026-11-20）

- 用例 A：**PASS**。
- 用例 B：**FAIL（open）**（Linux 侧裁决；用户未做主观判定）。
- §7.4（第 6 项）：**以每 5 s 的 `media.stats` 为回归守卫关闭**，不新增 catch-up/drop-oldest 代码
  （观测 pending=0 与 ≤66 ms 已满足字面 100 ms）。
- 第 3 项残留（drift 阶跃）与第 4 项（突发 trim/reanchor）**暂不改**，转
  [M5-09](../issues/m5-09-drift-step-and-burst-trim.md)（`needs-triage`）。
- 第 7 项（jitter hole-hold）仍待裁决。
- 用户决定：本轮不改代码，先收尾；`integration/m5` 保持 `431997b`，**不 FF beta**（B 未通过）。
