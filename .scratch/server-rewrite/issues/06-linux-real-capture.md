# 06: Linux real capture path

**What to build:** 把 PipeWire capture 接入 core；选定 Audio/Sink 后，新 Server 通过 Wire Protocol 发送真实音频；验证 monitor 与非 F32 路径；与旧 `as-cmd` A/B。

**Blocked by:** 03 (Linux PipeWire capture spike), 04 (Wire Protocol slice with fake capture + `as-cmd -b`)

**Status:** ready-for-agent

- [ ] 枚举 Audio/Sink，默认按 `priority.session` 选择。
- [ ] 选定 sink 后，scripted client 收到真实 monitor 音频。
- [ ] F32LE 为默认；s8 映射到 U8，s16、s24、s32 映射正确。
- [ ] 与旧 `as-cmd` 的 `AudioFormat` 和 PCM 输出对比无明显差异。
- [ ] 反复 start/stop 与切换 sink 不崩溃。
