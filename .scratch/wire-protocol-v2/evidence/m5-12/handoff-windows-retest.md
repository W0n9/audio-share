# Handoff — Windows 真机复测 M5-12（漂移修正的噪声饱和）

交接时间：2026-11-22。Linux 侧已完成实现、两轴审查、裁决落库与合入；本文件只写**下一步做什么**，背景一律引用路径。

## 起点

- 仓库 `/home/w0n9/workspaces/audio-share`，分支 `integration/m5` @ `4120cdd`，工作区干净。
- M5-12 实现已 `--no-ff` 合入（`a4b26fe`、`4120cdd`），提交都在 `integration/m5`；**未 push、未打 tag**。
- beta `integration/wire-protocol-v2-beta` @ `51a7aae`、`v0.5.0-beta` @ `362d023` 均未动，**本轮不得动**。
- 工单：`issues/m5-12-drift-correction-saturation.md`（`ready-for-human`，含维护者两项裁决）。
- Host 证据与判据：`evidence/m5-12/host-drift-saturation.md`（§0.4 SRC 风险、§4 GREEN、§4.3 暖机、§4.4 适用域、§7 残余风险）。
- 调研背景（非需求）：`research/drift-rate-control-survey.md`。
- 两轴审查结论与整改：工单 Comments。

## 目标（只做一件事）

在 Windows GUI Server + 单台 Android 真机上做一次 **≥8–10 分钟（建议 15 分钟）** 的稳态播放，回答两个问题：

1. 速率修正是否真的生效（`setPlaybackRate` 未被拒、`source=hardware`）；
2. 顶轨比例在**新控制器生效后的快照**里是否降到 **< 10 %**。

不测：听感 A/B（单独人工门）、84–100 ppm 漂移的适用域外结论、60 分钟 soak、网络损伤矩阵。

## 产物

| 文件 | 构建提交 | SHA-256 |
|---|---|---|
| `app-debug-arm64.apk` | `4120cdd` | `9a039a042e2f934e4b17334150273e72a4a01b5361ce17ea494f903ab25b8fdf` |
| `AudioShareServer.exe`（复用，服务端未改） | `431997b` | `ee7a35a4bccad6bc09344fd7672c703e6e5466f60859ffb4ff6911fb69bd94d8` |

- Linux 本地路径：`/home/w0n9/workspaces/audio-share/.build-artifacts/m5-12-4120cdd/`；`.build-artifacts/` 被 git 忽略，需自行传输。
- 传输后在 Windows 重算哈希，不一致就停。
- 不要用 `71e0155` 或更早的 APK；Server 用上面这个 exe（GUI 版需 WebView2，防火墙放行 UDP **65531**，解压到可写目录）。

## 开测前置（必须做并留记录）

1. **整数 Hz 执行器上限取决于采样率**：48 kHz → 83.3 ppm（4 Hz），44.1 kHz → 90.7 ppm（4 Hz），8 kHz → 0。记录本轮实际采样率，判据随之取上限。
2. Windows 默认播放设备格式优先与上一轮同条件：**48 kHz / 2 声道**（设置 → 系统 → 声音 → 更多声音设置 → 播放 → 默认设备 → 属性 → 高级 → 默认格式）。改不了就记录实际值，不要伪造。
3. **第一件事**：`adb logcat` 里 grep `setPlaybackRate(.*) rejected`。
   - 出现 → 速率修正整体失效，本轮记 `FAIL`（或按原因 `NOT RUN`），**不要**用顶轨数字下结论。
4. `playback.drift_clock source=hardware` 必须全程成立；`accepted` 说明播放头不可用，按原因记 `NOT RUN`/`FAIL`，并记录 `hardware_missing_reports`、`accepted_fallback_reports`、`head_reanchor_events`、`head_give_up_events`。
5. 服务端自检：`media.stats` 帧计数率与采样率一致（48 kHz ≈ 48,000/s、pacer ≈ 300/s）。

## 执行

- 屏幕常亮、同一 Server、同一路径、不注入网络损伤（抖动/重连属 M5-07）。
- 有效播放 **≥8–10 分钟，建议 15 分钟**；记录起止时间。
- 前 **180 s 是暖机**（长基线未满）：这段单列，**不进顶轨判据**。真机验收从第一条带 `residual_ppm=` 的 `playback.drift` 开始计。

## 判据（按 2026-11-22 维护者裁决）

1. **顶轨比例**：只统计 `residual_ppm != none` 的 `playback.drift` 快照；顶轨定义是**命令值停在执行器上限**（48 kHz：`|correction_ppm| ≥ 83`）。
   - **< 10 % → PASS**；否则 `FAIL`。
   - 不要把整段会话（含暖机/重建）的比例混进判据，但两者都要报告。
2. **`residual_ppm` 的长期行为**：中位数/均值接近 0（对照上一轮 48K：中位数 −1、均值 −9.6 ppm），并且逐窗原始 `drift_ppm` 的 ±1,000–1,800 ppm 噪声**不应**再传导致执行器。
3. **缓冲**：`buffer_current_ms` 不持续增长；记录分布（上一轮 0–28 ms，目标 60）。
4. **覆盖与重建**：`discontinuous_intervals`、`drift_window status=rebuilding` 次数。注意长基线是 180 s，每次不连续都会重建；重建频繁时新控制器很少生效，要单列。
5. **Server**：`pending_frames`、`media.drops`；re-anchor/trim 计数（历史单周期 +7 不得复现）。
6. **对照**：上一轮 48K 是 **118/186（63 %）** 顶在 ±100（旧实现、旧定义），只作量级对照，不能与新定义直接相减。

## 适用域（别误判）

- **|d| > 83.3 ppm**（48 kHz）时命令停在执行器上限是物理分辨率，不是噪声饱和；残差交给 §8 整帧干预（host 用例 `d = +90 ppm`：残差 +6.9 ppm）。这类会话不能据此判 `FAIL`，但要在报告里说明。
- 8 kHz 路由无法用速率修正（整数 Hz 下没有一步落在 ±100 ppm 内），依赖整帧干预。

## 回传格式

脱敏结果写到 `.scratch/wire-protocol-v2/evidence/m5-12/RESULT-WINDOWS.md`，包含：

- 产物哈希、设备/OS/API、**实际采样率与执行器上限**、Windows 默认格式、窗口起止；
- `setPlaybackRate rejected` 检查结果、`drift_clock` 计数；
- 快照总数与 `residual_ppm != none` 数、顶轨比例（新定义）、暖机段单列比例、`residual_ppm` 中位数/均值、`correction_ppm` 最大绝对值；
- 缓冲分布、`discontinuous_intervals`/重建次数、Server 统计、re-anchor/trim；
- 逐条判据结论（`PASS`/`FAIL`/`NOT RUN`/`WAIVED`）与 `NOT RUN` 清单；
- 与上一轮 48K 的偏差说明。

原始日志与截图留本地；入库前去掉地址、设备/身份标识、配对内容（见 [evidence-policy](/home/w0n9/workspaces/audio-share/docs/agents/evidence-policy.md)）。视觉读图可用 `magpie-vision` MCP，入库前先脱敏。

## 回传后（Linux 侧）

1. 按上面判据判 `PASS`/`FAIL`/`NOT RUN`，写进工单 Comments 与 `map.md`。
2. `FAIL` 且非适用域问题 → 按 `diagnosing-bugs` 先建 RED，不调阈值、不改 ±100/60/200。
3. 全程 SRC 被拒 → 工单标记为需要「实际接受速率」上行（超出本票，另开）；不要宣称噪声饱和已消除。
4. 听感 A/B 未做就继续记 `NOT RUN`；不要替维护者写 `PASS`。
5. beta 合入与 tag 都需维护者另行授权。

## 建议调用的 skills

- `diagnosing-bugs`：复测出现新的漂移/时钟异常时，先建失败复现。
- `code-review`：若需改生产代码，两轴审查固定点用 `4120cdd`。
- `handoff`：回传结果给下一会话时使用。
- `magpie-vision`（MCP，非 skill）：从截图读设置或状态，入库前先脱敏。
