# 11: Windows maintenance: Firewall Repair + Sound Panel

**What to build:** Windows 上提供提权修复 TCP/UDP 入站规则与打开系统声音面板；失败时给出可读错误。

**Blocked by:** 10 (Lifecycle + i18n + About + update check)

**Status:** ready-for-agent

- [ ] Firewall Repair 提权后重建入站规则，Android Client 可连接。
- [ ] Sound Panel 打开系统声音设置。
- [ ] 用户取消提权或修复失败时，GUI 给出明确错误。
- [ ] 非 Windows 平台不显示这两个入口。
