# 10: Lifecycle + i18n + About + update check

**What to build:** 开机启动（Windows 用户启动项 / XDG autostart）、启动行为（不操作 / 启动服务器 / 保持上次状态）、关闭行为（退出 / 最小化到托盘）、单实例、Hide 与 Exit；en-US 与 zh-CN 持久化并在启动时应用；About 显示版本、主页与许可证；每 3 小时查询 GitHub releases/latest，提示并打开 release 页面。

**Blocked by:** 09 (Server settings + config migration)

**Status:** ready-for-agent

- [ ] 开机启动在 Windows 与 Linux 都生效。
- [ ] 启动行为与关闭行为按设置执行。
- [ ] 单实例：重复启动只显示已有窗口。
- [ ] en-US 与 zh-CN 切换持久化，启动时应用。
- [ ] About 显示版本、主页与许可证。
- [ ] 更新检查每 3 小时一次且可关闭；有新版本时提示并打开 release 页面。
