# 03: 将 Settings 实现为分组展开结构

**What to build:** 用户进入 Settings 后可以按 Appearance、Startup & window、Updates、System 和 About 找到应用设置，不再看到重复的 Server 表单；全部现有应用、平台、更新和窗口行为继续可用。

**Blocked by:** 02 — 将主窗口实现为 Variant C 极简专注

**Status:** resolved

- [ ] Settings 使用单页滚动与分组展开结构，不包含 Host、Port、Endpoint、Encoding 或旧位置 Reset。
- [ ] Appearance 保留 Ticket 01 的 Theme 能力，并保留语言选择及重启生效提示。
- [ ] 登录启动、启动行为、关闭行为与自动更新保持原义，Server Running 时应用设置仍可编辑。
- [ ] 手动更新、更新结果、配置保存错误和更新 banner 继续工作并具有明暗主题样式。
- [ ] Windows 只显示防火墙修复与声音面板，Linux 只显示桌面声音设置与 PipeWire 文档入口。
- [ ] About 继续提供产品名、版本、主页、Apache-2.0 和版权信息，主页链接行为不变。
- [ ] Back、Hide、Quit 保持现有窗口和托盘行为。
- [ ] 所有新增分组与控件具有 en-US、zh-CN 文案、可见焦点和可访问名称。
- [ ] `420×640` 与 `380×560` 下 Settings 可滚动且所有功能可达，减少动态效果设置得到尊重。
- [ ] `svelte-check`、正式前端构建及相关现有测试通过。

## Comments

- 2026-11-11: implemented on `ticket/gui-mod-03-settings` (commit `04f5036`), merged fast-forward into `integration/server-gui-modernization`. Settings rebuilt as single-page scroll with grouped disclosures (Appearance / Startup & window / Updates / System / About); duplicate Server form and old Reset removed; Windows/Linux platform entries preserved; About inlined as disclosure content (homepage link behavior unchanged); Back/Hide/Quit preserved. en-US/zh-CN paired (strings parity test green). Theme selector editable while Server runs (`appLocked` only freezes busy/saving). All checks green. Closed by the /implement-spec run; integration tip `8aa5e54`.
