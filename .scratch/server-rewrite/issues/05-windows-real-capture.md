# 05: Windows real capture path

**What to build:** 把 Windows WASAPI loopback 接入 core，替换 synthetic source；选定 render Endpoint 后，新 Server 通过 Wire Protocol 发送真实系统音频。与旧 `as-cmd` 在同一台机器、同一音频源、同一 Capture Format 下做 A/B。

**Blocked by:** 02 (Windows WASAPI loopback spike), 04 (Wire Protocol slice with fake capture + `as-cmd -b`)

**Status:** ready-for-agent

- [ ] Windows 上启动 Server 后，scripted client 收到真实系统音频。
- [ ] 默认 Capture Format 来自 Endpoint mix format；encoding、channels、sample rate 请求遵循 closest-match 回退。
- [ ] 与旧 `as-cmd` 的 `AudioFormat` 和 PCM 输出对比无明显差异。
- [ ] 反复 start/stop 与切换 Endpoint 不崩溃。
- [ ] 采集线程与网络线程共享的 `AudioFormat` 无数据竞争。
