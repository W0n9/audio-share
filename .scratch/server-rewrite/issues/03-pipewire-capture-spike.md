# 03: Linux PipeWire capture spike

**What to build:** 一个最小 Rust 工具，在 Linux 上枚举 Audio/Sink、按 `object.serial` 与 `priority.session` 选择 Endpoint、采集 F32LE 和至少一种非 F32 格式，并验证 monitor 采集；记录测量结果。决定 `pipewire` crate 是否可直接采用；不达标则验证 `pipewire-sys` direct C API fallback。

**Blocked by:** 01 (New Rust workspace + Wire Protocol harness + v0.3.4 golden fixtures)

**Status:** ready-for-agent

- [ ] 枚举 Audio/Sink，默认按 `priority.session` 选择。
- [ ] 能按 target 采集指定 sink 的 monitor 音频。
- [ ] F32LE 与至少一种非 F32（s16、s24 或 s32）可采集并落盘。
- [ ] 记录 callback/quantum、CPU 与短时 glitch，并给出 `pipewire` 与 `pipewire-sys` 的结论。
- [ ] 反复启动/停止不崩溃。
