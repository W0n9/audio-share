# 08: Tauri tray-first shell + Start/Stop

**What to build:** Tauri v2 Server GUI 采用原型 A：托盘优先单窗口，显示 Server 状态与监听地址；Start/Stop 驱动同一个 core；托盘菜单 Show、Start、Stop、Exit；关闭窗口最小化到托盘。Windows 与 Linux 都能运行。

**Blocked by:** 05 (Windows real capture path), 06 (Linux real capture path)

**Status:** ready-for-agent

- [ ] 应用启动后可从托盘显示或隐藏窗口。
- [ ] Start/Stop 真实启动和停止 Server，状态与地址实时更新。
- [ ] 托盘菜单 Show、Start、Stop、Exit 可用；Linux 右键菜单可用。
- [ ] 关闭窗口默认最小化到托盘。
- [ ] Start/Stop 不阻塞 UI；错误以可读消息呈现。
- [ ] 视觉与信息架构遵循原型 A：状态和主操作优先。
