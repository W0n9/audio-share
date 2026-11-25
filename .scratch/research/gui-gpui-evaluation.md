# Audio Share 服务端 GUI 迁移评估：Tauri 2 + Svelte → GPUI

评估日期：2026-11-24  
代码基线：当前分支 `docs/v0.6.0-curation`，提交 `ddeb939`，版本 `0.6.0`[1][2][7]  
范围：仅评估 Windows/Linux 桌面「Audio Share Server」GUI 壳（Tauri v2 + Svelte 5）是否应换成 GPUI；音频采集、Wire Protocol、v2 配对运行时不在迁移范围内，但其现有 Rust 代码的可复用性在评估内。Android 客户端不在范围内。  
方法：先读本地 `gui/README.md`、`gui/src-tauri`、`tauri.conf.json`、ADR 0002 建立基线，再以 GPUI/Zed 的源码、crates.io API、GitHub issue/PR、官方博客、Tauri 官方文档与 Microsoft 文档为一手来源；每条外部事实标注编号引用。WebSearch/WebFetch 与已认证的 `gh` API 均可用；crates.io 的网页版为 SPA/403，版本数据统一改用 crates.io API；lib.rs、InfoQ 等个别页面未能取到正文，未作为结论依据。**本次未安装 GPUI、未编译、未运行任何 GPUI 原型**；所有“能否运行”的结论按证据分级标注，无法确认的写“未确认”。

## 摘要

**结论：技术上可行，但现阶段不值得换。建议保留 Tauri 2 + Svelte 5，不启动迁移；把 GPUI 列为“上游成熟后再评估”的候选，而不是当前路线。**

“可行”的部分是真实的：GPUI 能画这个 420x640 的单窗口，内置系统通知、打开浏览器、文件对话框和 SVG/图像渲染[13][11][64]；Audio Share 的 Rust 壳约 6.1k 行中约 5.0k 行（controller/model/update/autostart/i18n/logging/maintenance/v2 等）与 UI 技术无关，可直接复用（其中 614 行 maintenance 只需改触发入口）[6]；前端只有两个真正的文本输入（Host 与 Port），没有自由文本编辑场景[1]。因此这不是“做不出来”的问题。

“不值得”的三个决定性理由：

1. **GPUI 没有一个可依赖的稳定发布渠道。** crates.io 上的官方 `gpui` 停在 **0.2.2（2025-10-22）**，已约 13 个月未更新；维护者在 2026-10-30 仍只能说“自动发布很快会上线”，并建议用户临时 pin git revision[9][18][19]。当前真正被生态使用的 `gpui-pre` **不是 Zed 官方发布**：crates.io 显示其 owner/publisher 是 `huacnlee`（推测为 Longbridge/gpui-kit 维护者），只是每周对 Zed 仓库做快照（0.3.14，2026-11-23）[10]。上游 README 明确写着 pre-1.0、“版本之间经常有破坏性变更”[11]。2026-02 还有一条被社区广泛转述的 Discord 声明称，GPUI 非 Zed 自身用例的开发“被大幅踩刹车”[62]；这不是官方公告。更硬的证据是 crates.io 的停更本身；#66275 的回复反而说明自动发布正在推进，只是尚未落地[18][62]。换到一个没有官方稳定版、API 随编辑器周更的框架上，对本项目是净负债。
2. **桌面壳最关键的三个能力（托盘、单实例、打包），GPUI 恰好都缺官方支持；i18n 也没有内置框架。** 托盘：上游 `gpui` 源码中搜不到 tray/SystemTray API，Zed 自己作为编辑器也没有托盘，只能依赖第三方 `gpui-tray`（2026-08 创建、20 stars、4.5k 下载，且 pin 死某个 Zed git rev）[37][38][39][65]；单实例：GPUI 无官方支持，Zed 是在应用层自己用命名 mutex + pipe 实现[49]，社区 `single-instance` crate 自 2021 年停更[47]；打包：GPUI 没有官方 installer bundler，需用 `cargo-packager`/`cargo-bundle` 自己接[43][45]；i18n：无内置框架，localization issue 自 2024 年 open[28]。这些正是 ADR 0002 选择 Tauri 的核心理由[4]。
3. **无障碍与中文 IME 的成熟度不足以承担“中文用户硬需求”。** 上游 2026-05 才合入 AccessKit 支持[22]；Windows“读屏完全不可用”的 issue 开于 2025-10（早于 AccessKit，可能已过时，但未关闭）[21]；Linux 在 2026-11-21 仍有 AT-SPI 静态文本不可读、窗口 frame 无 bounds 的新 issue[23][24]。IME 三平台都有实现，但中文相关 open issue 密集：X11 下 Fcitx/Sogou 一次 XIM 错误后中文输入永久失效[25]，Windows 中文 IME 文本垂直错位[26]、无法取消 composition[27]。对本项目要说明的是：当前 UI 只有 Host/Port 两个 ASCII 输入框，IME 的实际暴露面比编辑器小，风险是“中等且可测”，不是“必然阻塞”；但它仍是一个必须真机验证的缺口。

**若换，主要缺口**：第三方/自研托盘、自研单实例、自建安装器与 CI、i18n 从前端迁到 Rust、无障碍与 IME 的上游 bug、Linux 无头/软件渲染下的冒烟测试不可假定可复用。迁移量粗估：前端 2.8k 行（其中 CSS 967 行；另有 locale 310 行）全部重写；Rust 侧 `main.rs` 774 行重写、`webview2_check.rs` 254 行删除、其余约 5.0k 行可复用（其中 614 行 maintenance 需改触发入口）[6][7]。

**重新评估的触发条件**（满足任意两条再启动原型）：上游出现官方、semver 化的 GPUI 发布（非第三方 `gpui-pre` 快照）；官方或成熟第三方补齐 tray + single-instance + installer；Windows/Linux 中文 IME 与 a11y 的 open issue 收敛；Tauri 侧出现无法绕过的阻塞问题（目前没有）。

### 关键版本（检索于 2026-11-24）

| 组件 | 版本 | 发布日期 | 来源与备注 |
|---|---|---|---|
| `gpui`（crates.io 官方 crate） | 0.2.2 | 2025-10-22 | Zed 维护者发布；已约 13 个月未更新[9][18] |
| `gpui-pre`（每周快照） | 0.3.14 | 2026-11-23 | **第三方** `huacnlee`（推测为 Longbridge/gpui-kit 维护者）发布，描述为 `zed@f126950` 快照，非 Zed 官方渠道[10] |
| `gpui-ce`（Community Edition） | 0.3.0 | 2026-10-20 | 社区 fork；官网写 MIT OR Apache-2.0，crates.io/仓库 LICENSE.md 为 Apache-2.0；1,676 stars[34][35][36] |
| `gpui-kit`（原 `gpui-component`） | 0.11.0 | 2026-11-23 | Apache-2.0，21,265 stars，pin `gpui-pre =0.3.14`[40][41][42] |
| `gpui-tray` | 0.3.0 | 2026-10-21 | Apache-2.0，20 stars，4,532 下载；默认依赖某个 Zed git rev[37][38][39] |
| Zed | 1.29.3 | 2026-11-20 | Windows/Linux/macOS 稳定版；1.30 仍在 pre[33] |
| wgpu | 30.0.2 | 2026-11-12 | GPUI Linux 渲染后端（vulkan + gles features）[16][55] |
| AccessKit | 0.24.0（GPUI pin）/ 0.25.2（最新） | 2026-10-25（0.25.2） | GPUI workspace pin 0.24.0；crates.io 最新 0.25.2[56][70] |
| Tauri | 2.12.7 | 2026-11-18 | 项目当前用 2.12.5[2][57] |
| Svelte | 5.59.1 | 2026-11-20 | npm 最新[58] |
| cargo-packager | 0.11.8 | 2025-11-27 | 支持 deb/AppImage/NSIS/MSI；仓库仍活跃但近一年未发版[43][44] |
| cargo-bundle | 0.12.0 | 2026-09-20 | deb/AppImage/MSI；Windows 支持仍标注 experimental[45][46] |
| `single-instance` | 0.3.3 | 2021-12-16 | 已停更，不建议作为长期依赖[47] |
| `interprocess` | 2.4.9 | 2026-11-18 | 可用于自研单实例的本地 IPC[48] |
| Rust（Zed pin） | 1.100.0 | — | stable，edition 2024；GPUI 不要求 nightly[59] |

## 1. 系统托盘

**官方能力：没有。** `crates/gpui/README.md` 只描述窗口、渲染和文本，没有 tray/menu-bar API[11]；对 Zed 仓库做代码搜索，`SystemTray` 命中 0，`tray` 在 `crates/gpui*` 中也没有托盘实现（仅有的 hit 是 IME/窗口通知）[65]。Zed 本身是编辑器，没有系统托盘，因此不存在“Zed 自己怎么做托盘”的参考实现。这一点是“官方源码确认”，不是社区说法。

**现成方案：** `gpui-tray` 是专门为 GPUI 写的第三方 crate，0.3.0（2026-10-21），Apache-2.0，20 stars、4,532 次下载；README 明确平台后端：

- Windows：`Shell_NotifyIconW` + Win32 菜单；
- Linux：StatusNotifierItem/DBusMenu，走 `zbus`，**不依赖 GTK**；
- macOS：AppKit/`objc2`。

它使用 GPUI 现有事件循环，菜单项是 `gpui::MenuItem`，通过 foreground executor 派发 GPUI action；提供 `on_activate`、图标替换、菜单状态刷新、Linux Wayland activation token 等[37]。这比“用 `tray-icon` crate 再自己接事件循环”更贴合 GPUI：`tray-icon` 0.26.1 在 Windows 和 Linux AppIndicator 后端都要求同线程有事件循环，默认 Linux 后端依赖 GTK3 + libappindicator，只有 `ksni` feature 才能避开 GTK；而 GPUI 有自己的事件循环，接线需要额外适配[71]。`gpui-tray` 的成熟度仍然很低：仓库 2026-08-29 才创建，只有 3 个 release，last push 2026-10-21[39]。

**集成风险：**

- `gpui-tray` 的默认 feature 依赖 `gpui = { version = "0.2.2", git = "<zed>", rev = "8166e3d..." }`，即把 GPUI 锁死到某个 git revision；另有两个 feature 分别接 `gpui-pre 0.3.4`（gpui-kit）和 `gpui-ce 0.3`，并要求应用与托盘“使用同一 GPUI 源和兼容版本”[38]。换句话说，选托盘 crate 会反过来决定你用哪条 GPUI 分支。
- Windows 左键切换窗口：Tauri 当前明确实现了这个行为[1]；GPUI 侧需要在 `on_activate` 里自己调 window show/hide，`gpui-tray` 提供回调但未承诺左键语义，需真机验证（**未确认**）。
- Linux：和 Tauri 一样依赖桌面环境的 StatusNotifier host（不同桌面/扩展的呈现方式不同）；Xvfb 下没有 tray host，菜单无法在冒烟测试中验证[37][52]。

**结论：** 托盘能做，但“官方无 API + 第三方 20 stars + 锁 git rev”的组合，与 ADR 0002 所说“Tauri 有最成熟托盘 story”正好相反[4]。

## 2. 原生通知

**官方能力：有，且是内建的（已在 main / gpui-pre 3.x 确认；0.2.2 是否包含未核实）。** `Platform` trait 提供：

- `show_system_notification(SystemNotification)` / `dismiss_system_notification(tag)`；
- `SystemNotification` 带 stable tag、标题、正文、action buttons；
- `on_system_notification_response` 回传用户点击/action id；
- 文档注明“在不支持通知的平台上 no-op，或投递被抑制时 no-op”[13]。

平台实现：Linux 在 `gpui_linux/src/linux/system_notifications.rs` 直接使用 `notify-rust`（`gpui_linux` 的 Cargo.toml 把 `notify-rust` 列为 always used）[15]；Windows 有 `gpui_windows/src/system_notifications.rs`[66]；仓库还有 `crates/gpui/examples/system_notifications.rs` 示例[64]。因此本项目“更新检查成功/失败发原生通知”的需求**不需要额外 crate**，比 Tauri 的 `tauri-plugin-notification` 更直接[1][2]。

**限制：** 通知点击回传的可靠性、Linux 无 notification daemon 时的行为、Windows toast 的 AppUserModelID/安装身份，都需要在打包后真机验证；无头 CI 下必须容忍失败（Tauri 侧现有代码已经这样做）[1]。但总体这是 GPUI 相对完整的系统集成项。

## 3. 单实例

**官方能力：没有。** GPUI 平台层不提供单实例原语；Zed 自己是在应用层实现：`crates/zed/src/zed/windows_only_instance.rs` 用 `CreateMutexW` 判断首实例，再用命名 pipe 把命令行参数转给首实例[49]。注意这是 Zed 编辑器的 Windows-only 实现，不是 GPUI API，也不覆盖 Linux。

**可选方案：**

- `single-instance` crate：0.3.3，最后发布 2021-12-16，已停更，不适合长期依赖[47]。
- `interprocess` 2.4.9（2026-11-18）：提供本地 socket/管道抽象，可自己实现“抢锁 + 传参”[48]。
- 自研：Windows 命名 mutex + `WM_COPYDATA`/命名 pipe；Linux 抽象 Unix socket 或 XDG runtime 下的 lock file。工程量约 100-200 行，和 Tauri 插件当前承担的工作相当，但需要自己写测试。

**结论：** 不是硬阻塞，但属于“Tauri 开箱即用、GPUI 要自己补”的典型项。现有 `tauri-plugin-single-instance` 的接入只有一行[2]；迁移后要恢复同等行为，需要新增平台代码和真机测试。

## 4. 打包与分发

**GPUI 没有官方 bundler。** 任何 Rust 二进制都可以用通用工具打包；目前两个现实选项：

- `cargo-packager` 0.11.8：README 明确支持 Linux `.deb`、`.AppImage`、Pacman，Windows NSIS `.exe`、WiX `.msi`，macOS DMG/`.app`；配置在 `Packager.toml` 或 `Cargo.toml` 的 `[package.metadata.packager]`，支持 `beforePackagingCommand`，也有 library API；示例覆盖 egui、Slint、Dioxus、wry、Tauri 等非 Tauri 应用[43]。但 crates.io 最后一次发版是 **2025-11-27**（0.11.8），仓库虽然 2026-11-23 还有提交，近一年没有 release[44]。要把它放进发布流水线，需要接受“用 git 版或自己维护 fork”的风险。
- `cargo-bundle` 0.12.0（2026-09-20）：支持 `.deb`、`.AppImage`、`.msi`，但 README 明确说 iOS 和 Windows 支持仍是 experimental[45][46]。

**Windows 便携 exe：** 最容易的一项。`cargo build --release` 产出的 exe 本身就是便携版；图标/版本资源用 `embed-resource`/`winres` 嵌入；GPUI 自己已经用 `embed-resource` 处理 Windows manifest[12][15]。不需要安装器。当前 Tauri 项目也是用 `--no-bundle` 出便携 exe[1]。

**签名、图标、安装器：** 没有 GPUI 专属方案，和任何 Rust 应用一样需要自备 `signtool`、图标资源和 CI 步骤。Tauri 目前由 bundler 统一处理 `webviewInstallMode`、图标、桌面文件、deb 依赖等[3][8]；迁移后这些都要在 `cargo-packager` 配置和 CI 里重建。**这是“把已经解决的问题重新变成项目工作”**。

**Zed 自身的先例：** Zed Linux 官方只发 `.tar.gz`（外加社区 Flatpak），没有官方 deb/AppImage；这不能说明 GPUI 不能打包，但说明“GPUI 官方没有替你解决 deb/AppImage”[32]。

## 5. 无障碍（a11y）

**有基础框架，成熟度不足。**

- GPUI 的 a11y 由 AccessKit 提供，`a11y.rs` 的架构图明确列出 macOS、Windows、Linux（D-Bus）三个 adapter；每帧构建 `TreeUpdate` 发给平台 adapter[14]。Linux 端是 `accesskit_unix`（AT-SPI），Windows 端是 `accesskit_windows`（UIA），版本由 workspace 固定为 `accesskit 0.24.0` / `accesskit_unix 0.22` / `accesskit_windows 0.34`[15][70]。
- AccessKit 支持是 **2026-05-27 才合入**的上游 PR（#56065）[22]。这很重要：任何早于该日期的“Windows 完全不可用”报告都可能已经过时。
- 但当前缺口仍具体：
  - Windows：issue #41138“Screen reader accessibility missing completely”（2025-10-24 开，2026-05 仍有更新，仍 open）说 JAWS/NVDA 完全无反馈[21]。它早于 AccessKit，**需要重新实测**，不能直接当作现状。
  - Linux：2026-11-21 新开 #67305：AT-SPI 输出里所有静态文本只作为 node 的 accessible name，没有任何 node 实现 `org.a11y.atspi.Text`，读屏的 browse mode/say-all 读不到非控件内容；复现环境明确包含 “X11/Xvfb harness”[23]。
  - Linux：#67304 同一天报告 accessible window frame 没有 bounds（-1,-1,-1,-1），元素 extents 是 window-local[24]。
  - 另有 #66729（X11 下 AccessKit 拿不到窗口屏幕位置）等[68]。

**对本项目的影响：** 如果 Audio Share 不承诺读屏支持，a11y 不是发布阻塞项；但 Tauri/WebView 的 HTML 语义天然能提供基础可访问性，而 GPUI 需要每个 `div` 显式设置 role/id 才会进入 AccessKit 树[14]。迁到 GPUI 意味着可访问性是“从有到弱”，而不是“从无到有”。如果未来有企业/无障碍合规要求，这是一个明确的减分项。

## 6. i18n 与中文 IME

### 6.1 i18n

**GPUI 没有内置 i18n。** 对 Zed 仓库 `crates` 搜索 `i18n` 命中 0；Zed 的 localization issue #7409 自 2024-02-05 开至今仍 open[28]。GPUI 只提供文本渲染（Linux 用 cosmic-text/font-kit，Windows 用 DirectWrite）[15][16][30]，不提供翻译资源、复数、运行时切换。

本项目现状是：`en-US.json`/`zh-CN.json` 为前端文案源，Rust 侧 `i18n.rs`/`strings.rs` 为托盘/通知文案，两边都嵌入二进制[1][6]。迁到 GPUI 后：

- Rust 侧 `i18n.rs`/`strings.rs` 可复用；
- 前端 310 行 locale JSON 和所有 `t("...")` 调用要迁成 Rust 侧查表（或引入 `rust-i18n`/`fluent`，需另选型）；
- “语言变更后重启生效”的逻辑可保留；
- 主题的 `prefers-color-scheme` 检测[7]要换成 GPUI 的窗口 appearance API 或自建系统主题检测。

这不是难点，但是纯增量工作，且会失去 Web 前端现成的 i18n 生态。

**字体回退：** Linux 用 cosmic-text + font-kit（依赖 fontconfig 做字体回退），Windows 用 DirectWrite[15][16][30]；GPUI 上游有 26 个 open 的 CJK 相关 issue，例如 #64832（CJK 斜体注释重启后字形错乱）、#65973（长行 CJK 全角字符被截断），但多数是编辑器特定场景，UI 标签/短文本是否受影响需要实测[72]。

### 6.2 中文 IME

**三平台都有实现，但中文场景 bug 密集。**

- Windows：`gpui_windows` 有 `ImeContext`/`ImeContext::get`、`ImmNotifyIME`、composition 处理；issue #59636 还提到 TSF 的 `GetTextExt`，说明走的是较现代的 TSF 路径[26][67]。但 open issue 包括：#59193“Text shifts vertically when using Chinese IME on Windows”（2026-06-12）[26]、#67180“无法在输入失焦时取消 IME composition”（2026-11-19）[27]、#61724 preedit 宽度与确认文本不一致、#64850 candidate window 位置错误[69]。
- Linux/X11：使用 XIM（`gpui_linux` 的 `x11` feature 启用 `xim`）[15]；#60578 报告 Fcitx + Sogou Pinyin 在 X11 下遇到一次 `XIMClientError` 后中文输入**永久失效，直到重启 Zed**（2026-07-08 开，2026-10-12 仍更新，open）[25]。
- Linux/Wayland：有 IME/preedit 处理路径（`ImeInput::SetMarkedText`/`UnmarkText` 等）[67]，但 #65302 报告 active preedit segment 没有视觉区分[69]。
- macOS 也有中文 IME 抢键问题 #65868（Option-only keybinding 在中文 IME ASCII 模式下不触发）[29]，说明 IME 路由策略本身仍在调整。

**对本项目的实际影响：** 需要区分“编辑器级 IME”与“本产品级 IME”。Audio Share GUI 只有 Host（文本）和 Port（数字）两个输入，没有自由中文输入场景[1][7]。因此中文 IME 不会像编辑器那样直接阻塞日常使用；但：

- 任何 `InputHandler` 都要正确处理 composition、preedit、候选窗定位，GPUI 核心不提供现成输入框组件，需自己实现或使用 `gpui-kit` 的表单组件[41]；
- X11 + Fcitx 的永久失效 bug 会影响 Linux 用户输入 Host/IP 时的体验；
- 如果未来增加“服务器名称/备注”等自由文本字段，风险会上升。

**结论：** IME 是“必须真机验证的中等风险”，不是“当前就不可用”；但结合 a11y，它是 GPUI 作为中文桌面壳的两大成熟度短板之一。

## 7. Windows 后端成熟度

**已进入上游并稳定发布，但仍是“随 Zed 演进”而非独立稳定 API。**

- 时间线：2025-08-19 Zed 博客说明 Windows 最初借用 Linux 的 Vulkan 后端，因驱动缺失问题改为**自建 DirectX 11 渲染后端**，文本从 Direct2D 改为 **DirectWrite** 栅格化[30]；2025-10-15 宣布 Windows 成为“fully supported platform”，稳定版可用，DirectX 11 渲染 + DirectWrite 文本，并有全职 Windows 团队[31]。
- 当前代码：`gpui_windows` 是独立 platform crate，`window.rs` 中出现 `DirectXRenderer`、DirectComposition（`disable_direct_composition`）、DWM composition attribute、IME 位置更新等[67]。Windows 10 1803+ 自带 DirectX 11/DirectWrite，满足本项目最低系统要求；官方博客称 DX11 “Windows 7 或更高版本保证可用”[30]。
- 稳定性：Zed 1.29.3（2026-11-20）正常周更，Windows 是一等平台[31][33]。但 GPUI 的 Windows API 仍在变（例如 #66275 提到 PinchEvent、`ScrollWheelEvent` → `ScrollEvent` 重命名、platform crate 拆分尚未发布）[18]。
- 已知限制：2025-08 的博客列出的 Windows 未完成项包括键盘布局差异、SSH remoting、WSL、扩展路径和性能监控[30]；这些是编辑器功能，不一定影响本项目的简单窗口，但说明 Windows 后端的边角仍在打磨。

**结论：** Windows 后端本身可用，且比 Linux 的 GPU 要求更宽松（DX11）；真正的风险不在“Windows 能不能画窗口”，而在“GPUI API 不稳定 + 托盘/IME/a11y 的第三方/上游缺口”。

## 8. Linux 渲染、X11/Wayland 与无头/软件渲染

**Blade 已被移除，当前是 wgpu。** PR #46758 “gpui: Remove blade, reimplement linux renderer with wgpu” 于 **2026-02-13 合并**；PR 描述明确说 Blade 造成 Zed 用户和第三方 GPUI 应用的多类问题，改用 wgpu 以复用 Rust 图形生态[17][61]。上游现在有独立的 `crates/gpui_wgpu`，Linux 下 wgpu 启用 `std, metal, gles, vulkan, wgsl, webgpu` features[16]。

**X11 与 Wayland：** `gpui_linux` 默认 features 为 `["wayland", "x11"]`，两个后端都有完整窗口实现；X11 用 `x11rb`，Wayland 用 `wayland-client`/`calloop`[15]。

**是否强依赖 Vulkan：** 需要区分“编译进去的后端”和“官方支持的要求”。

- wgpu 同时编译了 `vulkan` 和 `gles`[16]，理论上存在 GLES 路径；
- 但 Zed 官方 Linux 文档仍写：“Zed requires a Vulkan compatible GPU available. It uses Vulkan internally, and a missing GPU can trigger `NoSupportedDeviceFound`.”[32]
- 因此**不能把“有 gles feature”等同于“官方支持软件 GL/无 GPU 环境”**。对 Ubuntu 24.04 的真实用户，有 Vulkan 驱动的 GPU 是事实标准；对 CI/Xvfb，必须单独验证。

**Xvfb + 软件渲染：未确认，且现有冒烟测试不能假定可复用。**

- 现有 Tauri 冒烟测试依赖 `Xvfb + dbus-run-session + LIBGL_ALWAYS_SOFTWARE=1 + WEBKIT_DISABLE_COMPOSITING_MODE=1 + WEBKIT_DISABLE_DMABUF_RENDERER=1 + GDK_BACKEND=x11`[1]；这些变量是 WebKitGTK/GL 的，不是 GPUI/wgpu 的。
- `LIBGL_ALWAYS_SOFTWARE=1` 只影响 GL/GLES；如果 wgpu 选择 Vulkan，它不会让 Vulkan 走软件驱动。无 GPU 时通常需要 Mesa 的 **lavapipe**（`mesa-vulkan-drivers`）并可能设置 `VK_ICD_FILENAMES`。这是工程推断，**未在本次来源中找到 GPUI 官方文档确认**。
- 间接证据一：Zed issue #67305 的复现环境是 “Arch Linux (Wayland live session and **X11/Xvfb harness**)”，说明上游 GPUI 的 a11y 示例确实在 Xvfb 下跑过[23]。
- 间接证据二：discussion #57066 的提案基准在 headless Weston + Pixman + **Mesa llvmpipe** 环境下运行了上游 GPUI，基线“自动选择了 llvmpipe”，并报告了 CPU 占用与帧间隔；同讨论中另有用户在 VirtualBox 软件 GPU 下报告“只有几帧每秒”。该讨论是**未合并的软件渲染提案**，不是现有能力[20]。
- **重要：** `GPUI_RENDERER=software` 这类环境变量只出现在第三方/提案讨论中；对上游 `zed` 仓库代码搜索 `GPUI_RENDERER` 命中 0[20][65]。不能把它写成现成功能。

**结论：** GPUI 在 Linux 上不再是“Blade/Vulkan 唯一”，但官方仍要求 Vulkan；Xvfb + llvmpipe/lavapipe 下能否启动并完成 `--smoke-test`，**未确认**，必须原型验证。若不能，现有 Linux CI 的冒烟测试要么改成 Wayland/headless Weston，要么增加 lavapipe 依赖，要么放弃。

## 9. 发布与稳定性

**发布渠道是本次评估最不利的一项。**

| 维度 | 现状 | 证据 |
|---|---|---|
| crates.io 官方 crate | `gpui` 0.2.2，2025-10-22；之后无新版本 | [9][18] |
| 当前生态实际使用 | `gpui-pre` 0.3.14（2026-11-23），每周快照；**publisher 是 huacnlee，不是 Zed** | [10] |
| 发布承诺 | 维护者 2026-10-30：“正在做 GPUI 自动发布，希望很快落地”；期间建议 pin git revision | [18] |
| semver/API 稳定性 | README 明确 pre-1.0，“版本之间经常有 breaking changes” | [11] |
| MSRV | crates.io 未声明 `rust_version`；Zed 仓库 pin Rust 1.100.0 stable，edition 2024 | [9][59] |
| nightly 依赖 | 无；Zed 使用 stable 1.100.0 | [59] |
| 升级频率 | 上游随 Zed 周更；`gpui-pre` 每周一版（0.3.10→0.3.14 连续五周） | [10][33] |
| 破坏性变更历史 | 近期例子：`ScrollWheelEvent` 改名 `ScrollEvent`、新增 PinchEvent、platform crate 拆分；这些在 0.2.2 中都没有 | [18] |
| 社区 fork | `gpui-ce` 0.3.0（2026-10-20），自述“mostly API compatible with upstream, but that is changing!” | [35][36] |

**这意味着什么：**

1. 如果你用 crates.io 的 `gpui = "0.2"`，你拿到的是 2025-10 的 API，落后于后续的 wgpu、AccessKit、platform crate 拆分等变化（个别能力是否已在 0.2.2 中存在未逐项核对）[9][18]。
2. 如果你用 `gpui-pre`，你拿到的是每周快照，但它是**第三方发布**，且 `gpui-kit` 用 `=0.3.14` 精确锁定、`gpui-tray` 又锁另一个 git rev/0.3.4[10][38][40]。升级时所有相关 crate 必须一起跳，任何一方滞后都会导致类型不兼容。
3. 如果你用 `gpui-ce`，你得到一个独立治理的 fork，但只有 0.3.0、约 3 万下载、API 正在与上游分叉[35][36]。此外，`zed-industries/gpui` 这个 2022 年创建、2026-11-23 仍有推送的独立仓库目前只有一句 README：“See you in a few weeks.”，不能视为已发布的稳定项目[63]。

**结论：** 不存在“稳定的 GPUI 1.0”；当前最合理的用法是锁一个快照并接受周期性升级成本。对一个要长期维护的桌面产品，这是明确的风险项。

## 10. 许可证与第三方组件

| 组件 | 许可证 | 结论 | 证据 |
|---|---|---|---|
| `gpui` / `gpui_linux` / `gpui_windows` / `gpui_wgpu` | Apache-2.0 | 与本项目 Apache-2.0 兼容，无法务阻塞 | [12][15][16] |
| Zed 仓库整体 | GPL-3.0-or-later，部分组件 Apache-2.0 | 只使用标注 Apache-2.0 的 gpui crate 即可；不要把 GPL 编辑器代码整体拷入 | [60] |
| `gpui-ce` | 官网 MIT OR Apache-2.0；crates.io/仓库 LICENSE.md 为 Apache-2.0 | 两者都兼容，但许可证声明不一致，使用前需确认 | [34][35][36] |
| `gpui-kit`（原 gpui-component） | Apache-2.0（crates.io + LICENSE-APACHE） | 兼容；GitHub 的 NOASSERTION 是识别问题，不是许可证不明 | [40][42] |
| `gpui-tray` | Apache-2.0 | 兼容 | [37][39] |
| `notify-rust` | MIT OR Apache-2.0 | 兼容；GPUI 已在 Linux 内建使用 | [15] |
| `cargo-packager` | Apache-2.0 OR MIT | 兼容 | [43][44] |
| `accesskit` | MIT OR Apache-2.0 | 兼容 | [56] |

**第三方组件成熟度：**

- `gpui-kit`：21,265 stars、最近 6 个版本（0.8.0→0.11.0，约每周一版）、75+ 组件、AccessKit、跨平台、WebAssembly；仓库 2026-11-24 仍有提交[41][42]。这是 GPUI 生态里最成熟的 UI 组件库，但仓库已从 `longbridge/gpui-component` 改名为 `longbridge/gpui-kit`，且 pin `gpui-pre =0.3.14`[40][41]。使用它意味着接受 Longbridge 的快照节奏。
- `gpui-tray`：20 stars、4.5k 下载、2026-08-29 创建、last push 2026-10-21；功能设计正确，但 bus factor 和长期维护风险高[37][39]。
- `gpui-ce`：1,676 stars、30k 下载、0.3.0、pushed 2026-11-23；社区治理，明确“not affiliated with or endorsed by Zed”[34][35][36]。

**许可证结论：** 没有 Slint 那种 GPL/商业/royalty-free 的法务门槛；GPUI 路线在许可证上可行。真正的风险是维护活跃度和 API 稳定性，不是许可证。

## 11. 其他系统集成与性能

| 能力 | GPUI 现状 | 证据 |
|---|---|---|
| 打开浏览器 | 平台 trait 内建 `open_url`；Zed Linux 文档说链接点击走 XDG `org.freedesktop.portal.OpenURI` | [13][32] |
| 文件对话框 | 平台 trait 内建 `prompt_for_paths` / `prompt_for_new_path` + `PathPromptOptions`；Zed Linux 用 `org.freedesktop.portal.FileChooser` | [13][32] |
| 开机自启 | 无 GPUI API，但现有 Rust `autostart.rs`（Windows HKCU Run / XDG autostart）可直接复用；与 UI 技术无关 | [1][6] |
| 更新检查 | 现有 `update.rs`（GitHub releases/latest + 版本比较）可复用；通知用 GPUI 内建 | [1][6][13] |
| 单实例 | 无官方；见第 3 节 | [47][48][49] |
| 托盘 | 无官方；见第 1 节 | [37][38] |
| 主题（System/Light/Dark） | 无现成跨平台外观 API 的明确文档；需自建系统主题检测 + GPUI 主题变量，或使用 `gpui-kit` 主题 | [41] |
| QR 显示 | 现有 `qrSvg` 由 Rust 生成、前端用 `{@html}` 插入[7]；GPUI 有 SVG/图像渲染示例（`examples/svg`），可改为 GPUI image/SVG 元素 | [11][64] |
| 启动时间 | **未确认**；没有找到 GPUI 官方或可信的冷启动对比数据 | — |
| 包体大小 | **无本项目数据**。可参考的只有 Zed 自身发布物：Windows x86_64 `.exe` 65.4 MB、Linux x86_64 `.tar.gz` 91.5 MB（含编辑器资源，不等于最小 GPUI 应用） | [33] |
| Windows CRT | 与普通 Rust/MSVC 应用相同；无 GPUI 特有结论 | — |

**性能直觉（非实测）：** GPUI 是 GPU 渲染，理论上比 WebView 更省内存、启动更快；但 Zed 的发布物并不小，且 wgpu/DirectX 初始化、字体加载、shader 编译都有固定成本。没有本项目的最小原型数据，**不应把“更快更小”当作迁移理由**。

## 12. Tauri 侧的真实痛点（公平对比）

这一节只列有证据的痛点，并标出严重程度。

### 12.1 WebView2 运行时依赖与 preflight

- Windows 10 1803+ 已自带 WebView2；Windows 11 也自带；少数设备可能缺失，需要安装 Evergreen Runtime[50][51]。
- 缺失时 Tauri 默认的行为不够友好，所以本项目写了 254 行 `webview2_check.rs`：在 `tauri::Builder` 之前读 HKLM/HKCU 的 `pv` 注册表值或 `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER`，缺失时弹 Yes/No 对话框并可打开 Microsoft 下载页，然后干净退出[5]。
- Tauri 配置还启用了 `webviewInstallMode: downloadBootstrapper`[3]；Microsoft 文档说明在线 bootstrapper 约 2 MB，离线 standalone 安装器可用，fixed-version 运行时超过 250 MB[51]。
- **严重程度：中低。** 这是真实的代码/测试成本，但只影响少数缺运行时的机器；Win10 1803+ 目标下大部分用户无感。

### 12.2 WebKitGTK 依赖与 Linux 渲染问题

- Tauri Linux 依赖 `libwebkit2gtk-4.1-dev`/`webkit2gtk-4.1`、GTK3、libsoup3 等[50]；本项目 deb 的 `Depends` 明确包含 `libwebkit2gtk-4.1-0, libgtk-3-0, libayatana-appindicator3-1, libpipewire-0.3-0`[8]。
- WebKitGTK 的版本由发行版决定，Tauri 官方文档承认“很难准确整理各发行版的 WebKitGTK 信息”，并建议按最老的受支持组合测试；WebKitGTK 2.36 约等于 Safari 16，老发行版更旧[54]。
- Linux 图形问题主要出现在 NVIDIA：WebKitGTK 的 DMABUF renderer 请求驱动不支持的 buffer format，导致白屏、闪烁、resize 崩溃；官方 workaround 依次是 `__NV_DISABLE_EXPLICIT_SYNC=1`、`WEBKIT_DISABLE_DMABUF_RENDERER=1`、`WEBKIT_DISABLE_COMPOSITING_MODE=1`[53]。
- 本项目在 Xvfb 下还必须加 `WEBKIT_DISABLE_COMPOSITING_MODE=1`、`WEBKIT_DISABLE_DMABUF_RENDERER=1`、`LIBGL_ALWAYS_SOFTWARE=1`、`GDK_BACKEND=x11`[1]。
- **严重程度：中。** 这些是已经付过的成本；迁移到 GPUI 会消掉 WebKitGTK，但会引入“Vulkan/llvmpipe + 无头”的新成本，未必更省。

### 12.3 包体大小

本地观测（release 构建，Linux；bundle 产物的版本标签是 `0.5.0-beta`，不是当前 0.6.0，但量级可参考）：

- `target/release/AudioShareServer`：26 MB；
- `.deb`：7.6 MB；
- `.AppImage`：83 MB（AppImage 打包策略会把更多运行时依赖放进镜像，所以明显更大；具体包含的库未逐项核对）；
- 前端产物：`gui/dist` 的 index JS 100 KB、CSS 13 KB（未压缩）[7]。

**解读：** Tauri 的“应用本身”很小，因为它复用系统 WebView；代价是系统必须有 WebView/WebKitGTK。GPUI 会把渲染栈静态链进二进制，包体推测介于 26 MB 和 Zed 的 65 MB 之间（工程推测，未实测）。**不能用“Tauri 包体大”作为迁移理由**；Linux 的 AppImage 大是打包策略，不是 Tauri 独有。

### 12.4 跨平台渲染一致性

- Windows 是 WebView2（Chromium/Edge），Linux 是 WebKitGTK；两者不是同一引擎[50][54]。
- Tauri 官方文档明确说 WebKitGTK 版本随发行版变化，CSS/Web 平台特性支持要按最老目标测试[54]。
- 本项目代码里已经有平台分支（`LinuxSettings.svelte` 50 行）和 Linux 专用设置[7]，说明这种差异是实际存在的维护成本。
- **严重程度：中。** 但对一个 420x640、以表单/状态展示为主的界面，CSS 差异可控；比“换 UI 框架重写全部界面”便宜。

### 12.5 Svelte/Rust 双语言成本

- 前端 `gui/src` 共 2,783 行 Svelte/TS/CSS（另有 locale 310 行），Rust 壳 6,071 行；`ipc.ts` 222 行手写命令/事件类型[6][7]。
- 双语言意味着：前端构建（Vite/Svelte）、Rust 构建（Cargo/Tauri）、IPC 契约、两套依赖升级、两套测试。这是真实成本。
- 但迁到 GPUI 不是“消灭双语言”那么简单：你仍然要写 Rust UI 代码，且失去 Web 生态的现成组件（表单、下拉、对话框、滚动、无障碍语义），需要自己实现或依赖 `gpui-kit`。**净收益取决于 UI 复杂度**；本项目 UI 不复杂，收益有限。

### 12.6 Tauri 仍然占优的“桌面壳”能力

- 托盘：官方 `tray-icon` feature，菜单/事件齐全；Linux 点击事件不发送是已知限制，但右键菜单可用[52]。
- 通知：官方 plugin[2]。
- 单实例：官方 plugin，一行接入[2]。
- 打包：官方 bundler 直接出 deb/AppImage/NSIS/MSI，处理 WebView2 安装模式、图标、桌面文件[3][50]。
- 更新：官方 updater plugin（本项目目前是自研 GitHub 检查，未用 updater）[1]。

这些正是 GPUI 缺的。ADR 0002 的判断——“Tauri 有最成熟的打包和托盘 story”——在 2026-11-24 依然成立[4]。

## 13. 对 Audio Share 的具体影响

### 13.1 功能映射：可直接实现 / 必须自研或降级

| 现有功能 | GPUI 可行性 | 说明 |
|---|---|---|
| 420x640 单窗口、居中、可调整大小 | 直接实现 | GPUI WindowOptions 支持尺寸/位置/可调整；无风险[11] |
| `--hide` 启动隐藏、关闭到托盘 | 直接实现 + 自研托盘 | 窗口 show/hide 是平台 API；托盘见下 |
| 托盘 Show/Start/Stop/Exit | **需第三方或自研** | `gpui-tray` 可用但年轻、锁 git rev；Linux SNI host 依赖；Windows 左键 toggle 需实测[37][38] |
| Windows 左键切换窗口 | **需实测** | Tauri 现成[1]；GPUI 需在 `on_activate` 里实现并验证[37] |
| 原生通知（更新、错误） | 直接实现 | 内建 `show_system_notification`，带 tag/actions/response[13] |
| 单实例 | **必须自研** | 无官方；Zed 的 mutex+pipe 可参考；或 `interprocess`[47][48][49] |
| 开机自启（HKCU Run / XDG autostart） | 直接复用 | `autostart.rs` 与 UI 无关[1][6] |
| 更新检查（GitHub releases/latest） | 直接复用 | `update.rs` 可复用；通知改调 GPUI[6][13] |
| i18n（en-US/zh-CN、系统语言检测） | 复用 Rust 侧 + 迁移前端文案 | 无内置框架；`i18n.rs`/`strings.rs` 可用，前端 `t()` 要重写[6][28] |
| 主题 System/Light/Dark | 自研 | 需系统外观检测 + GPUI 主题；`gpui-kit` 有主题系统可选[41] |
| 配对对话框 / QR | 基本直接实现 | `qrSvg` 由 Rust 生成[7]；GPUI 有 SVG/图像渲染[11][64] |
| Windows 防火墙修复 | 直接复用 | `windows_maintenance.rs` 与 UI 无关[6] |
| WebView2 preflight | **删除** | 迁移后不需要[5] |
| 文件对话框 / 打开浏览器 | 直接实现 | GPUI 平台 trait 内建[13] |
| `--smoke-test`（Xvfb） | **未确认** | 见第 8 节；可能需要 lavapipe/Wayland headless，不能假定复用[1][23] |
| 日志（server.log、tracing bridge） | 直接复用 | `logging.rs`/`tracing_bridge.rs`[6] |

**需要降级/放弃的项：** 如果不在 `gpui-tray` 上投入，Linux 托盘左键行为可能不如 Tauri（Tauri 本来也不支持 Linux click，只有右键菜单，所以并非净损失）；a11y 质量会下降（从 HTML 语义到需要显式标注的 AccessKit 树）；IME 在 X11/Fcitx 下可能不如 WebView 稳定。

### 13.2 迁移工作量估计

**可以复用的 Rust（约 5.0k / 6.07k 行；其中 614 行 maintenance 需改触发入口）：**

| 模块 | 行数 | 可复用性 |
|---|---:|---|
| `controller.rs` | 1,902 | 完全复用；与 Tauri 无关，只依赖 core/v2 runtime[6] |
| `model.rs` | 816 | 完全复用[6] |
| `update.rs` | 430 | 完全复用；通知调用点改为 GPUI[6] |
| `maintenance.rs` + `windows_maintenance.rs` | 614 | 平台代码复用，触发入口重写[6] |
| `autostart.rs` | 290 | 完全复用[6] |
| `logging.rs` + `tracing_bridge.rs` | 403 | 完全复用[6] |
| `i18n.rs` + `strings.rs` | 278 | 复用，文案来源调整[6] |
| `help.rs` + `v2.rs` + `options.rs` + `lib.rs` | 310 | 基本复用[6] |
| **小计** | **约 5,043** | 约 5.0k 可复用；614 行 maintenance 需改触发入口，其余基本直接复用 |

**必须重写：**

- `main.rs` 774 行：Tauri builder、commands、tray、窗口事件、plugin 初始化、single-instance 接线全部换成 GPUI/自研[2][6]；
- `webview2_check.rs` 254 行：删除[5]；
- 前端 2,783 行（Svelte/TS/CSS）+ 310 行 locale：全部重写为 GPUI 视图/样式/文案；
- `ipc.ts` 222 行：消失，改为直接 Rust 调用（这是净简化）；
- 打包/CI：重建 deb/AppImage/Windows portable exe 流程；增加 Windows 签名步骤（如有）。

**粗估（工程判断，非实测）：**

- 1 名熟悉 Rust 且学习过 GPUI 的开发者：**6-10 周**达到功能对等（窗口、托盘、通知、设置、配对、更新、i18n、打包），不含上游 bug 绕行；
- Linux 无头/软件渲染、Windows 托盘/IME、a11y 的真机验证与修复：**额外 2-4 周**；
- 如果依赖 `gpui-pre`/`gpui-kit`，每次升级的回归成本另计（预计每季度数天）。

这个估计的关键变量不是代码量，而是**上游缺口**：托盘和单实例是自研，a11y/IME 是“上游修了才能好”，Xvfb 是“可能根本不工作”。如果原型验证失败，工期没有下限。

### 13.3 对 ADR 0002 的影响

ADR 0002 选择 Tauri 的两条明确理由[4]：

1. “Tauri has the most mature packaging and tray story”；
2. 音频路径用 Rust（`wasapi`/PipeWire）与 in-process core。

第 2 条与 UI 框架无关，换成 GPUI 仍然成立（Rust core 不变）。第 1 条在 GPUI 上**直接不成立**：GPUI 没有官方托盘、没有官方 installer bundler，单实例也没有官方支持；托盘只能靠 20 stars 的第三方 crate，打包只能靠近一年未发版的 `cargo-packager` 或 experimental 的 `cargo-bundle`[37][39][43][44][45][46]。因此：

- **若坚持迁移，必须新写一份 ADR 取代 0002**，明确记录“接受 GPUI 的 pre-1.0 API、第三方托盘、自建打包、a11y/IME 风险”，并给出原型验收证据；不能只在 0002 上打补丁。
- **当前建议：ADR 0002 继续有效。** 本次调研没有发现足以推翻它的新证据；相反，GPUI 的短板正好落在 0002 最看重的两项上。
- 可以在未来的 ADR/issue 中记录重新评估触发条件（见摘要），但本次任务不修改其他文件。

## 14. 验证建议

以下结论不能靠阅读源码定案，必须用 `/prototype` 或真机验证。证据标签按仓库 `docs/agents/evidence-policy.md` 使用 PASS/FAIL/NOT RUN/EMULATOR。

### 14.1 必须先验证的五个问题

1. **Linux 无头/软件渲染（最高优先级）。**
   - 在 Ubuntu 24.04 + Xvfb + `dbus-run-session` 下，先跑 `vulkaninfo --summary`，确认是否存在 llvmpipe/lavapipe 设备；再跑一个最小 GPUI 窗口（或 `cargo run -p gpui --example window`，以当时 tag 为准）。
   - 分别测试：仅 `LIBGL_ALWAYS_SOFTWARE=1`；安装 `mesa-vulkan-drivers` 后；设置 `VK_ICD_FILENAMES` 指向 lavapipe 后。
   - 验收：窗口能创建、首帧渲染、`--smoke-test` 能在 4 秒内退出；记录启动时间与 CPU。若失败，现有 CI 冒烟测试不能保留，需改 Wayland headless Weston 或放弃无头测试。
   - 当前状态：**NOT RUN / 未确认**；间接证据仅到“Xvfb harness 跑过 a11y 示例”和“Weston+llvmpipe 能跑；VirtualBox 软件 GPU 下报告几帧每秒”[20][23]。

2. **Windows 托盘与左键行为。**
   - 用 `gpui-tray` 做 Show/Start/Stop/Exit 菜单，验证 Windows 10/11 下左键切换窗口、右键菜单、图标 DPI、Explorer 重启后的图标恢复。
   - 验收：与当前 Tauri 行为一致；否则记录差异。

3. **中文 IME。**
   - Windows 10/11：微软拼音 + 微信输入法，验证 Host/Port 输入框的 composition、候选窗、失焦取消、Tab/Enter 行为；覆盖 #59193/#67180 场景[26][27]。
   - Ubuntu 24.04：Wayland + Fcitx5、X11 + Fcitx5（Xvfb 下可只测启动），验证中文输入、preedit、候选窗、长时间运行后是否失效；覆盖 #60578[25]。
   - 验收：无“输入永久失效”；候选窗位置可接受；否则记录为 FAIL。

4. **无障碍。**
   - Windows：NVDA/JAWS 打开最小 GPUI 窗口，检查按钮/输入框/标签是否可读、可操作；由于 #41138 早于 AccessKit，必须重新测，不能直接引用旧结论[21][22]。
   - Linux：Orca + AT-SPI，验证静态文本、输入框、焦点顺序；预计会复现 #67305/#67304[23][24]。
   - 验收：至少输入框和主要按钮可读；若达不到，需明确产品是否接受。

5. **打包与分发。**
   - 用 `cargo-packager` 出 deb、AppImage、Windows NSIS/MSI；用裸 `cargo build --release` 出 Windows portable exe；验证图标、桌面文件、依赖、安装/卸载、签名。
   - 验收：Ubuntu 24.04 上安装即用（无 WebKitGTK 依赖）；Windows 10 1803+ 便携 exe 可运行；包体与启动时间记录在案。

### 14.2 原型范围建议

如果决定做 `/prototype`，建议把范围压到最小，只验证上面五个问题：

- 单窗口 420x640，Show/Hide；
- 托盘四项菜单 + Windows 左键 toggle；
- 一个 Host 文本输入 + 一个 Port 数字输入（用于 IME 测试）；
- 一条原生通知；
- 单实例（第二个实例唤出第一个窗口）；
- 打包 deb/AppImage/portable exe。

**不要**在原型阶段重写配对、设置、更新、i18n 全量 UI；这些是“已知可做”的部分，先把上游缺口测出来。原型的验收结果应直接决定是否进入正式迁移，而不是先写 2.8k 行 UI 再发现 Xvfb/托盘/IME 不满足。

## 15. 参考来源

> 检索日期均为 2026-11-24。本地文件按仓库相对路径列出；外部页面注明版本/日期（若页面提供）。GitHub API 数据通过已认证的 `gh` 获取。

### 本地代码与仓库

1. `gui/README.md`，Audio Share Server GUI 的功能、构建、调试参数与 Xvfb 冒烟命令。
2. `gui/src-tauri/Cargo.toml`，Tauri 2.12.5、tray-icon、single-instance、notification、opener 依赖。
3. `gui/src-tauri/tauri.conf.json`，窗口 420x640、bundle 图标、`webviewInstallMode: downloadBootstrapper`。
4. `docs/adr/0002-server-rewrite-stack.md`，选择 Tauri 的理由与拒绝 Slint/Go 的理由。
5. `gui/src-tauri/src/webview2_check.rs`，254 行 WebView2 preflight（注册表检测 + 原生对话框）。
6. `gui/src-tauri/src/{controller,model,update,autostart,i18n,logging,strings,maintenance,windows_maintenance,options,help,v2,tracing_bridge,lib}.rs`，Rust 壳实现与行数。
7. 本地观测：`target/release/AudioShareServer` 26 MB、`target/release/bundle/deb/*.deb` 7.6 MB、`target/release/bundle/appimage/*.AppImage` 83 MB（产物标签 0.5.0-beta）、`gui/dist` index JS 100 KB/CSS 13 KB；`gui/src/views/PairingDialog.svelte` 的 `qrSvg`/`qrUri` 渲染、`gui/src/views/ServerView.svelte` 的 Host/Port 输入、`gui/src/lib/theme.ts` 的 `prefers-color-scheme` 检测。
8. `docs/server-rewrite/build-linux.md`，deb `Depends: libpipewire-0.3-0, libayatana-appindicator3-1, libwebkit2gtk-4.1-0, libgtk-3-0` 与构建依赖。

### GPUI / Zed

9. crates.io, `gpui` crate（0.2.2，2025-10-22；Apache-2.0；无 `rust_version`）. https://crates.io/crates/gpui
10. crates.io API, `gpui-pre`（0.3.14，2026-11-23；owner/publisher `huacnlee`；description “gpui-pre snapshot of zed@f126950”）. https://crates.io/api/v1/crates/gpui-pre
11. GitHub, zed-industries/zed, `crates/gpui/README.md`（hybrid immediate/retained、pre-1.0、Windows Win32/DirectWrite、Linux wayland/x11 features、无 tray）. https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md
12. GitHub, zed-industries/zed, `crates/gpui/Cargo.toml`（`license = "Apache-2.0"`、default features、accesskit 依赖）. https://github.com/zed-industries/zed/blob/main/crates/gpui/Cargo.toml
13. GitHub, zed-industries/zed, `crates/gpui/src/platform.rs`（`show_system_notification`、`SystemNotification`、`prompt_for_paths`、`open_url`）. https://github.com/zed-industries/zed/blob/main/crates/gpui/src/platform.rs
14. GitHub, zed-industries/zed, `crates/gpui/src/window/a11y.rs`（AccessKit 架构：macOS/Windows/Linux D-Bus）. https://github.com/zed-industries/zed/blob/main/crates/gpui/src/window/a11y.rs
15. GitHub, zed-industries/zed, `crates/gpui_linux/Cargo.toml`（`accesskit_unix`、`notify-rust`、`xim`、wayland/x11 features）. https://github.com/zed-industries/zed/blob/main/crates/gpui_linux/Cargo.toml
16. GitHub, zed-industries/zed, `crates/gpui_wgpu/Cargo.toml`（Linux wgpu features：`std, metal, gles, vulkan, wgsl, webgpu`）. https://github.com/zed-industries/zed/blob/main/crates/gpui_wgpu/Cargo.toml
17. GitHub, zed-industries/zed, PR #46758 “gpui: Remove blade, reimplement linux renderer with wgpu”，merged 2026-02-13. https://github.com/zed-industries/zed/pull/46758
18. GitHub, zed-industries/zed, Issue #66275 “gpui: publish a new crates.io release (0.2.2 predates PinchEvent)”，2026-10-29 开、2026-10-30 closed as not planned；维护者称自动发布“很快落地”，期间建议 pin git revision. https://github.com/zed-industries/zed/issues/66275
19. GitHub, zed-industries/zed, Discussion #55271 “Is it time to publish a new version of GPUI on crates.io?”，2026-04-30 开、2026-10-30 更新. https://github.com/zed-industries/zed/discussions/55271
20. GitHub, zed-industries/zed, Discussion #57066 “Improve usability with software-emulated GPUs in virtualized environments”，2026-05-17 开；提案含 `gpui_software`、WARP、llvmpipe、Weston headless；明确为未合并提案. https://github.com/zed-industries/zed/discussions/57066
21. GitHub, zed-industries/zed, Issue #41138 “Windows: Screen reader accessibility missing completely”，2025-10-24 开，仍 open. https://github.com/zed-industries/zed/issues/41138
22. GitHub, zed-industries/zed, PR #56065 “gpui: Accesskit support”，created 2026-05-07，merged 2026-05-27. https://github.com/zed-industries/zed/pull/56065
23. GitHub, zed-industries/zed, Issue #67305 “gpui_linux: static text exposes no AT-SPI Text interface…”，2026-11-21 开，复现环境含 X11/Xvfb harness. https://github.com/zed-industries/zed/issues/67305
24. GitHub, zed-industries/zed, Issue #67304 “gpui_linux: accessible window frame has no bounds…”，2026-11-21 开. https://github.com/zed-industries/zed/issues/67304
25. GitHub, zed-industries/zed, Issue #60578 “X11: IME permanently breaks after XIMClientError (no reconnect)”，2026-07-08 开、2026-10-12 更新，Fcitx + Sogou Pinyin. https://github.com/zed-industries/zed/issues/60578
26. GitHub, zed-industries/zed, Issue #59193 “Text shifts vertically when using Chinese IME on Windows”，2026-06-12 开. https://github.com/zed-industries/zed/issues/59193
27. GitHub, zed-industries/zed, Issue #67180 “Windows: no way to cancel an active IME composition when an input loses focus”，2026-11-19 开. https://github.com/zed-industries/zed/issues/67180
28. GitHub, zed-industries/zed, Issue #7409 “Localization and support for different locales”，2024-02-05 开，仍 open. https://github.com/zed-industries/zed/issues/7409
29. GitHub, zed-industries/zed, Issue #65868 “Option-only keybindings don't fire when a Chinese IME is active in ASCII mode”，2026-10-20 开. https://github.com/zed-industries/zed/issues/65868
30. Zed Blog, “Zed for Windows: What's Taking So Long?!”，2025-08-19，Max Brunsfeld；DirectX 11 后端、DirectWrite 文本、Windows 未完成项. https://zed.dev/blog/windows-progress-report
31. Zed Blog, “Windows When? Windows Now”，2025-10-15，Max Brunsfeld；Windows stable、fully supported、DirectX 11 + DirectWrite、全职 Windows 团队. https://zed.dev/blog/zed-for-windows-is-here
32. Zed Docs, “Linux”（tar.gz/Flatpak、glibc、Vulkan requirement、XDG portals）. https://zed.dev/docs/linux
33. GitHub, zed-industries/zed release `v1.29.3`（2026-11-20）及资产大小：`Zed-x86_64.exe` 65.4 MB、`zed-linux-x86_64.tar.gz` 91.5 MB. https://github.com/zed-industries/zed/releases/tag/v1.29.3
34. GPUI Community Edition 官网（fork、MIT OR Apache-2.0、独立治理、跨平台、AccessKit）. https://gpui-ce.github.io
35. GitHub, gpui-ce/gpui-ce README（“mostly API compatible with upstream, but that is changing!”）. https://github.com/gpui-ce/gpui-ce
36. crates.io, `gpui-ce` 0.3.0（2026-10-20，license `Apache-2.0`，约 30k downloads；官网写 MIT OR Apache-2.0，存在声明不一致）. https://crates.io/crates/gpui-ce
37. GitHub, domenkozar/gpui-tray README（Windows `Shell_NotifyIconW`、Linux StatusNotifierItem/DBusMenu via zbus、GPUI foreground executor、`on_activate`）. https://github.com/domenkozar/gpui-tray
38. GitHub, domenkozar/gpui-tray `Cargo.toml`（default `gpui = { version = "0.2.2", git = "...", rev = "8166e3d..." }`；`gpui-pre 0.3.4`；`gpui-ce 0.3`）. https://github.com/domenkozar/gpui-tray/blob/main/Cargo.toml
39. crates.io, `gpui-tray` 0.3.0（2026-10-21，Apache-2.0，4,532 downloads）；GitHub repo 2026-08-29 创建、20 stars、1 open issue. https://crates.io/crates/gpui-tray
40. GitHub, longbridge/gpui-kit `Cargo.toml`（`gpui = { package = "gpui-pre", version = "=0.3.14" }`）. https://github.com/longbridge/gpui-kit/blob/main/Cargo.toml
41. GitHub, longbridge/gpui-kit README（75+ components、AccessKit、WebAssembly、跨平台、主题/数据表/编辑器）. https://github.com/longbridge/gpui-kit
42. crates.io, `gpui-kit` 0.11.0（2026-11-23，Apache-2.0）与 `gpui-component` 0.11.0；repo 21,265 stars. https://crates.io/crates/gpui-kit
43. GitHub, crabnebula-dev/cargo-packager README（Linux deb/AppImage/Pacman；Windows NSIS/MSI；`beforePackagingCommand`）. https://github.com/crabnebula-dev/cargo-packager
44. crates.io, `cargo-packager` 0.11.8（2025-11-27）；仓库 pushed 2026-11-23. https://crates.io/crates/cargo-packager
45. GitHub, burtonageo/cargo-bundle README（deb/AppImage/MSI；iOS/Windows experimental）. https://github.com/burtonageo/cargo-bundle
46. crates.io, `cargo-bundle` 0.12.0（2026-09-20）. https://crates.io/crates/cargo-bundle
47. crates.io, `single-instance` 0.3.3（2021-12-16，MIT）. https://crates.io/crates/single-instance
48. crates.io, `interprocess` 2.4.9（2026-11-18，0BSD OR Apache-2.0）. https://crates.io/crates/interprocess
49. GitHub, zed-industries/zed, `crates/zed/src/zed/windows_only_instance.rs`（CreateMutexW + 命名 pipe）. https://github.com/zed-industries/zed/blob/main/crates/zed/src/zed/windows_only_instance.rs
50. Tauri v2 Docs, “Prerequisites”（Windows WebView2/Windows 10 1803+；Linux `libwebkit2gtk-4.1-dev`、GTK3 等）. https://v2.tauri.app/start/prerequisites/
51. Microsoft Learn, “Distribute your app and the WebView2 Runtime”，updated 2026-10-02（Evergreen、bootstrapper ~2 MB、fixed version >250 MB、Win10 1803+/Win11 自带）. https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution
52. Tauri v2 Docs, “System Tray”（Linux 事件不发送，仅右键菜单；`tray-icon` feature）. https://v2.tauri.app/learn/system-tray/
53. Tauri v2 Docs, “Linux Graphics Issues”（WebKitGTK DMABUF、NVIDIA、`WEBKIT_DISABLE_DMABUF_RENDERER`、`WEBKIT_DISABLE_COMPOSITING_MODE`）. https://v2.tauri.app/develop/debug/linux-graphics
54. Tauri v2 Docs, “Webview Versions”（WebView2/Chromium；WebKitGTK 随发行版变化；按最老组合测试）. https://v2.tauri.app/reference/webview-versions
55. crates.io, `wgpu` 30.0.2（2026-11-12）. https://crates.io/crates/wgpu
56. crates.io, `accesskit` 0.25.2（2026-10-25）. https://crates.io/crates/accesskit
57. crates.io, `tauri` 2.12.7（2026-11-18）；项目使用 2.12.5. https://crates.io/crates/tauri
58. npm, `svelte` 5.59.1（2026-11-20）. https://www.npmjs.com/package/svelte
59. GitHub, zed-industries/zed, `rust-toolchain.toml`（channel 1.100.0，profile minimal）与 workspace `edition = "2024"`. https://github.com/zed-industries/zed/blob/main/rust-toolchain.toml
60. GitHub, zed-industries/zed README（“Zed source code is licensed primarily under GPL-3.0-or-later, with Apache-2.0 components where marked”）与 `LICENSE-GPL`/`LICENSE-APACHE`. https://github.com/zed-industries/zed
61. Hacker News, item 47002825 “Zed editor switching graphics lib from blade to wgpu”，链接 PR #46758（社区讨论，2026-02）. https://news.ycombinator.com/item?id=47002825
62. Hacker News, comment 47003569（转述 Discord 声明：“GPUI development is getting some major brakes put on it… pushing off anything that isn't directly related to Zed's use case”；**社区转述，非官方公告**）. https://news.ycombinator.com/item?id=47003569
63. GitHub, zed-industries/gpui（2022-06-20 创建、2026-11-23 推送；目前只有 README：“See you in a few weeks.”）. https://github.com/zed-industries/gpui
64. GitHub, zed-industries/zed, `crates/gpui/examples/system_notifications.rs` 与 `crates/gpui/examples/svg/svg.rs`（通知与 SVG 示例）. https://github.com/zed-industries/zed/tree/main/crates/gpui/examples
65. GitHub 代码搜索（zed-industries/zed）：`SystemTray` 命中 0；`tray` 在 `crates/gpui*` 无托盘实现；`GPUI_RENDERER` 命中 0；`i18n` 在 `crates` 命中 0. https://github.com/search?q=repo%3Azed-industries%2Fzed+SystemTray&type=code
66. GitHub, zed-industries/zed, `crates/gpui_windows/src/system_notifications.rs`. https://github.com/zed-industries/zed/blob/main/crates/gpui_windows/src/system_notifications.rs
67. GitHub, zed-industries/zed, `crates/gpui_windows/src/window.rs`（DirectXRenderer、DirectComposition、IME position）与 `crates/gpui_linux/src/linux/{x11,wayland}/window.rs`（IME 处理）. https://github.com/zed-industries/zed/blob/main/crates/gpui_windows/src/window.rs
68. GitHub, zed-industries/zed, Issue #66729 “gpui_linux (X11): AccessKit never gets the window's screen position…”，2026-11-09 开. https://github.com/zed-industries/zed/issues/66729
69. GitHub, zed-industries/zed, Issue #65302 “Wayland: active IME preedit segment is not visually distinguished”，2026-10-08 开；Issue #61724 Windows IME preedit 宽度不一致，2026-07-27 开；Issue #64850 IME candidate window 位置错误，2026-09-28 开. https://github.com/zed-industries/zed/issues/65302
70. GitHub, zed-industries/zed, workspace `Cargo.toml`（`accesskit = "0.24.0"`、`accesskit_unix = "0.22"`、`accesskit_windows = "0.34"`）. https://github.com/zed-industries/zed/blob/main/Cargo.toml
71. GitHub, tauri-apps/tray-icon README（Windows/AppIndicator 要求同线程事件循环；Linux 默认 GTK3 + libappindicator；`ksni` 后端可避开 GTK）；crates.io `tray-icon` 0.26.1（2026-10-07）. https://github.com/tauri-apps/tray-icon
72. GitHub, zed-industries/zed, CJK 相关 open issue 搜索（26 条；示例 #64832、#65973）. https://github.com/zed-industries/zed/issues?q=is%3Aissue+is%3Aopen+CJK
