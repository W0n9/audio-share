# 13: Windows packaging

**What to build:** 便携 `AudioShareServer.exe`；启动时检测 WebView2，缺失则显示明确提示与下载链接；产物名保持；Windows 10+ 可直接运行。

**Blocked by:** 11 (Windows maintenance: Firewall Repair + Sound Panel)

**Status:** ready-for-agent

- [ ] 产物为便携 exe，无安装器。
- [ ] 有 WebView2 时正常启动。
- [ ] 无 WebView2 时显示明确提示与下载链接，不崩溃。
- [ ] 产物名保持 `AudioShareServer.exe`。
- [ ] 在 Windows 10 与 11 上运行并通过基本 GUI 验收。
