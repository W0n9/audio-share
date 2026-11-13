# 07: Server authorization, state machine, and capability negotiation

**What to build:** Server 侧的认证与控制平面语义：签名挑战认证全流程、§5 状态机 Server 表、能力协商、速率限制与撤销。交付后 Server 拥有完整的「最小认证控制通道」服务器半边（媒体为 ticket 10+ 的后续阶段）。

**Blocked by:** 06

**Status:** resolved

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

## Comments

### 2026-11-12 — implemented on `ticket/07-server-auth-state-machine`

Branch `ticket/07-server-auth-state-machine`, final code commit `7174d9b` (slices `6b3e5c3` auth primitives → `3eb127c` authorization store → `1a6fb84` §5.2 state machine → `ef9b201` box `Event::Frame` + echo request ids → `7eee34e` machine-driven fixture replay → `d546693` live-QUIC driver with TM-5 limits and supersede → `fbf04ba` as-cmd session wiring → `2d0e5d1` READY Subscribe over QUIC → `8eeccb7` pairing-401 replay → `7174d9b` compile-time gating of test-only bypasses), based on the ticket claim `43fc72b`. DoD verified with `cargo test --workspace` at every slice (v1 regression clean at each commit point) and `cargo clippy --workspace --all-targets` warning-free.

**Modules** (all inside ticket 06's `crates/v2-transport`; no new crate):
- `src/auth.rs` — auth_context TLV builder (`TLV(1,"auth") || TLV(2,server_random) || TLV(3,client_pubkey) || TLV(4,server_pubkey) || TLV(5,major u32 BE) || TLV(6,TLS-Exporter)`, pairing.md §9.1), PR-9 verification (one ECDSA-P256-SHA256 over the RAW context bytes, 64-byte r||s P1363), SEC1-compressed key parsing with a strict `0x02/0x03` prefix guard (p256's `from_sec1_bytes` accepts `0x05`-prefixed points with a valid x), and the capability-list rules (ascending, unique, ≤ 255) plus `count(u8) || id(u32 BE)` canonicalization.
- `src/auth_store.rs` — `<data dir>/pairing/clients.json` per pairing.md §8.1: public data only, `client_id` = first 8 bytes of SHA-256(pubkey), base64url keys, 0600 file / 0700 dir, atomic write + one backup generation, corruption fail-closed and never regenerated. `authorize()` is consulted at every authentication and every revocation poll — never a cached answer.
- `src/state_machine.rs` — a pure, synchronous §5.2 machine; the driver feeds one `Event` per occurrence and receives ordered `Outcome`s (frame + close code + transport-close code).
- `src/driver.rs` — live QUIC wiring: first Client-initiated stream is the control stream, 10 s auth deadline, TM-5 source-address budget, per-identity session registry (303), revocation polling, CLOSING grace, and the log red lines.
- `crates/as-cmd/src/main.rs` — the accept loop no longer parks connections: it opens the store (fail-closed: a corrupt store refuses v2 while v1/capture continue), builds one `SessionServer`, spawns `serve_connection` per admitted connection, and releases the capacity slot when the session ends.

**Test counts** (`cargo test --workspace`): 435 passed / 0 failed across 37 binaries (4 ignored platform-only tests; ticket-06 baseline was 353). v2-transport = 145: lib 70 + `state_machine` 27 + `state_machine_replay` 19 + `auth_primitives` 9 + `auth_store` 9 + `session_driver` 8 + `golden_vectors` 3. as-cmd = 23 (5 lib + 12 cli + 4 `v2_server` live-process + 1 `replay_fixture` v1 + 1 `server_process`).

**§5.2 Server table coverage** (pure rows in `tests/state_machine.rs`, byte-exact fixture rows in `tests/state_machine_replay.rs`, live QUIC in `tests/session_driver.rs`):
- CONNECTING: `AuthInit` → `AuthChallenge` (CSPRNG; fixed only under `test-support`); `PairingRequest` → malformed capability list 401 (`>255`, duplicates — the §4.4 decidability order, fixtures ×2) else `PairingResult`(201)+`Close`(201); malformed first frame → `Error`(401)+`Close`(401) (undecodable, unset body, zero-length prefix, oversize — fixtures); valid-but-forbidden → 402 fatal; stray `AuthProof` → 402 fatal; second Client stream → `Error`(402)+`Close`(402)+transport 402 (live, watched by a second-stream watcher task).
- AUTHENTICATING: valid + authorized → `AuthResult`(OK, session_id) byte-identical to the fixture (raw auth_context and signature bytes); 202 unknown vs 204 revoked/denied (retryability + re-pairing flags from the §6 registry); 203 invalid signature; 5th failure on the connection → `Close`(203) alone (live: attempts 1–4 answer `AuthResult`(203), the 5th is `Close`(203)); `AuthInit` retry → fresh challenge; source budget → `Close`(302).
- NEGOTIATING: happy path → `NegotiateResponse`(OK, server minor, empty accepted set) + `ConfigurationPublished` push, byte-identical; major ≠ 2 → `Error`(101)+`Close`(101) with request-id echo; malformed capability list → 401; unknown ids ignored (intersection empty — v2.0 has no optional capabilities); any other frame → 402.
- READY: `Subscribe` → `SubscribeResponse`(OK + current configuration) and `StreamStarted` only when a capture instance is active; duplicate `Subscribe` → non-fatal `Error`(402) keeping the connection (the fixture then serves `RequestConfiguration`); unhealthy capture → non-fatal 501; `Unsubscribe` idempotent OK; `RequestConfiguration` → `ConfigurationPublished` echo; `PlaybackDiagnostics` accepted silently; wrong-state frames → 402 fatal.
- any: advisory non-fatal `Error` ignored; fatal → matching `Error`+`Close`, processing stops; `Close` received → no echo, CLOSING; idle/transport timeout → transport close 601; transport lost → CLOSED (classified 601); `RevocationCheck` → `Error`(204)+`Close`(204)+transport 204.
- CLOSING: accepts no frames; the 5 s grace forces a transport close carrying the Close code (live-asserted). A duplicate-fatal-pair bug surfaced by the live revocation test led to an explicit CLOSING guard in `handle()`: a repeated revocation poll must not re-emit the pair after the stream was finished (it previously wrote a second pair, failed with `ClosedStream`, and dropped the connection with implicit code 0).
- Rows outside this phase by design: pairing admission success (`PairingResponse` minting), `PairingConfirm`, the 10-min pairing deadline (206) and pairing-failure rate, PLAYING/media rows and capture pushes (ticket 10+). `capture_active` / `capture_healthy` are the configuration seams later tickets drive.

**Golden-vector replay** (`tests/state_machine_replay.rs`, 19 cases, spec §11.1 contract item 4): inputs are the fixture's own `body_hex` decoded through the production `decode_body` (C→S steps are never re-encoded), and every emitted S→C frame is compared via `encode_body` against the fixture's `body_hex`, in order, with no extra frames. Covered byte-identically: auth 4-message happy path (including the raw auth_context + signature bytes), 202, 203 (the fixture's corrupted signature), 204 denied and 204 revoked-active, 303 supersede (new session id 2 + `Close`(303) on the old), negotiate happy path and unknown-capability, duplicate-`Subscribe` non-fatal 402, 501 unhealthy capture, the negative-space 402 ×3 and 401 ×1 cases, the two pairing capability-list 401 cases, unknown-field-ignored (prost drops field 15), unknown-oneof → framing-level 401, the four framing-level 401 fixtures, 101, 701, and the client-reported Error+Close rows. Cases whose decisive step lives outside the machine are recorded as skipped with reasons: capacity 301, address-rate 302, transport rows 601/602/599, and the 201-declined pairing case (its `PairingResult` is minted by the phase-6 admission gate); the live driver tests cover those.

**201-flag adjudication (ticket 06's flagged question) — stance:** the sender follows the §6 registry default. §4.2 says senders MUST equal the registry defaults, so the machine emits `PairingResult`(201) through `Status::registry(201)`, i.e. with `requires_repairing=true`; the declined-201 fixture body (`9a010e09…c901`) encodes the code only (proto3 defaults), so a sender-byte comparison against that literal deliberately fails. Per the work order the declined-201 case's sender assertion is kept separate from its receiver assertion — here the case is not replayed through the machine at all (the decisive `PairingResult` is phase-6 material), and the fixture was NOT modified. The comment at the CONNECTING `PairingRequest` row records this stance. The maintainer must still rule: either the vector gains the flag and both replay harnesses flip together, or the literal form is authoritative and §4.2's sentence needs a carve-out.

**Key decisions:**
- Machine is pure and synchronous; the driver owns I/O and time. Outcomes carry `frame`, `close_code`, and `transport_close_code`, so `flush()` writes frames, finishes the control stream, and arms the CLOSING grace; transport-close-only rows (301/601) close the transport directly.
- TLV(6) is `ServerConfig::auth_exporter`: the driver always fills the real per-connection `export_keying_material(..., "EXPORTER audio-share v2 auth", b"")` output — empty context, matching the Android client's derivation — and an exporter failure closes 599 instead of running an authentication that can never succeed.
- Rate limits layer deterministically: 5 failures per connection inside the machine (`Close`(203)); 20 per sliding 60 s per source in the driver (`AddressRateGuard` → `Close`(302)). A failure is counted on both layers; on the crossing event the address verdict replaces the machine's outcome, so the 20th failure is always 302 (live: 3 connections × 5 failures end 203, the 4th connection's 20th failure is 302, and a 5th connection is refused at `AuthInit` with 302).
- Per-Client registry: identity → live session; auth success registers and a newer session sends `Superseded` to the old one's driver → `Close`(303)+transport 303. `release()` only removes its own entry, so a superseded session cannot evict its replacement.
- `CapacityGuard` is now `Clone` over `Arc<Mutex<Counters>>`: the accept loop and the per-connection tasks share one guard and `session_ended()` finally runs when a driver returns. Before this, as-cmd never released slots — 32 lifetime connections would have refused everyone (latent ticket-06 bug).
- Log red lines: auth failures log only the §6 code plus a masked source (IPv4 `a.b.x.x`, IPv6 first two segments); no keys, proofs, auth_context, or media bytes appear in any log or error path.
- Test-only bypasses are compile-time gated per pairing.md §11.3: `ServerConfig::server_random_override` and `AuthorizationStore::insert_for_tests` exist only under the `test-support` feature. `auth_exporter` remains a normal field because the driver supplies the real per-connection value in production.

**Leftovers / NOT RUN:**
- Pairing production flow (admission gates, `PairingResponse` minting, `PairingConfirm`, 206 deadline, pairing rate limits) is phase 6; CONNECTING currently declines with `PairingResult`(201)+`Close`(201) and persists nothing.
- Media plane rows (`PLAYING`, `StreamStarted`/`StreamEnded`, config-generation pushes) are ticket 10+; `capture_active`/`capture_healthy` are the seams those tests will drive.
- Backup naming: pairing.md §8.1 says `clients.json.bak`, but the shared atomic-write helper names backups `<stem>.bak`, so the store's backup is `clients.bak` (same convention as `server-identity.bak`). Recorded here; aligning either side is a one-line follow-up.
- `protocol-harness replay --fixture fixtures/wire-protocol-v2` stays v1-only (its `Manifest.source` is typed as a string; the v2 manifest uses an object) — v2 replay lives in `tests/golden_vectors.rs` and `tests/state_machine_replay.rs`.
- The 10 s auth deadline is tested with a shortened override; the real 10 s value is not timed in CI. Windows ACL/log paths remain unvalidated on a Windows host (ticket-06 leftover).

### 2026-11-13 — maintainer adjudication (201 flag + 402 wording)

**201 `requires_repairing` — the vector was the bug, spec prevails.** `control/error-201-pairing-declined.json` was missing the flag: its `Status` encoded only `code=201` while the same-code sibling `error-201-pairing-proof-invalid.json` encoded `requires_repairing=true` (`18 01`). §4.2 is hard ("senders MUST equal the §6 registry defaults") and §6.2's 201 row has re-pairing = yes, with no carve-out. The vector is now corrected to `12 05 08 c9 01 18 01` (frame `00000013…`), and both replay harnesses (tickets 06/08) read the flag from `decoded`/bytes. The machine already emitted `Status::registry(201)`; no production code change was needed.

**402 fatality — Requirements wording was imprecise, code was correct.** The Requirements line "合法但状态不允许 402 且保持连接" over-generalized "keep the connection" to every 402. Per §5.2/§6.2 the fatality of 402 is decided by the state-machine row, not the code: the only non-fatal 402 is a duplicate `Subscribe` in READY/PLAYING; every other 402 is fatal (Error + Close, connection dropped). All four 402 scenarios in the implementation are correct as built.
