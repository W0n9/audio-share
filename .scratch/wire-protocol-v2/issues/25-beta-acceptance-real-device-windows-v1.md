# 25: Beta acceptance — real device, Windows, v1 regression, evidence

**What to build:** The beta is accepted for the 1–3 customer pilot with recorded evidence. One real-device session covers pairing, playback, lifecycle, revocation, and v1 coexistence; emulator evidence is recorded honestly; Windows is smoke-tested or explicitly NOT RUN.

**Blocked by:** 23, 24

**Status:** resolved

- [ ] One real arm64/API 31 device completes: signed install → QR pairing → SAS pairing → READY→PLAYING → 30 minutes continuous playback → lock/wake continuity → temporary network drop and recovery → revocation terminates the active session → re-pair → v1 APK coexistence smoke.
- [ ] API 26 functional evidence comes from the emulator and is recorded as `EMULATOR`, never as real-device PASS.
- [ ] Windows Server GUI pairing plus a 5-minute playback smoke is recorded, or Windows is explicitly `NOT RUN`.
- [ ] v1 fixture replay and the frozen v0.3.4 APK regression pass.
- [ ] Evidence records device model, OS/API level, app and Server versions, and the known limitations list.
- [ ] The beta verdict (go/no-go for the pilot) and the exit criteria are recorded.

## Comments

2026-11-16 — agent 侧工作已收口，本工单保持 `ready-for-human`，等待真机与 Windows 会话。

当前可用输入：
- 签名 universal APK：`~/.cache/audio-share-beta-worktrees/ticket-24/dist-android/audio-share-client-v2-0.2.0-beta.apk`，SHA-256 `6da98e4a…845141`，证书 `90:BE:DC:…:A1:3C`；arm64-v8a + x86_64。
- Linux Server 产物：`ticket-24/dist-linux/` 的 deb/AppImage/as-cmd（`0.5.0-beta`）。
- 模拟器证据（EMULATOR，非真机 PASS）：`evidence/ticket-20/`、`ticket-21/`、`ticket-23/`、`ticket-24/`。
- 配对/管理命令：`as-cmd pair [--sas] [--accept]`、`clients list|revoke`、`identity reset [--yes]`；GUI 配对对话框与设备列表。

仍需人工完成：
1. 一台真机 arm64/API 31 的完整会话（安装→QR 配对→SAS 配对→READY→PLAYING→30 分钟→锁屏/唤醒→断网恢复→撤销终止→重配对→v1 APK 共存）。
2. Windows Server GUI 配对 + 5 分钟播放 smoke，或显式 `NOT RUN`。
3. v1 fixture replay 与冻结 v0.3.4 APK 回归。
4. 记录设备型号、OS/API、app/Server 版本、已知限制与 go/no-go。

- 2026-11-16 补充：Windows 11 人工验收步骤已写入 `.scratch/wire-protocol-v2/windows-handoff.md`（产物传输、校验、防火墙端口、QR/SAS、播放/锁屏/断网、撤销/重置、v1 共存、证据模板）。Windows 二进制由 Linux 交叉构建后带到 Windows，Windows 端无需构建环境。
- 2026-11-16 追加：Windows 产物已就绪（built on Linux, not executed）：
  `~/.cache/audio-share-beta-release/dist-windows/AudioShareServer.exe`（SHA-256
  `2117e7af…b6712`）与 `audio-share-server-cmd-windows.zip`（SHA-256 `857c5e05…874f31`）。
  Windows 端无需构建环境；按 `.scratch/wire-protocol-v2/windows-handoff.md` 执行并记录。

## Windows 11 / 真机验收结果（2026-11-17）

证据：`evidence/ticket-25/`（`ticket-25-writeback.md`、`notes.md`、`hashes.txt`、`server.log`、`30min-logcat.txt`、93 张截图）。

- **PASS**：产物 SHA-256/签名、GUI/as-cmd/APK 版本、v1 65530 + v2 65531 启动、30 分钟连续播放、锁屏 2–5 分钟、15 秒断网后约 39 秒自动恢复、通知 Stop（701）、撤销后约 1.1 秒 `204` 且旧凭据立即被拒、身份重置后 `205`、v1 0.3.4 共存播放。
- **FAIL（阻塞 GUI beta）**：Windows GUI 的 QR 与 SAS 配对均无法批准——客户端到达 `PairingReady / AwaitingConfirmation`，GUI 从不显示待批准卡片；5 次复现（QR 粘贴、QR 人工扫码、SAS ×3，含撤销后重配对）。非 GUI 路径用 `as-cmd pair` 绕过完成了播放/生命周期测试，**不计为 GUI 配对 PASS**。
- **NOT RUN**：应用重启后自动连接（身份重置后手机无有效凭据）。
- **判定**：Windows GUI beta **NO-GO**，直到工单 29（PairingResponse 后、PairingConfirm 前展示 SAS）修复并复跑 GUI 配对。真机协议/媒体/生命周期与 v1 回归在非 GUI 路径上通过。
- **交接**：portable store 按 exe 目录隔离；`as-cmd clients/identity` 需与 GUI 的 `dist-windows` 同目录运行或使用相同 `AUDIO_SHARE_CONFIG`。已写入 handoff 修正项（工单 29）。
- 2026-11-17 复跑准备：工单 29（live SAS 呈现）已合入 `a380ee0`，Linux/Windows 产物已重建。请用新的
  `dist-windows/AudioShareServer.exe`（SHA-256 `e4575d25…c8e97c`）与
  `audio-share-server-cmd-windows.zip`（SHA-256 `2bc4ceb1…d8550d`）覆盖旧文件，重新执行：
  1. GUI 启动 server → 打开配对 → 手机 QR 或 SAS 连接；手机连入后 GUI 应立即显示
     `Pairing code` 与设备信息，确认手机后出现待批准卡片；接受设备。
  2. 5 分钟播放 + 通知 Stop。
  3. GUI 撤销 → 手机 204；重新配对（同一流程）。
  4. 记录 GUI 版本、Windows 版本、手机型号/API、时间线与 PASS/FAIL。
  APK 未变（`6da98e4a…5141`），无需重装。离线 `as-cmd` 管理仍需与 GUI 的
  `dist-windows` 同目录运行或共用 `AUDIO_SHARE_CONFIG`。

## Windows 真机复跑结果（2026-11-17，工单 29 修复后）

证据：`evidence/ticket-25/ticket-25-result.md`。

- **PASS**：产物哈希/PE/版本、GUI v1 65530 + v2 65531、QR 主路径配对、SAS 配对、30 分钟连续播放、锁屏 145 s 后仍 PLAYING、断网 15 s 约 3 s 恢复、通知 Stop → 701、App 重启自动连接、GUI 设备列表 + 撤销 → 204 且无自动重连、撤销后新连接立即被拒、撤销后重新配对、`as-cmd identity reset` + 旧凭据 205、`as-cmd clients list`、v1 0.3.4 共存播放、v1 明文警告、重置后重新配对并恢复 PLAYING。
- **阻塞项（条件 GO）**：撤销或被拒后点 “Pair with code” 会进入 AUTHENTICATING / PairingReady，但客户端不显示 SAS 确认界面，旧撤销/拒绝横幅不消失；清除 App 数据后正常。复现 2 次。→ 工单 30。
- **非阻塞观察**：QR 首次打开“剩余有效期”未显示倒计时；首次 QR 配对在 PairingReady 后 12 s 被客户端以 701 结束、邀请被消耗（原因未定位）；用户 Stop 后 Home 仍显示 “Connection lost”；server.log 缺少 701/配对/撤销事件。
- **判定**：有条件 GO；工单 30 修复并复跑阻塞场景后可给 GO。
- 2026-11-17 复跑准备（工单 30 后）：新签名 APK SHA-256
  `c1d1b23634abb05bf760bcb975bd442e65abc674610dfac11140f6dd7992af58`（`0.2.0-beta`，
  versionCode 2，arm64+x86_64）。用 `adb install -r` 覆盖旧 beta（同签名）。
  重点复跑两个阻塞场景：
  1. GUI 撤销 → 手机 204 → “Pair with code” → 应出现 SAS 确认界面，旧撤销横幅消失；
  2. GUI 拒绝配对 → 手机重新配对 → 同样应正常显示 SAS/确认，旧拒绝横幅消失。
  另复跑：通知 Stop 后 Home 不显示 “Connection lost”；QR 一步配对（扫描后手机自动确认，
  Server 仍需接受）；30 分钟播放/锁屏/断网/撤销/重配对/v1 共存可抽检。
  QR 首次倒计时与 12 s 701 观察仍待记录；server.log 事件缺口为非阻塞。
- 2026-11-17 最终复跑包（HEAD `0aec442`）：请同时更新 Server 与 APK。
  - `AudioShareServer.exe` SHA-256 `0a7dd9cef054e0d7a7175391144ae8d261a70cee675b8d7a11b15ce1c3dd4ca7`
  - `audio-share-server-cmd-windows.zip` SHA-256 `afe3b660a54172d9bbad8283db8e53f59ca1ee2fb7e110fb126d16c23233ff72`
  - APK `audio-share-client-v2-0.2.0-beta.apk` SHA-256 `c1d1b23634abb05bf760bcb975bd442e65abc674610dfac11140f6dd7992af58`
  复跑重点：撤销/拒绝后重新配对显示 SAS 且旧横幅消失；通知 Stop 后无 “Connection lost”；
  QR 一步配对（扫描后手机自动确认，Server 接受）；QR 过期后 GUI 原地刷新；30 分钟播放/锁屏/
  断网/撤销/重配对/v1 共存抽检。Windows 与真机结果记录到本工单。

- 2026-11-17 状态核验：阻塞项对应的工单 30 已 `resolved`（修复合并 `33276e8`，签名 APK 更新为
  SHA-256 `c1d1b23634abb05bf760bcb975bd442e65abc674610dfac11140f6dd7992af58`）。本工单保持
  `ready-for-human`，唯一待办是用该 APK 在真机复跑“撤销 → 重新配对显示 SAS 且旧横幅消失”
  （Windows GUI 同场景），并据此把 conditional GO 升级为 GO；其余验收项证据已齐。

## 2026-11-17 最终产物短 smoke（APK c1d1b236 / Server 0aec442）

证据：`evidence/ticket-30/evidence/notes.md`、`hashes.txt`、logcat、9 张截图。

- **PASS（本轮实测）**：4 个交付文件哈希重算一致；Windows 前置（WebView2、VC++、防火墙）、GUI 与 as-cmd 实际运行 0.5.0-beta；QR 一步配对（扫描自动确认 + Server 接受）；5 分钟播放无中断；Stop → “Connect to saved Server” 重连（干净安装）；撤销 204 → CLOSED；身份重置 → 旧凭据 205；v1 共存安装与明文警告。
- **按维护者决定不抽检、沿用此前 PASS**：30 分钟连续播放、锁屏唤醒、断网恢复、撤销后重配对、v1 实际播放。记录为 `PASS (prior run; not re-run 2026-11-17)`。
- **工单 30 阻塞项复跑**：模拟器已 RED→GREEN；真机“撤销/拒绝后重新配对”本轮 **NOT RUN**（维护者决定不抽检）。按维护者裁决沿用此前通过项，但不得写成真机 PASS。
- **FAIL**：`server.log` 全程只有启动/停止行，无 v2 配对/连接/Close(701)/撤销事件；§8.5 “Server 记录 701” 不可从 Server 日志验证 → 工单 33。
- **升级路径缺陷**：旧 beta 数据经 `install -r` 保留后，“Connect to saved Server” 报 205；干净安装不复现 → 工单 34。beta 发布说明至少需写“升级前卸载旧 v2 beta”。
- **NOT RUN**：About 页目视确认；SAS/无相机路径本轮（此前 PASS）；Windows 上 APK 证书独立复算（无 keytool/apksigner；Linux `apksigner` 已验证）；v1 实际播放本轮（此前 PASS）。
- **安全事件（已处置）**：验收过程中意外在会话中打印了 Server 私钥；已用 `identity reset` 轮换，证据文本扫描无密钥，可能含密钥的全屏截图已删除，旧密钥仅留在 `.bak`。
- **维护者裁决**：本轮短 smoke 通过；是否在工单 33/34 修复前发布由维护者决定。若接受“server.log 诊断缺口 + 升级需卸载旧 beta”为已知限制，可给 fresh-install pilot GO。

## 最终裁决（2026-11-17）

维护者决定：**GO**（fresh-install pilot），接受以下已知限制并写入 release notes：

1. `server.log` 暂只记录启动/停止；v2 会话/701/撤销事件缺失（工单 33，下一轮补）。
2. 从更早 v2 beta 经 `install -r` 升级可能触发 205；本轮 Beta 要求先卸载旧 v2 beta，再安装本 APK 并重新配对（工单 34）。
3. 最终短 smoke 未重跑 30 分钟/锁屏/断网/撤销后重配对/v1 实际播放，按维护者决定沿用此前 PASS 证据；工单 30 的 revoke/reject 后重新配对修复以模拟器 RED→GREEN 为准，真机未复跑，不写成真机 PASS。

安全事件：验收会话中意外打印了 Server 私钥；已执行 `identity reset` 轮换（新指纹 `498C3EF7…FA158AF3`），证据文本扫描无密钥，含密钥的全屏截图已删除。旧密钥仅留在 Windows 的 `identity/server-identity.pem.bak`；建议在该主机上删除该 `.bak` 以完成清理。

工单 25 由此标记 `resolved`。工单 24 仍为 `ready-for-human`（正式 GitHub Release 发布未授权/未执行）。
