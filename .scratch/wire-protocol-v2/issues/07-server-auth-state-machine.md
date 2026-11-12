# 07: Server authorization, state machine, and capability negotiation

**What to build:** Server 侧的认证与控制平面语义：签名挑战认证全流程、§5 状态机 Server 表、能力协商、速率限制与撤销。交付后 Server 拥有完整的「最小认证控制通道」服务器半边（媒体为 ticket 10+ 的后续阶段）。

**Blocked by:** 06

**Status:** claimed

**Type:** task

## Requirements

- 认证流程（pairing.md §9 + spec §4.4）：`AuthInit → AuthChallenge → AuthProof → AuthResult`；PR-9 语义照抄 —— 对**原始 auth_context 字节**做**单次** ECDSA-P256-SHA256，MUST NOT 先哈希再签；每连接 5 次失败 + 每 60 s 每源 20 次（TM-5）。
- 测试专用凭据预置：test-only 配置把「已授权 Client 公钥（P-256）列表」交给 Server（隔离的测试配置或测试构建；生产 pairing 是第 6 阶段工作）。授权身份持久化文件遵循 06 的存储规则。
- §5 状态机 Server 表逐行实现：CONNECTING → AUTHENTICATING → NEGOTIATING → READY（媒体态为后续阶段；本阶段 READY 即会话稳态）；非法转换与杂散帧按 §5.2 可判定行处置 —— 401 MALFORMED_MESSAGE / 402 INVALID_STATE（按错误性质拆分：格式或通用校验失败 401；合法但状态不允许 402 且**保持连接**，含 PLAYING/READY 下重复 Subscribe → 402 非致命）；CONNECTING 阶段 PairingRequest 等专门规则优先。
- 能力协商（裁决后语义）：`Negotiate`（oneof 7）/ `NegotiateResponse`（oneof 8）；capabilities 数值 ID 升序、唯一、`count(u8) || id(u32 BE)` canonicalization；重复/乱序/>255 → 401；PairingRequest.capabilities = offered set、PairingResponse 回显（若 pairing 在本阶段测试路径中出现）；**NegotiateResponse 是唯一权威会话交集**。
- 请求 id 与响应配对（spec §4）；错误携带 retryability + re-pairing-required；202 AUTHORIZATION_UNKNOWN（未知身份）vs 204 AUTHORIZATION_REVOKED（已撤销）区分 —— 撤销即时生效：新连接拒绝 + 活跃会话 ≤5 s 内终止（TM 行）。
- 超时：连接建立 deadline、AUTH 阶段 deadline（spec §3 deadlines 行）；Transport timeout 作为 close 无法送达时的兜底。
- 诊断日志红线：错误/日志不含凭据、私钥、原始媒体；auth 失败只报码 + 匿名化源。

## Definition of done

- 集成测试覆盖 §5 Server 表**每一行**（含非致命 402 保持连接、幂等/重复分支）+ §6 每个 error band 至少一例。
- 对 `fixtures/wire-protocol-v2/` 的 auth 4-message 流（含 raw auth_context + 签名字节）、202/204、402 非致命、negative-space 5 案例回放字节一致。
- 限流测试（第 6 次失败 → 可判定行为）+ 撤销 ≤5 s 测试。
- v1 回归全绿；v1 可构建可发布。
