# 01: 持久化 Theme 并建立明暗视觉基础

**What to build:** 用户可以在 Settings 中选择 System、Light 或 Dark，并立即看到整个 Server GUI 使用对应主题；选择跨重启保留，System 随操作系统变化，旧配置无需手工迁移。

**Blocked by:** None (can start immediately)

**Status:** resolved

- [ ] Appearance 提供 System、Light、Dark，默认 System，新增标签具有 en-US 与 zh-CN 文案。
- [ ] 主题在首屏挂载前应用，手动切换立即生效，System 监听系统明暗变化。
- [ ] Theme 通过现有应用配置路径持久化；缺失或未知值回退 System，合法值可往返保存，旧配置其他字段保持不变。
- [ ] 应用设置在 Server Running 时仍可编辑，Theme 不改变 Server Core、托盘或 Wire Protocol 行为。
- [ ] 明暗 tokens 覆盖现有主窗口、Settings、表单、banner、对话框、焦点和语义色，文本与控件保持可读。
- [ ] `prefers-reduced-motion: reduce` 下不存在非必要过渡。
- [ ] 现有 Rust 配置测试补充 Theme 默认值、合法值、未知值与旧配置兼容检查。
- [ ] `svelte-check`、正式前端构建及相关 Rust 测试通过。

## Comments

- 2026-11-11: implemented on `ticket/gui-mod-01-theme` (5 commits, `68a0083..6f77951`), merged fast-forward into `integration/server-gui-modernization`. Rust config tests cover theme default/round-trip/unknown-fallback/legacy migration; svelte-check, vite build, `cargo test --workspace` all green. Unknown theme values fall back to System with an InvalidValue notice (consistent with port/encoding strictness). Closed by the /implement-spec run; integration tip `8aa5e54`.
