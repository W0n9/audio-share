# M5 Windows 复测 handoff — 自包含

> 可在 Windows 机器上单独使用，不需要本仓库其它文档。
> 代码基线：`integration/m5` @ `431997b`（M5-08 修复 1+2+3+5 已合入）。未 push。

## 0. 本次复测要验证什么

上一轮（`800c40c`）发现：服务端 pending 钉在 ~480 ms 并持续丢帧；客户端 underrun 事件 5576/5min；
trim/reanchor 偏高；drift 499 ppm 离群。只读诊断定位为：

1. **pacer 令牌桶小数帧被截断**（少发 ~2000 ppm）——已修（进位）。
2. **underrun 门控依赖 MIUI 不可信的 `AudioTrack.underrunCount`**——已改为「持续空读 ~30 ms」触发，
   并打印每次 fill 的实际帧数（`concealed_frames` / `underrun_frames`）。
3. **漂移窗口被频繁重置**——已改为扣除已知位置偏移、要求 span≥10 s 且样本≥10 才发布，
   并打印 `span_ms`/`sample_count`。
5. **服务端缺乏周期埋点**——已新增 `media.stats`（每 5 s，即使无丢弃也打印
   `pending_frames` / `consumed_frames` / `pacer_calls` / `pacer_calls_per_sec`）。

**未改**：trim 阈值（第 4 项）、§7.4 字面语义（第 6 项）、jitter hole-hold（第 7 项），等本次数据再裁决。
60/200 ms、±100 ppm、100 ms 均未改。

## 1. 产物（Linux 侧 cargo-xwin 构建）

目录：`.build-artifacts/m5-windows-gui-431997b/`

| 文件 | 用途 | SHA-256 |
|---|---|---|
| `AudioShareServer.exe` | GUI Server（含 media.stats / pacer 修复） | `ee7a35a4bccad6bc09344fd7672c703e6e5466f60859ffb4ff6911fb69bd94d8` |
| `audio-share-server-cmd-windows.zip` | CLI Server（备选） | `15bdec49e28df4913216bd224479191d36222256c65e9eb1d9013712b63cd89e` |
| `app-debug-arm64.apk` | Android Client（arm64 debug） | `146e9ed29ff54de685591972dfa88100b310dfe08cb6b7556f041d2695ee8fad` |

Windows 上校验：`Get-FileHash .\AudioShareServer.exe -Algorithm SHA256` 等，必须一致。

## 2. Windows 前置（与上轮相同）

- Windows 11 x64；GUI 需要 WebView2（自带）；GUI 静态 CRT，CLI 才需 VC++ 2015-2022 Redistributable x64。
- 放行入站 **UDP 65531**；网络设为“专用”；关闭 AP 隔离；手机与 PC 同一 LAN，时间同步。
- `AudioShareServer.exe` 放可写目录（沿用 `C:\AudioShareBeta` 即可，防火墙规则已匹配）。
- 抓日志需要 `adb`（platform-tools）。

## 3. 启动与安装

```powershell
C:\AudioShareBeta\AudioShareServer.exe
# server.log 在同目录；确认出现 v2 identity ready / authorization store ready
adb install -r app-debug-arm64.apk
# 签名不一致则先 adb uninstall dev.audioshare.v2 再装（丢旧配对）
```
App 选 v2、端口 65531、配对。

## 4. 复测 A — M5-03 听测（5 min）

1. 播放 5 min，屏幕常亮；主观判定：连续、无新增咔哒/爆音/断续。
2. 抓取：
   ```powershell
   adb shell dumpsys media.audio_flinger > aflinger-A2.txt
   adb logcat -d > client-all-A2.log
   Select-String -Path client-all-A2.log -Pattern "playback\.|conceal|underrun|drift|trim" > client-A2.log
   Select-String -Path C:\AudioShareBeta\server.log -Pattern "media\.stats|media\.drops|session\." > server-A2.log
   ```
3. **本次要重点看**：
   - `playback.concealments ... concealed_frames=… underrun_frames=…`：现在有真实帧数。
   - underrun 事件数应**远低于**上轮 5576/5min（只有持续 ~30 ms 空读才触发）。
   - `media.stats`：`pending_frames` 是否仍逼近 500 ms；`consumed_frames` 的增速是否 ≈48000/s；
     `pacer_calls_per_sec`。
   - `media.drops`：`pump_dropped_chunks` 是否还每 5 s +1（pacer 修复后应显著减少/停止）。

## 5. 复测 B — M5-04 漂移基线（5 min）

1. 正常播放 5 min。
2. **本次要重点看** `playback.drift ... drift_ppm=… correction_ppm=… span_ms=… sample_count=…`：
   - 应只在 span ≥10 s、样本 ≥10 时发布；ppm 应比上轮稳定，不再出现 499 这类短窗口离群。
   - `buffer_current_ms` 保持有界（target 60），不持续增长。
   - 无周期性 `Drift` concealment。
3. ±100 ppm 注入仍 **NOT RUN**（Server 无速率偏移开关）。

## 6. 证据回传

放 `C:\AudioShareBeta\evidence-2\`：`aflinger-A2.txt`、`client-A2.log`、`server-A2.log`、
B 的对应文件、`RESULT2.txt`（设备/版本/哈希/时长/主观判定 PASS/FAIL/NOT RUN）。
回传后由 Linux 侧写 evidence 与工单状态；只有真跑达标的写 PASS。

## 7. 判定建议

- A PASS：主观连续无新增咔哒，且 underrun 事件不再异常暴涨、`concealed_frames` 可控。
- B PASS：ppm 稳定且 span/sample 满足，buffer 有界，无周期性 Drift；服务端 `pump_dropped_chunks`
  不再持续增长、`media.stats` 的 consumed 增速 ≈ 实时。
- 若 `pending_frames` 仍持续增长：记录 `media.stats`（这是第 6 项 §7.4 catch-up 的裁决数据），不要先改代码。
- 若仍有大量 underrun：记录 `concealed_frames`，说明门控仍需调（第 4 项）。

## 8. NOT RUN（不要写成 PASS）

- 用例 C 真实网络损伤全部行（无 tc netem，本轮跳过）。
- ±100 ppm 注入。
- 双真机并发、60 min soak、M5-05 L1 分项、M5-01 p50/p95/p99。
- Windows 上的实际执行本身（构建成功 ≠ 运行 PASS）。

## 9. 约束（回写侧）

- 不 push；提交不带 Co-Authored-By；`.scratch/` 不入库；`map.md` 保持工作区改动。
- 门通过后由 Linux 侧本地 FF `integration/wire-protocol-v2-beta`，不 push/不打 tag。
- JVM 测试需 JDK21 参数（Linux 侧）。
