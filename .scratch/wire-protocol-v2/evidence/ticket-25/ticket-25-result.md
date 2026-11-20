# 工单 25 验收结果（Windows 11 + Android 真机）

本仓库收录脱敏摘要；原始采集文件仅保留在本地。

日期: 2026-11-17
执行人: Claude（记录 + adb 操作）/ 用户（GUI 物理点击）
环境: Windows 11 Enterprise 10.0.26300 x64；WebView2 157.0.4341.46
工作区: C:\AudioShareBeta\（portable）
证据: C:\AudioShareBeta\evidence\（notes.md、hashes.txt、soak.log、screenshots\、server.log）

## 设备与版本

- 手机: Xiaomi M2007J22C (cannon), Android 12 / API 31, arm64-v8a
- 网络: Windows 移动热点，手机 <REDACTED>，PC <REDACTED>
- Server: AudioShareServer.exe 0.5.0-beta，SHA-256 e4575d25b19ab12fcdf626d1a592f52ac382a2f052c05e6bd82702a9c3c8e97c
- as-cmd: 0.5.0-beta，SHA-256 d2917467b28eb023e8dda7e675bd10a37595111fd14400219c4a966020e73ba5
- Client beta: dev.audioshare.v2 0.2.0-beta (2)，APK SHA-256 6da98e4adae0c72e04ca8c4aa6e6d4ed804a81c8651194ccf5a334a402845141（手机已装包与发布 APK 字节一致）
- v1: io.github.mkckr0.audio_share_app 0.3.4 (3004)，来源 GitHub Release
- Server v2 fingerprint: 初次 <REDACTED>；identity reset 后 <REDACTED>

## 结果

| 项 | 结果 |
|---|---|
| 产物哈希、PE 版本、as-cmd 运行 | PASS |
| GUI 启动/版本 0.5.0-beta、v1 65530、v2 65531 | PASS |
| QR 主路径配对 | PASS |
| SAS 路径配对 | PASS |
| 30 分钟连续播放（无断线） | PASS |
| 锁屏 145 s 后唤醒仍 PLAYING | PASS |
| 断网 15 s 恢复（约 3 s 恢复播放） | PASS |
| 通知 Stop → CLOSED / 701 CLIENT_STOPPING | PASS |
| App 重启自动连接（无需重配对） | PASS |
| GUI 设备列表 + 撤销 → 手机 204、no auto reconnect | PASS |
| 撤销后新连接立即被拒（204） | PASS |
| 撤销后重新配对成功 | PASS |
| as-cmd identity reset（clients 清空、新指纹） | PASS |
| 旧凭据 → 205 SERVER_IDENTITY_MISMATCH | PASS |
| as-cmd clients list 离线可用 | PASS |
| v1 0.3.4 共存、连接 65530、播放/停止 | PASS |
| v1 明文/无认证警告（release notes、About、as-cmd --help） | PASS |
| 重置后重新配对并恢复 PLAYING | PASS |

## 结论

**有条件 go**。

阻塞项（建议修复后再发布）：
1. 撤销或配对被拒后，点“Pair with code”进入 AUTHENTICATING 并收到 PairingReady，但客户端不显示 SAS 确认界面；旧的 “This device was revoked”/“Pairing was rejected” 横幅不会被新状态清除。清除 App 数据后同一流程正常。可复现两次。

非阻塞观察：
2. QR 模式首次打开时“剩余有效期”未显示倒计时（SAS 模式正常）；需专项复测。
3. 首次 QR 配对在 PairingReady 后 12 s 由客户端以 701 CLIENT_STOPPING 自行结束，邀请被消耗；原因未定位。
4. 用户主动 Stop 后，Home 仍显示 “Connection lost — Check the network…”。
5. server.log 只有启动/停止行，没有 ClientClosed(701)、配对或撤销事件，无法从服务端日志验证 §9.1 的 701 记录。

## 限制与说明

- 音质/时延/underrun 仅主观观察，未做像素级或统计断言。
- 未修改 v0.3.4 APK、fixtures、accepted 协议；未提交、未打 tag、未上传 Release。
- 本机无 keytool/apksigner，签名证书未独立复算；以已安装 APK 与发布 APK 字节一致作为身份证据。
- 证据编号 71 的整屏截图因拍入无关窗口内容已删除，不在证据集内。
