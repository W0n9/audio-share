# 02: Windows WASAPI loopback spike

**What to build:** 一个最小 Rust 工具，在 Windows 上从指定 render Endpoint 以 WASAPI shared-mode loopback 采集系统音频，支持 f32、s8、s16、s24、s32，记录 callback 间隔、CPU 与短时 glitch，并与 1 秒缓冲行为对齐。产出测量报告，决定 `wasapi` crate 是否可直接采用；不达标则验证 `windows-rs` fallback。

**Blocked by:** 01 (New Rust workspace + Wire Protocol harness + v0.3.4 golden fixtures)

**Status:** ready-for-agent

- [ ] 能枚举 render Endpoint，显示 IMMDevice ID 与友好名。
- [ ] 能采集默认 Endpoint 的系统音频并落盘为 PCM。
- [ ] 5 种 encoding 至少各验证一次；不支持的请求能回退到 closest match。
- [ ] 记录 callback 间隔、CPU 与短时 glitch，并给出 `wasapi` 与 `windows-rs` 的结论。
- [ ] 1 秒缓冲下反复启动/停止不崩溃。
