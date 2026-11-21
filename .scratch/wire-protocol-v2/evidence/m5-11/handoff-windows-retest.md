# Handoff — 到 Windows 上复测 M5-11（用例 B）

交接时间：2026-11-21。上一会话在 Linux 上完成 M5-11 实现、两轴审查与合入；本文件只写**下一步做什么**，背景一律引用路径。

## 起点

- Linux 仓库 `/home/w0n9/workspaces/audio-share`，分支 `integration/m5` @ `c7dd28a` 起，工作区干净。
- 未 push。beta 分支 `integration/wire-protocol-v2-beta` @ `8c35411`、标签 `v0.5.0-beta` @ `362d023` 均未动，**本轮不得动**。
- 工单：M5-[11](/home/w0n9/workspaces/audio-share/.scratch/wire-protocol-v2/issues/m5-11-playout-clock-source.md) `ready-for-human`（只剩真机门）、M5-[09](/home/w0n9/workspaces/audio-share/.scratch/wire-protocol-v2/issues/m5-09-drift-step-and-burst-trim.md) `Blocked by: M5-11`、M5-[10](/home/w0n9/workspaces/audio-share/.scratch/wire-protocol-v2/issues/m5-10-reanchor-burst-investigation.md) `needs-triage`（本轮不实施）。

## 目标

在 Windows GUI Server + 单台 Android 真机上重跑**用例 B**，有效播放 ≥5 分钟，正常 LAN、屏幕常亮、不注入网络损伤或时钟偏移。**开测前把 Windows 默认播放设备格式固定为 48 kHz / 2 声道并记录**（见清单第 2 步）；窗口内若仍为约 4 倍帧计数率，本轮记 `NOT RUN`，不与历史 B 比较。步骤与判据只写在 [Windows 复测清单](/home/w0n9/workspaces/audio-share/.scratch/wire-protocol-v2/evidence/m5-09/windows-retest.md)：**按清单执行，不另行发挥。**

两轮历史结果：[RESULT-WINDOWS-M5-11](/home/w0n9/workspaces/audio-share/.scratch/wire-protocol-v2/evidence/m5-11/RESULT-WINDOWS-M5-11.md)（`71e0155`，硬件时钟，FAIL）与 [RESULT-B3](/home/w0n9/workspaces/audio-share/.scratch/wire-protocol-v2/evidence/m5-09/RESULT-B3.md)：漂移精度 FAIL（−645…+532 ppm，67/67 区间越界），覆盖与 re-anchor 通过。

本轮（2026-11-21 决定）**只做 48 kHz 同条件重跑**，排除上一轮采集帧计数率约为 B3 4 倍的不可比因素；判据、代码和产物均不变。

## 本轮新增的硬性检查：时钟源

- 必须记录 `playback.drift_clock source=…`。
- **只有 `source=hardware` 才能用来判漂移精度。** 若为 `accepted`（`head_give_up_events > 0` 或启动回退），说明该设备的 `getTimestamp()`/播放头不可用，本轮按该原因记 `NOT RUN`/`FAIL` 并保留诊断，**不能**判 PASS。
- 同时记录 `hardware_missing_reports`、`accepted_fallback_reports`、`head_reanchor_events`、`head_give_up_events`，以及 `playback.drift_interval` 的覆盖与极值。

## 产物（Linux 本地，`.build-artifacts/` 被 git 忽略，需自行传输）

| 文件 | 构建提交 | SHA-256 |
|---|---|---|
| `app-debug-arm64.apk` | `71e0155` | `123e04fbd8c5e9ea15556bdfd38357a3ad4c8e93f0fc7275e615537a29d223e7` |
| `AudioShareServer.exe`（复用，Server 未改） | `431997b` | `ee7a35a4bccad6bc09344fd7672c703e6e5466f60859ffb4ff6911fb69bd94d8` |

- 传输后在 Windows 重算哈希，不一致就停。
- 不要用 `cb28095`、`4d23f89` 的旧 APK，也不要用 `v0.5.0-beta` 发布里的旧 Server。
- GUI Server 需 WebView2；防火墙放行 UDP **65531**；解压到可写目录。

## Windows 侧

- 仓库不必在 Windows 上；需带过去的是本文件、上面的复测清单、M5-09/10/11 三张工单（都已在 `c7dd28a`）。
- 采集：客户端 `playback.drift_clock` / `drift_interval` / `drift` / `trim` / 缓冲快照；`dumpsys media.audio_flinger`；Server `media.stats` / `media.drops`。
- 证据政策见 [evidence-policy](/home/w0n9/workspaces/audio-share/docs/agents/evidence-policy.md)：只按实际跑过的平台写 `PASS`；原始日志与截图留本地，回传前去掉地址、设备/身份标识和配对内容。

## 回传后（Linux 侧）

1. 按清单判 `PASS`/`FAIL`/`NOT RUN`；任一项未过都不关闭 M5-09，不判 B 通过，不合 beta。
2. 若 `source=accepted`：不判通过，把设备时钟源不可用的事实写进 M5-11，再决定是否需要第二档回退方案。
3. 若 re-anchor 仍突跳（历史单周期 +7）：补进 M5-10，仍不合并 beta。
4. 若有新的漂移异常：按 `diagnosing-bugs` 先建 RED，再改代码，不调阈值。
5. 更新 `map.md`、工单 Comments 与 [Windows 复测清单](/home/w0n9/workspaces/audio-share/.scratch/wire-protocol-v2/evidence/m5-09/windows-retest.md) 的产物表。

## 已知限制

- host 上硬件头矩阵逐窗 0 越界、均值 ±3 ppm、修正不顶轨（[host 证据](/home/w0n9/workspaces/audio-share/.scratch/wire-protocol-v2/evidence/m5-11/host-clock-source.md)）；裸 32 位播放头启动回退路径在 host 上仍可能越界，真机未验证。
- 真机 ±100 ppm 注入、真实网络损伤、双真机、API 26、60 分钟 soak 均 `NOT RUN`。
- 上一轮实测的应用 ring 为 0–26 ms（目标 60），按现有设计不触发插帧；本轮记录即可，不据此改代码。

## 建议调用的 skills

- `diagnosing-bugs`：复测出现新的漂移/时钟异常时，先建失败复现。
- `code-review`：若需改生产代码，两轴审查固定点用 `c7dd28a`。
- `implement-spec`：仅当后续按工单图继续推进时使用；本轮是人工验收，不需要。
- 视觉 MCP（`magpie-vision`，非 skill）：需要从截图读状态时使用，入库前先脱敏。
