# 14: Android PCM playback and PlaybackDiagnostics

**What to build:** M4 的 Android Client 纵向切片：接收工单 13 的认证 v2 DATAGRAM，按当前 instance/configuration 保护后进入 native direct-`ByteBuffer` ring，由前台 Service 的 `AudioTrack` 消费；周期及关闭时报告真实播放统计，不逐包跨 JNI。

**Blocked by:** 13, 05

**Status:** resolved

**Type:** task

## Confirmed scope and seams

维护者于 2026-11-14 确认 M4 与公共 seam：native 媒体交付/实际播放提交/统计 Interface，以及 Android 播放 Interface 与真实 Service/Binder。每个纵向行为先 RED 后 GREEN，预期值来自 normative spec / golden vectors；不能 mock 私有协作者来制造通过。

- native `Session` 是 QUIC/control 与接收媒体的所有者；一个小的媒体交付 Interface 应隐藏 header parse、extended sequence/dedup、未知代际有界保留、epoch 清理和统计快照。控制事件携带的 instance/config/anchor 必须传入这个唯一接收状态，不用 wire DTO 在 Kotlin/native/UI 各存一份真相。
- native ring 是唯一 PCM 跨 JNI 热路径。`ByteBuffer` 在 session 生命周期中只 attach 一次；media task 不得每包 JNI callback，不得把 native 指针暴露给任意调用者。attach 失败/未准备时必须有界计数，不得 block QUIC 或假装播放。
- Kotlin 以明确的播放所有者持有 ring buffer、playback worker 与 lifecycle。`AudioTrack.write` 的返回值（包括部分写入、ERROR_INVALID_OPERATION、ERROR_DEAD_OBJECT 等）决定实际 submitted sample-frame position；收包数、ring 写入数、AudioTrack 请求写入数均不能充当已提交播放证据。异步 write 若采用，必须能确定完成/失败与关闭竞态。处理 F32LE/U8/S16LE/S32LE 的平台映射；真正不能解码的合法格式按 403，而非静默换格式。保持 minSdk 26。
- 生产 Service 在收到有效 configuration 后自动 `Subscribe`（M3 当前没有播放按钮，M4 的 tracer bullet 不应永久停在 READY）；configuration 尚未来时等待，首包前不伪造 PLAYING。保留已有 source/lifetime token、同步状态 replay、STOP 与 rebind/recreate/lock 语义。前台通知改为真实播放状态；STOP 释放 AudioTrack/worker/ring 后 Close(701)，不能被迟到的 READY/PLAYING 覆盖。
- 按 §7.5：已知 (instance,generation) 才进入交付；新 instance 重置 playback 与统计；旧 generation 丢弃计 unknown_config_drops，reserved bits 2–31 忽略；payload frame alignment/0 id 检查、duplicates、reorder distance/extended sequence 统计。M4 不宣称完成 jitter playout scheduling，不能把乱序交付谎称连续播放。
- 按 §7.6：未知 generation 仅进入有界 hold（≤100 ms 媒体或≤32 datagrams，overflow drop oldest），绝不以旧 configuration 解码；可请求配置，2 s 未知则 Error(403)+Close(403)。hold 与 epoch change 必须释放，不能使用无界 Vec。
- §9 全字段有真实值或显式零值语义：extended highest sequence、signed cumulative/interval loss、received/duplicates/reordered/max distance、late drops、unknown config drops、concealment/concealed frames、underruns、RFC3550 A.8 jitter（sample clock 转 µs）、current/target buffer、实际 `submitted_position`。M4 未实现的 playout intervention 不得编数；完整 jitter buffer/drift/late-drop scheduling 属 M5，目标 buffer 常量不能谎称已实现 target algorithm。
- 每 5 s 在 PLAYING 由 native control plane 发送 `PlaybackDiagnostics`；entering CLOSING best-effort 最终一份。最多一个 pending send，失败/下一周期跳过不阻塞媒体，关闭后不再写已失效 stream。接收音频与发送统计分离开，不能由 Kotlin 每 250 ms 轮询代替协议周期上报。
- Close/StreamEnded/Unsubscribe/新 instance 对 AudioTrack、ring、统计的顺序必须可观察且无数据污染。普通 stream end 回 READY（订阅状态依 §5），关闭则不 echo Close；最终统计是 best-effort，不能无限等待阻碍 teardown。

## Evidence / definition of done

- 经公共 native Interface 的 golden header/generation/dedup/wrap/epoch RED/GREEN；播放 Interface 的部分写入、错误、submitted position 与 stop 的真实行为 RED/GREEN。
- 实际 native QUIC loop 接收工单 13 媒体，ring→AudioTrack 接线，非仅有 host helper；真实 Android Client 与 Rust Server 的播放/统计/关闭互操作另行记录 PASS/FAIL/NOT RUN。
- JVM 36 基线、native 两配置、默认 release（无 fixture identity）回归；不得改变 `android-app/`、v1 fixtures 或 frozen v0.3.4 APK。
- 不引入完整 M5 jitter/drift/concealment/reconnect matrix、生产 pairing/Keystore/GUI/multi-Client、外部 telemetry 或原始 PCM 日志。

## Answer

2026-11-14 — Android 侧 M4 纵向切片完成（host/JVM 证据；**真机播放 NOT RUN**，归工单 15）。

- `native/src/media_session.rs`：唯一媒体接收 Interface——§7.1 header、§7.5 dedup/reorder/gap/malformed 计数、旧代际丢弃、§7.6 有界 hold（≤32 datagrams / ≤100 ms，2 s give-up 时钟）、ring 交付、§9 `PlaybackDiagnostics`；`submitted_position = anchor + AudioTrack 实际接受字节/帧`。
- `native/src/media_control.rs`：首次配置后自动 `Subscribe`（M3 无播放按钮，不能永久停 READY）、5 s 诊断至多一个 pending、StreamEnded 后允许重新订阅。
- `native/src/session.rs`：控制循环新增 DATAGRAM 分支、媒体 tick（250 ms）、`StreamStarted`/`StreamEnded`/`ConfigurationPublished` 驱动媒体状态；本地致命错误（403/602）统一写入；新增 `attach_ring`/`ring_position`/`media_format_json`/`record_playback`。`StreamEnded` 不再清除订阅（§5.2），只有用户 Unsubscribe 清除。
- `native/src/jni.rs` + `app/ClientNative.kt`：`attachMediaRing` / `ringPosition` / `mediaFormatJson` / `recordPlayback`；ring 是唯一 PCM 热路径，无逐包 JNI。
- `app/MediaRingBuffer.kt` + `MediaPlayer.kt` + `AudioSink`：direct ByteBuffer 单调游标读取（lapped → resync），`AudioTrack` sink，`playOnce` 只计平台实际接受字节；S24LE 在 API 26 无 AudioTrack 编码，明确不静默改格式。
- `ClientSession.kt` / `ControlSessionService.kt`：播放所有者随 native 会话创建（`connect` 前 attach）、在 `destroy` 前停止；通知不再声称播放未实现。

验证：native 122 passed（默认与 `test-support` 各 106 lib + 5 + 11），JVM 46 passed / 0 failed，`assembleRelease` 默认构建成功（无 fixture 身份），clippy 干净。证据与 RED/GREEN 说明见 [evidence/ticket-14/README.md](../evidence/ticket-14/README.md)。

边界与未验证：
- jitter buffer / playout deadline / late drop / concealment / drift 属 M5；对应诊断字段报 0 而非编数，`buffer_target_ms` 亦报 0。
- 真实 `AudioTrack` 播放、锁屏播放、长时间音质、真机与 Rust Server 互操作 **NOT RUN**（本会话未接触设备）；工单 15 负责。
- 设备工装 `service-device-test.py` 尚未扩展媒体用例；不得把 host/JVM 结果当作真机 PASS。

## Comments

2026-11-14 — 已确认 seam，但按依赖尚未 claim。当前 `ClientNative` 只有 connect/auth/stop/state/stats/rebind；`media.rs` 与 `ring.rs` 是骨架，不能把已有 header parser 当作媒体播放已完成。

### 2026-11-14 — JNI 契约（两侧共同实现，先定后改）

native 与 Kotlin 各自只实现自己的一半，签名以 `ClientNative.kt` 的声明为准：

| 成员 | 方向 | 语义 |
|---|---|---|
| `attachMediaRing(handle: Long, buffer: ByteBuffer): Boolean` | Kotlin→native | 传一次 direct ByteBuffer（容量 ≥ 1 s 音频，建议 256 KiB）。只有 attach 成功后 native 才写 ring；重复 attach 返回 false。buffer 在 session 生命周期内由 Kotlin 持有，不得回收。 |
| `ringPosition(handle: Long): Long` | Kotlin→native | 已写入 ring 的累计字节数（单调）。Kotlin 用它检测 lapped（落后 > 容量则跳到当前值）并决定可读字节。`-1` 表示 handle 未知。 |
| `mediaFormatJson(handle: Long): String?` | Kotlin→native | 当前应播放的格式与 epoch：`{"instance":u32,"generation":u32,"sample_rate":u32,"channels":u32,"sample_format":u32,"anchor":u64}`；PLAYING 前为 `null`。Kotlin 只在值变化时重建 AudioTrack。 |
| `recordPlayback(handle: Long, submittedBytes: Long, underruns: Long, bufferCurrentMs: Int)` | Kotlin→native | 报告**实际交给 AudioTrack 并成功接受的字节数**（自当前 instance 起算）、underrun 次数与当前 ring 缓冲毫秒。native 用它填 `submitted_position = anchor + submittedBytes/frame_bytes`、`underruns`、`buffer_current_ms`；绝不把收包数当播放证据。 |

规则：媒体数据只经 ring；上述四个调用都是低频（≤ 50 Hz 轮询），不得每包调用。native 负责 header/代际/epoch/统计，Kotlin 负责 AudioTrack 与真实提交位置。60 s 后停止实现同一接口的不同副本：两侧都以此表为准。
