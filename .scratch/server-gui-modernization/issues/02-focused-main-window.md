# 02: 将主窗口实现为 Variant C 极简专注

**What to build:** 用户打开 Server GUI 后看到一个居中的真实 Server 状态和圆形主操作；常用音频设置可直接编辑，网络配置可展开，Start/Stop、Reset、错误和状态同步均沿用真实行为。

**Blocked by:** 01 — 持久化 Theme 并建立明暗视觉基础

**Status:** resolved

- [ ] 顶栏不再重复 Server 状态，主区域只显示一个 Stopped、Starting、Running 或 Stopping 状态及监听地址。
- [ ] Start 使用品牌色圆形主操作，Stop 使用克制危险样式，异步过渡期间提供反馈并阻止重复操作。
- [ ] Endpoint 与 Encoding 作为快速设置可见，继续使用真实 Endpoint 和现有六项 Encoding。
- [ ] Host 与 Port 位于可展开的 Network configuration 中，展开区域具有键盘与辅助技术可访问语义。
- [ ] Reset 位于主窗口并明确重置全部 Server 字段；它恢复 Host、Port、Endpoint、Encoding 默认值且保留应用设置。
- [ ] Server Running 或执行异步操作时 Server 字段锁定，停止后可编辑并自动保存。
- [ ] 启动失败、保存失败、配置警告和更新 banner 在明暗主题及新布局中保持可读。
- [ ] 托盘或其他入口造成的状态与地址变化无需打开额外视图即可同步到主窗口。
- [ ] 默认 `420×640` 与最小 `380×560` 下所有主窗口功能均可访问，未新增虚构仪表、日志或客户端数据。
- [ ] `svelte-check`、正式前端构建及相关现有测试通过。

## Comments

- 2026-11-11: implemented on `ticket/gui-mod-02-main-window` (4 commits, `36ee0db..8744fb1`), merged fast-forward into `integration/server-gui-modernization`. Hero status area with static dot + address chip, 64px circular primary action (brand Start, restrained-danger Stop), real endpoint enumeration, `<details>` Network configuration, Reset in the main view per spec, runtime field lock, tray-driven sync via existing events, shared SVG `Icon.svelte` (no icon dependency). Starting/Stopping pending feedback is a new design (prototype lacked it). All checks green; review follow-ups (setTheme lock, dedup, icon cleanup) fixed on the integration branch. Closed by the /implement-spec run; integration tip `8aa5e54`.
