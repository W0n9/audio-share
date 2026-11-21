# Handoff — Windows 上执行 48 kHz 同条件复测（用例 B）

交接时间：2026-11-21。上一会话在 Linux 上完成 M5-11 实现、合入与两轮真机复测；本文件只写**下一步做什么**，背景一律引用路径。

## 起点

- Linux 仓库 `/home/w0n9/workspaces/audio-share`，分支 `integration/m5` @ `d06ccb9`，工作区干净。
- 未 push。beta 分支 `integration/wire-protocol-v2-beta` @ `8c35411`、标签 `v0.5.0-beta` @ `362d023` 均未动，**本轮不得动**。
- 工单：[M5-11](/home/w0n9/workspaces/audio-share/.scratch/wire-protocol-v2/issues/m5-11-playout-clock-source.md) `ready-for-human`；[M5-09](/home/w0n9/workspaces/audio-share/.scratch/wire-protocol-v2/issues/m5-09-drift-step-and-burst-trim.md) `Blocked by: M5-11`；[M5-10](/home/w0n9/workspaces/audio-share/.scratch/wire-protocol-v2/issues/m5-10-reanchor-burst-investigation.md) `needs-triage`（不实施）。

## 目标（只做一件事）

在 Windows GUI Server + 单台 Android 真机上，把 **Windows 默认播放设备格式固定为 48 kHz / 2 声道**后，重跑**用例 B**：有效播放 ≥5 分钟、正常 LAN、屏幕常亮、不注入网络损伤或时钟偏移。

原因：上一轮（`71e0155` 硬件时钟）漂移精度 FAIL，但采集帧计数率约为历史 B3 的 **4 倍**且格式未记录，无法判断是否同条件。本轮只排除这个混淆，**判据、代码、产物都不变**。

步骤与判据只写在 [Windows 复测清单](/home/w0n9/workspaces/audio-share/.scratch/wire-protocol-v2/evidence/m5-09/windows-retest.md)：**按清单执行，不另行发挥。**

## 开测前置（必须做并留记录）

1. Windows：设置 → 系统 → 声音 → 更多声音设置 → 播放 → 默认设备 → 属性 → 高级 → 默认格式 → `48,000 Hz`、2 声道 → 应用。截图或抄下该设置（本地留存）。
2. 确认 APK 是 `71e0155` 构建（哈希见下），Server 是 `431997b` 的 exe。
3. 开测后先看服务端 `media.stats`：
   - **通过自检**：`consumed_frames` 折算约 48,000 计数/s、`pacer_calls_per_sec` 约 300/s；
   - **未生效**：仍约 192,000/s、pacer 约 1,200/s → 本轮记 `NOT RUN`，不与历史 B 比较，先修格式再重跑。

## 产物（Linux 本地，`.build-artifacts/` 被 git 忽略，需自行传输）

| 文件 | 构建提交 | SHA-256 |
|---|---|---|
| `app-debug-arm64.apk` | `71e0155` | `123e04fbd8c5e9ea15556bdfd38357a3ad4c8e93f0fc7275e615537a29d223e7` |
| `AudioShareServer.exe`（复用，Server 未改） | `431997b` | `ee7a35a4bccad6bc09344fd7672c703e6e5466f60859ffb4ff6911fb69bd94d8` |

- 传输后在 Windows 重算哈希，不一致就停。
- 不要用 `cb28095`、`4d23f89` 的旧 APK，也不要用 `v0.5.0-beta` 发布里的旧 Server。
- GUI Server 需 WebView2；防火墙放行 UDP **65531**；解压到可写目录。

## 本轮硬性检查

- `playback.drift_clock source=…`：**只有 `hardware` 才能判漂移精度**；`accepted` 说明播放头不可用，按原因记 `NOT RUN`/`FAIL`。
- 同时记录 `hardware_missing_reports`、`accepted_fallback_reports`、`head_reanchor_events`、`head_give_up_events`。
- `playback.drift_interval` 的覆盖（`valid_estimates == reports`）与**全区间极值**；`playback.drift` 只作诊断。
- 缓冲、Server `pending_frames` / `media.drops`、re-anchor 计数（历史单周期 +7 不得复现）。

历史结果供对照：[RESULT-B3](/home/w0n9/workspaces/audio-share/.scratch/wire-protocol-v2/evidence/m5-09/RESULT-B3.md)（accepted 时钟，−645…+532）、[RESULT-WINDOWS-M5-11](/home/w0n9/workspaces/audio-share/.scratch/wire-protocol-v2/evidence/m5-11/RESULT-WINDOWS-M5-11.md)（hardware，−1,390…+1,529，4× 采样率）。

## 回传格式

把脱敏结果写到 `.scratch/wire-protocol-v2/evidence/m5-11/RESULT-WINDOWS-48K.md`，包含：产物哈希、设备/OS/API、**采集格式与自检数值**、窗口起止、`drift_clock` 计数、区间极值与覆盖、re-anchor/trim、Server 统计、逐条判据结论、`NOT RUN` 清单、与上一轮的偏差。原始日志与截图留本地，去掉地址、设备/身份标识与配对内容（见 [evidence-policy](/home/w0n9/workspaces/audio-share/docs/agents/evidence-policy.md)）。

## 回传后（Linux 侧）

1. 判 `PASS`/`FAIL`/`NOT RUN`；任一项未过都不关闭 M5-09、不判 B 通过、不合 beta。
2. 若本轮仍 FAIL：把「48 kHz 同条件」事实补进 M5-11；是否修订判据（spec §8 只限制修正 ±100，§11.2 的漂移验收是「速率匹配介入、缓冲有界、修正计数」）或继续改估计器，由维护者裁决。
3. 若出现新的时钟/漂移异常：按 `diagnosing-bugs` 先建 RED，不调阈值。
4. 更新 `map.md` 与工单 Comments。

## 已知限制

- 真机 ±100 ppm 注入、真实网络损伤、双真机、API 26、60 分钟 soak 均 `NOT RUN`。
- 应用 ring 在历史两轮为 0–61 ms（目标 60），按现有设计不触发插帧；记录即可，不据此改代码。
- `getTimestamp()` 在本机可用（上一轮 `source=hardware` 全程有效），但逐窗精度未达标；本轮不假设它会改善。

## 建议调用的 skills

- `diagnosing-bugs`：复测出现新的漂移/时钟异常时，先建失败复现。
- `code-review`：若需改生产代码，两轴审查固定点用 `d06ccb9`。
- `implement-spec`：仅当后续按工单图继续推进时使用；本轮是人工验收，不需要。
- 视觉 MCP（`magpie-vision`，非 skill）：需要从截图读设置或状态时使用，入库前先脱敏。
