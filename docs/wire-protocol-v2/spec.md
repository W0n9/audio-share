# Wire Protocol v2 — Normative Specification

Status: proposed (2026-11-12)
Feature: wire-protocol-v2 (ticket 03) · Transport: ADR-0004 (accepted, 2026-11-12)

## 1. Scope

- This document is the **normative** specification for **Wire Protocol major version 2** ("v2"): control plane, media plane, session state machine, error registry, capability rules, and resource limits. Production code implements this contract.
- It does **not** modify the frozen v1 protocol ([`docs/protocol.md`](../protocol.md), frozen at v0.3.4). v1 framing, commands, UDP behavior, little-endian integers, and session rules are untouched and remain the compatibility contract for the released v0.3.4 Android Client.
- It **supersedes nothing**: v2 is a new, separately scoped protocol on its own port and connection lifecycle ([ADR-0001](../adr/0001-server-rewrite-baseline.md), [ADR-0004](../adr/0004-wire-protocol-v2-transport.md)).
- Normative key words MUST / MUST NOT / SHOULD / MAY follow RFC 2119/RFC 8174.
- Vocabulary follows [`GLOSSARY.md`](../../GLOSSARY.md): **Server**, **Android Client** (hereafter **Client**: the player-role peer connection, the only remote role in v2), **Server Core**, **Transfer Format**, **Endpoint**, **Capture Format**. The words "client app", "backend", and "CLI" are not used.
- The design baseline (`.scratch/wire-protocol-v2/spec.md`, cited below as **baseline §…**) supplies the accepted semantics; each section cites the block it implements. The prototype report (`.scratch/wire-protocol-v2/prototype-report.md`, rev 3, **GO**) and the throwaway prototype brief are **evidence only** — the prototype's wire format is explicitly non-normative, and every layout value here is a recorded decision, never a library default (baseline §Engineering Details Left Open).

## 2. Decision table

Every item the baseline's "Engineering Details Left Open" and ADR-0004's consequences list named, with the proposed decision, the evidence, and an empty **maintainer sign-off** column. Acceptance marks each row and moves this document's Status to accepted.

| # | Open item | Proposed decision | Evidence | Maintainer sign-off |
|---|---|---|---|---|
| D1 | Default v2 port | UDP **65531** (QUIC), default bind all interfaces dual-stack, user-configurable; never shared with v1's 65530. 65531 lies outside the default Linux ephemeral range (32768–60999). | Prototype ran 65531 on desktop and device (prototype-report §2, §3); ADR-0004 §Decision 2 (separate port; prototype "never v1's 65530") | |
| D2 | MTUD / DPLPMTUD policy | **DPLPMTUD enabled** (quinn default). Senders MUST size every media datagram from the current `max_datagram_size()` at send time (§7.3); caching a size across path changes and cross-end size assertions are forbidden; receivers MUST NOT validate a sender's datagram size against their own local limit. | Prototype observed DPLPMTUD racing its own cross-end assertions and pinned a value with it disabled (prototype-report §5; ADR-0004 §Consequences); mds 1162→1192 at path MTU 1280, 1414 at 1500 (prototype-report §4) | |
| D3 | Idle timeout + keep-alive | `max_idle_timeout = 30 s`, symmetric, both sides (value deliberately adopted, recorded — not a silently inherited default); `keep_alive_interval = 10 s` both sides (≥ 3 probes per window). No application-level heartbeat message in v2.0; media traffic and the 5 s diagnostics cadence keep live sessions busy; the idle timeout is the fallback when a Close cannot be delivered. | quinn's default 30 s silently killed idle prototype sessions on desktop and device (prototype-report §4, §5; ADR-0004 §Consequences) | |
| D4 | Media header layout | Fixed **24-byte** header (§7.1): `stream_instance_id` u32, `config_generation` u32, `sequence` u32, `timestamp` u64, `flags` u32 — all network byte order. Sequence width **32 bit**, wrap specified (§7.2). | Prototype's 20-byte BE header (instance/generation/sequence/timestamp) proved byte-exact on device (prototype-report §1); the flags word is a new decision the prototype left open | |
| D5 | Protobuf field-number allocation | Envelope `oneof` bands with uniform conventions (§4.1–§4.2): field 1 = `request_id` on every request/response, field 2 = `Status` on every response; bands 1–15 connection, 16–31 pairing, 32–47 subscription/configuration, 48–63 stream lifecycle, 64–79 diagnostics, 80+ reserved. v2 `SampleFormat` values intentionally mirror v1 `AudioFormat.Encoding` numbers. | New allocation (baseline left field numbers open); the v1 mirror reduces implementer confusion (`docs/protocol.md` §3) | |
| D6 | Error-code numbering | Seven stable bands, codes frozen once published, never renumbered: 100 version/capability · 200 pairing/auth/authz · 300 busy/rate/session · 400 control protocol · 500 Server Core lifecycle · 600 network/transport · 700 Client lifecycle (§6). 207–209 reserved for the pairing specification (ticket 04). | New registry (baseline left numbering open); the required classes are enumerated in baseline §Control-Plane Semantics | |
| D7 | Reviewed crypto suite choices | TLS 1.3 only (rustls); suites = rustls default set: `TLS_AES_256_GCM_SHA384`, `TLS_AES_128_GCM_SHA256`, `TLS_CHACHA20_POLY1305_SHA256`; key exchange X25519. Server identity: self-signed **ECDSA P-256** certificate, pinned by SHA-256 of the SPKI. Pairing nonce: **128-bit** from a CSPRNG. Pairing/client identity key algorithm and storage: owned by the pairing specification (ticket 04). | Prototype: ECDSA P-256 + SPKI pin verified on device — wrong pin rejected at the TLS handshake, wrong credential rejected with a defined error (prototype-report §1, §2 gate 3); baseline §First Pairing requires a ≥ 128-bit single-use nonce | |
| D8 | QR payload encoding | **Owned by ticket 04** (`docs/wire-protocol-v2/pairing.md`). Constraint from this spec: the payload MUST carry protocol major, Server address hint, full Server identity public key or long fingerprint, and a ≥ 128-bit single-use short-lived nonce (baseline §First Pairing); the on-wire messages that consume it are §4.4 here. | Baseline §Engineering Details Left Open; ticket 04 requirements | |
| D9 | SAS presentation | **Owned by ticket 04.** Constraints from this spec: ephemeral key exchange + hash commitment; an unverified SAS comparison claims no active-MITM protection; a six-digit code is never stretched into a session key; no custom PAKE in v2.0. | Baseline §First Pairing; ticket 04 requirements | |
| D10 | Resource-limit defaults | §10: 16 concurrent sessions total; 1 active session per Client identity (a new one supersedes the old, code 303); 32 concurrent pre-auth sessions with a 10 s auth deadline and a 120 s pairing deadline; 10 auth/pairing failures per 5 min per source → 302; 8 outstanding requests per session; per-Client media send queue 256 datagrams (drop newest); control frame cap 64 KiB. | Prototype: 64 KiB frame cap and 256-datagram app-level queue (prototype-brief, non-normative — values re-decided here); baseline §Resource and Privacy Controls requires every bound | |
| D11 | Impairment profiles | Conformance matrix §11.2: loss 2%, reorder 25%/50%, duplicate 5%, jitter 10 ms ± 5 ms, congestion 300 kbit/s (queue 50), path MTU 1280, clock drift ±100 ppm, 25 s idle survival, reconnect fallback. | Prototype impairment evidence (prototype-report §4; `scripts/impairment.sh`); baseline §Later Implementation Acceptance requires deterministic injection of exactly these classes | |
| D12 | Buffer tuning | Jitter-buffer target **60 ms**, hard limit **200 ms** (re-anchor above it); unknown-config hold ≤ 100 ms / ≤ 32 datagrams; send-ahead window ≤ 100 ms; drift rate-match ±100 ppm, accounted insert/delete when > 20 ms off target for > 5 s (§8). | 60 ms target = baseline §Measurable Initial Targets; the 256-datagram queue is prototype-proven; the remaining numbers are new explicit values, not library defaults | |

## 3. Transport connection and control framing

Implements: baseline §Candidate Transport, §Connection Rules, §Control-Plane Semantics (items 1–2), ADR-0004 §Decisions 1–3.

- Transport per ADR-0004: quinn 0.11.x, QUIC + TLS 1.3 (rustls), one UDP endpoint on D1's port. IPv4 and IPv6 are both supported; no 1492-byte or fixed-header-size assumption anywhere (baseline §IPv4, IPv6, and Datagram Size).
- **ALPN**: exactly `audio-share/v2`. Any other ALPN fails the handshake before application data. Capabilities negotiate optional features only within major 2 (§4.6).
- **0-RTT disabled** on both sides; no pairing, authorization, or control operation is accepted as early data. Mandatory security capabilities cannot be negotiated off.
- **Server identity**: SPKI-pinned certificate (D7). A pin mismatch fails the TLS handshake; the Client classifies it locally as code 205, MUST NOT fall back to v1 or plaintext, and MUST NOT retry without user action (re-pairing or explicit key-change confirmation — pairing spec, ticket 04).
- **Control stream**: exactly one bidirectional QUIC stream, opened by the Client immediately after the handshake. The Server never opens streams; v2.0 media never uses streams (QUIC DATAGRAM only). A second Client stream or any Server-opened stream is a violation: the peer sends Error(402)+Close(402) if the control stream exists, then transport-closes.

| Connection parameter | Value | Rule |
|---|---|---|
| QUIC handshake timeout (Client) | 5 s | exceed → classify 601, reconnect with backoff (§6.3) |
| Auth deadline (arrival of the first app message → outcome) | 10 s | exceed → Server closes, 601 |
| Pairing deadline (`PairingRequest` → `PairingResponse`) | 120 s | may await local admin approval; exceed → Error(206)+Close(206) |
| Idle timeout | 30 s (D3) | fires → classify 601; **the fallback when a Close message cannot be delivered** |
| Keep-alive interval | 10 s (D3) | both sides |
| QUIC application close code | §6 registry code (u64) | every transport close carries the registry code; reason = the Close detail. Transport closes are diagnosable without application data. |

- **Framing**: the control stream carries `u32 BE length | Frame` (§4). `length` is the exact byte length of the serialized `Frame` message. `length == 0` or `length > 65536` → Error(401)+Close(401), before any allocation. 64 KiB is the default cap; a per-message exception requires a reviewed spec revision naming the message — none exists in v2.0 (baseline §Control-Plane Semantics item 1).
- Requests MAY be pipelined; the receiver processes them in stream order (QUIC ordered stream). A response whose `request_id` matches no pending request is dropped without state change. A request in an illegal state is answered with a defined error — never a silent ignore (baseline §Control-Plane Semantics item 5).
- Request timeout (Client): 10 s per outstanding request → classify 601, tear down, reconnect.

## 4. Control-plane schema

Implements: baseline §Control-Plane Semantics (items 1, 3–5), §First Pairing (wire slots; cryptographic semantics owned by ticket 04).

### 4.1 Envelope and message bands

File `audio_share_v2.proto`, `syntax = "proto3"`, `package audio.share.v2`. One envelope message with a `oneof body`; the oneof case is the message type — behavior never depends on enum ordinals, struct layout, or the position of a four-byte command (baseline §Control-Plane Semantics item 3; the prototype's `u8 msg_type` byte is replaced by the oneof).

| Band | Fields (oneof cases) | Contents |
|---|---|---|
| 1–15 | 1 `Authenticate` · 2 `AuthenticateResponse` · 3 `Negotiate` · 4 `NegotiateResponse` · 5 `Error` · 6 `Close` | connection lifecycle, errors, close |
| 16–31 | 16 `PairingRequest` · 17 `PairingResponse` | pairing (crypto semantics: ticket 04) |
| 32–47 | 32 `Subscribe` · 33 `SubscribeResponse` · 34 `Unsubscribe` · 35 `UnsubscribeResponse` · 36 `RequestConfiguration` · 37 `ConfigurationPublished` | subscription and configuration publication |
| 48–63 | 48 `StreamStarted` · 49 `StreamEnded` | stream-instance lifecycle |
| 64–79 | 64 `PlaybackDiagnostics` | diagnostics |
| 80+ | — | reserved; assigned only by a reviewed spec revision |

Conventions (D5): field 1 of every request and response is `fixed64 request_id`; field 2 of every response is `Status`; payload data occupies fields ≥ 3. A `Frame` with no `body` set is malformed (401).

### 4.2 Shared types

```proto
message Status {
  uint32 code = 1;             // 0 = success; otherwise a §6 registry code
  bool retryable = 2;          // meaningful when code != 0
  bool requires_repairing = 3; // meaningful when code != 0
  string detail = 4;           // ≤ 256 bytes UTF-8; no secrets, paths, or stack traces
}
```

- `request_id`: sender-assigned, unique per sender per connection, monotonically increasing from 1; the receiver echoes it verbatim and never interprets it (opaque). `0` = "not a response" (pushes).
- `retryable` / `requires_repairing`: senders MUST equal the §6 registry defaults in v2.0; per-instance refinement is a future revision.
- `detail`: senders MUST cap at 256 bytes; receivers truncate for logging and MUST NOT reject a frame solely for a longer detail. Error responses are small by construction (≤ ~300 B typical, ≤ 64 KiB hard) — small unauthenticated requests never produce large error responses (baseline §Resource and Privacy Controls).

### 4.3 Message direction

| Message | Direction | Mode |
|---|---|---|
| `Authenticate`, `PairingRequest`, `Negotiate`, `Subscribe`, `Unsubscribe`, `RequestConfiguration` | Client → Server | request |
| `AuthenticateResponse`, `PairingResponse`, `NegotiateResponse`, `SubscribeResponse`, `UnsubscribeResponse` | Server → Client | response |
| `ConfigurationPublished`, `StreamStarted`, `StreamEnded` | Server → Client | push |
| `PlaybackDiagnostics` | Client → Server | push |
| `Error` | both | unsolicited error or fatal-error companion |
| `Close` | both | one-way; never echoed (§5.4) |

### 4.4 Messages and field allocations

**Connection lifecycle (band 1–15)**

```proto
message Authenticate {
  fixed64 request_id = 1;
  bytes credential = 2;        // pairing-derived authorization credential; derivation: ticket 04
}
message AuthenticateResponse {
  fixed64 request_id = 1;
  Status status = 2;
  fixed64 session_id = 3;      // Server-assigned (counter from 1); log/diagnostic correlation; set iff success
}
message Negotiate {
  fixed64 request_id = 1;
  uint32 protocol_major = 2;   // MUST be 2
  uint32 protocol_minor = 3;   // 0 for v2.0
  repeated uint32 capabilities = 4; // optional-feature ids, ascending, unique
}
message NegotiateResponse {
  fixed64 request_id = 1;
  Status status = 2;
  uint32 protocol_major = 3;   // accepted major
  uint32 protocol_minor = 4;   // Server's minor within major 2
  repeated uint32 capabilities = 5; // intersection honored for this session
}
message Error {
  fixed64 request_id = 1;      // echo of the failed request; 0 = unsolicited
  Status status = 2;           // status.code MUST be != 0
}
message Close {
  uint32 code = 1;             // §6 registry code; MUST be != 0
  string detail = 2;           // ≤ 256 bytes UTF-8
}
```

**Pairing (band 16–31)** — envelope slots fixed here; cryptographic semantics (nonce lifetime, transcript binding, proof of possession, SAS derivation, credential derivation, persistence) are the pairing specification's, ticket 04.

```proto
message PairingRequest {
  fixed64 request_id = 1;
  bytes pairing_nonce = 2;     // ≥ 16 bytes; from the QR payload; single-use, short-lived
  bytes client_public_key = 3; // Client identity public key; encoding: ticket 04
  string client_name = 4;      // ≤ 64 bytes UTF-8 display name
}
message PairingResponse {
  fixed64 request_id = 1;
  Status status = 2;
  bytes commitment = 3;        // hash commitment for SAS comparison; derivation: ticket 04
  bytes credential = 4;        // set iff success; the credential for future Authenticate calls
}
```

**Subscription and configuration (band 32–47)**

```proto
message Subscribe { fixed64 request_id = 1; }
message SubscribeResponse {
  fixed64 request_id = 1;
  Status status = 2;
  Configuration configuration = 3;   // set iff success
}
message Unsubscribe { fixed64 request_id = 1; }
message UnsubscribeResponse { fixed64 request_id = 1; Status status = 2; }
message RequestConfiguration { fixed64 request_id = 1; }
message ConfigurationPublished {
  fixed64 request_id = 1;      // 0 = unsolicited push; > 0 = response to RequestConfiguration
  Configuration configuration = 2;
}
message Configuration {
  uint32 config_generation = 1;    // ∈ [1, 2^32−1]; +1 per decode-affecting change within an instance
  Codec codec = 2;                 // CODEC_PCM in v2.0
  PcmParameters pcm = 3;           // set iff codec == CODEC_PCM
}
enum Codec {
  CODEC_UNSPECIFIED = 0;
  CODEC_PCM = 1;                   // required first codec
  // CODEC_OPUS = future; assignable only with a negotiated capability (baseline §Media-Plane §Encoding)
}
enum SampleFormat {               // numbers mirror v1 AudioFormat.Encoding (docs/protocol.md §3)
  SAMPLE_FORMAT_UNSPECIFIED = 0;
  SAMPLE_FORMAT_F32LE = 1;        // IEEE-754 binary32 little-endian
  SAMPLE_FORMAT_U8 = 2;
  SAMPLE_FORMAT_S16LE = 3;
  SAMPLE_FORMAT_S24LE = 4;        // packed 3-byte little-endian
  SAMPLE_FORMAT_S32LE = 5;
}
message PcmParameters {
  uint32 sample_rate = 1;         // Hz, > 0
  uint32 channels = 2;            // > 0
  SampleFormat sample_format = 3; // MUST NOT be UNSPECIFIED
}
```

**Stream-instance lifecycle (band 48–63)**

```proto
message StreamStarted {
  uint32 stream_instance_id = 1;
  uint32 config_generation = 2;   // generation of the media that follows
  fixed64 initial_timestamp = 3;  // capture-clock position of the first media frame the receiver will get
                                  // (0 for a fresh instance; mid-stream anchor for a late Subscriber)
}
message StreamEnded {
  uint32 stream_instance_id = 1;
  uint32 code = 2;                // §6: 501 CAPTURE_FAILURE, 502 STREAM_ENDED, 503 SERVER_STOPPING, …
  string detail = 3;
}
```

**Diagnostics (band 64–79)**: §9.

**Validation rules (decidable)**

| Condition | Outcome |
|---|---|
| Frame without a `body` set; frame length 0 or > 64 KiB; undecodable bytes | Error(401)+Close(401) |
| `Negotiate` with `protocol_major ≠ 2`, or minor > Server's minor | Error(101)+Close(101) |
| `Negotiate` listing a capability the Server cannot honor | Error(102)+Close(102); unknown-but-optional ids are ignored |
| `Configuration` invalid (codec UNSPECIFIED; `pcm` absent for CODEC_PCM; sample_rate 0; channels 0; sample_format UNSPECIFIED) | receiving side: Error(401)+Close(401) |
| `Error` with `status.code == 0`, or `Close` with `code == 0` | 401 |
| Unknown proto3 fields | ignored (standard evolution rule; new fields only, never renumbered) |
| `detail` longer than 256 bytes | truncated for logging; never rejected |

### 4.5 Capability rules

- Capability ids are `uint32` ≥ 1, assigned only by a reviewed spec revision; unknown ids in `Negotiate.capabilities` are ignored — capabilities carry optional features only (baseline §Connection Rules).
- v2.0 defines **no** optional capabilities: both sides send an empty list; the response echoes empty.
- Mandatory security (encryption, authentication, SPKI pinning) is never a capability and can never be negotiated off.
- A major-version mismatch is a transport failure (ALPN, before application data), never a capability outcome.

## 5. Session state machine

Implements: baseline §Control-Plane Semantics item 2 — explicit states, all legal transitions, terminal behavior. Both endpoints run the machine; state is per (Server, Client session).

### 5.1 States

| State | Meaning | Entry action |
|---|---|---|
| CONNECTING | transport handshake done, control stream open (Client opened it); no app message processed yet | Server: capacity check (§10) |
| AUTHENTICATING | a first app message (`Authenticate` or `PairingRequest`) is being validated | — |
| NEGOTIATING | identity authenticated; awaiting/validating version + capabilities | — |
| READY | authenticated ∧ negotiated. Configuration publication is a data condition, not a state | Server: push `ConfigurationPublished` if a Transfer Format is current |
| PLAYING | READY ∧ subscribed ∧ a stream instance is active for this session | — |
| CLOSING | `Close` sent or received; teardown in progress | flush best-effort data (Client: final diagnostics); start 5 s grace timer |
| CLOSED | terminal (§5.4) | release all session state |

### 5.2 Server transition table

| State | Event | Guard | Action | New state |
|---|---|---|---|---|
| CONNECTING | `Authenticate` | capacity ok | validate credential | AUTHENTICATING |
| CONNECTING | `PairingRequest` | capacity ok; rate limit ok | validate nonce (single-use, unexpired, ≥ 16 B); start 120 s pairing deadline | AUTHENTICATING |
| CONNECTING | capacity exceeded | — | transport close carrying 301 | CLOSED |
| CONNECTING | any other `Frame`; unset body; length violation | — | Error(401 or 402)+Close(same) | CLOSING |
| CONNECTING | second Client stream / Server-stream request | — | Error(402)+Close(402) | CLOSING |
| CONNECTING | auth deadline (10 s) exceeded | — | transport close carrying 601 | CLOSED |
| AUTHENTICATING | credential valid | — | send `AuthenticateResponse`(OK, session_id); if the identity already has an active session, close that one with Close(303) | NEGOTIATING |
| AUTHENTICATING | credential invalid/unknown | failure rate not exceeded | count failure; Error(203, retryable=false, re-pair=true)+Close(203) | CLOSING |
| AUTHENTICATING | identity denied/revoked | — | Error(204)+Close(204) | CLOSING |
| AUTHENTICATING | `PairingRequest` nonce invalid/expired/replayed | — | count failure; Error(201)+Close(201) | CLOSING |
| AUTHENTICATING | `PairingRequest` approved locally | — | persist pairing; send `PairingResponse`(OK, commitment, credential) | NEGOTIATING |
| AUTHENTICATING | `PairingRequest` declined locally | — | Error(201, detail "declined")+Close(201) | CLOSING |
| AUTHENTICATING | pairing deadline (120 s) exceeded | — | Error(206)+Close(206) | CLOSING |
| AUTHENTICATING | auth/pairing failure rate exceeded | — | Error(302)+Close(302) | CLOSING |
| NEGOTIATING | `Negotiate` major = 2, minor ≤ Server's | — | send `NegotiateResponse`(OK, Server minor, accepted capabilities); push `ConfigurationPublished` if a Transfer Format is current | READY |
| NEGOTIATING | `Negotiate` major ≠ 2 or minor > Server's | — | Error(101)+Close(101) | CLOSING |
| NEGOTIATING | `Negotiate` with an unhonorable capability | — | Error(102)+Close(102) | CLOSING |
| NEGOTIATING | any other `Frame` | — | Error(402)+Close(402) | CLOSING |
| READY | `Subscribe` | capture healthy | send `SubscribeResponse`(OK + current `Configuration`); if a stream is active, push `StreamStarted`(active instance, current generation, next-frame timestamp) | READY (subscribed) |
| READY | `Subscribe` | capture unhealthy or absent | Error(501, retryable=true); no state change | READY |
| READY | `Subscribe` | already subscribed | Error(402, retryable=false); no state change | READY |
| READY | `Unsubscribe` | — | send `UnsubscribeResponse`(OK) — idempotent | READY (unsubscribed) |
| READY | `RequestConfiguration` | — | send `ConfigurationPublished`(request_id echoed, current) | READY |
| READY | decode-affecting config change | config current | config_generation + 1; push `ConfigurationPublished` | READY |
| READY | capture started | subscribed | push `StreamStarted` | PLAYING |
| READY | capture started | not subscribed | no push | READY |
| READY | `Close` received | — | no echo | CLOSING |
| PLAYING | capture ended (normal) | — | push `StreamEnded`(502) | READY |
| PLAYING | capture failed | — | push `StreamEnded`(501, retryable=true) | READY |
| PLAYING | capture restart (new instance) | — | push `StreamEnded`(502), then push `StreamStarted`(new instance) | PLAYING |
| PLAYING | decode-affecting config change | — | config_generation + 1; push `ConfigurationPublished`; media switches generation per §7.6 | PLAYING |
| PLAYING | `Unsubscribe` | — | push `StreamEnded`(502, detail "unsubscribed"), then send `UnsubscribeResponse`(OK) — StreamEnded first | READY |
| PLAYING | `Close` received | — | no echo; capture continues for other Clients | CLOSING |
| any | `Error` received | — | advisory only; no state change (fatal errors always arrive paired with `Close`) | unchanged |
| any | fatal error raised | — | send matching Error + Close, stop processing requests | CLOSING |
| any | transport lost | — | classify 601 locally | CLOSED |
| any | idle timeout (30 s) | — | transport close carrying 601 | CLOSED |
| CLOSING | transport close completed | — | release session | CLOSED |
| CLOSING | 5 s grace elapsed without peer transport close | — | force transport close carrying the Close code | CLOSED |

CLOSING accepts no new requests; a Close in flight makes the teardown observable, so it is bounded rather than silent.

### 5.3 Client transition table

| State | Event | Guard | Action | New state |
|---|---|---|---|---|
| CONNECTING | handshake + stream open succeeded | SPKI pin verified | send `Authenticate` (paired) or `PairingRequest` (unpaired) | AUTHENTICATING |
| CONNECTING | handshake failed | pin mismatch | classify 205; no retry without user action | CLOSED |
| CONNECTING | handshake failed | other (timeout, unreachable) | classify 601; reconnect with backoff (§6.3) | CLOSED |
| AUTHENTICATING | `AuthenticateResponse` OK | — | send `Negotiate` | NEGOTIATING |
| AUTHENTICATING | `PairingResponse` OK | — | persist credential; send `Negotiate` | NEGOTIATING |
| AUTHENTICATING | response error | retryable=false codes (201/203/204/205) | surface to user; no automatic retry | CLOSED |
| AUTHENTICATING | response deadline (10 s) exceeded | — | classify 601; reconnect with backoff | CLOSED |
| NEGOTIATING | `NegotiateResponse` OK | — | store accepted capabilities | READY |
| NEGOTIATING | `NegotiateResponse` error | — | apply §6.3 policy | CLOSED |
| NEGOTIATING | response deadline (10 s) exceeded | — | classify 601; reconnect with backoff | CLOSED |
| READY | `ConfigurationPublished` | first or updated | store current config; if it is not decodable (unsupported codec/format), Error(403)+Close(403) | READY |
| READY | user subscribes | ≥ 1 `ConfigurationPublished` received | send `Subscribe`(request_id) | READY (awaiting) |
| READY | user subscribes | no configuration received yet | withhold `Subscribe` until the first publication (never a silent media wait) | READY |
| READY | `SubscribeResponse` OK | — | subscribed | READY (subscribed) |
| READY | `SubscribeResponse` error 501 | retryable | surface; MAY retry after the next `ConfigurationPublished` | READY |
| READY | `StreamStarted` | subscribed | initialize per-instance media state (§7.5) | PLAYING |
| READY | `StreamStarted` | not subscribed | Error(402)+Close(402) — Server violation | CLOSING |
| READY | `Close` received | — | send final diagnostics (best-effort); no echo | CLOSING |
| PLAYING | media datagrams arrive | — | §7.5 receiver rules run | PLAYING |
| PLAYING | 5 s diagnostics timer | — | send `PlaybackDiagnostics` snapshot (§9) | PLAYING |
| PLAYING | `StreamEnded`(501/502) | — | finalize instance statistics | READY |
| PLAYING | user unsubscribes | — | send `Unsubscribe`; await `StreamEnded` then `UnsubscribeResponse` | READY |
| PLAYING | `ConfigurationPublished` | — | apply; unknown generation per §7.6 | PLAYING |
| READY / PLAYING | `Close` received | — | → CLOSING (as above) | CLOSING |
| any | transport lost | — | classify 601; MAY reconnect with backoff (§6.3) | CLOSED |
| any | idle timeout (30 s) | — | classify 601 | CLOSED |
| CLOSING | final diagnostics sent (or 5 s grace elapsed) | — | transport close | CLOSED |

### 5.4 Invariants and terminal behavior

1. Every request is answered exactly once — with a response, or with Error+Close.
2. Media subscription (`Subscribe` → media flow) cannot complete before pairing, authentication, negotiation, and configuration publication, by state legality; any violation produces a defined error (402 or the specific code), never a silent ignore (baseline §Control-Plane Semantics item 4).
3. Only the Server pushes `ConfigurationPublished`/`StreamStarted`/`StreamEnded`; only the Client sends requests and `PlaybackDiagnostics`.
4. `Close` is one-way and never echoed; the receiver's transport close is its acknowledgment.
5. State never moves backward except PLAYING → READY (stream end or unsubscribe); CLOSING and CLOSED are absorbing except CLOSING → CLOSED.
6. **CLOSED (terminal)**: the session object and every per-instance media state, queue, and counter are released; no message is valid afterward (the transport is closed); a reconnecting Client starts a fresh session at CONNECTING; Server-side capture and other Clients are unaffected.

## 6. Error registry

Implements: baseline §Control-Plane Semantics items 6–7, §Resource and Privacy Controls.

### 6.1 Numbering rules (D6)

- Codes are `uint32`, frozen once published; new codes append within the matching band; numbers are never reused, renumbered, or reassigned. `0` is never sent (proto3 default — receiving 0 is 401).
- Bands: 100 version/capability · 200 pairing/auth/authz · 300 busy/rate/session · 400 control protocol · 500 Server Core lifecycle · 600 network/transport · 700 Client lifecycle. **207–209 are reserved for the pairing specification (ticket 04).**

### 6.2 Registry

| Code | Name | Meaning | Retryable | Re-pairing |
|---|---|---|---|---|
| 101 | VERSION_MISMATCH | major ≠ 2 or unsupported minor | no | no |
| 102 | UNSUPPORTED_CAPABILITY | a listed capability cannot be honored | no | no |
| 201 | PAIRING_REJECTED | nonce invalid/expired/single-use-replayed, or pairing declined | no | yes |
| 203 | AUTHENTICATION_FAILED | credential invalid or unknown | no | yes |
| 204 | AUTHORIZATION_DENIED | paired but denied (revoked, insufficient role) | no | no (admin action) |
| 205 | SERVER_IDENTITY_MISMATCH | pinned SPKI ≠ presented certificate (detected at handshake; used as local classification and Close code) | no | yes (user-confirmed re-pairing) |
| 206 | PAIRING_PENDING_TIMEOUT | no pairing decision within 120 s | yes | no |
| 301 | BUSY | Server at capacity (total sessions or pre-auth sessions) | yes | no |
| 302 | RATE_LIMITED | auth/pairing failure rate or outstanding-request rate exceeded | yes | no |
| 303 | SESSION_SUPERSEDED | the same Client identity opened a new session | no | no |
| 401 | MALFORMED_MESSAGE | undecodable, unset body, oversized frame, zero code, invalid `Configuration` | no | no |
| 402 | INVALID_STATE | request or push illegal for the current state | no | no |
| 403 | UNSUPPORTED_MEDIA_CONFIGURATION | receiver cannot decode the published Transfer Format | no | no |
| 501 | CAPTURE_FAILURE | Endpoint capture failed or absent | yes | no |
| 502 | STREAM_ENDED | stream instance ended normally | yes | no |
| 503 | SERVER_STOPPING | administrative Server stop | yes | no |
| 599 | INTERNAL_ERROR | unexpected Server error (bounded detail) | no | no |
| 601 | NETWORK_FAILURE | temporary network failure, idle timeout, undeliverable Close, deadline | yes | no |
| 602 | DATAGRAM_UNSUPPORTED | transport cannot carry media datagrams (no DATAGRAM support, or limit < header + one frame) | yes | no |
| 701 | CLIENT_STOPPING | normal Client stop | no | no |

Baseline class coverage: version/capability → 101/102 · pairing → 201 · authentication → 203 · authorization → 204 · busy/rate limit → 301/302 · malformed message/state → 401/402 · unsupported media configuration → 403 · capture failure → 501 · stream end → 502 · Server stop → 503 · normal Client stop → 701 · temporary network failure → 601.

### 6.3 Client reconnect policy

| Terminal condition | Client behavior |
|---|---|
| 701 | no reconnect (user-initiated stop) |
| 503, 601, 602 | reconnect with backoff |
| 301, 302 | reconnect with backoff (retryable) |
| 303 | no reconnect; surface (another session owns this identity) |
| 101, 102, 201, 203, 204, 205, 401, 402, 403, 599 | no automatic reconnect; surface to user |
| transport loss without a Close message | classify 601 → reconnect with backoff |

Backoff: 1 s base, ×2 per attempt, 30 s cap, ±20 % uniform jitter (the prototype's proven fallback used ±20 % jitter); reset to base when a session reaches READY. The Client never automatically falls back to v1 after a v2 failure, and a security failure never downgrades to plaintext or anonymous encryption (baseline §Dual-Protocol Migration, §Identity and Authorization).

## 7. Media plane

Implements: baseline §Media-Plane Semantics (Encoding and Packet Header; IPv4, IPv6, and Datagram Size).

### 7.1 Header (D4)

| Offset | Width | Field | Byte order | Rule |
|---|---|---|---|---|
| 0 | 4 | `stream_instance_id` | BE u32 | ∈ [1, 2^32−1]; 0 invalid |
| 4 | 4 | `config_generation` | BE u32 | ∈ [1, 2^32−1]; +1 per decode-affecting change within the instance |
| 8 | 4 | `sequence` | BE u32 | per instance: starts 0, +1 per datagram, mod 2^32 |
| 12 | 8 | `timestamp` | BE u64 | capture-clock sample-frame position of the first payload frame; per instance starts 0 |
| 20 | 4 | `flags` | BE u32 | bit field below; reserved bits sent as 0 |
| 24 | … | payload | — | PCM sample frames, Transfer Format encoding (little-endian sample bytes) |

`HEADER_SIZE = 24`. The header is **not** RTP wire-compatible; sequence, timestamp, loss, and jitter *semantics* follow RFC 3550 (baseline §Media-Plane §Encoding and Packet Header).

Flags:

| Bit | Name | Sender | Receiver |
|---|---|---|---|
| 0 (0x1) | STREAM_START | set on the first datagram of the instance (sequence 0, timestamp 0) | epoch/anchor hint; the instance and generation fields are authoritative |
| 1 (0x2) | CONFIG_CHANGE | set on the first datagram carrying a `config_generation` greater than the sender's previous one | fast-path hint to await/apply the new configuration; the generation field is authoritative |
| 2 (0x4) | SPARE | MUST send 0 | MUST ignore |
| 3–31 | reserved | MUST send 0 | MUST ignore (a future revision may promote specific bits to mandatory) |

### 7.2 Field semantics (decidable)

| Field | Semantics |
|---|---|
| `timestamp` | position, in **sample frames**, of the first PCM frame in the payload, measured on the capture sampling clock from the instance's first frame (= 0). One stereo frame counts as one frame — it is never a count of interleaved channel samples, never Unix time, never a send time. Continuity rule for a gapless stream: `ts(next) = ts(prev) + frames(prev)`. At 48 kHz a u64 wraps after ~12.2 million years: not reachable; receivers treat it as monotonic within the instance |
| `sequence` | 32-bit (D4), per instance from 0, wrapping mod 2^32 (0xffffffff → 0). Receivers maintain an RFC 3550 §A.1 extended 64-bit sequence (cycle folding) per instance; loss, reorder, duplicate, and jitter accounting use extended values |
| `stream_instance_id` | new value on every capture reset, capture restart, process restart, or discontinuous sampling clock (baseline §Media-Plane); monotonic +1 per Server process, wrapping 2^32−1 → 1. A new instance resets receiver playback and statistics state (§7.5) |
| `config_generation` | +1 on every decode-affecting change (`sample_rate`, `channels`, `sample_format`, `codec`); identical content keeps the generation; reaching 2^32−1 with a further change forces a new instance (generation restarts at 1). Media carries its generation because reliable control and DATAGRAM delivery have no cross-channel ordering guarantee |
| Endianness | every fixed-header integer is **network byte order** (big-endian). v1's little-endian control integers and little-endian PCM are unchanged — v2 sample payloads stay little-endian by their Transfer Format name (e.g. F32LE); only the v2 header is big-endian |

### 7.3 Datagram sizing

| Rule | Value / behavior |
|---|---|
| Payload limit | `payload_max = max_datagram_size() − 24`, re-queried at send time (D2 — never cached across path changes, never a fixed 1464/1500 assumption) |
| Frame alignment | every datagram payload is a whole number of sample frames: `payload_len % (bytes_per_sample × channels) == 0` |
| Frames per datagram | `floor(payload_max / frame_bytes)`; a capture callback is chunked consecutively; the final chunk carries the remainder (still frame-aligned) |
| Path-limit reduction | takes effect on the next datagram automatically (per-send sizing); senders re-chunk rather than skip where possible |
| Oversized send | a datagram larger than the current limit fails: re-query, re-chunk, or drop the batch (media is drop-eligible) |
| IP fragmentation | never relied on; application datagrams stay within the QUIC packet limit by construction |
| Interop floor | every implementation MUST accept media datagrams with payload up to 1176 bytes (1200 − 24) and any datagram ≥ header + one frame; receivers never assume a fixed maximum |
| Unsupported datagram path | no DATAGRAM support, or limit < 24 + one frame → Close(602) (explicit small-MTU/unsupported-datagram error; tested per §11.2) |

### 7.4 Sender pacing

- Sends are non-waiting into a bounded per-Client queue (256 datagrams, D10); on overflow the **newest** is dropped and counted locally (prototype-proven behavior). A stalled or congested Client never blocks capture or another Client.
- Pacing invariant: `capture_position − last_sent_position ≤ 100 ms` of media (send-ahead window, D12); a capture callback may burst several datagrams only within the window — never an unbounded whole-callback burst (baseline §Media-Plane §Live Playback).

### 7.5 Receiver rules (per Client, per instance)

Per-instance receiver state: extended sequence + cycle count, dedup window bitmap (span ≥ the jitter buffer in datagrams), last timestamp, jitter estimate, all §9 counters, jitter buffer.

| Condition | Action |
|---|---|
| (instance, generation) both known | process: dedup → reorder/late classification → jitter buffer |
| new instance id | begin a new receiver epoch: reset playback and statistics state; anchor at the first received datagram — STREAM_START confirms the 0 origin, and the reliable `StreamStarted` push carries the anchor, so loss cannot remove it while the connection lives |
| generation unknown (newer) | unknown-config rule (§7.6) |
| generation older than the current one (stale reorder across a change) | drop; count in `unknown_config_drops` (not decodable under the current configuration — never decoded with the previous generation) |
| payload not frame-aligned, reserved flag bits set, or impossible fields (0 ids) | malformed media: drop, log locally |
| duplicate (extended sequence already received within the dedup window) | drop; count `duplicates` |
| arrived after its playout deadline | drop; count `late_dropped` — **separate from network loss** |

### 7.6 Unknown configuration

Baseline: "Media with unknown configuration is dropped or held in a strictly bounded queue while configuration is requested; it is never decoded with the previous generation." v2.0 rule: **bounded hold, then drop**.

1. Media with an unknown generation is held, never decoded, in a bounded queue: ≤ 100 ms of media or ≤ 32 datagrams, whichever fills first; overflow drops the oldest held datagram.
2. The Client awaits `ConfigurationPublished` (the reliable control stream delivers it) and SHOULD also send `RequestConfiguration`.
3. If the generation remains unknown after 2 s, the Client sends Error(403)+Close(403).
4. Superseded-generation media (§7.5) is dropped and counted under all circumstances.

## 8. Live playback

Implements: baseline §Media-Plane Semantics (Live Playback and Recovery), §Measurable Initial Targets.

v2.0 has **no application-layer retransmission and no FEC**. Opus, FEC, DTX, and codec-specific PLC are future capabilities negotiated as capabilities — never implicit first-version requirements.

| Mechanism | Rule (v2.0) |
|---|---|
| Jitter buffer | bounded; reorders timely packets; target 60 ms (D12); arrivals after their playout deadline are dropped and counted as `late_dropped`, reported separately from network loss |
| Concealment | gap or underrun → **silence** (all-zero PCM frames in the active Transfer Format); comfort noise MAY be added; every intervention counted (`concealments`, `concealed_frames`). Concealment covers exactly four interventions: gap fill, underrun fill, accounted drift insert/delete, hard-limit re-anchor deletion |
| Paced sends | Server-side, §7.4; send-ahead ≤ 100 ms |
| Drift measurement | capture clock vs playback clock, 10 s rolling window |
| Drift correction | (1) playback-rate adjustment ≤ ±100 ppm — smooth, not frame-counted; (2) when the buffer deviates > 20 ms from target for > 5 s despite rate matching, insert or delete whole sample frames, counted as concealment |
| Hard-limit re-anchor | buffer > 200 ms: drop buffered media (drop-oldest) down to the target and jump the submitted position forward — the buffer never accumulates indefinite latency; deleted frames counted as concealment |
| Per-Client independence | each Client owns its session, sequence, buffer, pacing/backpressure, and statistics state; one slow Client never blocks capture or another Client |

## 9. Playback diagnostics

Implements: baseline §Playback Diagnostics.

**Rules.** The Client reports every **5 s** and best-effort once when entering CLOSING; diagnostics ride the control stream; values are snapshotted off the audio thread; **at most one diagnostics send may be pending** — if one is queued, the next cycle is skipped (statistics failures never block media delivery). **A QUIC ACK or socket receive is not proof of decode or playback**: the Server treats Client-reported statistics as the only playback evidence. Cumulative counters reset on stream-instance change; interval counters reset after each report.

```proto
message PlaybackDiagnostics {
  uint32 stream_instance_id = 1;
  uint32 config_generation = 2;
  fixed64 extended_highest_sequence = 3;        // RFC 3550 A.1 cycles folded in
  sint64 cumulative_lost = 4;                   // RFC 3550 A.3; signed — duplicates may drive it negative
  sint64 interval_lost = 5;
  fixed64 cumulative_received = 6;
  fixed64 interval_received = 7;
  fixed64 cumulative_duplicates = 8;
  fixed64 interval_duplicates = 9;
  fixed64 cumulative_reordered = 10;            // out-of-order arrivals
  fixed64 interval_reordered = 11;
  uint32 max_reorder_distance = 12;             // max observed sequence displacement, this instance
  fixed64 cumulative_late_drops = 13;
  fixed64 interval_late_drops = 14;
  fixed64 cumulative_unknown_config_drops = 15; // unknown or superseded generation (§7.5/§7.6)
  fixed64 interval_unknown_config_drops = 16;
  fixed64 cumulative_concealments = 17;         // interventions: gap, underrun, drift, re-anchor
  fixed64 interval_concealments = 18;
  sint64 concealed_frames = 19;                 // frames inserted − frames deleted, this instance
  fixed64 underruns = 20;
  uint32 jitter_microseconds = 21;              // RFC 3550 A.8 estimate, converted at the published sample_rate
  uint32 buffer_current_ms = 22;
  uint32 buffer_target_ms = 23;
  fixed64 submitted_position = 24;              // most recent frame position handed to the platform playback API,
                                                // on the capture clock of the active instance
}
```

## 10. Resource limits

Implements: baseline §Resource and Privacy Controls, §Connection Rules (D10).

| Limit | Value | Enforcement |
|---|---|---|
| Control frame size | 64 KiB hard | validate before allocation; violation = 401 |
| Concurrent sessions (Server total) | 16 | over → transport close carrying 301 |
| Sessions per Client identity | 1 active; a new session supersedes the old (303) | at auth success |
| Concurrent pre-auth sessions | 32 | over → transport close carrying 301 |
| Auth deadline / pairing deadline | 10 s / 120 s | §3 |
| Auth + pairing failure rate | 10 per 5 min per source address | over → 302 |
| Outstanding requests per session | 8 | over → 302 |
| Server per-Client media send queue | 256 datagrams, drop newest | §7.4 |
| Send-ahead window | 100 ms | §7.4 |
| Client unknown-config hold | ≤ 100 ms or ≤ 32 datagrams | §7.6 |
| Client jitter buffer | target 60 ms, hard limit 200 ms | §8 |
| Retained generations per instance | current + 1 previous | older generations are counted drops |
| Error `detail` | ≤ 256 bytes | §4.2 |

## 11. Verification

### 11.1 Golden vectors

Cross-language golden vectors (control frames, media headers, error codes, authentication flows) are ticket 05's deliverable and MUST be generated from this document once accepted. The deterministic-vector strategy is proven byte-exact across x86_64 and aarch64 (prototype-report §5).

### 11.2 Impairment conformance matrix (D11)

| Profile | Parameters | Gate |
|---|---|---|
| Loss | 2% independent | control clean; media gaps → concealment; loss accounting matches injection |
| Reorder | 25% and 50% | 0 loss; `reordered` / `late_dropped` accounted |
| Duplicate | 5% | 0 app-visible (QUIC packet-number dedup) |
| Jitter | 10 ms ± 5 ms | no deadlock; jitter estimate tracks |
| Congestion | 300 kbit/s, queue 50 | bounded backlog; full-rate recovery; capture never blocked |
| Path MTU | 1280 | DPLPMTUD re-probe; datagrams ≤ new limit; no fragmentation reliance |
| Clock drift | ±100 ppm | rate-match engages; buffer stays bounded; accounted corrections counted |
| Idle | 25 s | session survives (keep-alive); > 30 s silence → 601 |
| Reconnect | connection kill | teardown + backoff reconnect — the production promise when migration is unavailable |

### 11.3 Negative tests

Malformed lengths, unknown fields/messages, version/capability conflicts, state violations, authentication failures, unknown-generation media, oversized/undersized datagrams — each MUST produce the §6 code named here, never a silent ignore (baseline §Later Implementation Acceptance).

## 12. Parked for the maintainer

| Item | Where it landed | Note |
|---|---|---|
| Idempotent `Unsubscribe` (OK when not subscribed) | §5.2 | alternative: 402; idempotency chosen for reconnect flows |
| Duplicate `Subscribe` → 402 without close | §5.2 | v1 closed the connection; v2 has an error channel |
| Pairing deadline 120 s | §3, D10 | placeholder pending ticket 04's approval UX |
| Same-identity supersede (303) vs reject | §6.2 | supersede chosen: one identity per Client install |
| Reserved flag bits ignored (not rejected) | §7.1 | strict-drop alternative rejected for forward compatibility |
