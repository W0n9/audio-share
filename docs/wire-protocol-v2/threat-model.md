# Wire Protocol v2 — Threat Model

Status: accepted (2026-11-12, maintainer adjudication)

Feature: wire-protocol-v2 · Ticket: `.scratch/wire-protocol-v2/issues/04-threat-model-pairing-spec.md` · Companion document: [pairing.md](pairing.md)

## 0. Status and normativity

- This document is **accepted** (maintainer adjudication 2026-11-12; sign-off table in §10). It is the security reference for v2.0.
- Binding requirements are taken from the design baseline [`.scratch/wire-protocol-v2/spec.md`](../../.scratch/wire-protocol-v2/spec.md) — cited below as "design baseline §…". Every MUST there is binding here.
- The accepted transport posture is ADR-0004 ([`../adr/0004-wire-protocol-v2-transport.md`](../adr/0004-wire-protocol-v2-transport.md)): quinn + rustls/TLS 1.3, SPKI pinning, control-stream authorization, 0-RTT disabled, no downgrade.
- Field layouts, message numbering, and error-code values are **not** decided here; they are frozen in the normative wire specification at `docs/wire-protocol-v2/spec.md` (ticket 03). This document references that path only and does not depend on its contents.
- The key words MUST, SHOULD, and MAY are to be interpreted as described in RFC 2119. Where this document says "MUST" without a citation, it is a proposal of this document and inherits the same sign-off status.

## 1. System scope

Wire Protocol v2 carries a Server's captured audio to Android Clients as PCM, with one reliable QUIC control stream and unreliable QUIC DATAGRAM media, over a directly reachable Server address. The security goal of v2.0 is narrow and concrete: an unauthenticated network peer gains nothing, a paired Android Client gains only the audio player role, and a failure never silently weakens security.

In scope for v2.0 (design baseline §Network Scope, §Client and Role Scope):

- Directly reachable Server address: LAN, VPN, or a user-configured port forward. IPv4 and IPv6.
- Exactly one remote role: **player** — connect, subscribe to audio, receive required state, report playback diagnostics, disconnect.
- Local administration only: endpoint selection, capture configuration, pairing administration, and Server administration are performed in the Server GUI or `as-cmd` on the same computer as the Server Core.

Out of scope for v2.0 (design baseline §Network Scope): STUN/TURN, NAT hole punching, public rendezvous, relays, and an account system. Consequently there are no Internet-facing authentication endpoints, no shared secrets with third parties, and no third-party trust anchor in the v2.0 model. Wire Protocol v1 is a frozen compatibility adapter on its own port (design baseline §Dual-Protocol Migration); attacks against v1 are out of scope for this document (§9).

## 2. Assets

| Asset | What it is | Impact of compromise |
|---|---|---|
| Audio stream | The PCM captured from the Endpoint and carried to Android Clients (design baseline §Media-Plane Semantics) | Confidentiality: playback content (whatever the computer plays) leaks to an eavesdropper. Integrity: injected or substituted audio plays on paired Clients. |
| Server identity key | The Server's persistent Ed25519 private key; the self-signed TLS certificate and the SPKI that Android Clients pin are derived from it (design baseline §Security and Pairing Model — Identity and Authorization) | Holder can impersonate the Server to every paired Android Client and complete pairings as a false Server. Recovery requires identity reset and re-pairing of all Clients (pairing.md §9). |
| Pairing credentials and nonces | The single-use 128-bit QR pairing nonce; the Client identity signing keys on Android devices; per-connection SAS values (pairing.md §5–§7) | A live unspent nonce plus physical sight of the QR allows pairing a device the operator did not intend. A Client identity key allows authenticating as that Client until revoked. |
| Authorization state | The Server's pairing store: per-Client identity records, display names, active/revoked state (pairing.md §8) | Tampering admits unauthorized Clients or locks out legitimate ones. Leak exposes the list of paired devices (minor privacy, no secrets — the store holds public keys and metadata only). |
| Local diagnostics | Logs, playback statistics, exported diagnostic bundles (design baseline §Playback Diagnostics, §Resource and Privacy Controls) | Leak of addresses, device names, and error detail. MUST stay local, MUST be exportable only with redaction, and MUST NOT contain secrets, raw PCM recording, or secret logging. |

## 3. Actors

| Actor | Description | Assumed trust |
|---|---|---|
| Server operator | The person at the computer running Server Core, administering it through the Server GUI or `as-cmd` | Trusted root of the pairing decision. Every out-of-band trust transfer (QR scan, SAS comparison) resolves to this person's judgment. |
| Android Client owner | The person pairing a phone. Usually the same person; not assumed to be, in shared settings | Untrusted until paired: a not-yet-paired device is treated as a network attacker (§3, row below). After pairing it holds exactly the player role. |
| Network attacker | Any host on the LAN/VPN path or beyond a user port-forward. Passive (observes) or active (sends, modifies, drops, replays, terminates connections; uses arbitrary source addresses) | Controls its own packets and timing. Cannot read the Server's screen or files, cannot compromise either endpoint's OS, cannot force a human to scan a QR or compare codes. |
| Rogue Server | A Server-like endpoint operated by the network attacker, including one impersonating the real Server at the real address | Untrusted. Defeated by SPKI pinning unless the operator completes pairing against it (documented limit, §5.3). |
| Paired-but-malicious Android Client | An authorized player that misbehaves within the protocol | Holds the player role only. Bounded by per-Client resource budgets (§6). Gains no administrative capability, no other Client's credentials, no Server key material. |
| Out-of-model actors | Physical intruders, OS-level malware, LAN infrastructure operators | Out of scope (§9). |

## 4. Trust boundaries

| # | Boundary | What crosses | Controls on the crossing |
|---|---|---|---|
| TB1 | Network → Server Core (v2 UDP/QUIC port, separate from v1's port) | QUIC datagrams: TLS 1.3 handshake, control-stream Protobuf messages, media DATAGRAMs | TLS 1.3 only; control-stream authorization as a separate check from encryption; length validation before allocation; bounded queues; rate limits (§6) |
| TB2 | Network → Android Client | Same QUIC datagrams | TLS 1.3 with the SPKI pinned through pairing; no trust in network-provided addresses — a discovered (e.g. mDNS) name is not a trusted identity (design baseline §Network Scope) |
| TB3 | Local administrator → Server Core (Server GUI / `as-cmd` on the same computer) | Pairing administration, authorization changes, identity reset | Trusted local channel; this is NOT a protocol boundary in v2.0 — there is no remote administration role |
| TB4 | Server Core → host filesystem (identity key file, authorization store) | Key and store reads/writes | Explicit file permissions (pairing.md §10); exclusive-lock concurrency; corruption handling fail-closed (pairing.md §10.3) |
| TB5 | Android app process → device OS / Android Keystore | Signing operations with the non-exportable Client identity key | Keystore protection; malicious OS out of scope (§9). The v2.0 boundary: Keystore keys need NOT act as rustls TLS client-certificate keys — authorization is an application-layer check over the encrypted control stream (design baseline §Security and Pairing Model — Identity and Authorization) |
| TB6 | Human → pairing UI (QR scan, SAS comparison) | The out-of-band trust transfer itself | Integrity of the transfer depends on the human: the QR must come from the Server's own screen; the SAS must actually be compared. The documented limits of the SAS path live at this boundary (§5.2, pairing.md §7) |

## 5. Attack analysis

Summary mapping; each row is analyzed below with its mitigation and its documented limit.

| # | Attack | v2.0 mitigation | Documented limit |
|---|---|---|---|
| A1 | Passive LAN capture | TLS 1.3 on all application data, control and media (§5.1) | Traffic analysis remains possible (out of scope, §9) |
| A2 | Active first-connection MITM | QR pairing (SPKI pin from QR + single-use nonce) or verified SAS (§5.2) | An UNVERIFIED SAS comparison protects against nothing; protection requires actual comparison plus confirmation on both ends |
| A3 | Rogue Server | SPKI pinning; anonymous self-signed certificates are insufficient (§5.3) | If the operator completes pairing against a rogue Server, v2 cannot detect it — pairing is an out-of-band trust transfer |
| A4 | Unauthorized Client | Control-stream authorization, separate from encryption (§5.4) | A previously paired Client whose key was copied stays authorized until revoked |
| A5 | Replay / duplicate / malformed media | Bounded consumption: validation before allocation, generation checks, bounded queues (§5.5) | Within its per-Client budget, a malicious Client still consumes bounded resources by design |
| A6 | Resource exhaustion | Bounded handshakes, pairing attempts, sessions per Client, total Clients; rate-limited auth failures; no small-request/large-response asymmetry (§5.6, §6) | Bandwidth-level flooding of the UDP port is not distinguishable from congestion; availability against a network-level flood is out of scope |
| A7 | Downgrade | Explicit ALPN-major failure; mandatory security capabilities cannot be negotiated off; no fallback of any kind (§5.7) | v1 itself, when the operator explicitly enables it, is plaintext by design — that is an accepted, labeled operator choice, not a v2 fallback |
| A8 | Key compromise lifecycle | Rotation = re-pairing; identity reset invalidates pairings; no recovery backdoor (§5.8) | Compromise of an identity key is detectable only by the operator; v2 has no out-of-band revocation telemetry |

### 5.1 Passive LAN capture (A1)

An attacker observes all traffic between the Server and Android Clients on the shared LAN/VPN segment.

**Mitigation (v2.0).** Every application byte — control-stream Protobuf messages and media DATAGRAMs alike — is carried inside QUIC with TLS 1.3 (RFC 9001); TLS 1.3 only, no renegotiation, 0-RTT disabled so no early data is observable or replayable (design baseline §Security and Pairing Model; ADR-0004 decision 1 and 3). A passive observer sees connection metadata (QUIC header fields), packet sizes, and timing, but not PCM, credentials, capability lists, or pairing payloads.

**Documented limit.** Traffic analysis (§9) can still reveal that playback is occurring and roughly when and how much. v1 on its own port remains unencrypted by design; anyone wanting confidentiality must use v2 (design baseline §Dual-Protocol Migration: v1 is labeled "trusted networks only; no encryption or authentication").

### 5.2 Active first-connection MITM (A2)

An attacker terminates both TLS legs on the first connection, presenting its own certificate to the Android Client and its own client-facing connection to the Server, and relays application traffic.

**Mitigation (v2.0).** Two paths, per pairing.md:

- **QR pairing (primary).** The Android Client derives the pin from the QR payload's full Server identity public key before any key exchange; the attacker cannot present a certificate whose SPKI matches without the identity private key. The single-use nonce inside the QR additionally binds the handshake to the pairing session just displayed (pairing.md §4–§6). The prototype verified the negative case on a real device: a wrong SPKI pin is rejected at the cryptographic handshake (prototype report §1, gate 3).
- **SAS fallback (no camera).** SAS derived from the completed ephemeral key exchange with hash commitment (pairing.md §7). Protection exists **only if the user actually compares the two displayed codes and confirms on both ends**.

**Stated plainly.** An UNVERIFIED SAS comparison does not protect against an active first-connection MITM. If the user skips the comparison and only taps "confirm", the connection is trust-on-first-use with no MITM protection: a terminating MITM completes both legs and both ends confirm. The six-digit SAS carries ≈ 19.93 bits: a verification attempt succeeds with probability ≈ 10⁻⁶ **when actually compared** — not the 2⁻³⁰ earlier drafts claimed (adjudication 2026-11-12). The undetected-MITM probability is **bounded by guess attempts**: the pairing-failure rate limits of §6 (5 failures per sliding 60 s per source address) cap an attacker at ~50 attempts inside the 10-minute pairing window (≈ 5×10⁻⁵ per window per address); compensating controls are those rate limits, the out-of-band SPKI fingerprint path, and the address hint displayed beside the code (pairing.md §7.2).

**Documented limit.** Protection at boundary TB6 is a human property: the QR must come from the Server's own screen (a QR printed by an attacker pairs the Client to the attacker's Server), and the SAS must be read on both screens. The pairing UI mitigates this by displaying the Server's endpoint/address hint next to the code so the operator can notice a mismatch (pairing.md §7.3), but v2 cannot verify what the human scanned or read.

### 5.3 Rogue Server (A3)

An attacker runs a Server-like endpoint, or impersonates the real Server at its address, hoping an Android Client connects to it.

**Mitigation (v2.0).** Android Clients pin the Server's SPKI, obtained through pairing (design baseline §Security and Pairing Model — Identity and Authorization; ADR-0004 decision 3). An anonymous self-signed certificate is insufficient: the Client's certificate verifier fails any certificate whose SPKI hash does not match the pinned value, before any application data. A rogue Server reached by mistake produces a hard, explicit error naming the pin mismatch and requiring re-pairing — it never silently replaces the pin (pairing.md §8.2).

**Documented limit.** SPKI pinning authenticates only a Server the operator actually paired with. If the operator completes the pairing flow against a rogue Server (scanned a rogue QR, or confirmed a rogue SAS without noticing the address hint), the Client is correctly and confidently paired to the wrong party. v2.0 accepts this as an out-of-band human decision, not a protocol failure.

### 5.4 Unauthorized Client (A4)

An unpaired or de-paired host connects to the Server's v2 port and attempts to subscribe to audio.

**Mitigation (v2.0).** Encryption and authorization are separate checks (design baseline §Security and Pairing Model — Identity and Authorization): a successful TLS handshake with a pinned Server never authorizes anything. Authorization rides the encrypted control stream as a signed challenge against the Client's persisted identity key (pairing.md §9); media subscription is refused until pairing/authentication/negotiation complete (design baseline §Control-Plane Semantics). Tests MUST reject both an unpinned Server and an unauthorized Client (design baseline §Security and Pairing Model). Pairing itself now requires proof of possession: the Server verifies the Client's `client_proof` signature over the pairing transcript (pairing.md §6.3.1) BEFORE persisting an identity, so a public-key-submission attacker cannot mis-bind a third party's identity — the earlier `binding_echo`-only design proved only TLS-Exporter possession and is superseded. QUIC connection migration does not bypass this: authorization is bound to the authenticated session, not to the network address.

**Documented limit.** Authorization is binary and pairwise, not capability-fine: every paired Client receives the same audio stream (design baseline §Client and Role Scope — one shared capture feeds independently authenticated Client sessions). A paired Client whose device was copied or stolen stays authorized until the operator revokes it (pairing.md §9).

### 5.5 Replay, duplicate, and malformed media (A5)

An attacker, or a misbehaving path, injects replayed datagrams, duplicates, or malformed media into a Client's receive path; or a paired Client sends malformed control messages to the Server.

**Mitigation (v2.0).** Bounded consumption (design baseline §Resource and Privacy Controls, §Media-Plane Semantics):

- Message and datagram lengths are validated before allocation; queues and incomplete handshakes are bounded (§6).
- Media carries `stream_instance_id`, `config_generation`, a ≥32-bit sequence, and a 64-bit capture-clock timestamp; unknown-generation media is dropped or held in a strictly bounded queue while configuration is requested, never decoded with the previous generation.
- Duplicates: packet-level duplicates are absorbed by QUIC packet-number deduplication (prototype report §5); any application-level duplicate still passes through bounded accounting and is counted.
- Malformed control messages produce a defined protocol error or close action, never silent ignore (design baseline §Control-Plane Semantics).

**Documented limit.** A malicious paired Client can still spend its own per-Client resource budget (bounded queue, bounded session) and can send garbage up to the rate limits; the design guarantees containment and accounting, not zero resource use.

### 5.6 Resource exhaustion (A6)

Unauthenticated network peers attempt to exhaust the Server: endless handshakes, pairing-guessing, auth-guessing, or provoking large error responses with tiny requests.

**Mitigation (v2.0).** All of the following are binding (design baseline §Resource and Privacy Controls) with the concrete default values proposed in §6:

- Bounded concurrent handshakes, bounded concurrent pairing sessions, bounded sessions per Client identity, bounded total Clients.
- Rate-limited authentication and pairing failures, per connection and per source address.
- No small-request/large-response asymmetry: general pre-authentication responses are a fixed small bound (≤ 128 B; the one exception, a successful `PairingResponse`, has its own 320 B cap — §6), error payloads omit secrets and stack traces, and an unauthenticated request never triggers work that is a large multiple of itself.

**Documented limit.** A network-level flood of the Server's UDP port is indistinguishable from congestion at this layer; the Server's obligations are the bounds above plus correct behavior for legitimate Clients under loss (design baseline §Live Playback and Recovery). Availability against a flood is out of scope (§9).

### 5.7 Downgrade (A7)

An attacker or a misconfiguration attempts to make a v2 failure fall back to v1, plaintext, anonymous encryption, or a weaker security mode.

**Mitigation (v2.0).** A protocol or authentication failure never downgrades to v1, plaintext, anonymous encryption, or a weaker mode (design baseline §Security and Pairing Model — Identity and Authorization). Concretely:

- The Wire Protocol major rides ALPN (`audio-share/v2`); a major mismatch fails explicitly before any application data (design baseline §Connection Rules; ADR-0004 decision 1). The QR payload's protocol-major field must equal the ALPN major or pairing fails (pairing.md §4.1).
- Mandatory security capabilities cannot be negotiated off; capabilities negotiate optional features only within a compatible major (design baseline §Connection Rules).
- 0-RTT is disabled for this major; pairing, authorization, and control operations are never accepted as early data (ADR-0004 decision 3).
- New Android Clients prefer v2 and never automatically fall back to v1 after a v2 failure (design baseline §Dual-Protocol Migration).

**Documented limit.** v1, when the operator explicitly enables it, is plaintext by design and labeled accordingly (design baseline §Dual-Protocol Migration). That is an accepted operator choice on a frozen protocol, not a v2 fallback path; v2 code contains no code path that switches to v1.

### 5.8 Key compromise lifecycle (A8)

The Server identity key, or a Client identity key, is copied or otherwise compromised.

**Mitigation (v2.0).**

- **Server key compromise.** The operator resets the Server identity: a new key is generated and all existing pairings are invalidated in the same atomic step (pairing.md §8.3). Every Android Client then sees a pin mismatch on its next connection and is prompted to re-pair explicitly — a new key is never silently accepted (design baseline §Security and Pairing Model — First Pairing).
- **Client key compromise.** The operator revokes the Client: new connections are refused immediately and affected active sessions are terminated promptly (pairing.md §9).
- **Rotation = re-pairing.** There is no in-place key rollover protocol in v2.0; rotation of either identity is a reset plus explicit re-pairing (pairing.md §8).
- **No recovery backdoor.** There is no universal recovery credential, vendor key, or unauthenticated recovery mode (design baseline §Security and Pairing Model — Key Lifecycle and Local Administration). The only recovery paths are local administration plus physical re-pairing.

**Documented limit.** v2 has no out-of-band channel by which a Client could learn its Server's key was reset by someone else, or its own key was copied: detection depends on the operator or on the affected human noticing the re-pairing prompt.

## 6. Resource-exhaustion limits (defaults)

All values are Server defaults, overridable by configuration within documented ranges; the normative values are frozen in `docs/wire-protocol-v2/spec.md` (ticket 03). These are this document's decisions, signed off in §10 (design baseline §Resource and Privacy Controls).

| Limit | Proposed default | Rationale |
|---|---|---|
| Concurrent TLS/QUIC handshakes in progress | 16 | Bounded memory for incomplete handshakes; surplus connections rejected immediately |
| Total established v2 Client sessions | 32 (range 1–256) | Small-LAN product; bounds capture fan-out and memory |
| Sessions per Client identity | 1 active; a newly authenticated session from the same identity replaces the older one (reconnect/migration path), and the replacement is counted in diagnostics | Supports the migration fallback (ADR-0004 decision 4) without doubling state |
| Authentication failures per connection | 5, then close with an authorization-failure error | Brute-force containment per connection |
| Authentication failures per source address | 20 per sliding 60 s window | Slows distributed offline guessing; generous for real reconnects |
| Concurrent pairing sessions | 8 | Bounded state for pending pairings |
| Outstanding pairing nonces | 8; oldest evicted beyond that | Bounds nonce-table memory; matches at most 8 live QRs |
| Pairing-session lifetime | 10 min | Generous human time; closes the SAS/QR window |
| Pairing failures per source address | 5 per sliding 60 s window | Pairing is human-paced; failures signal attack or misconfiguration |
| General pre-authentication response size | ≤ 128 B — challenges, rejections, errors; no `detail` field | No amplification before trust is established |
| Successful PairingResponse size | ≤ 320 B `Frame.length` — the sole larger pre-auth response | Pairing admission is gated per mode, not by response size: QR pairing requires a valid, unused nonce; SAS pairing requires the administrator-enabled local pairing window (SAS has no nonce) (pairing.md §5, §7.3) |
| Error payload size | ≤ 512 B, stable codes only (registry owned by `docs/wire-protocol-v2/spec.md`) | No large error responses; no secrets, internal paths, or stack traces |
| Control-message maximum | 64 KiB | Design baseline §Control-Plane Semantics default |
| Per-Client application media queue | 256 datagrams (prototype value) | Honest application-level drop accounting (prototype report §5) |

## 7. Prohibited fallbacks (downgrade checklist)

For every failure mode below, the required behavior is identical: explicit failure with a stable error code that carries retryability and whether re-pairing is required (design baseline §Control-Plane Semantics). The following transitions MUST NOT exist in v2.0 code (design baseline §Security and Pairing Model):

| Failure | MUST NOT fall back to |
|---|---|
| v2 handshake, protocol, or capability failure | v1, plaintext, or any unauthenticated mode |
| SPKI pin mismatch | Accepting the presented certificate; silent pin replacement |
| Authorization failure | Anonymous or read-only access; continuing media delivery |
| Negotiation failure of a mandatory security capability | Proceeding without that capability |
| 0-RTT / early data | Accepting pairing, authorization, or control operations as early data |

## 8. Key-compromise lifecycle summary

| Event | Required v2.0 behavior | Where specified |
|---|---|---|
| Server identity key suspected/known compromised | Operator resets Server identity: new key generated, all pairings invalidated atomically; all Clients prompted to re-pair; old key never accepted anywhere | pairing.md §8.3 |
| Client identity key suspected/known compromised | Operator revokes the Client: new connections refused immediately, active session terminated within ≤ 5 s | pairing.md §9 |
| Routine key rotation | Not a protocol feature in v2.0 — rotation is reset + re-pairing for every affected pairing | pairing.md §8 |
| Server reinstall / data loss | Identity key is not silently regenerated into a working system: loss of the key file is an identity reset, and Clients re-pair | pairing.md §10.4 |
| Recovery from lockout | Local administration only (Server GUI / `as-cmd`); no universal recovery credential exists | design baseline §Security and Pairing Model |

## 9. Out-of-scope threats

Explicitly out of scope for the v2.0 threat model. They are not silently ignored — each is recorded here so the boundary is decidable:

| Threat | Why out of scope |
|---|---|
| Physical device compromise (stolen or unattended unlocked phone; shoulder-surfing the QR or the SAS; tampering with the Server host) | No protocol can defend an attacker holding an unlocked endpoint; the QR/SAS trust transfer is human by definition (TB6) |
| Malicious OS / malware on either endpoint (rooted Android, KernelSU, infected Server host, memory scraping) | The trust boundaries assume the endpoint OS preserves process and Keystore isolation; defeating that defeats any application-layer protocol |
| Traffic analysis / metadata (packet timing and sizes reveal that playback occurs, its volume, and roughly when) | QUIC header protection leaves timing metadata; full metadata hiding is a different product goal and conflicts with real-time transport (design baseline §Goals and Priorities) |
| Network-level flooding of the Server's UDP port | Indistinguishable from congestion at this layer (§5.6) |
| Attacks on v1 | v1 is a frozen compatibility adapter labeled "trusted networks only; no encryption or authentication" (design baseline §Dual-Protocol Migration) |
| Side channels via the host (audio audible through other applications, OS audio routing) | Outside the Wire Protocol boundary |

## 10. Decisions proposed

Signed off by maintainer adjudication 2026-11-12; see tickets 03/04 Comments.

| ID | Decision | Proposed value | Basis | Maintainer sign-off |
|---|---|---|---|---|
| TM-1 | Concurrent handshakes in progress | 16 | §6 | ✅ accepted (2026-11-12) |
| TM-2 | Total established Client sessions | 32 default, range 1–256 | §6 | ✅ accepted (2026-11-12) |
| TM-3 | Sessions per Client identity | 1 active; newer authenticated session replaces older, replacement counted | §6 | ✅ accepted (2026-11-12) |
| TM-4 | Auth failure rate limits | 5 per connection then close; 20 per sliding 60 s per source address | §6 | ✅ accepted (2026-11-12) |
| TM-5 | Pairing bounds | 8 concurrent pairing sessions; 8 outstanding nonces; 10 min lifetime; 5 failures per 60 s per address | §6, pairing.md §5 | ✅ accepted (2026-11-12) |
| TM-6 | Pre-auth response bounds | General pre-auth responses (challenges, rejections, errors — no detail) ≤ 128 B; a successful PairingResponse ≤ 320 B Frame.length; admission gates: QR pairing = a valid, unused nonce, SAS pairing = the admin-enabled local pairing window | §6, pairing.md §5/§7.3 | ✏️ accepted as revised (2026-11-12) |
| TM-7 | Error payload bound | ≤ 512 B, stable codes only | §6 | ✅ accepted (2026-11-12) |
| TM-8 | Per-Client media queue | 256 datagrams (application-level) | §6 | ✅ accepted (2026-11-12) |
| TM-9 | Key-compromise behavior | Rotation = reset + re-pairing; revocation of active sessions ≤ 5 s; no recovery backdoor | §8, pairing.md §8–§9 | ✅ accepted (2026-11-12) |

## 11. References

- Design baseline: [`../../.scratch/wire-protocol-v2/spec.md`](../../.scratch/wire-protocol-v2/spec.md) — §Security and Pairing Model, §Network Scope, §Client and Role Scope, §Connection Rules, §Control-Plane Semantics, §Media-Plane Semantics, §Resource and Privacy Controls, §Dual-Protocol Migration.
- ADR-0004: [`../adr/0004-wire-protocol-v2-transport.md`](../adr/0004-wire-protocol-v2-transport.md) — accepted transport and security posture.
- Prototype report: [`../../.scratch/wire-protocol-v2/prototype-report.md`](../../.scratch/wire-protocol-v2/prototype-report.md) — on-device evidence for SPKI-pinning rejection, credential authorization, and QUIC dedup behavior.
- Pairing and key lifecycle: [pairing.md](pairing.md) (this directory).
- Normative wire specification (field layouts, error-code registry, message numbering; path referenced only): `docs/wire-protocol-v2/spec.md` (ticket 03).
- External: RFC 8446 (TLS 1.3), RFC 9000/9001 (QUIC), RFC 9221 (QUIC DATAGRAM), RFC 6189 (ZRTP SAS background), RFC 9383 (SAS design background).
