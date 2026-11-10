# Audio Share 服务端迁移评估：C++ → Rust / Go

评估日期：2026-11-09  
代码基线：`refactor` 分支，提交 `342751f`（版本 0.3.4）[1][3]  
范围：`server-core`（C++20 命令行服务端）与 `server-mfc`（Windows MFC GUI）；Android 客户端不在范围内，但其 wire protocol 是硬兼容约束[2][11]。  
方法：先读本地代码建立现状基线，再以官方文档、官方源码、release notes、issue tracker 为一手来源；无法确认的条目标注“未确认”。本次 WebSearch/WebFetch 可用；所有外部引用均在 2026-11-09 重新检索。本报告未安装软件、未编译、未运行任何候选栈；工程可行性需按 5.2 的原型验证。

## 摘要

- 现状是“一个 core、两个壳”：C++20 core 用 asio 做 TCP 控制面、UDP 广播裸 PCM，protobuf 描述音频格式；Windows 用 WASAPI loopback，Linux 用 PipeWire capture[7][8][10][11]。MFC GUI 不是链接库，而是直接把 core 的 `.cpp` 编进可执行文件[18]。
- GUI 的真实功能面比任务描述略窄：有托盘、两个 tab、设置、i18n CSV、更新检查、关于、端点/编码选择、启动/停止、防火墙修复；**没有内嵌日志查看器**，也**没有通道数/采样率选择**（这两项只在 CLI）[4][13][15][16]。
- 音频采集是迁移的主风险，GUI 主要是成熟度与打包问题。Windows WASAPI loopback 本身有明确的共享模式限制；Rust 的 `wasapi` crate 对 loopback 的支持最直接，Go 的 malgo/miniaudio 也提供 `Loopback` 设备类型，但仅限 WASAPI[24][32][36][39]。
- Linux 上 Rust 有原生 `pipewire` crate（0.10.1）和官方 `audio-capture` 示例；Go 没有找到成熟的 native PipeWire binding，实际路线会退到 PulseAudio 兼容层（`pipewire-pulse`）或 ALSA，语义与当前“枚举 Audio/Sink、按 `priority.session` 选默认”的实现不同[8][46][47][48][49][50]。
- GUI 方面，Rust 的 Tauri v2（2.12.5）功能最全、打包最成熟；Slint（1.18.1）是最像“原生应用”的 Rust 选项；Go 的 Wails v3（3.0.0/3.1.0）功能接近 Tauri，但要求 cgo/MinGW，Linux 依赖 GTK4 + WebKitGTK 6.0[52][53][56][62][63][67][68]。
- 部分迁移是认真选项：当前 core 的公开 API 很小（列端点、取默认端点、start/stop、取格式二进制、capture_config），适合抽 C ABI；但 core 内部在错误路径直接 `exit()`，且 `stop()` 会 join 线程，直接塞进 GUI 进程会杀进程/卡 UI，必须先改[5][6][7][8][10]。
- 最低风险的第一步不是重写 core，而是把现有 `as-cmd` 当 sidecar：Tauri 官方支持 sidecar，Go 可用 `os/exec`；GUI 只负责启动/停止、列端点、看日志，core 一行不改[4][55][109]。
- 若追求“一套代码 + 原生 PipeWire + 保留 Windows endpoint ID”，Rust 全量重写是更干净的长期路线；若团队以 Go 为主、接受 PulseAudio 兼容层与 Wails 的 cgo 约束，Go 全量重写也可行。
- 最大未知是真实环境下的 latency/glitch：没有找到任何跨 cpal/wasapi/malgo/miniaudio 的公开对比数据；当前实现又是 1 秒缓冲 + 轮询，必须用 8 小时 soak test 实测后才能定案[7][31][38]。

### 关键版本（检索于 2026-11-09）

| 组件 | 版本 | 日期 | 备注 |
|---|---|---|---|
| Rust | 1.99.1 | 2026-10-15 | stable[94] |
| Go | 1.27.3 | 2026-10-19 | stable[95] |
| Tauri | 2.12.5 | 2026-10-21 | v2 stable；v3 alpha 为 2026-11-06[60][61] |
| Wails | 3.1.0 | 2026-11-01 | v3.0.0 于 2026-10-26 稳定；v2.17.1 仍在维护[62][63][116] |
| Slint | 1.18.1 | 2026-09-21 | SystemTrayIcon 自 1.17[70][69] |
| egui | 0.37.0 | 2026-10-15 | eframe[110] |
| iced | 0.14.0 | 2025-12-07 | 自述 experimental[75] |
| Dioxus | 0.7.10 | 2026-07-30 | 0.8 alpha 为 2026-10-16[78][79] |
| Fyne | 2.8.2 | 2026-10-12 | tray 2.2+，i18n 2.5+[111][80][81] |
| Gio | 0.10.3 | 2026-09-28 | tag commit date[84] |
| cpal | 0.18.2 | 2026-08-16 | WASAPI backend[29] |
| wasapi crate | 0.27.0 | 2026-11-06 | loopback 一等能力[32] |
| pipewire crate | 0.10.1 | 2026-08-19 | safe binding[47] |
| malgo | 0.11.27 | 2026-10-29 | miniaudio 绑定[39] |
| jfreymuth/pulse | 0.1.3 | 2026-08-17 | 纯 Go PulseAudio[50] |
| prost | 0.14.4 | 2026-06-07 | passively-maintained[96] |
| tokio | 1.54.1 | 2026-10-21 | async runtime[97] |
| clap | 4.6.7 | 2026-09-14 | CLI[98] |
| google.golang.org/protobuf | 1.36.12 | 2026-08-10 | Go protobuf[99] |
| cobra | 1.10.2 | 2025-12-04 | CLI[100] |

## 1. 现状盘点

### 1.1 `server-core`：C++20 命令行服务端

| 维度 | 现状 | 证据 |
|---|---|---|
| 语言/构建 | C++20；CMake ≥3.20；vcpkg 提供 asio/protobuf/spdlog/cxxopts；Linux 额外 `libpipewire-0.3` | [3]:1-60 |
| 可执行文件 | 只产出 `as-cmd`，没有 library target，也没有 C ABI | [3]:42-45, 62 |
| 协议 | TCP：`uint32 cmd` + 可选 `uint32 size` + protobuf；UDP：客户端先发 `int id` 绑定，然后收裸 PCM | [2], [10]:206-264, 328-341 |
| 心跳 | 服务端每 3 秒发 `CMD_HEARTBEAT`，5 秒无心跳断连 | [10]:266-304；[9]:90 |
| UDP 分片 | MTU 固定 1492，减去 20 IP + 8 UDP 头，再按 `block_align` 对齐 | [10]:396-425 |
| 音频格式 | protobuf `AudioFormat{encoding, channels, sample_rate}`，encoding 枚举 5 种 PCM | [11]:21-35 |
| Windows 采集 | WASAPI shared-mode loopback，`AUDCLNT_STREAMFLAGS_LOOPBACK`；1 秒缓冲；按设备周期轮询 | [7]:222-223, 239-289 |
| Linux 采集 | PipeWire `pw_stream` capture；枚举 `Audio/Sink`，用 `object.serial` 作 ID，按 `priority.session` 选默认 | [8]:350-443 |
| 编码/声道/采样率 | CLI 支持 `--encoding`、`--channels`、`--sample-rate`；Windows 上会经 `IsFormatSupported` 回退到 closest match | [4]:24-35；[7]:152-211 |
| 端点列表 | Windows 枚举 `eRender` active endpoints，取 `IMMDevice::GetId` + `PKEY_Device_FriendlyName`；Linux 枚举 `Audio/Sink` | [7]:292-330；[8]:350-401 |
| 线程模型 | 采集线程 + asio `io_context` 网络线程；`stop()` 直接 join | [6]:10-22；[10]:176-194 |

协议层有两个容易在重写时踩坑的细节：

1. **字节序没有显式声明**。`cmd_t` 是 `enum class : uint32_t`，`size` 是 `uint32_t`，`id` 是 `int`，代码直接把 `&cmd`、`&size`、`&id` 塞进 asio buffer[9]:47-52；[10]:210, 221-226, 241-245。当前发布目标是 x86_64，因而协议事实上是小端。Rust/Go 重写必须显式 `to_le_bytes`/`binary.LittleEndian`，否则 Android 端会解析错位。
2. **protobuf 只保证 wire 兼容，不保证字节稳定**；字段号与 wire type 是兼容基础，序列化字节不要求稳定[104]。因此重写只能保证 `AudioFormat` 的字段号和枚举值不变，不能依赖“同样的 proto 序列化出同样的字节”。

### 1.2 MFC GUI：真实功能清单

MFC GUI 是 Windows-only（工程配置只有 Win32/x64）[18]:3-19；README 也只描述 Windows GUI，Linux 只提供 CLI[1]:58-69。这意味着新 GUI 若选跨平台栈，可以顺带补齐 Linux GUI；当前 Linux 用户只有 `as-cmd`。

| 功能 | 现状 | 证据 |
|---|---|---|
| 托盘图标 | `Shell_NotifyIconW`，tooltip “Audio Share Server”，左键显示窗口，右键弹菜单（Show / Start Server / Exit） | [12]:78-118, 247-269；[16]:312-322 |
| 气泡通知 | `NIM_MODIFY` + `NIF_INFO`；点击气泡打开更新链接 | [12]:102-118, 264-266 |
| 单实例 | 命名 mailslot；第二个实例写入命令后 `ExitProcess(0)`，第一个实例收到后 `ID_APP_SHOW` | [15]:188-218 |
| Tab | 两个：Server、App Settings；About 是模态对话框 | [12]:184-207；[16]:107-164 |
| Server 页 | Host 下拉、Port 编辑框、Audio Endpoint 下拉（含 Default）、Encoding 下拉、Start/Stop、Reset、Sound Panel | [13]:35-45, 81-163 |
| 启动/停止 | 同一个按钮切换；启动时禁用输入控件；配置写回 `config.ini` | [13]:165-236 |
| 声音面板 | `control.exe /name Microsoft.Sound`，等待进程退出 | [13]:249-263 |
| 设置页 | 开机启动、启动时行为（Do nothing / Start server / Keep last state）、关闭时行为（Exit / Minimize to tray）、自动检查更新、语言、Repair Firewall、Hide、Exit、About | [14]:46-68, 74-119；[16]:137-164 |
| 更新检查 | 每 3 小时（可关）请求 GitHub `releases/latest`，用 nlohmann-json 解析；版本比较后气泡提示；只打开网页，不下载安装 | [14]:24, 26, 262-325；[12]:264-266；[22]:7-38 |
| 关于 | 从版本资源读 ProductName/ProductVersion/LegalCopyright；Home Page 链接 | [23]:82-98；[16]:93-105 |
| i18n | `strings.csv` 两列 en-US/zh-CN；构建时 `generate_rc.py` 生成 `AudioShareServer_zh-CN.rc` | [17]:1-51；[18]:140-147 |
| 语言切换 | 写入 `config.ini`；启动时 `SetProcessPreferredUILanguages` 应用；代码中未见运行时刷新 | [14]:338-342；[15]:118-124 |
| 日志 | 只写 `server.log`（spdlog basic file sink）；**没有日志查看器/日志 tab** | [15]:54-55；[16]:107-164 |
| 通道/采样率 | GUI **没有**这两个控件；只有 CLI 有 | [13]:35-45, 133-162；[4]:31-32 |
| 防火墙 | 没有启动时主动弹窗；Windows 首次监听时由系统提示；GUI 提供“Repair Firewall”按钮，提升权限后重建 TCP/UDP 入站规则 | [1]:68；[14]:128-212 |
| 配置文件 | `config.ini` 放在 exe 同目录；configVersion 不匹配时会删除旧文件并重建 | [15]:61-62, 109-116 |
| 开机启动 | 写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，值为 `exe /hide` | [14]:214-226 |
| 关闭行为 | `WhenClose=0` 销毁窗口退出；否则 `HideApplication()` | [12]:224-233 |

**对任务描述的两点纠正**：当前 MFC GUI 没有“log view”；`server.log` 只能外部打开。也没有“channels/sample rate”UI；这两项是 CLI 能力，在音频格式选项里 GUI 只暴露 encoding[4][13]。

### 1.3 构建、打包与测试现状

- Windows：MSVC v143，MFC 动态链接（`UseOfMfc=Dynamic`），vcpkg triplet `x64-windows-static-md`[18]:34, 101-112；Windows core 用 WIL 管理 COM（`wil::com_ptr`）[7]:24, 119-137；README 仍要求安装 VC++ 2015-2022 Redistributable[1]:52。
- Linux：CMake 默认 `AUDIO_SHARE_STATIC_LIBCPP=ON`，用 `-static-libstdc++`，但 `libpipewire-0.3` 仍是动态依赖[3]:12, 47-49, 56-60；`-static-libstdc++` 的语义见 GCC 文档[105]。
- 打包：`pack.sh` 只产出 `audio-share-server-cmd-linux.tar.gz` / `...-windows.zip`，没有 MSI/NSIS/deb/AppImage[21]:9-18。
- 发布：GitHub Actions 用 `windows-latest` + `ubuntu-22.04`；MFC 与 core 分开构建；release workflow 中没有代码签名步骤[20]:47-139。
- 测试：只有 `unit-test`，覆盖 `util::split_string` 与 `util::is_newer_version`；没有音频、网络、协议、GUI 的自动化测试[19]:9-90。
- MFC GUI 与 core 的耦合方式：`.vcxproj` 直接 `ClCompile` 了 `audio_manager.cpp`、`network_manager.cpp`、`win32/audio_manager_impl.cpp`、`client.pb.cc`[18]:285-302。这意味着“只换 GUI”不是链接一个现成库，而是先把 core 抽成库。

## 2. 对比矩阵

置信度说明：**高** = 官方文档/源码直接支持；**中** = 官方资料支持但需要工程验证；**低/未确认** = 没有找到一手证据或只有间接证据。矩阵中的“强/中强/弱”是**能力匹配度**，不是置信度；置信度看单独的列。

### 2.1 音频采集

| 维度 | Rust | Go | 置信度 | 关键证据 |
|---|---|---|---|---|
| Windows WASAPI loopback | 强：`wasapi` crate 明确支持 loopback，有 `loopback` example；cpal 也能用，但属于“在 render device 上建 input stream”的隐式行为 | 强：malgo/miniaudio 有 `DeviceTypeLoopback`；miniaudio 文档明确 loopback 仅 WASAPI | 高 | [32][33][27][39][36] |
| Windows 端点枚举/选择 | 强：`Device::get_id()` 返回 IMMDevice ID，`get_friendlyname()` 返回友好名，和当前实现一一对应 | 中强：`Context.Devices(Playback)` + `DeviceInfo.ID/Name/IsDefault`；miniaudio 的 WASAPI `ma_device_id.wasapi` 直接存 `IMMDevice::GetId` 的 wchar_t 字符串（上限 64 wchar），但 malgo 以定长 byte array 暴露，需要 UTF-16 转换[120] | 中 | [34][39][120] |
| Windows 编码 | 强：`WaveFormat` 用 `SampleType::{Float,Int}` + storebits/validbits 表达 8/16/24/32-bit；shared mode 下 `initialize_client` 接受几乎任意格式并由引擎转换 | 强：malgo 支持 U8/S16/S24/S32/F32；8-bit 是 unsigned，和当前 `ENCODING_PCM_8BIT` 语义一致 | 高 | [32][33][35][40] |
| Windows 声道/采样率 | 强：`WaveFormat::new(..., samplerate, channels, ...)` 显式指定；shared mode 可由引擎转换 | 强：`DeviceConfig.SampleRate` + `SubConfig.Channels`；miniaudio 内部做 format/channel/rate 转换 | 高/中 | [33][35][39][38] |
| 事件驱动 vs 轮询 | `wasapi` 支持 `StreamMode::Events` / `Polling`；cpal 默认 event callback | miniaudio 内部封装，上层通过 callback 收数据，不直接暴露 WASAPI 事件/轮询开关 | 中 | [33][27][38] |
| Linux 原生 PipeWire | 强：`pipewire` crate 0.10.1，有 `audio-capture` 示例；`target.object` 在 `v0_3_44` feature 下可用；`stream.capture.sink` 可通过 properties 设置（示例中该行被注释） | 弱：没有成熟的 native PipeWire binding；`purego-pipewire` 自述 early stage 且只覆盖 playback；malgo 的 Linux backend 是 PulseAudio/ALSA/JACK | 中 | [47][48][44][49][39] |
| Linux monitor 采集 | 中强：`stream.capture.sink=true` 或 target 到 sink 是 PipeWire 的官方机制，但 Rust 示例未直接演示 monitor，需要原型验证 | 中：`jfreymuth/pulse` 的 `RecordMonitor` 可录 sink monitor，但依赖 `pipewire-pulse` 兼容层 | 中 | [44][45][46][50][48] |
| 端点 ID 语义 | Windows 可保留 IMMDevice ID；Linux 可保留 PipeWire `object.serial` | Windows 可保留 IMMDevice ID（miniaudio 存为 `wchar_t[64]`）；Linux 是 Pulse sink name/index，旧配置需要重新映射 | 中 | [34][39][50][120] |
| 延迟/glitch | 未确认：可配置 buffer，但没有公开对比数据 | 未确认：miniaudio 会做重采样/混音，行为与当前 WASAPI 最近格式回退不同 | 低 | [31][33][38] |

### 2.2 GUI 候选

| 维度 | Tauri v2 (Rust) | Slint (Rust) | Wails v3 (Go) | Fyne (Go) |
|---|---|---|---|---|
| 最新版本（2026-11-09） | 2.12.5（2026-10-21）[60]；v3 仍是 alpha[61] | 1.18.1（2026-09-21）[70] | 3.0.0（2026-10-26）[62] / 3.1.0（2026-11-01）[63] | 2.8.2（2026-10-12）[111] |
| Windows + Linux | 是[56] | 是[71] | 是[62] | 是[80] |
| 渲染方式 | 系统 WebView：Windows WebView2，Linux WebKitGTK 4.1[56] | 原生渲染（femtovg/skia/software）[71] | 系统 WebView：Windows WebView2，Linux GTK4 + WebKitGTK 6.0[62][65] | 原生 OpenGL/Material 风格[114] |
| 托盘 | 内置，需 `tray-icon` feature；Linux 上 click 事件不发送，菜单仍可用[52] | `SystemTrayIcon`（1.17+）；Linux 走 StatusNotifierItem，GNOME 需扩展[68][69] | 内置 tray + menu[62] | 内置 `SetSystemTrayMenu`（2.2+）[80] |
| i18n | Web 前端自选 JS i18n（评估） | `@tr` + gettext[72] | Web 前端自选 JS i18n（评估） | 内置 `lang.L` + JSON（2.5+）[81] |
| Windows 安装器 | MSI（WiX v3）/ NSIS[53]；WebView2 可下载/嵌入/离线[53] | 未找到官方 installer bundler（需自备 WiX/NSIS/cargo-packager）（评估） | NSIS / MSIX[62] | `fyne package` 只产 exe[82] |
| Linux 包 | deb/rpm/AppImage/Snap/Flatpak/AUR[54] | 需自备（评估） | AppImage/deb/rpm/Arch[62] | `fyne package` 只产 tar.gz[82] |
| 官方体积数字 | 最小 app <600KB[57] | 未确认（只说“更小”） | 未确认 | 未确认 |
| 与 C++ core 集成 | Rust 可直接 link（cxx/cc）[101][103]；也可 sidecar[55] | 同 Tauri（Rust）[101][103] | cgo；Windows 需 MinGW-w64，和 MSVC 静态库有 ABI 风险[67] | cgo；同上[67] |
| 维护活跃度 | 很大生态；1511 open issues；v3 alpha 进行中[112][61] | 751 open issues；许可证需审查[71] | 393 open issues；v3 刚稳定[64][62] | 755 open issues；打包弱[114] |

GUI 表置信度：版本、平台、打包为**高**（官方 release/docs）；tray/i18n 行为为**中**（平台差异需实测）；体积与长期维护为**低/未确认**。

### 2.3 打包、工具链与维护

| 维度 | Rust | Go | 置信度 | 关键证据 |
|---|---|---|---|---|
| Windows CRT 依赖 | MSVC 默认动态 CRT，仍需 VC++ Redist；可 `+crt-static` 改静态 | 纯 Go 默认无 VC++ 依赖；cgo/MinGW 可能带 libgcc/libstdc++ | 高/中 | [86][87][88][67] |
| Windows 安装器 | Tauri 成熟（MSI/NSIS）；Slint/egui/iced 需自备 | Wails 有 NSIS/MSIX；Fyne 只有 exe | 高 | [53][62][82] |
| Linux 依赖 | pipewire + （Tauri）WebKitGTK 4.1；或原生 GUI 的 X11/Wayland/GL | PulseAudio/ALSA + （Wails）GTK4/WebKitGTK 6.0 | 高 | [56][62][67] |
| 交叉编译 | MSVC target 基本要在 Windows 上构建；Tauri 的 MSI 只能 Windows 构建，NSIS 跨编译是 last resort | 纯 Go 交叉简单；cgo 必须提供 C cross-compiler | 高 | [53][89] |
| protobuf | `prost` 0.14.4，但自述 passively-maintained | `google.golang.org/protobuf` v1.36.12，活跃 | 高 | [96][99] |
| 异步网络 | tokio 1.54.1（TCP/UDP/定时器） | 标准库 `net` + goroutine | 高 | [97][91] |
| 日志 | `tracing` / `log` | 标准库 `log/slog` | 高 | [115][92] |
| CLI | clap 4.6.7 | cobra 1.10.2 | 高 | [98][100] |
| 测试 seam | 需要自己设计 trait/接口；当前 core 无接口 | 接口 + `net.Pipe`/httptest 风格更直接 | 中 | [19][91] |
| 长期风险 | MFC 已停止加功能；Rust GUI 生态 0.x 项目多；Slint 许可证 | Go GUI 生态更年轻；Wails v3 刚稳定；native PipeWire 缺 binding | 中 | [85][71][73][74][62][49] |

## 3. 详细发现

### 3.1 音频采集

#### 3.1.1 Windows WASAPI loopback：能力边界先于语言选择

Microsoft 对 loopback 的定义很明确：要捕获某个 render endpoint 正在播放的音频，必须拿到该 render endpoint 的 `IMMDevice`，在该设备上初始化一个 **capture** stream，并置 `AUDCLNT_STREAMFLAGS_LOOPBACK`；`IAudioClient::GetService` 拿 `IAudioCaptureClient`[24]。三个硬限制：

- 只能 shared mode；exclusive mode 不能 loopback[24]。
- Windows 10 1703 之前，event-driven + loopback 不工作；1703+ 才支持 event-driven loopback[24]。
- shared-mode stream 的声道数和采样率通常必须与 device mix format 相同；音频引擎只在 PCM sample size 上做有限转换[25]。

当前 C++ 实现正是按这个模型写的：`GetMixFormat` 拿 mix format，按 `--encoding/--channels/--sample-rate` 改 `WAVEFORMATEX`，再 `IsFormatSupported`；不支持就退到 closest match，然后 `Initialize(..., AUDCLNT_STREAMFLAGS_LOOPBACK, 1s buffer, ...)`，最后按 `GetDevicePeriod` 的周期轮询 `GetNextPacketSize/GetBuffer/ReleaseBuffer`[7]:140-289。它没有用 event callback，也没有做重采样。

**Rust 路线**

- `wasapi` crate 0.27.0（2026-11-06）把 loopback 作为一等能力：支持 shared/exclusive、event/polled、loopback、格式探测、设备变更通知；`AudioClient::initialize_client` 在 shared mode 下“almost any format can be accepted”并由引擎转换；loopback 的用法是从 render endpoint 取 client，`Direction::Render` 创建、`Direction::Capture` 初始化；`initialize_shared_audio_stream` **不支持** loopback[32][33]。这几乎可以逐行映射当前 C++ 逻辑，且能保留 `IMMDevice::GetId` 字符串[34]。
- `cpal` 0.18.2（2026-08-16）也能 loopback，但它是隐式的（loopback 支持 2019 年经 PR #339 合入[26]，但一直没有成为公开的一等 API[29]）：在 `build_input_stream_raw_inner` 里，如果 `self.data_flow() == eRender`，就 `stream_flags |= AUDCLNT_STREAMFLAGS_LOOPBACK`；默认 flag 是 `AUDCLNT_STREAMFLAGS_EVENTCALLBACK`[27]。官方文档没有把 loopback 作为公开能力描述，issue #516（2020-12-05 开，仍 open）报告“stream 起来了但 callback 不触发”，维护者 2026-09-26 仍在要完整复现[28]。cpal 也不做自动格式转换，`SampleFormat` 必须是 backend 支持的编码[30]。因此 **cpal 不适合作为 loopback 的唯一依赖**；要用就用 `wasapi` crate。
- `windows-rs` 直接调 COM 也可行，但需要自己写 `Activate/Initialize/GetService` 和轮询/事件循环；适合作为 `wasapi` crate 覆盖不到时的 fallback[108][24]。`wasapi` crate 本身是小型项目（GitHub 95 stars、3 open issues，检索于 2026-11-09），bus factor 低，长期维护风险要纳入评估[118]。

**Go 路线**

- `malgo` v0.11.27（2026-10-29）绑定 miniaudio，`DeviceType` 有 `Playback/Capture/Duplex/Loopback`；`Context.Devices(Playback)` 枚举播放设备，`DeviceInfo.ID/Name/IsDefault` 可用于选择；`DeviceConfig` 有 `SampleRate`、`PeriodSizeInFrames`、`PeriodSizeInMilliseconds`、`Periods`，格式/声道在 `Playback/Capture SubConfig` 里[39]。miniaudio 的 WASAPI `ma_device_id.wasapi` 存的是 `IMMDevice::GetId` 的 wchar_t 字符串（`wchar_t[64]`，含结尾 NUL），所以 WASAPI endpoint ID 可以保留；但 malgo 的 `DeviceID` 是定长 byte array，`String()` 返回十六进制，拿 endpoint ID 需要按 UTF-16 解 `wasapi` 字段[120][39]。miniaudio 的 loopback 文档明确“currently only supported on WASAPI”，示例用 `ma_device_type_loopback` 并把 `capture.pDeviceID` 指向播放设备[36][37]。格式支持 U8/S16/S24/S32/F32[40]；miniaudio 内部支持 sample format、channel count、sample rate 转换（`ma_data_converter`，resampler 有 Linear/Custom）[38]。
- `oto` 是播放库，不支持采集[51]。
- PortAudio 上游有 `PaWasapi_IsLoopback`，但它只“检测”设备是否 loopback，flag 列表里没有明确的“创建 loopback stream”选项[41]。因此 Go + PortAudio 不是直接可用的 loopback 方案。
- `golang.org/x/sys/windows` 提供 Win32/syscall 包装和 `CoInitializeEx`/`CoGetObject`/`GUID`；从文档列表看没有通用 COM 智能指针/`QueryInterface` helper，直接写 WASAPI 需要手写 vtable 调用，工程量明显高于 Rust 的 `wasapi` crate[42]。

**结论**：Windows 上两者都能做，Rust 的路径更“显式、可验证”，Go 的路径更“少代码但多一层 miniaudio 转换”。若要求与当前行为尽量一致（尤其是“请求 44.1kHz/2ch，失败就退 closest match”），Rust + `wasapi` 更容易复现；若接受 miniaudio 的自动重采样/混音，Go + malgo 更省事。

#### 3.1.2 Linux PipeWire：Rust 是原生，Go 是兼容层

当前 Linux 实现是纯 PipeWire C API：`pw_init` → `pw_main_loop_new` → `pw_context_connect`；capture 用 `pw_stream_new_simple` + `PW_KEY_MEDIA_TYPE=Audio`、`PW_KEY_MEDIA_CATEGORY=Capture`、`PW_KEY_TARGET_OBJECT=<object.serial>`；format 默认 `SPA_AUDIO_FORMAT_F32_LE`，`s8` 映射到 `SPA_AUDIO_FORMAT_U8`；endpoint 列表枚举 `PW_TYPE_INTERFACE_Node` 且 `media.class=Audio/Sink`，用 `object.serial` 作 ID、`priority.session` 选默认[8]:100-443。PipeWire 自带的 `pw-cat`/`pw-record` 也通过 `--target` 指定 node，和 `PW_KEY_TARGET_OBJECT` 是同一套寻址思路[43]。

- **Rust**：`pipewire` crate 0.10.1（2026-08-19）是 PipeWire 项目维护的 safe binding；官方 `audio-capture` 示例演示了 `MainLoop`/`Context`/`StreamBox`、`AUTOCONNECT|MAP_BUFFERS|RT_PROCESS`、`param_changed` 解析 `AudioInfoRaw`、`process` 里 `dequeue_buffer` 读取 chunk[47][48]。示例只 offer F32LE，rate/channels 留空让 graph 决定；`--target` 只在 `v0_3_44` feature 下可用；示例里 monitor 的 `stream.capture.sink` 行是注释掉的[48]。这意味着“枚举 Audio/Sink、按 serial 选目标、抓 monitor”都能做，但 **非 F32 格式和 monitor 路径需要自己补**，不能直接照抄示例。
- **Go**：没有找到成熟的 native PipeWire binding（也没有找到成熟的 cgo binding）。`purego-pipewire` 自述 early stage、API 可能变、只覆盖 playback 和 video producer，capture/monitor 不在 API 里[49]。`malgo` 的 Linux backend 是 PulseAudio/ALSA/JACK，不是 PipeWire[39]；它能通过 `pipewire-pulse` 工作，但不是 native。`jfreymuth/pulse` 是纯 Go 的 PulseAudio 客户端，提供 `RecordMonitor(sink)`、`RecordSource`、`DefaultSink`、`ListSinks`，README 明确说“shm support is still missing”[50]。PipeWire 官方 `libpipewire-module-protocol-pulse` 提供 PulseAudio server 兼容层，`pactl`/`pavucontrol` 等工具可继续工作[46]。
- **会丢失什么**：走 Pulse 兼容层时，端点 ID 从 PipeWire `object.serial` 变成 Pulse sink name/index；默认端点从“最高 `priority.session`”变成 Pulse 的 default sink；`target.object` 的精确 node 选择变成 sink 选择。编码上还有一个具体缺口：`jfreymuth/pulse` 的高层 writer 只有 U8/S16/S32/F32，`proto` 包没有 24-bit 常量（PulseAudio 的 24-bit 值被保留但未暴露）；若必须保留 S24，需要直接给 `SampleSpec.Format` 填保留值（未确认可用）或转成 S32，24-bit 端到端保真会受影响[50][121]。功能上仍能“列 sink、选 sink、录 monitor”，但配置迁移、编码和默认语义要改。若走 ALSA，则需要 `snd-aloop` 内核模块提供虚拟 loopback 声卡；它不做 rate/format/channel 转换，每个设备最多 8 个 substream，也拿不到 PipeWire 的 per-app 路由/默认 sink 语义[119]。若坚持 native PipeWire，Go 需要自己写 cgo binding，风险和工期都高。

**结论**：Linux 是 Rust 的明显优势项。Go 的可行路线是“PulseAudio 兼容层”，不是“PipeWire native”。

#### 3.1.3 延迟、格式与故障行为

当前实现的延迟下限由 1 秒 buffer 决定，轮询周期是 `GetDevicePeriod` 的最小周期；没有重采样、没有 underrun 处理、没有时间戳/序号[7]:218-289。[10]:396-425 的 UDP 分片也没有 pacing，弱网下会出现突发和丢包。迁移不会改变这些协议层特性；改变的是采集侧的 buffer 和转换行为。

- Rust `wasapi`：`StreamMode` 同时包含 share mode、`Events/Polling`、buffer duration（100ns 单位）；`get_device_period()` 可取默认/最小时长；loopback 下 period 参数会被忽略[33]。
- Rust `cpal`：`BufferSize::Default` 可能“surprisingly large”，`Fixed(n)` 只是请求，实际大小由 backend 决定；小 buffer 降延迟但增加 dropout 风险[31]。
- Go `malgo`：`PeriodSizeInFrames`/`PeriodSizeInMilliseconds`/`Periods` 可配；miniaudio 的 `ma_data_converter` 在设备初始化的请求格式与 backend native 格式之间做转换，resampler 是 Linear/Custom[38][39]。

**未确认**：没有任何公开资料给出 cpal/wasapi/malgo/miniaudio 在 WASAPI loopback 下的延迟、CPU、glitch 对比；必须用同一台 Windows 机器、同一段音频、同一 1 秒 buffer 做 A/B，记录 callback 间隔、丢帧、漂移。Linux 侧同理，需要 PipeWire graph 的 quantum/rate 固定后再测。

### 3.2 GUI 替换

#### 3.2.1 Tauri v2：功能最全，但绑定系统 WebView

Tauri v2.12.5 是当前 stable（2026-10-21）；v3.0.0-alpha.6 仍是 alpha（2026-11-06），不建议新项目从 v3 alpha 起步[60][61]。Tauri 用系统 WebView：Windows 是 WebView2，Linux 是 `webkit2gtk-4.1`；构建依赖 MSVC、WebView2 Evergreen Bootstrapper、Linux 的 `libwebkit2gtk-4.1-dev` 等[56]。官方称最小 Tauri app 可小于 600KB，因为它不打包浏览器引擎[57]。

对本项目最关键的能力：

- **托盘**：`tauri = { features = ["tray-icon"] }`；Rust `TrayIconBuilder` 或 JS `TrayIcon.new`；菜单、click/double-click/hover 事件都有；但 Linux 上 **click 事件不发送**，只有右键菜单可用[52]。
- **安装器**：MSI（WiX v3）或 NSIS；MSI 只能在 Windows 上构建；NSIS 跨编译是 last resort，需要 `cargo-xwin` + LLVM/LLD[53]。
- **Linux 包**：deb、rpm、AppImage、Snap、Flatpak、AUR[54]。
- **Sidecar**：`bundle.externalBin`，按 target triple 命名；Rust `app.shell().sidecar(name)`；这使“GUI 壳 + 现有 `as-cmd`”成为官方支持的路径[55]。
- **更新**：官方 updater plugin，支持 Windows/Linux/macOS；更新必须签名，不能关闭[58]。
- **签名**：官方 Windows signing guide 覆盖 OV（2023-06-01 前签发）与 EV（走 issuer 文档）；`signtool` 由 Tauri 调用，也可用 `signCommand` 自定义；跨编译必须自定义 sign command[59]。

**评价**：如果目标是“最快复刻现有 GUI + 跨平台 + 成熟打包”，Tauri v2 是首选。代价是 Linux 用户需要 WebKitGTK 4.1（一个不小的运行时依赖），以及 WebView 的安全面/内存占用；Windows 10 1803+/11 通常已自带 WebView2，Linux 的 WebKitGTK 4.1 才是更明显的额外依赖[56]。Rust 后端可以直接 `cxx`/`cc` link C++ core，也可以走 sidecar[101][103][55]。

#### 3.2.2 Wails v3：Go 侧最接近 Tauri 的选择

Wails v3.0.0 于 2026-10-26 稳定，v3.1.0 于 2026-11-01 发布；要求 Go 1.26+[62]。它把 app/window/service/event 做成显式对象，支持多窗口、native menu、system tray、Go service、生成的 TypeScript bindings、in-app updater；Windows 是 WebView2，Linux 默认 GTK4 + WebKitGTK 6.0（GTK3 可用 `-tags gtk3` 到 v3.1）[62][65][66]。打包支持 NSIS、MSIX、AppImage、deb、rpm、Arch[62]。v3.1 增加了实验性的 macOS/Linux CGO-free backend，但 Windows 仍需 cgo[63]。

Wails 的托盘/菜单是内置能力，比 Fyne 的 Material 风格更接近“系统原生壳 + Web UI”[62]。对本项目，Wails 可以直接用 Go 写服务端逻辑（`net` + protobuf + malgo/pulse），GUI 用 HTML 表单/日志区，i18n 用前端方案。Windows 上 WebView2 在绝大多数安装中已存在；Linux 的 GTK4/WebKitGTK 6.0 是更明显的依赖[66][62]。

**代价**：

- Windows 构建要求 cgo + MinGW-w64（MSYS2）[67]；这与当前 MSVC/vcpkg 工具链不同。若要把 MSVC 编译的 C++ core 静态库链进 Go，ABI/CRT 不匹配的风险很高（见 3.5）。
- Linux 构建要求 GTK4 + WebKitGTK 6.0 开发包；Ubuntu 24.04+/Debian 13+ 才比较顺，老 distro 要用 GTK3 路径[67][62]。
- v3 刚稳定，生态/文档仍在追赶 v2；v2 仍在维护（v2.17.1，2026-11-01），但 v3 是重新设计，迁移是 port 而非版本升级，新项目应直接用 v3[62][63][116]。

#### 3.2.3 Slint：最像原生应用的 Rust 选项

Slint 1.18.1（2026-09-21）是声明式 UI，编译成机器码；渲染后端有 femtovg（OpenGL ES 2.0）、Skia、software，Qt style 在系统有 Qt 时可用[71]。它支持 Rust/C++/JavaScript/Python，桌面/移动/嵌入式都能跑[122][71]。

- **托盘**：`SystemTrayIcon` 在 1.17 引入（2026-06-24），支持 icon/tooltip/visible/title、`clicked()`、一个 Menu 子元素；Windows 用 `Shell_NotifyIcon`，macOS 用 `NSStatusItem`，Linux/*BSD 用 StatusNotifierItem/AppIndicator；GNOME 需要 AppIndicator/KStatusNotifierItem 扩展[68][69]。
- **i18n**：`@tr(...)` + gettext；支持插值和复数；运行时切换语言在 2023 年的文章里还是“计划”，到 1.18 是否完全落地未确认[72]。
- **打包**：未找到官方 installer bundler；需要自己接 WiX/NSIS 或 `cargo-packager`（未确认后者对 Slint 的官方支持）。
- **许可证**：Slint 提供三选一：royalty-free（桌面/移动/Web 的专有应用）、GPLv3、商业许可[71]。本项目是 Apache-2.0[1]；GPLv3 不兼容，royalty-free 条款需要法务确认。这是选型前必须解决的问题。

**评价**：Slint 的 native 渲染、托盘、i18n 和小体积很适合“桌面工具”，但生态比 Tauri 小，打包要自己补，许可证要审查。若团队不接受 WebView，Slint 是 Rust 侧最合理的 native 方案。

#### 3.2.4 Fyne：Go 侧最完整的 native 方案

Fyne 2.8.2（2026-10-12）是 Go 的 Material 风格 GUI，支持 Windows/Linux/macOS[114]。托盘从 2.2.0 起内置：`desk.SetSystemTrayMenu(fyne.NewMenu(...))`，macOS/Windows/Linux 都支持[80]。i18n 从 2.5 起内置：`lang.L` / `lang.X` / `lang.N`，JSON 翻译文件 + `//go:embed` + `lang.AddTranslationsFS`，BCP-47 匹配[81]。打包 `fyne package` 在 Windows 只产 exe（图标/元数据嵌入），Linux 只产 tar.gz（`usr/local/` 结构），没有 MSI/deb/AppImage[82]。

**评价**：如果坚持 Go + native，Fyne 是比 Gio 更合适的选择：托盘/i18n/表单/列表都有，学习成本低。缺点是打包弱、UI 风格固定、复杂日志视图/表格不如 Web UI 灵活。

#### 3.2.5 egui / iced / Dioxus / Gio

- **egui/eframe 0.37.0（2026-10-15）**：immediate mode、纯 Rust、native + web；官方 README 没有 native menu/tray API，需要 `tray-icon` 等第三方 crate[107]；i18n 也要自己接[73][110]。适合工具型 UI，不适合“系统托盘常驻 + 设置表单”这种传统桌面壳。
- **iced 0.14.0（2025-12-07）**：native Rust、Windows/Linux/macOS/Web、内置 widgets、async；README 自述“currently experimental software”，且未列出 tray/i18n 内置能力[74][75]。0.14 有大量 breaking changes（windowing API、keyboard subscription 等）[75]。
- **Dioxus 0.7.10（2026-07-30）**：desktop 用系统 WebView（wry），官方称典型 app <5MB；`dx bundle` 支持 Windows `.msi/.exe`、Linux `.appimage/.rpm/.deb`，但只能构建 native platform/architecture[76][77][78]；仓库 901 open issues（检索于 2026-11-09）[113]。0.8 仍是 alpha[79]。文档未列出内置 tray；i18n 走 Web 前端。
- **Gio 0.10.3（2026-09-28）**：immediate mode Go，支持 Windows/Linux/macOS/Android/iOS/BSD/WebAssembly；README 未列出 tray/i18n/installer；API 0.x，breaking change 走 minor[83][84]。适合自定义绘制的轻量工具，不适合本项目这种设置项较多的 GUI。

#### 3.2.6 现有功能到新 GUI 的映射

| 现有功能 | Tauri v2 | Wails v3 | Slint | Fyne | 备注 |
|---|---|---|---|---|---|
| 托盘 + 菜单 + 气泡 | 内置（Linux click 不支持） | 内置 | 内置 | 内置 | Tauri 的 Linux click 是已知限制[52] |
| 两个 tab / 设置表单 | HTML/CSS 最灵活 | HTML/CSS 最灵活 | 原生控件 | 原生控件 | 都可做 |
| 端点/编码下拉 | 简单 | 简单 | 简单 | 简单 | 数据来自 core CLI/FFI |
| 启动/停止 + 状态 | Rust command + event | Go service + event | Rust callback | Go callback | 需把阻塞 stop 放后台 |
| 日志查看器 | `<pre>` + tail | `<pre>` + tail | TextEdit/ScrollView | TextGrid/Scroll | 当前 GUI 没有，新增成本低 |
| i18n | 前端 i18n | 前端 i18n | `@tr`/gettext | `lang.L` | 现有 CSV 需转换 |
| 更新检查 | 官方 updater | 内置 updater | 自己请求 GitHub API | 自己请求 | 都可复用 GitHub releases |
| 关于 | HTML | HTML | 原生 | 原生 | 简单 |
| 防火墙修复 | Rust windows-rs COM 或 `netsh` | Go COM/`netsh` | Rust COM/`netsh` | Go COM/`netsh` | Windows-only；需提权 |
| 单实例 | 插件/自己实现 | v3 内置 | 自己实现 | 自己实现 | 逻辑简单 |
| 开机启动/关闭到托盘 | 插件/自己实现 | 内置/自己实现 | 自己实现 | 自己实现 | 平台 API |

### 3.3 打包与分发

#### Windows

- 当前：MFC 只发 exe；core 另发 zip/tar.gz；MFC 动态链接；vcpkg `x64-windows-static-md`；README 要求 VC++ Redist[1][18][21]。
- Rust：MSVC 默认动态 CRT，仍然依赖 VC++ Redist；`RUSTFLAGS='-C target-feature=+crt-static'` 可静态 CRT，但所有 native 依赖（protobuf、pipewire、C++ core）必须用匹配的 CRT 重新编译[88][87]。Tauri 的 MSI/NSIS 都会处理 WebView2 bootstrapper[53]。
- Go：纯 Go 二进制没有 VC++ 依赖；但 Wails/cgo 需要 MinGW-w64，可能引入 libgcc/libstdc++ DLL，需显式静态链接或随包分发（未确认 Wails 默认行为）[67]。
- 代码签名：当前 release workflow 没有签名步骤[20]；Tauri 文档要求 code-signing certificate，OV（2023-06-01 前）或 EV，`signtool` + timestamp[59]。不签名会影响 Windows 分发信任（SmartScreen 的具体触发行为未在本次来源中确认）；这是所有方案共同成本，不是迁移新增。

#### Linux

- 当前：只发 tar.gz，动态依赖 `libpipewire-0.3`，静态 libstdc++[3][21]。
- Tauri：deb/rpm/AppImage/Snap/Flatpak/AUR；但需要 WebKitGTK 4.1 运行时[54][56]。
- Wails：AppImage/deb/rpm/Arch；需要 GTK4 + WebKitGTK 6.0（或 GTK3 兼容路径）[62][67]。
- Dioxus：AppImage/rpm/deb[77]；Fyne：tar.gz[82]；Slint/egui/iced：自己打包。
- PipeWire 开发依赖：当前 CMake 用 `pkg_check_modules(libpipewire-0.3)`[3]:56-60；Rust `pipewire` 通过 `pipewire-sys` 绑定系统库（`system-deps` 构建依赖）[117]；若 Go 走 native PipeWire cgo，也需要 `libpipewire-0.3-dev`。这一点不会因为换语言而消失。
- AppImage 的通用打包规则可参考官方 quickstart；它只说明“下载、chmod +x、运行”，不解决打包内部细节[106]。

#### 交叉编译与 CI

- Go：纯 Go 交叉编译简单；一旦 cgo（Wails 或 C++ core），必须提供 C cross-compiler，`CC`/`CXX` 按 target 设置[89]。
- Rust：`x86_64-pc-windows-msvc` 需要 MSVC linker/Windows SDK；Tauri 的 MSI 只能在 Windows 构建，NSIS 跨编译是 last resort（`cargo-xwin` + LLVM/LLD）[53][56]。
- Dioxus `dx bundle` 只能构建 native platform/architecture，不能从 Linux 交叉出 Windows 包[77]。
- 当前 CI 是 `windows-latest` + `ubuntu-22.04` 的矩阵[20]:81-139；新栈大概率保持同样矩阵，但 Linux runner 要加 `libwebkit2gtk-4.1-dev`/`libgtk-4-dev`/`libwebkitgtk-6.0-dev` 等，Windows runner 要加 Rust MSVC 或 MSYS2/MinGW。

### 3.4 维护成本

**构建系统**：当前是 CMake + vcpkg + MSVC（Windows）/ GCC（Linux）[3][18]；Rust 是 Cargo（依赖、编译、打包一体）[93]；Go 是 modules + `go build`[90]。评估时 Rust stable 为 1.99.1（2026-10-15）[94]，Go 为 1.27.3（2026-10-19）[95]。两者都能把“拉依赖 + 编译”收敛为一条命令；C++ 的 vcpkg triplet/CRT 组合仍是最大的心智负担。

**protobuf**：`prost` 0.14.4（2026-06-07）功能齐全（proto2/proto3、oneof、map、no_std），但 docs.rs 明确标注“passively-maintained”，维护者不再加新功能，只修 bug/security[96]。Go 的 `google.golang.org/protobuf` v1.36.12（2026-08-10）是官方实现，含 `protojson`/`prototext`/reflection[99]。对本项目这种只有 3 个字段的 `AudioFormat`，prost 的风险可控；但若未来扩展 proto，Go 的生态更稳。

**异步网络**：当前 asio C++20 coroutine（`co_await async_read/async_write`、`co_spawn`）[10]:206-341；Rust 用 tokio 1.54.1 的 TCP/UDP/定时器[97]；Go 用标准库 `net`，`TCPConn`/`UDPConn` 支持并发读写[91]。Go 的 goroutine-per-connection 模型最接近当前“每个 peer 一个 read_loop/heartbeat_loop”的结构，迁移心智负担最低；tokio 的 task/spawn 也直接对应。

**日志**：当前 spdlog[3]；Rust 用 `tracing`[115]；Go 用标准库 `log/slog`（结构化、Text/JSON handler）[92]。GUI 日志查看器可以直接订阅这些 logger 的 writer。

**CLI**：当前 cxxopts[3]；Rust clap 4.6.7[98]；Go cobra 1.10.2[100]。当前 CLI 很小（`-l`、`--list-encoding`、`-b`、`-e`、`--encoding`、`--channels`、`--sample-rate`、`-V`、`-v`）[4]:24-35，迁移成本低。

**测试 seam**：当前只有 `util` 的单元测试[19]；音频/网络/协议没有自动化测试。重写时应该先把 `audio_manager` 抽象成接口（list/default/start/stop/format），用 fake 实现测 `network_manager` 的 framing/heartbeat/MTU 分片；再用真实设备做集成测试。Go 的 `net.Pipe`/`httptest` 风格和 Rust 的 trait object 都能做，Go 更直接。

**长期风险**：

- MFC 官方明确“continues to be supported，但不再加功能、不再更新文档”[85]。这不是立刻不能用，而是长期只能维护、不能演进。
- Tauri v2 稳定、生态大，但 v3 alpha 已开始，未来 1-2 年会有 v2→v3 迁移成本[60][61]。
- Wails v3 刚稳定（2026-10-26），v2 仍在维护；v3 的 Linux GTK4/WebKitGTK 6.0 对老 distro 不友好[62][67]。
- Slint 的许可证是硬门槛；egui/iced 仍是 0.x，iced 自述 experimental；Dioxus 0.8 还在 alpha[71][73][74][75][79]。
- cpal 的 loopback 是隐式能力，issue #516 长期未闭；PipeWire Rust 示例的 monitor 行是注释的，非 F32 格式要自己补[28][48]。
- Go 没有成熟 native PipeWire binding；`purego-pipewire` 仍是 early stage[49]。

### 3.5 部分迁移：保留 C++ core，只换 GUI

这是最值得认真评估的路线，因为当前 core 的公开面很小：

```cpp
// audio_manager.hpp:82-90
void start_loopback_recording(shared_ptr<network_manager>, const capture_config&);
void stop();
std::string get_format_binary();
endpoint_list_t get_endpoint_list();
std::string get_default_endpoint();
```

`capture_config` 只有 `endpoint_id/encoding/channels/sample_rate` 四个字段[5]:72-77。`network_manager` 的对外 API 也只有 `start_server/stop_server/wait_server/is_running/broadcast_audio_data`[9]:64-81。理论上，抽一个 C ABI（`create/destroy/list_endpoints/default_endpoint/start/stop/get_format`）不需要改太多业务代码。

但当前代码有三个结构性障碍：

1. **没有 library target / C ABI**：CMake 只 `add_executable`[3]:42-45；MFC 直接编译 core 源文件[18]:285-302；仓库里没有 `extern "C"` 导出。必须先加 `add_library(server-core STATIC ...)` 和一层 C wrapper。
2. **错误路径直接 `exit()`**：Windows `exit_on_failed()` 在 HRESULT 失败时 `exit(-1)`[7]:477-483；Linux 在不支持的格式时 `exit(EXIT_FAILURE)`[8]:249。这在 CLI 里可以接受，在 GUI 进程里会直接杀掉整个 app。抽库前必须改成返回错误码/异常。
3. **`stop()` 阻塞 join**：`audio_manager::stop()` join 采集线程[6]:18-22；`network_manager::stop_server()` join 网络线程并 `_audio_manager->stop()`[10]:183-194。MFC 目前在 UI 线程同步调用[13]:225；新 GUI 若直接 FFI 调用会卡 UI，必须放到后台线程/async command。

另外，`_format` 是 `shared_ptr<AudioFormat>`，采集线程写、网络线程读，没有锁[5]:95；[7]:213；[10]:220。重写/抽库时应明确同步，否则 format 查询和 stream 启动并发时会有数据竞争风险。

**三条部分迁移路线**

| 路线 | 做法 | 优点 | 缺点 | 适用 |
|---|---|---|---|---|
| A. Sidecar（推荐先做） | GUI 启动/停止现有 `as-cmd` 进程；用 `-l` 列端点、`--list-encoding` 列编码、`-b host:port` 启动；stdout 收日志 | core 零改动；Tauri 官方 sidecar 支持；Go `os/exec` 直接做；风险最低 | 无进程内状态/格式查询；stop 靠杀进程；`-l`/`--list-encoding` 是人类可读文本，GUI 要解析；日志解析脆弱；无法做细粒度错误处理 | 快速验证 GUI/打包/托盘/更新 |
| B. C++ core + Rust GUI | 抽 static lib + C ABI；Rust 用 `cxx` 或 `cc`/bindgen 调用；Tauri/Slint/egui/iced 都可 | MSVC 工具链一致；`wasapi` 式精确控制留在 C++；Rust GUI 生态成熟；FFI 面小 | 需要重构 core（错误码、线程、C ABI）；cxx 不支持 `HashMap`/`std::map`/模板，当前 `vector<pair<string,string>>` 建议改成自定义 `struct Endpoint`，不要假设 `std::pair` 能直接过桥[101]；cbindgen 覆盖面有限 | 想保留现有采集实现、逐步换 GUI |
| C. C++ core + Go GUI | 抽 static lib + `extern "C"` wrapper；Go 用 cgo 链接；Wails/Fyne | Go GUI 开发快；core 复用 | Windows 上 Go/cgo 用 MinGW-w64，和 MSVC 编译的 C++ 静态库存在 ABI/CRT 混链风险；需要 `-lstdc++` 和 C++ runtime；调试困难 | 团队以 Go 为主且愿意承担工具链风险 |

**FFI 细节**

- `cxx` 提供安全桥：一个 `#[cxx::bridge]` 声明两边签名，生成 `extern "C"` shim + static assertions；支持 `std::string`、`std::vector`、`SharedPtr`、opaque 类型，但 `Option`/`HashMap`/`std::map`/模板未实现，`CxxString`/`CxxVector` 不能按值传，opaque 类型只能通过引用/Box/UniquePtr[101]。当前 API 的 `vector<pair<string,string>>` 需要改成 `struct Endpoint { string id; string name; }` 的 vector，或走 C ABI。
- `cbindgen` 从 Rust 生成 C/C++11 header；适合“Rust 是库、C++ 是调用方”的反向场景，也适合生成 C header 给 Go cgo；但项目自述覆盖是 adhoc，可能漏类型[102]。
- `cc` crate 可在 `build.rs` 里编译 C/C++ 成静态库；C++ 要 `.cpp(true)`，MSVC 下不链接 C++ 标准库（由 MSVC 自己处理），其他平台默认 `libstdc++`/`libc++`；C++ name mangling 需要 `extern "C"`[103]。
- Rust `staticlib` 会把本地 crate + 所有上游依赖打成一个 `.a`/`.lib`，适合链进非 Rust 应用；动态依赖要手动指定，导出符号可能需要 version script/def 文件[88]。
- Go cgo 会自动编译包内的 `.cc/.cpp`，用 `CXXFLAGS`/`LDFLAGS`；但 Go 不能直接表达 C++ 类型，必须写 `extern "C"` wrapper；C++ 异常不能跨 cgo 边界（未确认具体行为，需实测）[89]。

**结论**：如果近期就要交付新 GUI，**先做 A（sidecar）**；如果确定要长期去 MFC，**B（C++ lib + Rust GUI）** 的工程风险明显低于 C，因为 Windows 上 Rust/MSVC 与现有 C++ 工具链一致。C 只有在“团队 Go 能力强 + 能接受 MinGW 工具链”时才建议。

## 4. 风险与未知

1. **没有实测 latency/glitch 数据**。cpal/wasapi/malgo/miniaudio 的 loopback 行为差异、CPU 占用、设备切换/睡眠恢复表现都未确认；必须做 8 小时 soak + 设备热插拔测试。
2. **cpal loopback 是隐式能力**，issue #516 仍 open；不能把 cpal 当作唯一方案[27][28]。
3. **PipeWire Rust 示例只覆盖 F32LE**，monitor 行是注释的；S16/S24/S32、多声道、target 选择的实际行为需要原型验证[48]。
4. **Go native PipeWire binding 不成熟**；Pulse 兼容层会改变端点 ID/默认端点语义，旧 `config.ini` 需要迁移[49][50][46]。
5. **Go + MSVC C++ 静态库的 ABI/CRT 风险**：Wails Windows 用 MinGW-w64[67]，与 vcpkg `x64-windows-static-md` 的 MSVC 库不是同一工具链；未确认可行，需最小链接实验。另外，core 的 Windows 实现依赖 WIL（`wil::com_ptr`）[7]:24, 119-137，MinGW 下能否直接编译也未确认。
6. **C++ core 的 `exit()` 会杀 GUI 进程**；不先重构就不能安全 FFI[7][8]。
7. **Tauri v3 alpha**：现在选 v2 稳定，但 1-2 年内可能面临 v3 迁移[60][61]。
8. **Wails v3 年轻**：v3.0.0 才 2026-10-26 稳定；Linux GTK4/WebKitGTK 6.0 对 Ubuntu 22.04 不友好（22.04 是当前 CI 的 Linux runner）[62][67][20]。
9. **Slint 许可证**：GPLv3/commercial/royalty-free 三选一；Apache-2.0 项目使用前需法务确认 royalty-free 条款[71]。
10. **egui/iced 0.x**：iced 自述 experimental；官方文档均未列出内置 tray/i18n，需要额外 crate，长期 breaking change 风险高[73][74][75]。
11. **代码签名**：当前 release 未签名；Windows 分发信任（SmartScreen）是共同成本，具体触发行为未在本次来源中确认[20][59]。
12. **体积数字大多未确认**：只有 Tauri（<600KB）和 Dioxus（<5MB）有官方数字；Wails/Slint/Fyne/egui/iced 没有官方可比数字，需要实测[57][76]。
13. **Android 兼容性**：帧格式、protobuf 字段号、UDP 分片规则必须兼容；当前协议没有显式字节序，重写必须固定 little-endian[9][10][11][104]。
14. **现有 GUI 功能面比任务描述窄**：没有 log view、没有 channels/sample-rate UI；新 GUI 若按任务描述做“log view”，是新增功能而非复刻[13][15][16]。

## 5. 建议

### 5.1 Shortlist

**首选（长期）：Rust 全量重写 + Tauri v2 GUI**

- Windows：`wasapi` crate 0.27.0 + `windows-rs`（防火墙/单实例）；保留 IMMDevice ID。
- Linux：`pipewire` crate 0.10.1 + `stream.capture.sink`/`target.object`；保留 `object.serial`。
- 网络：tokio + prost + clap + tracing。
- GUI：Tauri v2（最快复刻、打包最全）；若必须 native，换 Slint 1.18（先解决许可证）。
- 理由：音频路径最显式、Windows endpoint ID 可保留、Rust/MSVC 与现有 C++ 工具链一致、Tauri 打包成熟[32][33][47][53][56]。

**次选（最低风险，先落地）：Rust GUI + 现有 `as-cmd` sidecar**

- Tauri v2 sidecar 或 Slint + `std::process`；GUI 只做托盘、设置、启动/停止、端点/编码列表、日志 tail。
- 优点：core 零改动；1-2 周可出可试用版本；后续再决定是否抽库/重写[4][55]。
- 缺点：进程外控制、日志解析、无法细粒度查询格式；但足以验证 GUI/打包/更新/托盘。

**备选（团队 Go 为主）：Go 全量重写 + Wails v3**

- Windows：malgo `Loopback` + miniaudio 自动转换；Windows 可保留 IMMDevice ID（需从 malgo 的定长 byte array 按 UTF-16 解出）；Linux 需要配置迁移。
- Linux：`jfreymuth/pulse` `RecordMonitor`，依赖 `pipewire-pulse`；接受端点 ID/默认语义变化。
- GUI：Wails v3（WebView2/GTK4），或 Fyne（native + 内置 tray/i18n，但打包弱）。
- 风险：Go native PipeWire 缺失、Wails cgo/MinGW、Windows endpoint ID 虽可保留但 malgo 转换较绕、Linux 端点语义变化、Pulse 路线缺 S24 高层支持[39][49][50][62][67][120][121]。

**不建议**：

- 从 Tauri v3 alpha 起步（v2 稳定，v3 未稳定）[61]。
- 用 cpal 做 loopback 唯一依赖而不做 spike（issue #516）[28]。
- 直接 cgo 调 native PipeWire（没有成熟 binding）[49]。
- 在未移除 `exit()` 前把当前 C++ core 塞进 GUI 进程[7][8]。

### 5.2 原型/下一步研究

1. **Sidecar spike（1-2 周）**：Tauri v2 壳 + `as-cmd`；托盘、Start/Stop、`-l` 端点列表、`--list-encoding`、stdout 日志 tail；打 MSI/NSIS + AppImage/deb；测启动/停止/异常退出。验收：Windows 10/11 + Ubuntu 22.04/24.04 可安装运行[55][53][54]。
2. **Rust 音频 spike（2-3 周）**：Windows 用 `wasapi` crate 枚举 render endpoints、列 ID/name、loopback 采集 f32/s16/s24/s32、44.1/48k、1s buffer；Linux 用 `pipewire` crate 枚举 Audio/Sink、target.object、`stream.capture.sink`、F32LE；与 `as-cmd` 同机同音频 A/B，记录 callback 间隔、丢帧、漂移、CPU。验收：8 小时无 crash/明显 glitch；格式/端点行为与当前一致或差异有解释[32][33][47][48]。
3. **Go 音频 spike（2-3 周）**：Windows malgo `Loopback` + `DeviceConfig`；Linux `jfreymuth/pulse` `RecordMonitor`；重点测 miniaudio 自动重采样/混音对音质/延迟的影响、S24 缺口，以及 device ID 的 UTF-16 转换/迁移。验收：同上；明确“接受/不接受”的差异清单[39][38][50][120][121]。
4. **FFI spike（2 周）**：把 core 抽 `add_library` + C ABI（`create/destroy/list/default/start/stop/format`），把 `exit()` 改成错误码，`stop()` 改可取消；用 `cxx` 或 `cc` 从 Rust 调用；测 Windows MSVC + Linux GCC 两边链接。验收：Rust GUI 能列端点/启动/停止/拿 format，不崩不卡 UI[3][5][7][101][103]。
5. **打包/CI spike（1 周）**：Tauri v2 在 `windows-latest` + `ubuntu-22.04` 构建；检查 WebView2/WebKitGTK 依赖、VC++ Redist/static CRT、代码签名、安装器体积；对比当前 zip/tar.gz 的用户体验[20][53][56][59]。
6. **兼容性回归**：用现有 Android APK 对重写后的服务端做协议回归（GET_FORMAT/START_PLAY/heartbeat/UDP id/MTU 分片），并覆盖 44.1/48k、单/双声道、5 种 encoding。验收：Android 端无感知差异[2][10][11]。

### 5.3 决策标准

- 若“音频零回归 + Windows endpoint ID 保留 + 长期可维护”权重最高：Rust 全量重写（Tauri v2 或 Slint）。
- 若“3 个月内出跨平台 GUI + 不动 core”权重最高：Rust GUI + sidecar。
- 若“团队 Go 熟练度 + 尽量静态分发”权重最高，且能接受 Pulse 兼容层：Go + Wails/Fyne（Wails 需 cgo，Fyne 更接近纯 Go）。
- 若“native 外观 + 无 WebView”权重最高：Slint（先过许可证）或 Fyne（Go）。
- 无论哪条，先做 sidecar 和音频 spike；不要先写 GUI 表单再发现采集层不满足。

## 6. 参考来源

> 检索日期均为 2026-11-09。本地文件按仓库相对路径列出；外部页面注明版本/更新日期（若页面提供）。

### 本地代码与仓库

1. `README.md`，Audio Share v0.3.4；需求、GUI/CLI 用法、编译说明。
2. `docs/protocol.md`，TCP/UDP 时序图。
3. `server-core/CMakeLists.txt`，C++20、protobuf、asio/spdlog/cxxopts、PipeWire、static libstdc++。
4. `server-core/src/main.cpp`，CLI 参数与启动流程。
5. `server-core/src/audio_manager.hpp`，encoding/capture_config/公开 API。
6. `server-core/src/audio_manager.cpp`，采集线程与 stop/join。
7. `server-core/src/win32/audio_manager_impl.cpp`，WASAPI loopback、格式协商、轮询、端点枚举。
8. `server-core/src/linux/audio_manager_impl.cpp`，PipeWire capture、Audio/Sink 枚举、默认端点。
9. `server-core/src/network_manager.hpp`，peer 表、命令枚举、心跳超时。
10. `server-core/src/network_manager.cpp`，TCP/UDP 协议、心跳、MTU 分片。
11. `protos/client.proto`，`AudioFormat` 与 encoding 枚举。
12. `server-mfc/audio-share-server/CMainDialog.cpp`，托盘、tab、关闭行为、更新气泡。
13. `server-mfc/audio-share-server/CServerTabPanel.cpp`，Server 页、端点/编码、启动/停止、声音面板。
14. `server-mfc/audio-share-server/CAppSettingsTabPanel.cpp`，设置、防火墙、开机启动、更新检查、语言。
15. `server-mfc/audio-share-server/AudioShareServer.cpp`，日志、config.ini、单实例。
16. `server-mfc/audio-share-server/AudioShareServer.rc`，对话框、控件、托盘菜单。
17. `server-mfc/audio-share-server/i18n/strings.csv`，en-US/zh-CN 文案。
18. `server-mfc/audio-share-server/audio-share-server.vcxproj`，MFC/vcpkg、i18n 生成、直接编译 core。
19. `server-mfc/unit-test/unit-test.cpp`，仅 util 测试。
20. `.github/workflows/release.yml`，Windows/Linux 构建矩阵、无签名步骤。
21. `server-core/pack.sh`，tar.gz/zip 打包。
22. `server-mfc/audio-share-server/util.cpp`，版本比较。
23. `server-mfc/audio-share-server/CAboutDialog.cpp`，关于对话框版本信息。

### 外部来源

24. Microsoft Learn, “Loopback Recording,” 更新于 2025-04-16. https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording
25. Microsoft Learn, “Device Formats,” 更新于 2025-03-11. https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-formats
26. RustAudio/cpal, Issue #251 “Support WASAPI Loopback,” 2018-11-24 开，2019-11-17 closed. https://github.com/RustAudio/cpal/issues/251
27. RustAudio/cpal, `src/host/wasapi/device.rs`（master，检索于 2026-11-09）. https://raw.githubusercontent.com/RustAudio/cpal/master/src/host/wasapi/device.rs
28. RustAudio/cpal, Issue #516 “Wasapi capture doesn't work: Stream is spawned but callback not called,” 2020-12-05 开，2026-09-28 仍有讨论，open. https://github.com/RustAudio/cpal/issues/516
29. docs.rs, `cpal` 0.18.2（2026-08-16）. https://docs.rs/crate/cpal/latest
30. docs.rs, `cpal::SampleFormat`（0.18.2）. https://docs.rs/cpal/latest/cpal/enum.SampleFormat.html
31. docs.rs, `cpal::BufferSize`（0.18.2）. https://docs.rs/cpal/latest/cpal/enum.BufferSize.html
32. docs.rs, `wasapi` 0.27.0（2026-11-06）. https://docs.rs/wasapi/latest/wasapi/
33. docs.rs, `wasapi::AudioClient`（0.27.0）. https://docs.rs/wasapi/latest/wasapi/struct.AudioClient.html
34. docs.rs, `wasapi::Device`（0.27.0）. https://docs.rs/wasapi/latest/wasapi/struct.Device.html
35. docs.rs, `wasapi::WaveFormat`（0.27.0）. https://docs.rs/wasapi/latest/wasapi/struct.WaveFormat.html
36. mackron/miniaudio, Issue #90 “wasapi loopback,” 2019-08-15 开，2019-08-27 closed. https://github.com/mackron/miniaudio/issues/90
37. miniaudio, “simple_loopback” example. https://miniaud.io/docs/examples/simple_loopback.html
38. miniaudio manual, “Data Conversion” section. https://miniaud.io/docs/manual/index.html
39. pkg.go.dev, `github.com/gen2brain/malgo` v0.11.27（2026-10-29）. https://pkg.go.dev/github.com/gen2brain/malgo
40. pkg.go.dev, `malgo.Format`（v0.11.27）. https://pkg.go.dev/github.com/gen2brain/malgo#Format
41. PortAudio, `pa_win_wasapi.h` v19 doxydocs. https://files.portaudio.com/docs/v19-doxydocs/pa__win__wasapi_8h.html
42. pkg.go.dev, `golang.org/x/sys/windows` v0.49.0（2026-10-06）. https://pkg.go.dev/golang.org/x/sys/windows
43. PipeWire docs, `pw-cat` man page. https://docs.pipewire.org/page_man_pw-cat_1.html
44. PipeWire source, `src/pipewire/keys.h`（master，检索于 2026-11-09）. https://raw.githubusercontent.com/PipeWire/pipewire/master/src/pipewire/keys.h
45. PipeWire docs, “Loopback” module. https://docs.pipewire.org/page_module_loopback.html
46. PipeWire docs, “Protocol Pulse” module. https://docs.pipewire.org/page_module_protocol_pulse.html
47. docs.rs, `pipewire` 0.10.1（2026-08-19）. https://docs.rs/crate/pipewire/latest
48. docs.rs, `pipewire` 0.10.1 `examples/audio-capture.rs`. https://docs.rs/crate/pipewire/0.10.1/source/examples/audio-capture.rs
49. bnema/purego-pipewire（early stage，无 release tag，检索于 2026-11-09）. https://github.com/bnema/purego-pipewire
50. pkg.go.dev, `github.com/jfreymuth/pulse` v0.1.3（2026-08-17）. https://pkg.go.dev/github.com/jfreymuth/pulse
51. ebitengine/oto README（播放库，检索于 2026-11-09）. https://github.com/ebitengine/oto
52. Tauri v2 docs, “System Tray.” https://v2.tauri.app/learn/system-tray
53. Tauri v2 docs, “Windows Installer.” https://v2.tauri.app/distribute/windows-installer
54. Tauri v2 docs, “Distribute.” https://v2.tauri.app/distribute/
55. Tauri v2 docs, “Embedding External Binaries (Sidecar).” https://v2.tauri.app/develop/sidecar
56. Tauri v2 docs, “Prerequisites.” https://v2.tauri.app/start/prerequisites/
57. Tauri v2 docs, “Start.” https://v2.tauri.app/start/
58. Tauri v2 docs, “Updater.” https://v2.tauri.app/plugin/updater/
59. Tauri v2 docs, “Windows Code Signing.” https://v2.tauri.app/distribute/sign/windows/
60. GitHub, tauri-apps/tauri release `tauri-v2.12.5`（2026-10-21）. https://github.com/tauri-apps/tauri/releases/tag/tauri-v2.12.5
61. GitHub, tauri-apps/tauri release `tauri-v3.0.0-alpha.6`（2026-11-06）. https://github.com/tauri-apps/tauri/releases/tag/tauri-v3.0.0-alpha.6
62. GitHub, wailsapp/wails release `v3.0.0`（2026-10-26）. https://github.com/wailsapp/wails/releases/tag/v3.0.0
63. GitHub, wailsapp/wails release `v3.1.0`（2026-11-01）. https://github.com/wailsapp/wails/releases/tag/v3.1.0
64. GitHub, wailsapp/wails README（检索于 2026-11-09）. https://github.com/wailsapp/wails
65. Wails docs, “Window Options”（Windows/WebView2）. https://wails.io/features/windows/options
66. Wails docs, “Installation”（WebView2 Runtime）. https://v3.wails.io/getting-started/installation
67. Wails docs, “Development Setup”（Windows 需要 MinGW-w64 via MSYS2；Linux GTK4/WebKitGTK 6.0）. https://v3.wails.io/contributing/setup
68. Slint docs, “SystemTrayIcon.” https://docs.slint.dev/latest/docs/slint/reference/window/systemtrayicon
69. Slint blog, “Slint 1.17 Released,” 2026-06-24. https://slint.dev/blog/slint-1.17-released
70. GitHub, slint-ui/slint release `v1.18.1`（2026-09-21）. https://github.com/slint-ui/slint/releases/tag/v1.18.1
71. GitHub, slint-ui/slint README/licensing（检索于 2026-11-09）. https://github.com/slint-ui/slint
72. Slint blog, “Introducing Translation Infrastructure in Slint.” https://slint.dev/blog/translation-infrastructure
73. GitHub, emilk/egui README（检索于 2026-11-09）. https://github.com/emilk/egui
74. GitHub, iced-rs/iced README（检索于 2026-11-09）. https://github.com/iced-rs/iced
75. GitHub, iced-rs/iced release `0.14.0`（2025-12-07）. https://github.com/iced-rs/iced/releases/tag/0.14.0
76. Dioxus docs 0.7, “Desktop.” https://dioxuslabs.com/learn/0.7/guides/platforms/desktop
77. Dioxus docs 0.7, “Bundle.” https://dioxuslabs.com/learn/0.7/tutorial/bundle
78. GitHub, DioxusLabs/dioxus release `v0.7.10`（2026-07-30）. https://github.com/DioxusLabs/dioxus/releases/tag/v0.7.10
79. GitHub, DioxusLabs/dioxus release `v0.8.0-alpha.3`（2026-10-16）. https://github.com/DioxusLabs/dioxus/releases/tag/v0.8.0-alpha.3
80. Fyne docs, “System Tray Menu.” https://docs.fyne.io/explore/systray
81. Fyne docs, “Adding app translations.” https://docs.fyne.io/explore/translations
82. Fyne docs, “Packaging.” https://docs.fyne.io/started/packaging
83. GitHub, gioui/gio README（检索于 2026-11-09）. https://github.com/gioui/gio
84. GitHub, gioui/gio tag `v0.10.3`（commit date 2026-09-28）. https://github.com/gioui/gio/tags
85. Microsoft Learn, “MFC Desktop Applications,” 更新于 2026-02-23. https://learn.microsoft.com/en-us/cpp/mfc/mfc-desktop-applications
86. Microsoft Learn, “Latest Supported Visual C++ Redistributable Downloads,” 更新于 2026-08-04. https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist
87. Microsoft Learn, “/MD, /MT, /LD (Use runtime library),” 更新于 2025-03-28. https://learn.microsoft.com/en-us/cpp/build/reference/md-mt-ld-use-run-time-library
88. Rust Reference, “Linkage.” https://doc.rust-lang.org/reference/linkage.html
89. Go docs, `cmd/cgo`. https://pkg.go.dev/cmd/cgo
90. Go Modules Reference. https://go.dev/ref/mod
91. pkg.go.dev, `net` package（go1.27.3）. https://pkg.go.dev/net
92. pkg.go.dev, `log/slog` package（go1.27.3）. https://pkg.go.dev/log/slog
93. The Cargo Book. https://doc.rust-lang.org/cargo/
94. Rust Blog, “Announcing Rust 1.99.1,” 2026-10-15. https://blog.rust-lang.org/
95. Go release history, go1.27.3（2026-10-19）. https://go.dev/doc/devel/release
96. docs.rs, `prost` 0.14.4（2026-06-07）. https://docs.rs/crate/prost/latest
97. docs.rs, `tokio` 1.54.1（2026-10-21）. https://docs.rs/crate/tokio/latest
98. docs.rs, `clap` 4.6.7（2026-09-14）. https://docs.rs/crate/clap/latest
99. pkg.go.dev, `google.golang.org/protobuf` v1.36.12（2026-08-10）. https://pkg.go.dev/google.golang.org/protobuf
100. GitHub, spf13/cobra release `v1.10.2`（2025-12-04）. https://github.com/spf13/cobra/releases/tag/v1.10.2
101. GitHub, dtolnay/cxx README（检索于 2026-11-09）. https://github.com/dtolnay/cxx
102. GitHub, mozilla/cbindgen README（检索于 2026-11-09）. https://github.com/mozilla/cbindgen
103. docs.rs, `cc` crate. https://docs.rs/cc/latest/cc/
104. Protobuf docs, “Encoding.” https://protobuf.dev/programming-guides/encoding/
105. GCC online docs, “Link Options”（`-static-libstdc++`）. https://gcc.gnu.org/onlinedocs/gcc/Link-Options.html
106. AppImage docs, “Quickstart.” https://docs.appimage.org/introduction/quickstart.html
107. GitHub, tauri-apps/tray-icon README（检索于 2026-11-09）. https://github.com/tauri-apps/tray-icon
108. GitHub, microsoft/windows-rs README（检索于 2026-11-09）. https://github.com/microsoft/windows-rs
109. pkg.go.dev, `os/exec` package（go1.27.3）. https://pkg.go.dev/os/exec
110. GitHub, emilk/egui release `0.37.0`（2026-10-15）. https://github.com/emilk/egui/releases/tag/0.37.0
111. GitHub, fyne-io/fyne release `v2.8.2`（2026-10-12）. https://github.com/fyne-io/fyne/releases/tag/v2.8.2
112. GitHub, tauri-apps/tauri repository（open issues 1511，检索于 2026-11-09）. https://github.com/tauri-apps/tauri
113. GitHub, DioxusLabs/dioxus repository（open issues 901，检索于 2026-11-09）. https://github.com/DioxusLabs/dioxus
114. GitHub, fyne-io/fyne repository（open issues 755，检索于 2026-11-09）. https://github.com/fyne-io/fyne
115. docs.rs, `tracing` crate. https://docs.rs/tracing/latest/tracing/
116. GitHub, wailsapp/wails release `v2.17.1`（2026-11-01）. https://github.com/wailsapp/wails/releases/tag/v2.17.1
117. docs.rs, `pipewire-sys` 0.10.1（FFI bindings，`system-deps` 构建依赖）. https://docs.rs/crate/pipewire-sys/latest
118. GitHub, HEnquist/wasapi-rs repository（95 stars、3 open issues，检索于 2026-11-09）. https://github.com/HEnquist/wasapi-rs
119. ALSA Project Wiki, “Matrix:Module-aloop”（snd-aloop 虚拟 loopback 声卡）. https://www.alsa-project.org/wiki/Matrix:Module-aloop
120. miniaudio source, `miniaudio.h`（master，检索于 2026-11-09）：`ma_device_id.wasapi` 为 `wchar_t[64]`，WASAPI 后端用 `IMMDevice::GetId` 填充. https://raw.githubusercontent.com/mackron/miniaudio/master/miniaudio.h
121. pkg.go.dev, `github.com/jfreymuth/pulse/proto`（v0.1.3）：Format 常量仅 U8/S16LE/BE/F32LE/BE/S32LE/BE，无 24-bit 常量. https://pkg.go.dev/github.com/jfreymuth/pulse/proto
122. Slint 官网（桌面/移动/嵌入式、Rust/C++/JavaScript/Python），检索于 2026-11-09. https://slint.dev/
