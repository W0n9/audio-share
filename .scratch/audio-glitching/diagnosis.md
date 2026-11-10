# 诊断：实际使用中的断续卡顿与刺啦爆音

Status: diagnosed 2026-11-10. Server-side parity fix applied; the dominant cause is
the released v0.3.4 Android Client's 1x AudioTrack buffer, not packet loss.

## 症状与测量回路

真实设备：Xiaomi M2007J22C（Android 12/API 31），已安装 v0.3.4 APK，通过
Windows Mobile Hotspot（PC `192.168.137.1`，手机 `192.168.137.212`）连接。

`measure-playback.ps1` 启动真实 Server、点击客户端播放，然后每约 0.5 s 采样：

- 手机 `wlan0` 累计接收字节（`/proc/net/dev`）；
- App 的 AudioTrack `Underruns` 计数（`dumpsys media.audio_flinger`）。

Underrun 帧数增长 = 播放缓冲被混音器读空，即用户听到的断续/爆音的客观代理。

## 关键结果（f32/2ch/48 kHz，客户端默认缓冲 1x = 2052 帧 ≈ 42.75 ms）

| Server | 客户端缓冲 | 测量时长 | Underrun | 手机接收速率 |
|---|---|---:|---:|---:|
| 0.4.0 CI `as-cmd` | 1x | 22.6 s | 2.24 % | 399.5 KB/s |
| 0.4.0 CI `as-cmd` | 1x（第二次） | 32.5 s | 2.56 % | 397 KB/s |
| 0.4.0 CI `as-cmd` | 4x（8208 帧） | 22.2 s | **0 %** | 399.2 KB/s |
| v0.3.4 C++ `as-cmd` | 1x | 21.8 s | 5.98 % | 400.2 KB/s |
| 0.4.0 + timer fix | 1x | 20.9 s | 2.02 % | 399.1 KB/s |

手机接收速率与完整码流（384 KB/s PCM + UDP/IP/Wi-Fi 开销 ≈ 396.6 KB/s）
一致：**没有丢包**。因此不是网络丢包或 UDP 发送缓冲丢弃。

16:39 的 GUI 实机会话（server.log UTC `08:39:18–08:40:06`，本地
`16:39:18–16:40:06`）在 40 s 内累计 260,085 underrun 帧 ≈ 5.4 s，即约
13.5 %；同一时段手机接收正常。

## 机制

v0.3.4 客户端（`NetClient.kt` / `AudioPlayer.kt`）：

- `AudioTrack` 缓冲 = `getMinBufferSize() * bufferScale`，默认 `bufferScale=1`
  → 48 kHz float stereo 下 16,416 B = 2052 帧 ≈ 42.75 ms；
- 每个 UDP datagram 在 **Main dispatcher** 上启动协程并调用
  `write(..., WRITE_NON_BLOCKING)`，返回值被忽略；
- 客户端从不 pre-roll：`play()` 时缓冲为空，之后按实时速率写入。

服务端按实时速率发送（48,000 帧/s），所以客户端缓冲长期停在 **0 附近**，
没有任何余量吸收以下抖动：Windows 调度、Wi-Fi（Mobile Hotspot 与外部 AP
共用射频）、主线程协程调度。任何超过缓冲余量的到达/写入延迟都会产生
underrun；持续 underrun 还会触发 `restartIfDisabled`（`releaseBuffer()`），
造成可听的断音与爆音。

4x 缓冲（171 ms）提供余量后，同样网络、同样 Server 下 underrun 为 0，
验证了缓冲余量是直接控制量。

## 服务端发现与已保留的修复

Windows 默认进程定时器精度约 15.6 ms，而采集循环用
`Condvar::wait_timeout(min_period=3 ms)` 轮询：

- 修复前：trace 显示读取成对出现（间隔 0 ms 与 14–16 ms），每次突发
  10–30 ms 音频，p50 = 14 ms；
- v0.3.4 C++ Server 的 asio steady timer 能按约 3 ms 触发；
- 修复 `capture/windows/backend.rs` 在采集线程生命周期内调用
  `timeBeginPeriod(1)`/`timeEndPeriod(1)` 后：p50 = 10.0 ms，p99 = 13 ms，
  max = 14 ms，恢复按设备周期发送。

该修复消除了服务端突发，但在客户端 1x 缓冲下 underrun 仅从 ≈2.2 % 降到
≈2.0 %（随测量波动），**不能替代客户端缓冲余量**。

## 已排除的方案

- **服务端 pre-roll**：在 UDP bind 时补发最近 40–100 ms 音频，实测
  underrun 仍为 1.6–2.1 %，没有实质改善（补发量已远超 42 ms 客户端缓冲）。
  说明客户端没有把这段突发保留为可用余量，已回退该实现。
- **UDP 发送丢弃**（`try_send_to` WouldBlock）：server.log 无
  "UDP send buffer full" 警告，手机接收速率完整。
- **0.4.0 相对 v0.3.4 的回归**：旧版 C++ Server 在 1x 下同样 underrun
  （实测 5.98 %，高于 0.4.0），这是 v0.3.4 客户端 + 弱抖动余量的结构性问题。

## 建议

1. **用户侧（立即可用）**：在 Android Client 的「音频 → 缓冲区倍数」把 1x
   调到 3x–4x。实测 4x 下 underrun 为 0。
2. **客户端后续版本**：在客户端加入 jitter buffer / 启动 pre-roll
   （先填满约 40–80 ms 再开始播放）。0.4.0 冻结客户端，因此只能作为后续
   版本工作。
3. **服务端**：保留 timer-resolution 修复（恢复与 v0.3.4 一致的采集节奏）。
   若要进一步降低对 1x 客户端的影响，需要客户端侧可观测性（
   `write()` 返回值、到达时间戳），当前仓库没有 Android 构建环境
   （无 SDK/JDK），未做客户端插桩。

## 证据

- `.scratch/audio-glitching/measure-playback.ps1`：可重复的 HITL/adb 回路。
- 原始测量数据（`run1/`、`ab1-new-buf1/`、`ab-new-buf4/`、`ab-old/`、
  `final-timer-buf1/` 等各次的 summary.json / monitor.csv / server trace）
  只保留在本地，由 `.gitignore` 排除，可用脚本重新生成。
- GUI 会话证据：`audio-share-artifacts/ci-42777988420/gui/server.log` 与手机
  `dumpsys media.audio_flinger` 轨道历史（16:39–16:40 会话，
  track 133：260,085 underrun 帧）。
