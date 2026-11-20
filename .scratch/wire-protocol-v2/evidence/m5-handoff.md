# M5 单真机验收 handoff（Windows GUI Server；每项 5 min）

- 代码基线：`integration/m5` @ `800c40c`（M5-03/04/05/06 + 两轮 review 修复）
- 口径：单真机 + Windows Server，**每项 5 min**；本版本**忽略双真机并发**；真实网络损伤行本轮**跳过（NOT RUN）**。
- 未 push、未打 tag。Windows 侧无需 Rust。

## 0. 现成产物（Linux 上 cargo-xwin 交叉构建）

目录：`.build-artifacts/m5-windows-gui-800c40c/`

| 产物 | 说明 | SHA-256 |
|---|---|---|
| `AudioShareServer.exe` | **GUI Server**（Tauri，portable，静态 CRT，含 M5-05 timestamp 重锚 + `media.drops`），15,076,864 B | `321b747ae536c5bcc5d10f543957e5972e866a1dc4f7178ebd1c4b5dc5df81ea` |
| `audio-share-server-cmd-windows.zip` | CLI Server（`as-cmd.exe`，同一 M5 代码） | `4a4c8983bee7df14b512b27ca3a2dd22da1a8b4136a589c098fdc8420dd8b9d3` |
| `app-debug-arm64.apk` | Android Client（arm64-v8a，debug，`dev.audioshare.v2` 0.2.0-beta） | `56e27380b86f6ff2e2144f576575946245f488f76051327699f996ccedaf2878` |

传输到 Windows 后请重算哈希。GUI 与 CLI 均由 `800c40c` 构建；**不要再用发布里旧的 0.5.0-beta 产物**。

## 1. Windows 前置

- Windows 11 x64。
- **GUI**：需要 WebView2 Runtime（Win11 通常自带）；静态 CRT，无需 VC++ Redist。
- **CLI（`as-cmd.exe`）**：动态依赖 `VCRUNTIME140.dll`，需要 VC++ 2015-2022 Redistributable (x64)。
- 防火墙放行入站 **UDP 65531**（v2；如同时测 v1 则 65530）。网络配置文件设为“专用”，关闭 AP 隔离。
- 把 `AudioShareServer.exe` 解压到**可写目录**（如 `C:\AudioShareM5\`）；`config.ini` 与 `server.log` 写在该目录。

## 2. 启动 GUI Server

```powershell
C:\AudioShareM5\AudioShareServer.exe
```

- 在 GUI 中确认 v2 已启用（UDP 65531，ALPN `audio-share/v2`）。
- 确认同目录 `server.log` 出现 `v2 identity ready` 与 `v2 authorization store ready`，以及 v2 会话事件。
- 记录 GUI 显示版本（应为 `0.5.0-beta`）与 `AudioShareServer.exe` 哈希。
- 若需 headless/命令式操作，可用 zip 内的 `as-cmd.exe`（同一数据目录或 `AUDIO_SHARE_CONFIG`）。

## 3. 安装 Client

```powershell
adb install -r app-debug-arm64.apk
# 若与已装版本签名不一致：adb uninstall dev.audioshare.v2 后重装（会丢旧配对）
adb shell dumpsys package dev.audioshare.v2 | findstr /C:"versionName" /C:"versionCode"
```

打开 App，选择 v2，输入 Windows 主机 LAN 地址与端口 **65531**，完成配对。

## 4. 用例 A — M5-03 听测（5 min）

1. 开始播放，屏幕常亮，计时 ≥5 min。
2. 判定：连续、无新增咔哒/爆音/断续（对照 M5-02 基线）。
3. 抓取（有 adb 时）：
   ```powershell
   adb shell dumpsys media.audio_flinger > aflinger-A.txt
   adb logcat -d | findstr /I "playback drift conceal trim underrun" > client-A.log
   ```
   Server 侧：从 `server.log` 摘出 `media.drops`（服务端丢帧）与 v2 会话事件。
4. 关注：`playback.trim`（软修剪，应少量、不计 concealment）、`concealments`/`concealed_frames`、`late_dropped`、平台 `underruns`；`media.drops` 应与客户端 `cumulative_lost`/`late_dropped` 分开。

## 5. 用例 B — M5-04 漂移基线（5 min）

1. 正常播放 5 min，记录 `playback.drift` 的 ppm 估计与 correction ppm。
2. 判定（基线）：ppm 接近 0；无周期性 `Drift` concealment；缓冲保持有界。
3. **±100 ppm 注入：本轮 NOT RUN**（Server 无 capture 速率偏移开关；不要写 PASS）。

## 6. 用例 C — M5-06 真实网络损伤

**本轮跳过（NOT RUN）**：Windows 无 `tc netem`，且你已选择不在本轮测网络行。保留到有 Linux 网关/VM 时再补。

## 7. 证据回传格式

- 设备型号/OS、app versionName/versionCode、GUI/CLI 版本与哈希、Windows 版本；
- 每项：起止时间、时长、主观听感一句话、判定 PASS/FAIL/NOT RUN；
- `aflinger-*.txt`、`client-*.log`、`server.log` 关键摘录（或整文件）。

我据此写 `evidence/m5-03..05/` 并更新工单 `## Comments` 与 status；只有真实跑过且达标的才写 PASS。

## 8. 本版本明确 NOT RUN

- M5-06 真实网络损伤全部行（本轮跳过）。
- M5-04 的 ±100 ppm 注入。
- 双真机并发、60 min soak（留 stable/M7）。
- M5-05 的 L1 分项与上游调优（实验分支不转正）。
- M5-01 可听延迟 p50/p95/p99（需录音设备，并行）。
- Windows 上实际执行：构建成功不等于运行 PASS；执行结果由你在 Windows 上记录。
