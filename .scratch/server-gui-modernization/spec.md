# Server GUI 现代化

Status: resolved

## Problem Statement

当前 Server GUI 的功能已经完整，但主窗口重复展示运行状态，Server 配置同时出现在主窗口与 Settings 中，视觉仍停留在单一浅色主题、传统表单和字符图标。用户希望在不改变核心音频流程的前提下，获得更现代、更清晰、更克制的桌面界面，并能在浅色与深色环境中舒适使用。

## Solution

采用用户通过交互原型选定的 **Variant C「极简专注」** 方向，重新实现正式 Server GUI：

- 主窗口以唯一的运行状态和 Start/Stop 主操作为视觉中心。
- Endpoint 与 Encoding 作为快速设置保持可见；Host 与 Port 收纳到可展开的 Network configuration 区域。
- Stop 使用用户选定的克制危险样式，而非整块高饱和红色。
- Settings 不再重复 Server 字段，改为 Appearance、Startup & window、Updates、System 与 About 等分组展开区域。
- 提供 System、Light、Dark 三种主题选择，默认跟随系统并持久化用户选择。
- 保留全部现有 Server 行为、应用设置、托盘、更新、国际化和平台维护能力。

## User Stories

1. 作为 Server GUI 用户，我希望首屏只出现一次明确的 Server 状态，以便立即判断 Server 是否运行。
2. 作为 Server GUI 用户，我希望状态下方始终显示监听地址，以便确认 Android Client 应连接的地址。
3. 作为 Server GUI 用户，我希望 Start 是停止状态下最明确的操作，以便快速开始分享音频。
4. 作为 Server GUI 用户，我希望 Stop 清楚可辨但不过度刺眼，以便结束 Server 而不让界面持续呈现警报感。
5. 作为 Server GUI 用户，我希望 Starting 与 Stopping 有明确过渡反馈并阻止重复操作，以便理解异步命令正在执行。
6. 作为 Server GUI 用户，我希望通过托盘或其他入口改变状态后窗口同步更新，以便始终看到真实状态。
7. 作为 Server GUI 用户，我希望 Endpoint 使用真实枚举结果并保留默认项，以便选择捕获来源。
8. 作为 Server GUI 用户，我希望 Encoding 保留现有六项选择，以便维持既有 Capture Format 行为。
9. 作为 Server GUI 用户，我希望 Endpoint 与 Encoding 无需展开即可调整，以便完成最常用的配置。
10. 作为 Server GUI 用户，我希望 Host 与 Port 位于清晰命名的 Network configuration 区域，以便在需要时找到网络设置。
11. 作为 Server GUI 用户，我希望 Network configuration 默认折叠且展开状态易于识别，以便保持首屏专注。
12. 作为 Server GUI 用户，我希望 Server 运行时所有 Server 字段都被锁定，以便避免修改不会立即生效的配置。
13. 作为 Server GUI 用户，我希望停止后可以编辑并自动保存 Server 字段，以便保留现有配置流程。
14. 作为 Server GUI 用户，我希望 Reset 恢复 Host、Port、Endpoint 与 Encoding 的默认值，同时保留应用设置，以便只重置 Server 配置。
15. 作为 Server GUI 用户，我希望无效输入、启动失败和保存失败仍显示可读错误，以便诊断问题而不使布局崩溃。
16. 作为 Server GUI 用户，我希望 Settings 不再重复 Host、Port、Endpoint 与 Encoding，以便理解主窗口负责 Server、Settings 负责应用。
17. 作为 Server GUI 用户，我希望选择 System 主题后随操作系统明暗变化，以便界面与桌面环境一致。
18. 作为 Server GUI 用户，我希望可以手动固定 Light 或 Dark，以便覆盖系统主题。
19. 作为 Server GUI 用户，我希望主题选择跨重启保留，以便不必每次重新设置。
20. 作为 Server GUI 用户，我希望首次启动和切换主题时不会出现明显的错误主题闪屏，以便获得稳定的视觉体验。
21. 作为 Server GUI 用户，我希望两种主题中的文本、控件、状态色与焦点指示都清晰可读，以便在不同环境下操作。
22. 作为 Server GUI 用户，我希望启动登录、启动行为和关闭行为保持原义并可编辑，以便不改变既有工作流。
23. 作为 Server GUI 用户，我希望应用设置在 Server 运行时仍可编辑，以便保留当前行为。
24. 作为 Server GUI 用户，我希望自动更新、手动更新和更新 banner 在新布局中继续工作，以便管理版本。
25. 作为 Server GUI 用户，我希望语言切换仍在重启后生效并显示提示，以便保持现有国际化行为。
26. 作为 Windows 用户，我希望防火墙修复和声音面板入口仍只在 Windows 显示，以便使用平台维护能力。
27. 作为 Linux 用户，我希望桌面声音设置和 PipeWire 文档入口仍只在 Linux 显示，以便获得正确平台指引。
28. 作为 Server GUI 用户，我希望 About 继续展示产品名、版本、主页、许可证和版权信息，以便查看应用信息。
29. 作为 Server GUI 用户，我希望 Back、Hide 与 Quit 仍可操作，以便保持窗口与托盘工作流。
30. 作为使用键盘的用户，我希望按钮、表单、展开区域和对话框具有可见焦点与合理标签，以便不依赖鼠标完成操作。
31. 作为偏好减少动态效果的用户，我希望系统开启减少动态效果后界面取消非必要过渡，以便避免不适。
32. 作为使用最小窗口的用户，我希望 `380×560` 下所有功能仍可访问且 Settings 可滚动，以便保持紧凑窗口可用性。
33. 作为中英文用户，我希望所有新增标签都进入 en-US 与 zh-CN 资源，以便界面不存在缺失翻译。
34. 作为现有配置用户，我希望配置迁移提示、恢复警告和更新提示在新布局中继续可见，以便处理重要状态。
35. 作为依赖单实例与托盘的用户，我希望这些原生行为完全不变，以便视觉升级不引入生命周期回归。
36. 作为 Android Client 用户，我希望 Wire Protocol 与连接行为不变，以便继续连接现有 Server。

## Implementation Decisions

- 正式实现采用 Variant C 的信息架构与视觉方向，但按生产标准重写，不直接提升 throwaway 组件。
- 保持现有 Tauri command/event、ServerController、Server Core 与共享 store 的行为契约；Start/Stop 仍异步执行。
- 主窗口顶栏只承载品牌与 Settings 入口，不再重复显示 Server 状态。
- 主窗口使用居中的状态区域、静态状态点、监听地址与圆形主操作；不得添加持续动画或虚构音频仪表。
- Stop 使用浅色危险背景与危险色图标/文字；Running、错误与一般危险操作继续使用各自语义色。
- Endpoint 与 Encoding 作为快速设置显示；Host 与 Port 放入原生可展开区域，展开状态只影响展示，不影响配置值。
- Reset 移至主窗口 Server 配置区域，文案明确它重置全部 Server 字段而非仅网络字段。
- Settings 使用单页滚动与分组展开结构，不引入标签路由或宽屏侧栏。
- Settings 移除重复的 Server 表单及原位置的 Reset；保留全部应用设置和平台条件入口。
- Appearance 提供 System、Light、Dark 三种选择；System 为默认值并监听系统主题变化。
- Theme 是 Server GUI 的应用偏好，通过现有应用配置持久化路径以向后兼容的可选值保存；缺失或未知值回退 System，不改变旧配置的其他字段。
- 主题在首屏挂载前应用，原生托盘菜单、Server Core 与 Wire Protocol 不感知 Theme。
- 使用统一 CSS tokens 管理明暗表面、文本、边框、品牌色与语义色；保留系统字体栈，不引入打包字体。
- 使用一致的 SVG 图标替代 `♪`、`⚙`、`←` 字符图标，不为了原型引入大型图标依赖。
- 保留现有窗口默认 `420×640`、最小 `380×560`、可调整大小和单窗口模型。
- 保留配置警告、更新 banner、错误 alert、状态 aria-live、键盘焦点和 About 内容；新展开控件使用原生可访问语义。
- 过渡仅覆盖颜色、透明度和展开反馈，并在 `prefers-reduced-motion: reduce` 下禁用非必要过渡。
- 原型入口继续严格限制在开发模式；正式构建不得包含方案切换器或落选变体。

## Testing Decisions

- 不引入 Playwright、Vitest 或其他新的前端测试框架；用户已确认沿用现有验收边界。
- 好的测试应验证外部行为：真实状态同步、配置持久化、运行时锁定、主题回退与持久化、平台条件入口、中英文资源和现有窗口/托盘流程；不断言内部 DOM 结构或 CSS 类名。
- 在现有 Rust 配置测试边界覆盖 Theme 的默认值、合法值往返、未知值回退以及旧配置无损迁移。
- 继续使用 `svelte-check` 与正式 Vite 构建验证全部 Svelte/TypeScript 代码和生产隔离。
- 继续执行现有配置、协议和 Rust 测试，证明视觉改版未改变 Server Core 或 Wire Protocol。
- 扩展现有 Server GUI 人工验收清单，覆盖 Variant C 的主窗口、Settings、Stopped/Running、Starting/Stopping、错误与 banner、明暗主题、系统主题切换、主题重启持久化、`420×640` 与 `380×560`、键盘和减少动态效果。
- 在 Windows 与 Linux 分别保留主窗口和 Settings 的明暗截图；现有真实桌面托盘、平台维护入口与 Android v0.3.4 回归仍按原清单执行，未执行的项目必须明确标记。

## Out of Scope

- Android Client 的视觉或行为改版。
- Wire Protocol、Capture Format、Transfer Format 或 Server Core 行为变化。
- 新增音频波形、电平表、客户端列表、日志查看器或自动 capture recovery。
- 将紧凑单窗口改为宽屏仪表盘、多标签导航或 macOS 支持。
- 新增页面级前端自动化测试框架。
- 直接复用原型中的无持久化 stub、无完整错误处理代码或方案切换器。

## Further Notes

- 设计主源为本地 `prototype/server-gui-modernization` 分支，提交 `604f0d5`；用户选择了 Variant C，并单独选择克制 Stop 样式。
- 原型的运行脚本、完整 A/B/C 变体和 `?variant=A|B|C` 切换入口均保存在该分支，正式 `main` 工作树不保留这些 throwaway 文件。
- 正式实现应重写选定方向并引用该原型提交，不直接提升原型中的无持久化 stub、无完整错误处理代码或方案切换器。
- 本决策是可逆的视觉与信息架构选择，沿用 ADR-0002 已确定的“由 throwaway prototype 决定视觉”原则，不新增 ADR。

## Comments

- 2026-11-11: implemented via /implement-spec on `integration/server-gui-modernization` (tip `8aa5e54`, 15 commits over main, linear). Tickets 01–03 resolved; ticket 04 ready-for-human (Windows suite, real-desktop tray, live system-theme switch, reduced-motion, Starting/Stopping visuals, Android v0.3.4 regression remain manual; see `docs/server-rewrite/gui-modernization-acceptance.md`). Design source: `prototype/server-gui-modernization` @ `604f0d5` (untouched). Code review (Standards + Spec axes) run on the branch; all findings fixed, two benign scope additions retained (main-view footer hints, Settings-level server.error display).
