# 13: Server PCM media delivery and pacing

**What to build:** M4 的 Server 纵向切片：让共享 capture 的原始 PCM 以有界、非阻塞方式进入已认证且订阅的 v2 session，按 normative 24-byte 媒体头发送 QUIC DATAGRAM；建立真实 stream instance / sample-frame clock / configuration generation 观察，不改动 frozen v1。

**Blocked by:** 12

**Status:** resolved

**Type:** task

## Confirmed scope and seams

2026-11-14 维护者确认仅实施 M4（单 Android Client PCM、播放统计、显式关闭），并确认以下公共测试 seam：

1. Server 公共 capture 输入 → 认证 QUIC 媒体/控制输出；用真实 Rust 两端和既有 live-QUIC adapter，测试动态 MTU、字节、订阅门控与 pacing，不测试私有方法。
2. Android native 媒体交付、实际播放提交与统计 Interface（工单 14）。
3. Android 播放 Interface 及真实 Service/Binder（工单 14/15）。

测试预期来自 `docs/wire-protocol-v2/spec.md` 和 `fixtures/wire-protocol-v2/`，不得由同一发送器自行推导。每个纵向行为一条真实 RED → 最小 GREEN，不先横切全部测试。工作树为 `phase-4/single-client-pcm` @ `a5de2d9`；不要提交、push 或开 PR。

## Requirements

- 在 Server Core 的共享原始 PCM drain 处提供协议中立的只读/有界 fan-out seam。v2 观察不得放在 v1 peers-empty 或 v1 1464-byte 分段之后，不得让 v1 Protobuf 或 MTU 成为内部媒体模型；v1 framing / UDP / little-endian / fixture 保持不变。不得在 capture 线程等待 QUIC、控制流或单 Client 消费。
- 明确 capture sample-frame position 与 stream instance 的唯一来源。初始 sequence/timestamp 为 0；sequence 每 datagram +1 mod 2^32；timestamp 是 payload 第一帧在 capture sampling clock 上的 sample-frame 位置，不是 wall time 或交错 sample 数。instance 非零、进程内递增，2^32−1 后绕回 1；capture restart/discontinuity 必须新 instance 并通知会话。format 不可变期间 generation 为 1；真实 decode-affecting 变化才 +1，溢出时新 instance。不要用收到的字节数伪造已知 upstream gap/discontinuity；现有 backend 未交付这些信息时记录证据边界。
- 将同一 capture 观察接入生产 `as-cmd` → `SessionServer`，并主动发布开始/结束/变化；`Subscribe` 仍须以最新 capture 健康状态门控，501 不订阅。新订阅活动的 instance 发 `StreamStarted` 的真实 id / generation / next-frame anchor；`PLAYING` 建模不能删除既有认证/撤销/303/5 s grace 保护。
- 24-byte BE header 精确符合 §7.1：`instance u32 | generation u32 | sequence u32 | timestamp u64 | flags u32`；PCM sample bytes 仍 LE。首包 STREAM_START；generation 首次切换 CONFIG_CHANGE；reserved bits 发送 0。复用现有 media golden vectors，不改写它们来迁就实现。
- 每个实际 datagram 发送时重新调用 `max_datagram_size()`，减 24 后按完整 sample frames 切分；path limit 缩小须重切，不依赖 IP fragmentation，不接收方本地 MTU 断言。不支持 DATAGRAM 或装不下一帧时按 602 关闭。空 PCM 不产生无意义媒体包。
- 非等待发送，每 Client 有界应用队列（256 datagrams，overflow drop newest，本地计数），完整 capture 输入链路有界。 paced sends 满足 capture_position−last_sent_position ≤100 ms，且不能用 wall-clock/audio-clock 失配将音频突发成快于媒体时间线的播放；对 fake-capture 的 +14% 遗留现象做真实测量，区分工装 buffer 几何与生产 pacing，不用放宽断言掩盖。慢 Client 不阻塞 capture。
- `StreamEnded` 正常/失败分别 502/501；用户 Unsubscribe 在 PLAYING 必须先 502(detail `unsubscribe` 的规范原文 `unsubscribed`) 后 OK。普通 capture end/failure 回到 READY 且仍 subscribed，可接受后续新 instance；用户 unsubscribe 才清 subscribed。Close 单向、无 echo、携带 registry code；共享 capture 不受单 Client 关闭影响。
- `PlaybackDiagnostics` 在 Server 以有界快照保存并可通过低频公共观察读取；认证/QUIC ACK 或媒体发送量不能生成播放证据。统计输入/输出失败不阻塞媒体。日志不得含原始 PCM、私钥、stack trace 或不必要的内部路径。
- 已有 capture 格式/线程健康观察与 ADR-0003 保留：线程结束后保留 immutable format，不自动 restart。不得修改 `android-app/`、frozen v0.3.4 APK/fixtures 来通过测试。

## Evidence / definition of done

- 公共 seam 的真实 RED/GREEN 日志（测试名、修复前失败断言、修复后通过）；生产 `as-cmd` adapter 不只测一个无接线 helper。
- 真实认证 QUIC Subscribe 收到与独立 golden vector 一致的 PCM header/payload；可变发送限额、wrap、代际/实例、队列/pacing 的可观察行为覆盖。
- 对 capture end/failure、Unsubscribe 顺序、Close 无 echo、旧 session cleanup 有断言；frozen v1 既有回归不弱。
- 工单 14 可通过同一 live Server seam 消费；不把 host 测试称为 Android 真机/音质/60 分钟 PASS。
- 未解决的设备或后端限制明确记录；不自动扩大至 M5 jitter/drift/impairment、M6 pairing/GUI/multi-Client 或 M7 release。

## Answer

2026-11-14 — Server 侧 M4 纵向切片完成（隔离分支 `m4-server-pcm`，基于 `a5de2d9`）。真机/Android 播放不在本票范围，见工单 14/15。

- `crates/audio-share-core/src/capture/media.rs`：协议中立的 `MediaTap`——有界、非等待 fan-out，携带 `stream_instance_id`/`config_generation`/sample-frame 位置；publish 为 O(subscribers) `try_send`，满队列丢 newest 并计数；非整帧缓冲拒绝并计数；`end()`/`fail()` 区分 502/501。
- `crates/audio-share-core/src/server/mod.rs`：`broadcast_loop` 在每个 capture buffer 上、v1 的 peers-empty 与 1464-byte 分段**之前** publish；循环退出时按 shutdown/通道关闭区分 `end()`/`fail()`；`ServerHandle::media_tap()` 提供只读接口。v1 framing/UDP/fixtures 未变。
- `crates/v2-transport/src/media.rs`：24-byte BE 头、逐发送 `max_datagram_size() − 24` 重切、整帧对齐、STREAM_START/CONFIG_CHANGE、reserved bits 恒 0、sequence mod 2³²、timestamp 按帧推进。
- `crates/v2-transport/src/send_queue.rs`：256 datagram 有界队列（drop newest 计数）与 §7.4 100 ms send-ahead pacer（注入时钟，idle 不累积 burst）。
- `crates/v2-transport/src/media_pump.rs`：保留未编码 PCM，**发送时**才按当前 limit 打包，因此路径限额缩小对下一个 datagram 立即生效；pending 上限 500 ms，超限丢 newest 并计数；迟到订阅者从通报的 anchor 开始。
- `crates/v2-transport/src/media_source.rs` + `crates/as-cmd/src/lib.rs`：`MediaSource` 接口与 `CaptureMediaSource` 适配器；`v2_capture_source` 现在把 tap 的真实 instance/next-frame 填入 `CaptureSnapshot`，`StreamStarted` 字段不再占位。
- `crates/v2-transport/src/state_machine.rs`：新增 `ServerState::Playing`、`Event::StreamLifecycle`（Started/Ended/Restarted/ConfigChanged）与 §5.2 PLAYING 行（重复 Subscribe 非致命 402、Unsubscribe 先 `StreamEnded(502,"unsubscribed")` 后 OK、诊断记录）；`Subscribe` 在活动流上进入 PLAYING。capture end/failure 回 READY 且**保留订阅**，只有用户 Unsubscribe 清除。
- `crates/v2-transport/src/driver.rs`：`media_source` 注入、10 ms 媒体 tick、每会话独立 `MediaTask`（真实 `send_datagram`，非等待；`max_datagram_size()` 每 tick 重查）、`PlaybackDiagnostics` 有界快照与低频读取接口 `playback_diagnostics(session_id)`；DATAGRAM 不可用 → 602。
- 真实 live-QUIC 测试 `crates/as-cmd/tests/v2_media.rs`：生产 `Server::start` + 原样 `as_cmd::v2_capture_source`/`capture_media_source`，认证后 Subscribe 收到真实 DATAGRAM（头字段、STREAM_START、逐包 timestamp 连续、frame-aligned），Unsubscribe 顺序正确。

验证：`cargo test --workspace` 516 passed / 0 failed / 4 既有 ignored；`cargo clippy` 对三 crate 无警告。RED→GREEN 证据见 [evidence/ticket-13/README.md](../evidence/ticket-13/README.md)。

未做/边界：jitter buffer、concealment、drift、impairment、fuzz（M5）；多 Client 隔离/soak、生产 pairing、GUI（M6）；Windows/API26/60 分钟音质（M7）。`--fake-capture` +14% pacing 遗留未在本票重测。

## Comments

2026-11-14 — 由交接 `/tmp/audio-share-handoff-next-step-20261114-003051.md` 继续；roadmap 中 13–21 原本只是提案，本次只批准 M4 的 13–15。`media.rs` / `send_queue.rs` 在基线不存在，不能把常量或 fixture 当成生产媒体路径。
