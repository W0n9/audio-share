# 03: Normative Wire Protocol v2 specification

**What to build:** the normative protocol specification that production code will implement — control plane, media plane, state machine, error registry, and capability rules. Supersedes nothing; `docs/protocol.md` (v1) remains frozen.

**Blocked by:** 02

**Status:** resolved

**Type:** task

## Requirements

- Suggested location: `docs/wire-protocol-v2/spec.md` (suite root for 03–05 artifacts). One decision table up front recording every choice spec.md left open, each with the prototype evidence and a maintainer sign-off column:
  - default v2 port (prototype: 65531);
  - MTUD/DPLPMTUD policy (quinn default on; races observed);
  - idle timeout / keep-alive defaults (quinn default 30 s);
  - resource-limit defaults (queue caps, message caps, sessions per Client).
- Control plane: length-delimited **Protobuf** messages on the reliable ordered stream (spec §Control-Plane Semantics), default max 64 KiB; explicit message types; opaque request ids; pairing/auth/negotiation complete before media subscription; invalid transitions produce defined errors, never silent ignore.
- State machine: full table for `CONNECTING`, `AUTHENTICATING`, `NEGOTIATING`, `READY`, `PLAYING`, `CLOSING`, `CLOSED` — every legal transition, terminal behavior, and which side may trigger it.
- Error registry: stable numbered codes covering version/capability mismatch, pairing/auth/authz failure, busy/rate limit, malformed message/state, unsupported media configuration, capture failure, stream end, Server stop, normal Client stop, temporary network failure — each with retryability and whether re-pairing is required; transport timeout as fallback when close cannot be delivered.
- Media plane: compact v2 binary header (NOT RTP wire-compatible; reuse RTP semantics for sequence/timestamp/loss/jitter) carrying at least `stream_instance_id`, `config_generation`, sequence, timestamp, required flags; ≥32-bit sequence + 64-bit sample-position timestamp with initial values and wrap behavior specified; network byte order (v1 stays little-endian); timestamp = first PCM sample frame on the capture clock (not Unix time, not send time, not interleaved-sample count); capture reset/restart ⇒ new `stream_instance_id`; every decode-affecting config change ⇒ `config_generation` bump; unknown-configuration media dropped or held in a strictly bounded queue, never decoded with the previous generation.
- Datagram sizing: query the transport DATAGRAM limit, subtract header, align to complete sample frames; react to path-limit reductions; no IP-fragmentation reliance; explicit small-MTU / unsupported-datagram errors.
- Live playback: no app-layer retransmission, no FEC in v2.0; bounded jitter buffer with late-drop counting separate from loss; defined concealment; paced sends with bounded send-ahead; drift measurement + correction strategy; hard-limit re-anchor.
- Diagnostics: the playback-statistics set from spec §Playback Diagnostics, 5 s cadence, best-effort on close, never blocking media.

## Definition of done

Draft on the phase-2 branch; reviewed and accepted by the maintainer; then 05 unblocks.

## Comments

### 2026-11-12 — drafted, reconciled with ticket 04, awaiting maintainer review

`docs/wire-protocol-v2/spec.md` (615 lines, Status: proposed 2026-11-12) on `ticket/03-normative-wire-spec` @ `6c98ef5`, merged into `phase-2/wire-protocol-v2` @ `47acf5f` — the file is in the main checkout. Section map: §1 scope · §2 decision table D1–D12 (empty sign-off column) · §3 transport/framing · §4 control schema · §5 state machine (full Server+Client tables) · §6 error registry (bands 100–700) · §7 media plane (24-byte BE header) · §8 live playback · §9 diagnostics · §10 resource limits · §11 verification · §12 parked items.

Headline proposals: **D1** port 65531/udp · **D2** DPLPMTUD enabled + per-send max_datagram_size sizing (cross-end size assertions banned) · **D3** 30 s idle timeout + 10 s keep-alive, no app heartbeat · **D4** 24-byte header (instance u32, generation u32, sequence u32, timestamp u64, flags u32, all BE) · **D5** oneof envelope bands (connection 1–15, pairing 16–31, subscription/config 32–47, stream 48–63, diagnostics 64–79; field 1 = request_id, field 2 = Status) · **D6** error bands 100–700, 202 AUTHORIZATION_UNKNOWN vs 204 AUTHORIZATION_REVOKED · **D7** rustls default TLS 1.3 suites, X25519, SPKI-SHA-256 pin, Ed25519 Server key + P-256 Keystore Client key + signed-challenge auth (aligned to 04 PR-1/2/9) · **D10** limits adopted from TM-1–8 · **D11** impairment matrix incl. drift ±100 ppm · **D12** jitter target 60 ms / hard limit 200 ms / hold 100 ms·32 dg / send-ahead 100 ms / rate-match ±100 ppm.

The ticket-04 documents landed mid-draft; reconciliation replaced the bearer-credential auth with 04's signed-challenge flow, adopted its identities and rate limits, and expanded pairing to the 4-message exchange. **Flagged back to ticket 04 (parked, §12):** 04's pairing exchange shows no Client proof-of-possession of its identity key before `PairingResult`; TM-6's 128 B pre-auth bound vs ~250 B PairingResponse (nonce-gated exemption — needs confirmation); PairingRequest.capabilities = offered-set vs 04's "negotiated" wording; idempotent Unsubscribe and duplicate-Subscribe→402-without-close choices.

### 2026-11-12 — maintainer adjudication (covers 03 + 04): revise then sign

完整裁决见 ticket 04 的同名 Comments。对 03 的同步要求：

1. **Client 持有证明**：§4 控制 schema 的 PairingConfirm 新增 64 B `client_proof`（Client 以申报身份私钥对「pairing 专用域标签 + binding」签名）；Server 在持久化身份/返回成功前必须验证；不增加往返。
2. **TM-6 对齐**（D10 修订）：普通 pre-auth 响应 ≤128 B（挑战/拒绝/错误，不携带 detail）；成功 PairingResponse 独立上限 320 B Frame.length；QR 准入门 = 有效未用 nonce，SAS 准入门 = 管理员开启的本地 pairing 窗口。
3. **Capabilities（D5 修订）**：PairingRequest.capabilities = offered set；PairingResponse 回显（transcript binding 用）；NegotiateResponse 为唯一权威交集；canonicalization 统一为数值 ID 升序、唯一编码。
4. **两个协议选择（已批准，§5 需补齐）**：重复 Unsubscribe 幂等返回 OK；重复 Subscribe 返回 402 但**不断开连接** —— §5 状态机需补 PLAYING 状态的重复 Subscribe 分支，并明确允许「返回非致命 Error、保持连接」的响应类别。
5. **SAS 声称修正（D9 对齐 04）**：六位 SAS ≈ 19.93 bit、单次匹配 ~10⁻⁶ —— 降低安全声称（不写 2⁻³⁰）。

签核状态：可签 D1–D4、D6–D8、D11、D12；修订后签 D5、D9、D10。修订落地并填签核列后本 ticket resolved；05 待两张工单都 resolved 后解锁。

### 2026-11-12 — revisions landed, ticket resolved

Spec synced to the adjudication on `ticket/03-04-adjudication-fixes` @ `9ac1eae` (f7131ab spec sync), merged into `phase-2/wire-protocol-v2` @ `63c8a5e`: §4.4 PairingConfirm + client_proof (field 3, verify-before-persist) · offered-set/echo capability semantics + malformed-list validation (401) · §5.2 PLAYING duplicate-Subscribe → non-fatal 402 (connection kept) + idempotent Unsubscribe + explicit non-fatal-vs-fatal Error class (fatality is a property of the state-machine row, not the code) · admission gates in CONNECTING PairingRequest rows · D5/D9/D10 revised · one self-created contradiction (§5.4 invariant 1) found and fixed. Sign-off cells 9✅ + 3✏️, zero empty. 05 unblocked.
