# L1 阶段 1 — Windows Server 采集时间戳埋点（handoff）

## 这是什么

带两个**只加日志、不改音频行为**的埋点的 Windows Server 构建，用于测端到端延迟的分项：
- 采集侧：`AUDIO_SHARE_CAPTURE_TIMING=1` 时每秒输出「采集帧位置 ↔ WASAPI QPC 时间戳」
- 播放侧：`AUDIO_SHARE_L1_TIMING=1` 时每次 §9 诊断输出客户端回报的 `submitted_position`

两个日志都走 stderr，GUI 启动时会写进 `server.log`。环境变量不设时**零开销、行为不变**。

产物（本机 Linux 用 cargo-xwin 交叉编译，与正式发布同路径）：
- `AudioShareServer.exe` 14.4 MB `sha256 dd6ba94850f3b5fc1b1200b8edbe5707514165073f9c711b9ff30afabd1257fc`
- `as-cmd.exe` 5.0 MB `sha256 018096a0ef7d8fef2f24fa00139b0e231608af98256e2ceeea53ed3f47b6c6df`

> 修订：第一版用了 `eprintln!`，但 GUI 的 `server.log` 只接收 `tracing` 事件
> （`BridgeLayer` 仅放行 `v2_transport::session_log` target），日志里看不到。已改用
> `tracing::info!` 并走该 target，日志格式见下。

源码分支：`experiment/m5-audiotrack-buffer`（未提交、未推送）。

## Windows 上怎么做

1. **先停止当前正在运行的 Server**（GUI 和 as-cmd 都要停，端口 65530/65531 不能被占）。
2. 备份当前目录里的 `identity/` 和 `pairing/`（配对数据）——**不要删**，新 exe 复用它们，手机不用重新配对。
3. 把 `AudioShareServer.exe` 覆盖到你现有的 `dist-windows` 目录（同名替换）。
4. **设置两个环境变量后启动**（PowerShell，在该目录下）：

```powershell
$env:AUDIO_SHARE_CAPTURE_TIMING = "1"
$env:AUDIO_SHARE_L1_TIMING = "1"
.\AudioShareServer.exe
```

   （如果用 cmd：`set AUDIO_SHARE_CAPTURE_TIMING=1` 然后 `set AUDIO_SHARE_L1_TIMING=1` 再 `AudioShareServer.exe`）

5. 像平常一样在 GUI 里开始播放，让手机连上并播放至少 **2 分钟**。
6. 播放时**保持屏幕常亮**，不要锁屏。
7. 停止播放后，把 `server.log` 发回（或直接让我从日志里读——见下）。

## 日志里要看什么

```
[时间] [info] v2 L1 capture timing event=l1.capture_timing frame=<采集帧位置> qpc_100ns=<WASAPI QPC> device_index=<设备帧号> frames_in_packet=<包内帧数> silent=<bool>
[时间] [info] v2 L1 playback timing event=l1.playback_timing session_id=<会话> submitted_position=<客户端播放到的采集帧位置> buffer_current_ms=<ring 深度> underruns=<计数> instance=<实例>
```

两者用同一个「采集帧位置」坐标系：`submitted_position` 落后 `frame` 多少帧，乘以 20.83 µs/帧（48 kHz）就是**采集→客户端提交的延迟**。`buffer_current_ms` 用来判断延迟是否还在 ring 里。

## 日志怎么回传

本机 Linux 与这台 Windows（测试主机 B / `<REDACTED>`）不同 tailnet 账号，无法 Taildrop。可选：
- 你直接把 `server.log` 贴给我；或
- 在 Windows 上起个文件服务让我拉；或
- 在 Windows 上开 SSH/taildrop 接收，我把日志解析脚本送过去。

## 恢复

测量完把原来的 `AudioShareServer.exe` 覆盖回去即可，`identity/` 和 `pairing/` 不变，配对仍然有效。
