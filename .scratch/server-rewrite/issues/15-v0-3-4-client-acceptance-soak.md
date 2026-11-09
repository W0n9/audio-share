# 15: v0.3.4 Android Client acceptance + soak

**What to build:** 用真实 v0.3.4 APK 对完整新 Server 做验收：5 种 encoding、单/双声道、44.1/48 kHz、多客户端、重连、heartbeat 超时；8 小时 soak、设备热插拔、睡眠恢复、反复 start/stop；完成 GUI 手工验收清单。

**Blocked by:** 07 (as-cmd parity), 11 (Windows maintenance), 12 (Linux equivalents), 13 (Windows packaging), 14 (Linux packaging)

**Status:** ready-for-agent

- [ ] 真实 v0.3.4 APK 在全部组合下无感播放，无解析错位。
- [ ] 多客户端、重连与 heartbeat 超时行为与 v0.3.4 一致。
- [ ] 8 小时 soak 无 crash、无明显 glitch 倒退。
- [ ] 设备热插拔与睡眠恢复后能恢复采集。
- [ ] GUI 手工验收清单全部通过。
- [ ] 记录 A/B 结果与已知差异。
