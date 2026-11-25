# 04: Threat model and pairing/key-lifecycle specification

**What to build:** the security architecture documents for v2 — a threat model and the pairing/key-lifecycle specification — aligned with spec §Security and Pairing Model. The wire-level framing they depend on comes from 03; security decisions themselves do not depend on 03's layout details, so this ticket only needs 02.

**Blocked by:** 02

**Status:** resolved

**Type:** task

## Requirements

- Threat model (`docs/wire-protocol-v2/threat-model.md`): assets (audio stream, Server identity key, pairing credentials, authorization state), actors, trust boundaries (LAN/VPN reachability; no rendezvous in scope), and the attacks in scope for v2.0: passive capture, active first-connection MITM (addressed by pairing, not by anonymous TLS), rogue Server, unauthorized Client, replay/duplicate/malformed media, resource exhaustion (bounded handshakes, pairing attempts, sessions per Client, total Clients; rate-limited auth failures; small-request/large-response asymmetry), downgrade (a protocol/auth failure never falls back to v1 or plaintext).
- Pairing spec (`docs/wire-protocol-v2/pairing.md`):
  - QR payload structure: protocol major, Server address hint, full Server identity public key or long fingerprint, random ≥128-bit short-lived **single-use** nonce; binary/text encoding chosen and recorded.
  - Live handshake binds identities + nonce + protocol major + capabilities into the authenticated transcript and proves possession of the private key.
  - SAS comparison as the no-camera fallback: ephemeral key exchange + hash commitment; unverified SAS claims no active-MITM protection — stated explicitly.
  - Six-digit codes never stretched into session keys; no custom PAKE in v2.0.
  - Post-pairing persistence: both identities persisted; key change ⇒ explicit re-pairing or confirmation, never silent replacement; Server identity reset invalidates old pairings; no recovery backdoor.
  - Revocation: affects new connections immediately, terminates active sessions promptly.
  - Key storage: Android Keystore where available; Server key/authorization files with explicit permissions, concurrent-access behavior, corruption handling, backup guidance; GUI and `as-cmd` share one pairing Core; headless pairing path with QR where practical and SAS/fingerprint always.
- Test-only auth bypasses confined to test builds/config.

## Definition of done

Draft on the phase-2 branch; maintainer acceptance resolves this ticket (05 needs both 03 and 04).

## Comments

### 2026-11-12 — drafted, awaiting maintainer review

`threat-model.md` + `pairing.md` (both `Status: proposed (2026-11-12)`) on branch `ticket/04-threat-model-pairing` @ `c32a3d7`, merged into `phase-2/wire-protocol-v2` @ `148b8cc`. The files are in the main checkout under `docs/wire-protocol-v2/`.

Headline proposals with empty sign-off cells (TM-1..9 / PR-1..14): Server identity = Ed25519, pin = SHA-256(SPKI DER); Android Client pairing key = ECDSA P-256 in Android Keystore (API-26 rationale; Keystore keys need not be rustls client-cert keys — no mTLS); QR = text URI `audioshare-pair://p1?…` (ECC M, byte mode) carrying a 128-bit nonce with 10-min lifetime, burned on first well-formed PairingRequest, ≤ 8 outstanding, memory-only; transcript binding = SHA-256 TLV canonicalization + TLS-Exporter labels; SAS = 6 digits `mod 10⁶` displayed `123 456`, fingerprint = SPKI SHA-256 in 8×8 hex groups; auth = signed challenge (ECDSA over exporter-bound context), 5 failures/connection + 20/min/address; resource bounds 16 handshakes / 32 sessions (newer replaces older) / 8 pairing / ≤128 B pre-auth responses / ≤512 B errors / 64 KiB control / 256-datagram queues; storage 0600/0700 with refuse-to-enable-v2 on loose perms (Linux; Windows warn-only), atomic writes + 1 backup, corruption fail-closed; revocation ≤ 5 s; test bypasses compile-time only.

Parked judgment calls for the maintainer: Ed25519/P-256 split, burn-on-first-presentation nonce (failed pairing needs fresh QR), newer-session-replaces-older, Windows ACL warn-only, 2⁻³⁰ SAS bound as a human-dependent guarantee.

### 2026-11-12 — maintainer adjudication (covers 03 + 04): revise then sign

整体方向可行；35 项中 28 项可签、7 项修订后签。以下为裁决原文（对 04 的要求）：

1. **Client 持有证明：必须增加**（03 的发现成立）。当前 binding_echo 只证明对方掌握本连接的 TLS Exporter，不证明其持有所申报 Client 公钥对应的私钥。不能依赖后续 AuthProof：Server 在 PairingResult(OK) 前就会持久化身份，随后会话直接进入 NEGOTIATING —— 攻击者可提交他人公钥造成身份错绑。裁决：PairingConfirm 新增 64 B `client_proof`；Client 用申报身份的私钥对「pairing 专用域标签 + binding」签名；Server 必须在持久化身份、返回成功结果之前验证签名；不增加往返。
2. **TM-6 修订**：保留普通 pre-auth 128 B 限制，成功的 PairingResponse 设 320 B Frame.length 独立上限。不得沿用「整个 pairing 由 nonce-gated 豁免」（SAS 没有 nonce）：QR 以有效未用 nonce 为准入门；SAS 以管理员开启本地 pairing 窗口为准入门；普通挑战与拒绝响应仍 ≤128 B 且不携带 detail。
3. **Capabilities**（PR-6/7 修订）：PairingRequest.capabilities = Client 的 offered set；PairingResponse 回显该集合（用于 transcript binding）；NegotiateResponse 是唯一权威的会话能力交集。04 的 ASCII 名称排序 canonicalization 改为与 03 一致的数值 ID 升序、唯一编码。
4. **签核前必须纠正**：
   - 六位 SAS 仅约 19.93 bit，随机匹配概率 ~10⁻⁶，不是文档声称的 2⁻³⁰ —— 保留六位但必须降低安全声称（裁决采纳此选项）；
   - PR-9 双重哈希歧义：明确对原始 auth_context 使用一次 ECDSA-P256-SHA256，不得先 SHA-256 再交 SHA256withECDSA。

签核状态：可签 TM-1–TM-5、TM-7–TM-9；修订后签 TM-6、PR-6、PR-7、PR-9；其余 PR-1–PR-5、PR-8、PR-10–PR-14 可签。修订落地并填签核列后本 ticket resolved。

### 2026-11-12 — revisions landed, ticket resolved

All adjudication items executed on `ticket/03-04-adjudication-fixes` (5569859 pairing+threat-model, 9ac1eae sign-off pass), merged into `phase-2/wire-protocol-v2` @ `63c8a5e`. client_proof (P1363 r||s over domain-labeled TLV binding, verify-before-persist) · TM-6 split rows with per-mode admission gates · capabilities offered/echo/NegotiateResponse-authoritative · numeric-ID canonicalization · SAS claim corrected to 19.93 bits / ~10⁻⁶ per attempt with attempt-bounded guarantee (~50 attempts/10-min → ~5×10⁻⁵) · PR-9 single ECDSA-P256-SHA256 over raw auth_context. All sign-off cells filled (11✅ + 3✏️, zero empty); both doc Status lines flipped to `accepted (2026-11-12, maintainer adjudication)`. Consistency check passed both directions. 05 unblocked.
