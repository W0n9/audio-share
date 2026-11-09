# Server Rewrite — C++/MFC → Rust/Tauri v2

Status: ready-for-agent
Feature: server-rewrite

## Problem Statement

The Server is split between a C++20 Server Core and a Windows-only MFC Server GUI, built with vcpkg, MSVC, and MFC. The GUI cannot evolve beyond maintenance-mode MFC, Linux users only get `as-cmd`, and the build chain is heavy. The Server Core has no automated tests, and the Server GUI compiles core sources directly instead of using a clean library boundary.

The maintainer wants a Server that is cheap to build and maintain, with a real Server GUI on both Windows and Linux. Existing users must not notice a protocol change: the released v0.3.4 Android Client must keep working without an APK update. The fork's `main` branch carries experimental UDP header/reorder work that never became a released contract, so the rewrite must target the last released baseline instead of that experiment.

## Solution

Rewrite the Server side — Server Core, Server GUI, and `as-cmd` — in Rust. A shared in-process core crate owns capture, the Wire Protocol, and the network layers; `as-cmd` and the Tauri v2 Server GUI both link it directly.

The Server GUI uses the tray-first single-window layout chosen in the throwaway prototype: server status and Start/Stop are primary, settings live in a secondary view, and the tray menu provides Show / Start / Stop / Exit. It targets Windows 10+ and Ubuntu 24.04+, keeps en-US and zh-CN, and preserves the existing `config.ini`, CLI flags, artifact names, and update-check behavior.

The Wire Protocol stays at upstream v0.3.4. The released v0.3.4 Android Client is the acceptance client. The replacement ships as a big-bang v0.4.0: Windows gets a portable `AudioShareServer.exe` with WebView2 detection; Linux gets deb and AppImage packages. No protocol changes, no Android Client changes, and no public intermediate releases.

## User Stories

### Windows Server operator

1. 作为 Windows 用户，我希望 Server GUI 以托盘优先的单窗口打开，以便一眼看到服务器状态并直接 Start/Stop。
2. 作为 Windows 用户，我希望点击托盘图标显示或隐藏窗口，以便应用常驻但不占桌面。
3. 作为 Windows 用户，我希望 Start/Stop 是主界面上的主操作，以便不需要进入设置。
4. 作为 Windows 用户，我希望看到当前状态（Stopped / Running）和监听地址，以便确认手机该连哪里。
5. 作为 Windows 用户，我希望选择 Host 和 Port，以便绑定到正确的网卡和端口。
6. 作为 Windows 用户，我希望列出并选择 Audio Endpoint，以便共享正确的播放设备。
7. 作为 Windows 用户，我希望选择 Encoding，以便在带宽和音质之间取舍。
8. 作为 Windows 用户，我希望默认值在大多数机器上直接可用，以便开箱即用。
9. 作为 Windows 用户，我希望设置位于二级视图，以便主窗口保持聚焦。
10. 作为 Windows 用户，我希望关闭窗口时最小化到托盘，以便服务器继续运行。
11. 作为 Windows 用户，我希望可以选择关闭时退出，以便按我的习惯工作。
12. 作为 Windows 用户，我希望开机自动启动，以便不用每次手动打开。
13. 作为 Windows 用户，我希望选择启动行为（不操作 / 启动服务器 / 保持上次状态），以便符合我的使用习惯。
14. 作为 Windows 用户，我希望应用是单实例，以便重复点击不会启动多个服务器。
15. 作为 Windows 用户，我希望托盘菜单提供 Show / Start / Stop / Exit，以便不打开窗口也能控制。
16. 作为 Windows 用户，我希望一键修复防火墙入站规则，以便手机能连上。
17. 作为 Windows 用户，我希望打开系统声音面板，以便调整默认格式或设备。
18. 作为 Windows 用户，我希望每 3 小时检查更新并在有新版本时提示，以便知道升级。
19. 作为 Windows 用户，我希望关闭自动更新检查，以便不被提示打扰。
20. 作为 Windows 用户，我希望 About 显示版本、主页和许可证，以便确认信息。
21. 作为 Windows 用户，我希望界面支持 en-US 和 zh-CN，以便使用我的语言。
22. 作为 Windows 用户，我希望现有 `config.ini` 被读取并迁移，以便不丢失设置。
23. 作为 Windows 用户，我希望在 Windows 10+ 上运行，以便覆盖现有用户。
24. 作为 Windows 用户，我希望缺少 WebView2 时看到明确提示和下载链接，以便修复。
25. 作为 Windows 用户，我希望下载便携 exe 直接运行，以便保持现有分发体验。
26. 作为 Windows 用户，我希望产物名保持 `AudioShareServer.exe`，以便升级路径不变。

### Linux Server operator

27. 作为 Linux 用户，我希望获得 Server GUI，而不只是 `as-cmd`，以便桌面使用。
28. 作为 Linux 用户，我希望列出并选择 PipeWire Audio/Sink，以便共享正确的输出。
29. 作为 Linux 用户，我希望默认 Endpoint 按 `priority.session` 选择，以便与系统默认一致。
30. 作为 Linux 用户，我希望默认 Capture Format 为 F32LE，并保留 s8 / s16 / s24 / s32 的映射，以便与现有 Android Client 兼容。
31. 作为 Linux 用户，我希望通过 XDG autostart 配置开机自启动，以便桌面登录后可用。
32. 作为 Linux 用户，我希望关闭到托盘，以便服务器继续运行。
33. 作为 Linux 用户，我希望更新检查、语言和 About 与 Windows 一致，以便体验一致。
34. 作为 Linux 用户，我不需要 Windows 的防火墙修复和声音面板；我希望看到说明链接，以便知道去哪里配置。
35. 作为 Linux 用户，我希望在 Ubuntu 24.04+ 上通过 deb 或 AppImage 安装，以便选择分发方式。
36. 作为 Linux 用户，我希望托盘右键菜单可用，以便控制服务器；我理解 Tauri 在 Linux 上不发送托盘左键 click 事件。

### Android Client user

37. 作为 v0.3.4 Android Client 用户，我希望不改 APK 就能连接新 Server，以便现有安装继续工作。
38. 作为 Android Client 用户，我希望 `GET_FORMAT` 返回的 `AudioFormat` 与 v0.3.4 完全兼容，以便解码正确。
39. 作为 Android Client 用户，我希望 `START_PLAY` 返回的 id 和 UDP 绑定流程保持一致，以便音频到达。
40. 作为 Android Client 用户，我希望 3 秒 heartbeat 和 5 秒超时行为保持一致，以便断线检测不变。
41. 作为 Android Client 用户，我希望 5 种 encoding、单/双声道、44.1/48 kHz 都能播放，以便覆盖现有设备。
42. 作为 Android Client 用户，我希望多个客户端和重连行为保持一致，以便共享给多台手机。
43. 作为 Android Client 用户，我希望 MTU 分片和 UDP 包边界保持一致，以便不出现解析错位。
44. 作为 Android Client 用户，我希望 Server 停止后连接按现有方式关闭，以便客户端能重试。

### as-cmd user

45. 作为 `as-cmd` 用户，我希望 `-b/--bind` 的行为和默认地址选择保持一致，以便脚本不变。
46. 作为 `as-cmd` 用户，我希望 `-l/--list-endpoint`、`--list-encoding`、`-e/--endpoint`、`--encoding`、`--channels`、`--sample-rate`、`-V/--verbose`、`-v/--version`、`-h/--help` 全部保留，以便现有用法不变。
47. 作为 `as-cmd` 用户，我希望退出码和错误信息保持可理解，以便自动化脚本判断。
48. 作为 `as-cmd` 用户，我希望 CLI 与 Server GUI 共用同一个 core，以便行为一致。
49. 作为 `as-cmd` 用户，我希望 Linux 上无 GUI 也能运行，以便 headless 或服务器环境。
50. 作为 `as-cmd` 用户，我希望 `server.log` 继续写入，以便排障。

### Maintainer

51. 作为维护者，我希望一个 Rust workspace 包含 core、CLI、GUI 三部分，以便共享实现。
52. 作为维护者，我希望不再需要 vcpkg、MSVC 或 MFC，以便一条 cargo 命令构建。
53. 作为维护者，我希望 core 的公开 API 很小且返回 `Result`，以便 GUI 和 CLI 复用并易于测试。
54. 作为维护者，我希望 core 不再在错误路径直接 `exit()`，以便 Server GUI 进程不被杀死。
55. 作为维护者，我希望 `stop()` 可取消且不阻塞 UI 线程，以便界面不冻结。
56. 作为维护者，我希望 Wire Protocol 实现位于独立模块或接口后面，以便未来协议演进。
57. 作为维护者，我希望 framing、heartbeat、MTU 分片和配置迁移有单元测试，以便快速定位回归。
58. 作为维护者，我希望有一个 Wire Protocol parity harness，以便对比旧 `as-cmd` 和新 Server。
59. 作为维护者，我希望 CI 在 `windows-latest` 和 `ubuntu-24.04` 上构建、测试和 lint，以便每次提交可验证。
60. 作为维护者，我希望旧 Server Core 和 MFC Server GUI 保留到 cutover，之后删除，以便可回退。
61. 作为维护者，我希望重写不依赖 `main` 分支的 UDP 包头/重排实验，以便基线可控。
62. 作为维护者，我希望 Android Client 与 Wire Protocol 在本轮不变，以便控制范围。
63. 作为维护者，我希望文档记录协议、构建、发布和已知限制，以便未来维护。
64. 作为维护者，我希望 Server GUI 的视觉与信息架构按原型 A 落地，以便不再回到 MFC 的两 tab 结构。

### Release manager

65. 作为发布者，我希望版本号为 `0.4.0`，以便区分重写版本。
66. 作为发布者，我希望产物名沿用 `AudioShareServer.exe` 和 `audio-share-server-cmd-*`，以便下载路径不变。
67. 作为发布者，我希望 Windows 发布便携 exe，Linux 发布 deb 和 AppImage，以便覆盖目标平台。
68. 作为发布者，我希望本轮不做代码签名和 Tauri 自动更新，以便控制范围。
69. 作为发布者，我希望 big-bang 发布，不发布公开中间版本，以便用户只看到稳定替换。

### Translator

70. 作为译者，我希望 en-US / zh-CN 文案集中在可维护的资源文件，以便新增或修改语言。
71. 作为译者，我希望语言选择持久化并在启动时应用，以便与现有行为一致。

### Tester

72. 作为测试者，我希望用 scripted client 自动回归 Wire Protocol，以便每次改动可验证。
73. 作为测试者，我希望用真实 v0.3.4 APK 做最终验收，以便确认无感。
74. 作为测试者，我希望与旧 `as-cmd` 做同机同音频源 A/B，以便比较格式、字节、延迟和 glitch。
75. 作为测试者，我希望做 8 小时 soak、设备热插拔、睡眠恢复和反复 start/stop，以便覆盖长期运行。
76. 作为测试者，我希望 Server GUI 有手工验收清单，以便覆盖托盘、设置、i18n、单实例、更新检查和配置迁移。

## Implementation Decisions

### 目标、基线与范围

- 主目标是可维护的跨平台 Server GUI（Windows + Linux），顺带摆脱 MFC、vcpkg 和 MSVC；Server Core 现代化是次目标。
- 仓库是独立产品，upstream 只作参考。重写基线是 upstream `v0.3.4` 的行为与 Wire Protocol；`main` 分支的 UDP 包头/重排/隐藏实验不在本轮范围内。记录见 ADR-0001（Server rewrite baseline）。
- 只动 Server 侧：Server Core、Server GUI、`as-cmd`。Android Client 保持 Kotlin/Compose 与 v0.3.4，Wire Protocol 不变。
- 目标平台：Windows 10+（x86_64）与 Ubuntu 24.04+（x86_64）。macOS 不做。
- 单人维护、无硬截止；选择 Rust，选型理由见 ADR-0002（Server rewrite stack）。

### 技术栈与模块

- Rust stable；Tauri v2；Svelte + Vite + TypeScript；同一个进程内的 core crate，Server GUI 通过 Tauri command/event 直接调用，不引入单独的服务进程或 IPC 协议。
- Rust 依赖：`tokio`（TCP/UDP/定时器）、`prost`（protobuf）、`clap`（CLI）、`tracing`（日志）、`rust-ini`（配置）。音频依赖：Windows 用 `wasapi` crate；Linux 用 `pipewire` crate；spike 不达标时分别回退到 `windows-rs` 直接调用 WASAPI 或 `pipewire-sys` 直接调用 PipeWire C API。
- 新增一个 Rust workspace，与旧 Server Core / MFC Server GUI 并存直到 cutover；cutover 后删除旧实现。CI 与本地构建都使用 cargo，不再引入 vcpkg 或 MFC。
- core 的公开面保持小：列出 Endpoint、取默认 Endpoint、读取或更新 Capture Format、启动/停止 Server、取 `AudioFormat` 的二进制表示。所有错误通过 `Result` 返回，不在库代码里 `exit()`；启动/停止不阻塞调用线程，停止时可取消。
- 采集与网络的共享状态（尤其是当前 `AudioFormat`）必须有明确同步，避免采集线程与网络线程的数据竞争。

### Wire Protocol（冻结）

- 与 upstream `v0.3.4` 完全一致：TCP 控制通道承载 `uint32 cmd`、可选 `uint32 size` 和 protobuf；UDP 由 Android Client 先发送 `int id` 绑定，Server 再发送裸 PCM。
- 多字节整数显式使用 little-endian，不依赖宿主字节序。
- `client.proto` 的字段号、字段类型和 encoding 枚举值不变；`AudioFormat` 只有 encoding、channels、sample_rate。
- 命令保留 `GET_FORMAT`、`START_PLAY`、`HEARTBEAT`；Server 每 3 秒发 heartbeat，5 秒无活动关闭会话。
- UDP 分片：MTU 1492，减去 20 字节 IP 头和 8 字节 UDP 头，再按 `block_align` 对齐；不引入序号、包头或重排缓冲。
- 本轮不允许任何 Wire Protocol 变更；协议实现留在独立模块/接口后，供未来版本演进。

### 音频采集

- Windows：WASAPI shared-mode loopback；枚举 render Endpoint，使用 `IMMDevice` ID 与友好名；默认 Capture Format 来自 Endpoint 的 mix format；用户指定 encoding/channels/sample rate 时先做格式支持探测，不支持则回退到 closest match；1 秒缓冲，按 device period 轮询；支持 f32、s8、s16、s24、s32。
- Linux：PipeWire capture；枚举 `Audio/Sink` 节点，用 `object.serial` 作为 Endpoint ID，用 `priority.session` 选择默认 Endpoint；通过 target object 选择节点；默认 encoding 为 F32LE，s8 映射到 U8，其余保持 s16/s24/s32；保留 monitor 采集能力；非 F32 格式与 monitor 路径必须在音频 spike 中验证。
- Capture Format 与 Transfer Format 本轮保持一致；不做重采样，除非底层库的格式协商需要。
- Server GUI 只暴露 Endpoint 与 Encoding，与 MFC Server GUI 对齐；channels 和 sample rate 仍只由 `as-cmd` 暴露。

### Server GUI

- 采用原型 A：托盘优先的单窗口。主视图显示 Server 状态、监听地址、Start/Stop、Host、Port、Endpoint 和 Encoding；设置位于二级视图；关闭窗口默认最小化到托盘。
- 托盘菜单提供 Show、Start、Stop、Exit；Windows 用系统托盘 API，Linux 用 StatusNotifierItem（右键菜单可用，左键 click 不作为依赖）。
- 功能面对齐 MFC Server GUI：Host、Port、Endpoint、Encoding、Start/Stop、Reset、Sound Panel、开机启动、启动行为、关闭行为、单实例、更新检查、语言、Repair Firewall、Hide、Exit、About；Linux 提供等价能力，Firewall Repair 与 Sound Panel 为 Windows-only，Linux 给出说明链接。
- 单实例使用 Tauri 生态能力或平台机制；开机启动 Windows 写用户级启动项，Linux 写 XDG autostart。
- 更新检查沿用现有行为：每 3 小时请求 GitHub releases/latest，有新版本时提示并打开 release 页面；不下载、不静默安装，不引入 Tauri updater。
- i18n 保留 en-US 与 zh-CN；文案放在可维护的资源文件中；语言切换持久化，启动时应用（与 MFC 行为一致）。
- 视觉与信息架构按原型 A 重新设计，不复刻 MFC 的两个 tab；日志查看器本轮不做。
- Server GUI 的启动/停止必须异步执行，避免阻塞 UI；错误以可读消息呈现，不弹系统级崩溃。

### 配置与兼容

- 继续使用 Server 可执行文件同目录的 `config.ini`；读取现有键并迁移到新版本，保留 host、port、endpoint、encoding、autostart、onLaunch、onClose、autoUpdate、language 和 configVersion 的语义。
- 配置写入要原子化，避免崩溃时留下半写文件；configVersion 不匹配时给出明确迁移结果，而不是静默丢弃用户设置。
- `as-cmd` 保留全部现有 flags 与语义；人类可读输出可以改进，但不承诺逐字节稳定；退出码保持可判断。
- 版本号 `0.4.0`；产物名保持 `AudioShareServer.exe`、`audio-share-server-cmd-windows.zip`、`audio-share-server-cmd-linux.tar.gz`。

### 打包、发布与 CI

- Windows：便携 `AudioShareServer.exe`；启动时检测 WebView2，缺失则显示明确提示和下载链接；本轮不做 MSI 或 NSIS 安装包，不做代码签名。
- Linux：deb 与 AppImage，基线 Ubuntu 24.04+；依赖系统 WebKitGTK 4.1 与 PipeWire；CI 使用 `ubuntu-24.04`。
- Windows CI 使用 `windows-latest`；构建、单元测试、clippy、rustfmt 和 Tauri 打包都在 CI 中运行。
- Big-bang 发布：不发布公开中间版本；内部音频 spike、Wire Protocol parity harness 和 GUI 验收清单是 cutover 前的门槛。

### 已知风险

- PipeWire crate 的 monitor 采集和非 F32 格式未在官方示例中完整覆盖；WASAPI loopback 的延迟、CPU 与 glitch 没有公开对比数据。两者都必须通过 spike 和 8 小时 soak 实测。
- Tauri 在 Windows 依赖 WebView2；Linux 依赖 WebKitGTK 4.1。Windows 10 1803+ 通常已有 WebView2，Linux 的 WebKitGTK 是明确的运行时依赖。
- Tauri 在 Linux 上不发送托盘左键 click 事件；托盘控制以右键菜单为准。
- 本轮不解决 `main` 分支上的音频 glitch 工作；它作为独立后续工作。

## Testing Decisions

- 好测试只断言外部可观察行为：Wire Protocol 上的控制消息、heartbeat 时序、UDP 包结构与 PCM 字节、CLI 的进程行为、GUI 的用户可见行为。不测私有函数，不把实现细节写进断言。
- **唯一自动化集成 seam 是 Wire Protocol。** 用一个 scripted client 驱动 TCP 控制与 UDP PCM；同一套用例也跑旧 `as-cmd` 作为基线，再跑新 Server 对比。最终用真实 v0.3.4 Android Client 做人工验收。
- 用例覆盖：`GET_FORMAT` 的 `AudioFormat`、`START_PLAY` 与 UDP id 绑定、heartbeat 与 5 秒超时、MTU 分片与 `block_align` 对齐、5 种 encoding、单/双声道、44.1/48 kHz、多客户端、重连、反复 start/stop、Server 停止后的连接关闭。
- CLI 只做薄断言：`--help`、`--version`、`-l`、`--list-encoding` 的输出与退出码；启动/停止行为通过 Wire Protocol seam 覆盖。
- Server GUI 使用手工验收清单：托盘显示与菜单、主视图 Start/Stop、Endpoint/Encoding、设置持久化、`config.ini` 迁移、en-US/zh-CN、单实例、更新检查、About、Windows 的 Firewall Repair 与 Sound Panel、Linux 的等价项。tauri-driver 或 Playwright 自动化不进入 v1。
- 单元测试只覆盖纯逻辑，作为定位辅助而不是主要保证：TCP framing、UDP 分片与 `block_align`、heartbeat 状态、配置迁移与版本比较。
- 音频 A/B：同一台机器、同一音频源、同一 Capture Format 下，对比旧 `as-cmd` 与新 Server 的输出；记录 callback 间隔、丢帧、漂移和 CPU。再跑 8 小时 soak、设备热插拔、睡眠恢复和反复 start/stop。
- 现有先例：`refactor` 基线没有 Server Core 自动化测试；MFC 工程里的 unit-test 只覆盖 `util`；`main` 分支上 Android 侧的音频管线测试不在本轮范围。parity harness 和新的单元测试是新增资产。

## Out of Scope

- Android Client 的任何改动；以 v0.3.4 APK 为验收对象。
- Wire Protocol 的任何变更，包括 `main` 分支上的 UDP 包头、序号、重排、缓冲和隐藏。
- `main` 分支上的音频 glitch 修复工作；重写稳定后另起一轮。
- macOS 支持。
- MSI / NSIS 安装包与代码签名。
- Tauri updater 或应用内自动下载安装。
- 日志查看器；本轮只写 `server.log`。
- Server GUI 中的 channels / sample rate 控件；保持 CLI-only。
- 运行中切换语言；语言在启动时应用。
- 降低 1 秒缓冲或做超出 parity 的延迟优化。
- Go、Slint、Wails、Fyne 等未选方案。
- 公开的增量版本；本轮只有 big-bang 的 `0.4.0`。
- 与 upstream 的持续同步或合并；本仓库作为独立产品维护。
- tauri-driver / Playwright 的 GUI 自动化。

## Further Notes

- 决策记录：ADR-0001（重写基线为 upstream v0.3.4，`main` 的协议实验不在范围内）；ADR-0002（Rust + Tauri v2 + 同进程 core）。
- 调研依据：Rust 与 Go 的服务端迁移对比报告，覆盖 WASAPI loopback、PipeWire、Tauri/Wails/Fyne、打包与维护成本。
- GUI 原型：throwaway 分支上的三个布局变体，结论为 A（tray-first single window）；原型只作为 primary source，不进入实现分支。
- 里程碑顺序（内部，非公开发布）：音频 spike 与 Wire Protocol harness → core 与 CLI 实现 → Server GUI → 打包与 CI → 真实 v0.3.4 APK 验收 → big-bang cutover 到 `0.4.0`。
- 发布前硬门槛：真实 v0.3.4 Android Client 无感知差异；旧 `as-cmd` A/B 通过；8 小时 soak 无 crash 或明显倒退；GUI 验收清单完成。
