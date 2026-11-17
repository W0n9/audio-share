# Windows 11 Beta 验收 Handoff

Status: 人工验收指引（2026-11-16）  
Feature: wire-protocol-v2 customer beta  
适用工单: 24（Windows 产物）、25（真机 + Windows + v1 回归）

本机（Linux）没有 Windows 构建环境，因此 Windows 产物由 Linux 交叉构建后随 APK
一起带到 Windows 上验收。Windows 上**不需要**安装 Rust、Node、Visual Studio 或
Windows SDK；只需要解压、校验、运行。

本文件只指导人工验收，不改变任何 accepted 协议或验收门槛。所有 Windows 结果按
`PASS` / `FAIL` / `NOT RUN` 记录；模拟器结果不能写成真机 PASS。

---

## 1. 需要带到 Windows 的文件

从 Linux 主机取以下目录（完整清单与 SHA-256 见各 `.sha256` 文件）：

```
~/.cache/audio-share-beta-release/
├── dist-android/
│   ├── audio-share-client-v2-0.2.0-beta.apk
│   └── audio-share-client-v2-0.2.0-beta.apk.sha256
├── dist-linux/                  # Linux 验收用，不是 Windows 产物
├── dist-windows/                # Windows portable（见第 2 节）
│   ├── AudioShareServer.exe
│   ├── AudioShareServer.exe.sha256
│   ├── audio-share-server-cmd-windows.zip
│   └── audio-share-server-cmd-windows.zip.sha256
├── release-notes-v0.5.0-beta.md
└── windows-build.log
```

另外需要（不随 beta 包提供）：

- 官方 **v1 Android APK 0.3.4**（GitHub Release 或 F-Droid 上的既有发布件），用于
  共存 smoke。不要重新构建或修改冻结的 v0.3.4 APK。
- 一部 Android 真机（arm64 / API 31 优先），与 Windows 主机在同一 LAN 或同一 VPN。

传输方式任选：U 盘、`scp`、Tailscale、共享目录。传输后请在 Windows 上重新计算
哈希，不要只信任传输过程。

---

## 2. Windows 产物状态

`dist-windows/` 由 Linux 上的 `cargo-xwin` 交叉构建（2026-11-17 重建，含工单 29 的
live SAS 修复，`0.5.0-beta`）：

```
e4575d25b19ab12fcdf626d1a592f52ac382a2f052c05e6bd82702a9c3c8e97c  AudioShareServer.exe
2bc4ceb18bd0103286c22029f602fa1aed7e8329c4faf9bc56a04a21d9d8550d  audio-share-server-cmd-windows.zip
```

- `AudioShareServer.exe` — 15,006,208 bytes，PE32+ x86-64 Windows GUI，Tauri GUI，
  portable，无需安装。前端资源已嵌入 exe。
- `audio-share-server-cmd-windows.zip` — 2,061,474 bytes；解开后是
  `audio-share-server-cmd/as-cmd.exe`（5,220,864 bytes，PE32+ console，
  SHA-256 `d2917467b28eb023e8dda7e675bd10a37595111fd14400219c4a966020e73ba5`）
  加上 LICENSE / README。

状态：**在 Linux 上构建成功，未在 Windows/Wine 上运行**（本机无 Wine）。实际运行结果
由你在 Windows 上记录；不要把“构建成功”写成 Windows PASS。

2026-11-17 重建包含工单 29 的修复：Server 在发送 `PairingResponse` 后、客户端
`PairingConfirm` 前就展示 live SAS。复跑 Windows GUI 配对时，手机连入后应立即在
配对对话框看到 `Pairing code`/设备信息；确认手机后出现待批准卡片。

若 `dist-windows/` 不存在或 `windows-build.log` 记录了交叉构建失败，则 Windows
GUI 产物为 **NOT RUN**，不要用 Linux 的 deb/AppImage 冒充。此时可先用 `as-cmd`
的 Windows 版本做 headless 验收，GUI 部分记录 `NOT RUN`，并在工单 25 注明。

Windows 产物只在本机“构建成功、未执行”（没有 Wine），实际运行状态由你在 Windows
上记录。

---

## 3. Windows 前置条件

- Windows 11 x64（Windows 10 未验证）。
- **WebView2 Runtime**：Windows 11 通常自带。若 GUI 打不开并提示 WebView2，安装
  Microsoft Edge WebView2 Evergreen Runtime。
- **VC++ 运行库（仅 `as-cmd.exe`）**：`as-cmd.exe` 动态导入 `VCRUNTIME140.dll`，需要
  Microsoft Visual C++ 2015-2022 Redistributable (x64)。GUI 使用静态 CRT，不依赖
  `VCRUNTIME140.dll`，但仍需要 WebView2。
- 将 `AudioShareServer.exe` 解压到**可写目录**，例如 `C:\AudioShareBeta\`。
  portable 模式下 `config.ini` 与 `server.log` 会写在该 exe 同目录；不要放在
  `Program Files` 这类只读目录。
- 防火墙：允许 UDP **65530**（v1）与 UDP **65531**（v2）入站。GUI 启动后如有
  “修复防火墙”入口，可用管理员权限执行；否则在“Windows Defender 防火墙 →
  高级设置 → 入站规则”手动放行这两个 UDP 端口（专用网络）。
- 网络配置文件设为“专用”，关闭 AP 隔离；若用 VPN，确认手机与 PC 在该 VPN 内互通。
- 手机与 PC 时间同步（配对 nonce 有效期 10 分钟；时钟漂移会导致 206/401）。

---

## 4. 校验产物（PowerShell）

在解压目录打开 PowerShell：

```powershell
Get-FileHash .\AudioShareServer.exe -Algorithm SHA256
Get-FileHash .\audio-share-server-cmd-windows.zip -Algorithm SHA256
Get-FileHash .\audio-share-client-v2-0.2.0-beta.apk -Algorithm SHA256
```

与对应 `.sha256` 文件比对。Windows 产物预期 SHA-256：

```
e4575d25b19ab12fcdf626d1a592f52ac382a2f052c05e6bd82702a9c3c8e97c  AudioShareServer.exe
2bc4ceb18bd0103286c22029f602fa1aed7e8329c4faf9bc56a04a21d9d8550d  audio-share-server-cmd-windows.zip
```

APK 预期 SHA-256：

```
c1d1b23634abb05bf760bcb975bd442e65abc674610dfac11140f6dd7992af58  audio-share-client-v2-0.2.0-beta.apk
```

APK 签名证书 SHA-256：

```
90:BE:DC:D1:D8:15:A0:D7:72:89:7E:92:8E:B2:C9:89:4F:6D:1C:A0:38:C5:51:9A:36:DA:6D:B2:7A:A7:A1:3C
```

如需在手机上核对签名，可用 `apksigner verify --print-certs`（Android SDK
build-tools）或 `keytool -printcert -jarfile`。

---

## 5. 启动 Server

1. 双击 `AudioShareServer.exe`，确认 About/版本显示 **0.5.0-beta**。
2. 点击启动 Server。GUI 同时启动：
   - v1：UDP 65530，保持默认启用，供 v1 Client 使用；
   - v2：UDP 65531，ALPN `audio-share/v2`。
3. 打开配对对话框。应显示：
   - QR 码与可复制的 `audioshare-pair://...` URI；
   - Server fingerprint；
   - 到期倒计时与“取消”；
   - SAS 在手机连接后、待批准卡片出现时显示（SAS 由连接派生，不能提前显示）。
4. `server.log` 位于 exe 同目录；日志中不得出现私钥、配对 secret、SAS 或原始音频。

---

## 6. 安装 Android Client

Beta APK 的 `applicationId` 是 `dev.audioshare.v2`，与 v1 的
`io.github.mkckr0.audio_share_app`（v0.3.4）不同，可共存。

有 adb 时：

```powershell
adb install -r .\audio-share-client-v2-0.2.0-beta.apk
```

无 adb 时：把 APK 传到手机，允许“安装未知来源应用”后安装。打开 App，确认：
- 版本 **0.2.0-beta**；
- Home 可输入 Server 地址（Windows 主机 LAN/VPN 地址，v2 端口 65531）；
- About 显示 beta 限制、v1 明文警告与手动更新说明。

v1 APK 安装后应仍可独立打开，v1 与 v2 两个图标共存。

---

## 7. 配对路径（两条都做）

### 7.1 QR 主路径（一步配对）

QR 模式按 2026-11-17 的维护者裁决采用一步配对：二维码已经绑定 Server 公钥，
手机扫描后自动确认，不需要再输入 SAS；Server 操作员仍需在 GUI 接受设备。

1. Server GUI 显示 QR。
2. 手机 Pairing → “扫描二维码”，授予相机权限，扫描。
3. 手机自动确认并进入等待；手机仍显示 Server fingerprint 与 SAS，可选用它们与
   GUI 上的信息比对。
4. GUI 待批准卡片显示设备名、`client_id`、来源地址、SAS；点击“接受”。
5. 手机显示配对完成并自动连接；Server 设备列表出现该设备。
6. 若二维码过期，GUI 会隐藏失效码并提供原地刷新/重新开始，不需要重启 Server。

### 7.2 SAS / 无相机路径

1. GUI 选择 SAS 模式（只开 SAS 窗口）。
2. 手机手动输入 Server 地址，选择 SAS 配对并连接。
3. 手机与 GUI 各自显示 SAS（格式 `123 456`）；在手机上输入 GUI 显示的码并确认。
4. GUI 接受设备。
5. SAS 不匹配时不得确认；记录错误提示。

---

## 8. 播放与生命周期（工单 25 主项）

按以下顺序记录时间点与截图：

1. 手机状态 `CONNECTING → AUTHENTICATING → READY → PLAYING`。
2. 连续播放 **30 分钟**；期间观察是否中断、是否自动重连、Server 是否记录异常。
3. 锁屏 2–5 分钟，再唤醒：应仍为 `PLAYING`，前台媒体 Service 存活，通知可停止。
4. 临时断网（关闭手机 Wi-Fi 或切换飞行模式 10–20 秒）再恢复：
   应自动重连并恢复播放，不需要重新配对；记录是否出现 302/601 与恢复时间。
5. 前台通知的 Stop：手机回到 `CLOSED`，Server 记录 `ClientClosed(701)`。
6. 应用重启：已有凭据时自动连接，不需要重新配对。

Windows GUI smoke（工单 25 要求）：

- GUI 配对 + **5 分钟**播放；
- 停止、重新连接；
- 设备列表与撤销。

---

## 9. 撤销、重配对与身份重置

### 9.1 撤销

1. Server GUI 设备列表 → 对当前手机选择“撤销”。
2. 预期：活动会话在数秒内终止（手机 `CLOSED`，错误码 204 `AUTHORIZATION_DENIED`），
   新连接立即被拒。
3. 手机上重新配对（新 QR/SAS），应成功。

### 9.2 身份重置（仅 `as-cmd`，GUI 内无此操作）

portable 模式下数据目录（`identity/`、`pairing/`）**按 exe 所在目录隔离**，不是全局共享：
`as-cmd.exe` 在 `extracted\` 下运行会创建它自己的 `pairing\`，看不到 GUI 的配对。
因此必须把 `as-cmd.exe` 放到 GUI 的 `dist-windows\`（与 `AudioShareServer.exe` 同目录）再运行，
或者给 GUI 与 `as-cmd` 设置相同的 `AUDIO_SHARE_CONFIG`。

先**完全停止 GUI Server / GUI 进程**（Store 锁会阻止离线管理）：

```powershell
# 把 audio-share-server-cmd-windows.zip 里的 as-cmd.exe 复制到 dist-windows\，
# 与 AudioShareServer.exe 同目录（共享同一个 portable store）
cd .\dist-windows
.\as-cmd.exe identity reset --yes
```

若改用 `AUDIO_SHARE_CONFIG`，GUI 与 `as-cmd` 必须指向同一个配置文件（数据目录随该配置解析）；
否则 `identity reset` 只会重置另一个空 store，GUI 的旧配对仍然有效。

重置后所有旧配对失效；手机上旧凭据连接应得到 205 `SERVER_IDENTITY_MISMATCH`，
必须重新配对。没有通用恢复后门。

### 9.3 离线管理

```powershell
# 与 GUI 使用同一 store：在 dist-windows\ 下运行，或设置相同的 AUDIO_SHARE_CONFIG
cd .\dist-windows
.\as-cmd.exe clients list
.\as-cmd.exe clients revoke <client_id>
```

必须在 Server 停止时运行。Server 运行时会返回：
`the Server is running and owns the authorization store; stop the Server first`。
不要为了绕过该错误而删除 `clients.json.lock`。

---

## 10. v1 共存与回归

1. 用官方 v0.3.4 APK 安装 v1（`io.github.mkckr0.audio_share_app`），与 beta（`dev.audioshare.v2`）共存。
2. Server 保持 v1 默认启用；用 v1 Client 连接 UDP 65530，播放并停止。
3. 确认 v1 仍可用，且 release notes / `as-cmd --help` 明确标注：
   **v1 无加密、无认证，仅限受信网络**。
4. 不要尝试在 beta 中关闭 v1 的独立开关（beta 不提供）。
5. 记录 v1 的 APK 来源与版本；冻结的 v0.3.4 APK 不得修改或重建。

---

## 11. 证据收集

每个用例记录：

- Windows：版本、`AudioShareServer.exe` SHA-256、`server.log` 片段；
- 手机：型号、Android 版本 / API、App 版本 0.2.0-beta；
- 时间戳、状态截图（配对对话框、待批准、设备列表、手机 READY/PLAYING/CLOSED）；
- 结果：`PASS` / `FAIL` / `NOT RUN`，失败附错误码；
- 断网恢复时间、撤销到 CLOSED 的时间；
- 不做像素级断言；音质/时延/underrun 只记录主观观察，不作为通过门槛。

建议保存到：

```
C:\AudioShareBeta\evidence\
├── screenshots\
├── server.log
├── hashes.txt
└── notes.md
```

然后把结论写回工单 25（设备型号、OS/API、版本、限制、go/no-go）。

---

## 12. 常见问题

| 现象 | 处理 |
|---|---|
| GUI 打不开，提示 WebView2 | 安装 WebView2 Evergreen Runtime |
| 手机看不到 Server | 检查同一 LAN/VPN、Windows 网络为“专用”、UDP 65530/65531 入站规则、AP 隔离 |
| QR 扫不出来 | 用 SAS 或粘贴 `audioshare-pair://` URI；检查相机权限 |
| 配对码 206 | nonce 过期或时钟偏差；重新开始配对并校准时间 |
| 配对码 201 | 邀请已取消/已使用/被拒绝；重新开始配对 |
| 配对码 202 | 未知客户端；重新配对 |
| 配对码 204 | 设备已撤销；重新配对 |
| 配对码 205 | Server 身份已重置/不匹配；重新配对 |
| `stop the Server first` | GUI Server 正在运行，先停止再运行 `as-cmd clients/identity` |
| 断网后不恢复 | 等待有界退避；确认网络恢复；记录 601/302 与恢复时间 |
| Windows 10 | 未验证，记录 `NOT RUN` 或明确说明 |

---

## 13. 不要做的事

- 不要使用 `-PtestSupport=true` / `-PdeviceTests=true` 构建的 APK 做发布验收；
  本次提供的是正式签名 universal APK。
- 不要提交、复制或截图 keystore 密码、`keystore.properties` 内容或私钥。
- 不要把模拟器结果写成真机 PASS；不要把 Linux 产物写成 Windows PASS。
- 不要为了“通过”而放宽断言、跳过 lock/断网/撤销步骤或伪造日志。
- 不要修改冻结的 v1 fixtures、v0.3.4 APK 或 accepted 协议/ADR。
- 不要发布、打 tag 或上传 Release（本轮授权仅本地产物）。
