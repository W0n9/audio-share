# 06: Server v2 transport module — production skeleton

**What to build:** 在生产 Server Core 中落地 v2 传输模块骨架：quinn endpoint、Server 身份钥、连接生命周期、资源限额、golden-vector 回放工装。这是 Tracer-Bullet 第 3 步（最小认证 v2 控制通道）的服务器侧地基；本工单交付到「能接受连接、拒绝不合规者」，认证流程在 07。

**Blocked by:** 02, 05

**Status:** claimed

**Type:** task

## Requirements

- 在 `crates/audio-share-core` 新增独立 v2 模块（建议 `src/v2/`，与 v1 `src/protocol/` 平行；Implementation Boundary：v2 handshake/control/sessions 独立成模块，不复用 v1 Protobuf 做内部 API）。若 crate 边界更清晰可建 `crates/v2-transport`，二选一并记录理由。
- quinn 0.11 endpoint：ALPN `audio-share/v2`（major 不匹配在应用数据前显式失败）、rustls TLS 1.3（rustls 默认套件 + X25519，D7）、0-RTT 永久禁用、默认监听 `65531/udp`（D1；与 v1 的 65530 并行、独立可禁用）。
- Server 身份钥（pairing.md：Ed25519，PR-1）：生成 + 持久化（0600/0700、原子写 + 1 备份、损坏 fail-closed 不自动再生、POSIX 宽松权限拒绝启用 v2 + `--allow-loose-key-permissions` 覆盖、Windows 仅警告）；SPKI SHA-256 指纹在启动日志输出。
- 资源限额落地（threat-model TM-1..8 与 spec §10）：并发握手、总 session 数（32）、每 Client 1 session（新替旧）、pre-auth 响应 ≤128 B、错误 ≤512 B/detail ≤256 B、控制消息 64 KiB 上限（超限 401+Close，先于任何分配）。非等待发送 + 应用层有界队列。
- QUIC close 携带 §6 错误注册表的应用错误码；连接生命周期状态经域 API 暴露最小可见性（as-cmd/GUI 观察共同 v1/v2 生命周期状态，不解析线缆字节 —— 本工单只做管道，不做 UI）。
- **Golden-vector 回放工装**：Rust 侧 encoder 对 `fixtures/wire-protocol-v2/` 的 control happy/error 案例（34+3 个）做双向字节对拍（复用/扩展 ticket 05 的回放契约）；实现不得有自己的私有编码。
- 规范红线：v1 模块、`docs/protocol.md`、v1 fixtures 一律不动；v1 在每个提交点保持可构建可发布。

## Definition of done

- 新增模块 `cargo test` 绿：身份钥生命周期测试（生成/重载/损坏/权限）、限额测试、close-错误码测试、非等待队列测试。
- Vector 回放：全部 control 案例（error-registry 25 + negative 5 + happy 6）字节一致。
- 回归：`protocol-harness replay` 全绿；v1 集成测试全绿；现有 `as-cmd`/GUI 行为不变（v2 关闭时零行为差异）。
- `as-cmd --v2-port=65531`（或等价配置）能起停 v2 监听并有日志；默认随 Server 启用。
