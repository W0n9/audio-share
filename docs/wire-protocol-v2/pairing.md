# Wire Protocol v2 — Pairing and Key Lifecycle Specification

Status: accepted (2026-11-12, maintainer adjudication)

Feature: wire-protocol-v2 · Ticket: `.scratch/wire-protocol-v2/issues/04-threat-model-pairing-spec.md` · Companion document: [threat-model.md](threat-model.md)

## 0. Status and normativity

- This document is **accepted** (maintainer adjudication 2026-11-12; sign-off table in §12). It is the pairing/key-lifecycle reference for v2.0.
- Binding requirements come from the design baseline [`.scratch/wire-protocol-v2/spec.md`](../../.scratch/wire-protocol-v2/spec.md) (cited as "design baseline §…"), notably §Security and Pairing Model. The accepted transport posture is ADR-0004 ([`../adr/0004-wire-protocol-v2-transport.md`](../adr/0004-wire-protocol-v2-transport.md)).
- Protobuf message numbering, error-code values, and field-number assignments are frozen in `docs/wire-protocol-v2/spec.md` (ticket 03). This document references that path only; the structures below are defined at the semantic level and carry no wire-field numbers.
- The key words MUST, SHOULD, and MAY are to be interpreted as described in RFC 2119. Uncited MUST/SHOULD statements are proposals of this document with the same sign-off status as §12.

## 1. Overview

Pairing transfers, out-of-band, enough mutual identity for automatic authenticated reconnection afterward:

- The Android Client learns the Server's identity public key and pins it.
- The Server learns the Android Client's identity public key and authorizes it.
- Both persist the result. Later connections authenticate over the encrypted control stream without repeating the out-of-band step.

Two pairing modes exist, selected by what the humans can do:

| Mode | Requires | Out-of-band material | MITM protection on first connection |
|---|---|---|---|
| QR (primary) | The Android Client can scan a QR displayed by the Server | Full Server identity public key + single-use nonce, via the QR (§4) | Cryptographic: pin known before key exchange (§6.4) |
| SAS (fallback) | Two humans can read codes on both screens and compare | The Server address, typed or otherwise conveyed; SAS compared on both screens | Only when the SAS is actually compared and confirmed on both ends; otherwise none (§7.2) |

A fingerprint comparison path (§7.4) is always available in both modes as the last-resort verification for headless or accessibility-constrained setups.

Pairing, authentication, version/capability negotiation, and configuration always complete before media subscription (design baseline §Control-Plane Semantics).

## 2. Identities

| Identity | Algorithm | Held where | Used for |
|---|---|---|---|
| Server identity key | Ed25519 | Server data directory, PKCS#8 PEM file (§10.2) | Self-signed TLS certificate; SPKI pin derivation; possession proven in every TLS 1.3 handshake (CertificateVerify) |
| Client identity key | ECDSA P-256 (SHA-256) | Android Keystore where available (§10.1) | Application-layer signed-challenge authorization over the encrypted control stream (§9); persisted as an authorized identity by the Server (§8) |

Rationale for the split: the Server identity key must match the SPKI-pinning and rustls certificate path proven in the prototype (Ed25519 certificate via rcgen/rustls; prototype report §4); the Client identity key must live in Android Keystore across the whole supported range, and Keystore EC P-256 signing is available from API 26 (the binding minimum; ADR-0004 decision 5), whereas Keystore Ed25519 requires newer APIs. The v2.0 boundary holds regardless: Keystore keys need NOT act as rustls TLS client-certificate keys — authorization is an application-layer check over the encrypted control stream (design baseline §Security and Pairing Model — Identity and Authorization). There is no mutual TLS in v2.0.

All hashing in this specification is SHA-256. All random values come from a CSPRNG.

## 3. Key agreement

Every connection uses TLS 1.3 ephemeral key exchange; static RSA and pre-shared-key modes are not enabled, and 0-RTT is disabled for this major (design baseline §Connection Rules; ADR-0004 decision 3). Consequences this specification relies on:

- The handshake transcript commits to both ephemeral keys, the certificate, and the Server's CertificateVerify signature before any application data flows.
- The TLS exporter (RFC 8446 §7.5) can derive connection-scoped secret material bound to that transcript.

No pairing mechanism in this document may weaken or replace the transport key exchange.

## 4. QR pairing payload

### 4.1 Encoding (decided here; PR-3 signed off 2026-11-12)

The QR payload is **text in QR byte mode**, a URI of the form:

```
audioshare-pair://p1?pm=<major>&addr=<host:port>&key=<b64url>&nonce=<b64url>
```

- Scheme: `audioshare-pair`. Authority: the literal payload-format tag `p1` (payload format version 1). Query fields in the fixed generation order shown; parsers MUST accept fields in any order and MUST reject unknown fields (unknown shapes are a payload-format change and MUST bump `p1` to `p2`).
- `b64url`: base64url without padding (RFC 4648 §5).
- The payload contains **no secret**; its security property is integrity of origin — it must be taken from the Server's own screen or produced by `as-cmd` on the Server computer (threat-model.md §5.2, TB6).
- QR error correction: level M; level L is permitted if the payload does not fit at M. Expected length ≈ 160–170 characters with an IPv4 address hint.

### 4.2 Field table

| Field | Size / format | Content | Rules |
|---|---|---|---|
| scheme + tag | fixed text | `audioshare-pair://p1` | Exact match required; anything else is not a v2 pairing payload |
| `pm` | decimal ASCII | Wire Protocol major, `2` for v2.0 | MUST equal the ALPN major of the connection used for pairing; mismatch fails explicitly (threat-model.md §5.7) |
| `addr` | `<host>:<port>` ASCII | Server address **hint** | Host: IPv4 dotted quad, bracketed IPv6, or DNS/mDNS name (`[A-Za-z0-9.-]+`, no percent-encoding). Port: 1–65535 decimal. The Client pre-fills its address field from this; the user may edit it. A hint is never a trusted identity (design baseline §Network Scope) |
| `key` | base64url, 43 chars (32 bytes raw) | Full Server identity Ed25519 public key | The Client derives the pin from this key: SPKI = the 44-byte DER `SubjectPublicKeyInfo` for the raw Ed25519 key (fixed 12-byte algorithm prefix + key); pin = SHA-256(SPKI DER) (§6.1) |
| `nonce` | base64url, 22 chars (16 bytes raw) | Random ≥128-bit pairing nonce | Lifecycle in §5; single-use |

Worked example (key/nonce values illustrative):

```
audioshare-pair://p1?pm=2&addr=192.168.1.20:65531&key=PQJfWm3y8r0KkQ2x7TnBvD1cHh5ZsA9eUgXwMkIoPjE&nonce=3bF9xQ2mN7vRtK1cYdSw4A
```

The same string is the **manual transfer format**: the Server GUI shows it as copyable text under the QR, and `as-cmd` prints it (§11.2). Conveying it by any channel is acceptable because it contains no secret.

## 5. Pairing nonce lifecycle

| Property | Rule |
|---|---|
| Generation | Server-side CSPRNG, 128 bits (16 bytes), when the pairing dialog opens in the Server GUI or when `as-cmd` pairing is invoked |
| Minimum entropy | ≥ 128 bits (design baseline §Security and Pairing Model — First Pairing) |
| Lifetime | 10 minutes from generation, measured by the Server's monotonic clock; expired nonces fail with a nonce-expired pairing error |
| Single use | A nonce is consumed atomically by the **first well-formed `PairingRequest` that presents it, regardless of the pairing's eventual outcome**; every later request fails with a nonce-invalid error. A failed pairing therefore requires a fresh QR |
| Outstanding bound | At most 8 nonces outstanding; generating the 9th evicts the oldest first |
| Storage | Memory only, inside the running Server Core; not persisted. A Server restart voids all pending nonces |
| Closing the dialog | Closing the Server GUI pairing dialog does not consume nonces; they simply expire |

Cited: design baseline §Security and Pairing Model — First Pairing ("random **at-least-128-bit**, short-lived, single-use pairing nonce").

## 6. Live pairing handshake

### 6.1 Preconditions

1. A QUIC connection exists with ALPN `audio-share/v2` (major mismatch already failed explicitly at the transport layer).
2. **QR mode:** the Android Client's certificate verifier has already accepted the Server because the presented SPKI hash equals the pin derived from the QR `key` field; otherwise the handshake fails before any pairing message (prototype-verified behavior; prototype report §1).
3. **SAS mode:** the Client has no pin yet and connects without a pin for this one session (TOFU-with-verification; see §7).
4. All pairing messages below travel on the reliable encrypted control stream, length-delimited Protobuf (design baseline §Control-Plane Semantics).

### 6.2 Message sequence

```
C→S  PairingRequest {
       pairing_mode        // "qr" | "sas"
       pairing_nonce[16]   // QR mode only; omitted in SAS mode
       client_random[32]   // CSPRNG
       client_pubkey       // ECDSA P-256, SEC1 compressed, 33 bytes
       wire_protocol_major // must equal ALPN major
       capabilities        // the Client's offered capability set (§6.3); the only
                           // session-authoritative intersection is NegotiateResponse
       client_display_name // ≤ 63 bytes UTF-8; default = device model name
     }

S→C  PairingResponse {
       pairing_mode echo
       pairing_nonce echo        // QR mode only
       client_random echo
       server_random[32]         // CSPRNG
       server_pubkey[32]         // raw Ed25519 identity public key
       wire_protocol_major echo
       capabilities echo         // echo of the Client's offered set (transcript binding, §6.3)
       binding[48]               // §6.3
       client_id                 // §8.1
       accepted_display_name
     }

C→S  PairingConfirm {
       binding_echo[48]          // Client's own exporter derivation
       client_proof[64]          // ECDSA-P256-SHA256 signature by the claimed identity key
                                 // over the pairing proof input (§6.3.1)
     }

S→C  PairingResult { ok | error_code }   // OK only after binding_echo + client_proof
                                         // verification and local approval; the Server
                                         // persists the Client record here (§6.3.1)
```

The Client also independently checks `server_pubkey` against the QR `key` field (QR mode) and aborts on mismatch — belt and braces on top of the SPKI pin. `wire_protocol_major` inside the messages MUST equal the ALPN major; a mismatch is an explicit protocol failure, never a fallback (threat-model.md §5.7).

### 6.3 Transcript binding

The pairing binds **identities + nonce + protocol major + capabilities** into the authenticated transcript, as required by design baseline §Security and Pairing Model — First Pairing. The mechanism:

```
binding_context = SHA-256( TLV(1, mode) || TLV(2, protocol_major)
                        || TLV(3, pairing_nonce)     // QR mode only
                        || TLV(4, client_random) || TLV(5, server_random)
                        || TLV(6, client_pubkey) || TLV(7, server_pubkey)
                        || TLV(8, capabilities_canonical)
                        || TLV(9, client_display_name) )

TLV(tag, value) = tag(u8) || len(u16, big-endian) || value
capabilities_canonical = count(u8) || for each id in ascending numeric order:
                         id(u32, big-endian)

TLV value encodings:
  mode                = the lowercase ASCII mode string "qr" or "sas"
                        (2 bytes — never the PairingMode enum number)
  protocol_major      = fixed 4-byte big-endian unsigned integer
                        (v2 = 00 00 00 02 — never a varint, never 1 byte)
  client_display_name = its UTF-8 bytes
  every other value   = its raw byte string as carried on the wire
```

The TLV value encodings above are normative (adjudication 2026-11-12) and **the same rule covers the pairing and the auth transcripts**: §9.1's `TLV(5, protocol_major)` is the identical fixed 4-byte big-endian integer, and every TLV there uses these value encodings.

```
binding[48]     = TLS-Exporter("EXPORTER audio-share v2 pairing", binding_context, 48)
sas_seed[32]    = TLS-Exporter("EXPORTER audio-share v2 sas",    binding_context, 32)
```

- The exporter output is connection-scoped and transcript-bound (RFC 8446 §7.5): only the two TLS endpoints of this exact handshake can derive it, and it cannot be recomputed on a different connection.
- The Server echoes nothing secret: `binding` is a proof both peers completed the same handshake with the same pairing inputs.
- `capabilities` is the Client's **offered** set of uint32 optional-feature ids (the normative wire field, spec §4.4) — negotiation happens later, so nothing "negotiated" exists yet; `PairingResponse` echoes it unchanged, and the echo is what the canonicalization above commits into the transcript. The list on the wire MUST already be unique and in ascending numeric order, and the canonical encoding is the validated list prefixed by its id count; a list with duplicates, descending order, or more than 255 ids is malformed and fails pairing with the registry's malformed-message code (spec §4.4). This numeric-ID-ascending, unique encoding replaces earlier drafts' ASCII-name-sorted canonicalization (adjudication 2026-11-12). `NegotiateResponse` (the later Negotiate exchange, spec §4.4) is the **only** authoritative session capability intersection.
- The Client MUST verify `binding` equals its own derivation before sending `PairingConfirm`. The Server MUST verify **both** `binding_echo` and `client_proof` (§6.3.1) **before** persisting the Client identity or sending `PairingResult`(OK). Any mismatch aborts pairing with a pairing-failure error.

#### 6.3.1 Client proof of possession (`client_proof`)

`binding_echo` alone is **superseded** (adjudication 2026-11-12). It proves only possession of this connection's TLS-Exporter secret — i.e. that the peer is one endpoint of *this* handshake — not possession of the private key of the `client_public_key` it submitted. That is insufficient because the Server persists the Client record before `PairingResult` and the session proceeds straight to NEGOTIATING (and thence READY and media subscription): a public-key-submission attacker could have a third party's identity persisted — and the current session authorized for media — without ever holding that key (identity mis-binding). `client_proof` replaces the binding_echo-only semantics; `binding_echo` remains as the symmetric exporter check but is no longer a possession proof.

The Client signs a pairing-specific, domain-separated input with the claimed identity's private key (ECDSA P-256 per PR-2, §2):

```
proof_input    = TLV(1, "audio-share v2 pairing confirm") || TLV(2, binding)
client_proof   = ECDSA-P256-SHA256(client_identity_key, proof_input),
                 encoded as fixed 64-byte r||s (IEEE P1363)
```

- Exactly **one** hash applies: the SHA-256 inside ECDSA-SHA256, over the raw `proof_input` bytes. Implementations MUST NOT pre-hash `proof_input` and pass the digest to a `SHA256withECDSA`-style API (same single-hash rule as §9.1).
- `binding` commits the whole transcript, including the claimed `client_public_key`, so the signature proves possession of exactly the submitted key on exactly this connection; the domain label keeps the signature non-replayable against the §9.1 auth signature.
- The Server verifies `client_proof` BEFORE persisting the identity and BEFORE returning a success `PairingResult`; verification failure is a pairing failure (201) and nothing is persisted. The proof rides the existing `PairingConfirm` message — no extra round-trip.

### 6.4 Possession of the Server private key

Proof of possession is the TLS 1.3 `CertificateVerify` signature over the handshake transcript, made with the identity key that produced the QR public key:

- **QR mode:** the Client derived the pin from the QR `key` and accepted only a handshake whose certificate SPKI hashes to that pin; `CertificateVerify` therefore proves the peer holds that exact private key. No additional application-layer Server signature exists in v2.0 — adding one would duplicate the proof the transport already provides.
- **SAS mode:** the Server presents its certificate unpinned (first contact); `CertificateVerify` proves possession of that presented key, and the SAS (§7) commits the human's confirmation to exactly that key.

## 7. SAS fallback (no camera)

### 7.1 Derivation

`SAS_seed = sas_seed` from §6.3. Then:

```
SAS = decimal( big-endian u32 of the first 4 bytes of sas_seed  mod 1_000_000 ),
      zero-padded to 6 digits
```

The modulo-1,000,000 mapping from 32 bits introduces a relative bias below 10⁻⁵, which is negligible for a human-compared 6-digit code. The six-digit SAS carries **≈ 19.93 bits**: a single random verification attempt matches with probability **≈ 10⁻⁶** (adjudication 2026-11-12 — earlier drafts wrongly claimed a 2⁻³⁰ per-attempt bound; §7.2 states the corrected claim and its compensating controls). The SAS is a **commitment to the completed ephemeral key exchange**: it is computed from the finished transcript, so neither side can adjust key material after seeing the other's value (design baseline §Security and Pairing Model — First Pairing: "ephemeral key exchange with hash commitment").

### 7.2 What an unverified SAS does and does not claim

Stated explicitly, as required by the design baseline:

- **An unverified SAS comparison does not protect against an active first-connection MITM.** If the users skip the comparison and merely confirm, the flow is trust-on-first-use with no MITM protection: a terminating attacker completes both legs, both ends display codes, and no one looks.
- **When actually compared and confirmed on both ends**, the SAS bounds an undetected MITM at **≈ 10⁻⁶ per verification attempt** (six digits ≈ 19.93 bits, single-attempt random match). A terminating MITM must grind ephemeral key choices or guess the code; each attempt succeeds with probability ≈ 10⁻⁶, so the undetected-MITM probability grows with the number of attempts (≈ attempts / 10⁶) and the guarantee is **bounded by guess attempts**: the pairing-failure rate limit of threat-model.md §6 (5 failures per sliding 60 s per source address) caps an attacker at ~50 attempts inside the 10-minute pairing window, i.e. a per-window per-address bound of ≈ 5×10⁻⁵.
- **Compensating controls** for the deliberately small code space (six digits retained by the adjudication): the verification-attempt rate limits above; the out-of-band verification paths — the always-available SPKI fingerprint comparison (§7.4) and the Server address hint displayed beside the code (§7.3); and the fact that every guess is an observable pairing failure that feeds those rate limits.
- The SAS path therefore provides its protection **only** under the human step at trust boundary TB6 (threat-model.md §4).

### 7.3 Presentation format (decided here; PR-7 signed off 2026-11-12)

| Property | Value |
|---|---|
| Digits | 6 decimal digits, zero-padded |
| Grouping | `123 456` — one space between the third and fourth digit |
| Server GUI | Displayed under the QR, labeled `Pairing code`, with the Server address hint shown alongside so a user pairing against the wrong host can notice |
| as-cmd | Printed as `Pairing code: 123 456` |
| Android Client | Displayed prominently during pairing, labeled `Pairing code` |
| Confirmation | Both ends require an explicit user action: the Client "Confirm pairing", the Server "Accept device" / `as-cmd` confirmation. Pairing completes only after both |
| Admission gate (SAS mode) | The Server accepts SAS-mode `PairingRequest`s only while a local pairing window is open — the Server GUI pairing dialog or an active `as-cmd pair` invocation (an admin action, TB3). SAS has **no nonce**; this admin-opened window is the admission control (threat-model.md §6) |

### 7.4 Fingerprint path (always available)

Alongside the SAS, both ends always display the Server SPKI fingerprint:

```
Fingerprint = SHA-256(SPKI DER of the Server certificate),
              rendered as 8 groups of 8 uppercase hex characters, space-separated
Example: 1A2B3C4D 5E6F7081 92A3B4C5 D6E7F809 1A2B3C4D 5E6F7081 92A3B4C5 D6E7F809
```

This is the verification path when no camera is available and the SAS display is impractical, and the forensic path when the SAS cannot be trusted visually (screen reader, accessibility constraints, suspected tampering).

### 7.5 Hard rules (binding from the design baseline)

| Rule | Source |
|---|---|
| A six-digit code is never directly stretched into a session key. The SAS is only a human-comparison token derived from the transcript; all key material comes from TLS 1.3 | design baseline §Security and Pairing Model — First Pairing |
| No custom password PAKE is introduced in v2.0 | design baseline §Security and Pairing Model — First Pairing |
| An unverified SAS comparison claims no active-MITM protection | design baseline §Security and Pairing Model — First Pairing; §7.2 above |

## 8. Post-pairing persistence

### 8.1 Server-side authorization store

A single JSON file, `pairing/clients.json` in the Server data directory (§10.2):

```json
{
  "version": 1,
  "clients": [
    {
      "client_id": "first 8 bytes of SHA-256(public_key), hex",
      "public_key": "base64url of the 33-byte compressed P-256 point",
      "display_name": "UTF-8, ≤ 63 bytes",
      "state": "active | revoked",
      "created_at": "RFC 3339 UTC",
      "last_seen_at": "RFC 3339 UTC"
    }
  ]
}
```

The store contains **public keys and metadata only — no secrets**. `client_id` is derived, not allocated, so concurrent writers cannot collide on identifiers.

### 8.2 Client-side pairing store

One record per paired Server, in Android app-private storage:

| Field | Content |
|---|---|
| server_spki_sha256 | 32 bytes — the pin |
| server_display_name | Shown in the Client's Server list |
| address_hint | Last-used address(es), editable |
| own_key_alias | Android Keystore alias of this pairing's Client identity key |
| paired_at | RFC 3339 UTC |

### 8.3 Key-change rules

| Event | Required behavior |
|---|---|
| Server presents a certificate whose SPKI hash ≠ the Client's stored pin | Handshake fails with an explicit pin-mismatch error; the Client prompts **explicit re-pairing**. The Client MUST NEVER silently replace a stored pin because a different key appeared (design baseline §Security and Pairing Model — First Pairing: "Key changes require explicit re-pairing or confirmation, never silent replacement") |
| Server receives an auth attempt from an unknown Client identity | Refused with an authorization-failure error. The Server MUST NEVER replace an existing Client record because a new key appeared; replacement of a pairing is an explicit admin action in the Server GUI / `as-cmd` ("Replace pairing for device X?") |
| Server identity reset (operator action) | A new identity key is generated **and** all Client records are invalidated in one atomic step. Every previously paired Client then fails its pin check and is prompted to re-pair (design baseline §Key Lifecycle and Local Administration: "Resetting a Server identity invalidates old pairings") |
| Client identity key lost/replaced (new phone, app data cleared) | The Server sees an unknown identity; the operator pairs again, optionally after removing the stale record. There is no key-rollover handshake in v2.0 — **rotation = reset + re-pairing** |
| Recovery | Local administration only. **No universal recovery backdoor** — no master credential, vendor key, or unauthenticated recovery mode exists (design baseline §Key Lifecycle and Local Administration) |

Confirmation surfaces: the Server GUI shows a dialog naming the consequence; `as-cmd` requires an explicit `--yes` flag after printing the warning for identity reset and pairing replacement.

## 9. Revocation and later-connection authentication

### 9.1 Authentication on later connections

```
C→S  AuthInit   { client_pubkey }
S→C  AuthChallenge { server_random[32] }
C→S  AuthProof  { signature }
S→C  AuthResult { ok | error_code }
```

```
auth_context = TLV(1, "auth") || TLV(2, server_random) || TLV(3, client_pubkey)
            || TLV(4, server_pubkey) || TLV(5, protocol_major)
            || TLV(6, TLS-Exporter("EXPORTER audio-share v2 auth", "", 32))
signature   = ECDSA-P256-SHA256(client_identity_key, auth_context),
              encoded as fixed 64-byte r||s (IEEE P1363)
```

- Exactly **one** hash applies (adjudication 2026-11-12, resolving the double-hash ambiguity of earlier drafts): the SHA-256 inside ECDSA-SHA256 runs over the **raw `auth_context` bytes**. Implementations MUST NOT pre-hash `auth_context` and pass the digest to a `SHA256withECDSA`-style API — that double-hashes and breaks verification.
- `TLV(5, protocol_major)` uses the §6.3 TLV value encodings — a fixed 4-byte big-endian unsigned integer (v2 = `00 00 00 02`); one encoding rule covers the pairing and the auth transcripts.
- The signature's exporter component binds it to this exact TLS connection; the fresh `server_random` makes replay across connections impossible even if both randomness and transcript were recorded.
- A Client MAY present more than one stored Server record's identity — each pairing has its own key, and the Server matches records by public key.
- Failed attempts are rate-limited (threat-model.md §6): 5 failures close the connection; 20 failures per sliding 60 s window penalize the source address.

### 9.2 Revocation semantics

Binding rule (design baseline §Key Lifecycle and Local Administration): **revocation affects new connections immediately and terminates affected active sessions promptly.**

| Aspect | Rule |
|---|---|
| New connections | The authorization state is consulted at every `AuthInit`; a revoked or deleted identity is refused immediately. No cache outlives the store |
| Active sessions | The Server Core watches the authorization-state generation; on change, every session authenticated by an affected identity is closed with a defined close reason in the pairing/authorization-failure category (non-retryable without re-pairing), **within ≤ 5 s** |
| Revoke vs delete | `revoke` keeps the record with `state: revoked` (the `client_id` stays reserved, visible in the Server GUI for audit); `delete` removes it entirely. Both refuse access |
| Client-side | A Client that deletes a paired Server stops connecting to it immediately; there is no Server-side notification channel required in v2.0 |

## 10. Key storage

### 10.1 Android Client (Android Keystore where available)

| Property | Rule |
|---|---|
| Algorithm | ECDSA P-256, purpose SIGN, digest SHA-256 |
| Generation | `KeyGenParameterSpec` with `setUserAuthenticationRequired(false)` — background reconnects must not be gated on user presence (design baseline §Android Shape) |
| StrongBox | Used when available and key generation succeeds; otherwise fall back to ordinary Keystore |
| Keystore unavailable (generation fails) | Generate the key in app-private storage (file mode 0600) and mark the pairing record software-backed (diagnostic field only — no protocol difference). This is the documented degradation, visible in the Client's device list |
| Exportability | Keystore keys are non-exportable; there is no Client-side key backup by design. Losing the Client key = re-pairing |
| v2.0 boundary | Keystore keys need NOT act as rustls TLS client-certificate keys; no mutual TLS exists in v2.0 (design baseline §Identity and Authorization) |

### 10.2 Server identity key and authorization store files

| File | Content | Permissions (POSIX) | Format |
|---|---|---|---|
| `<data dir>/identity/server-identity.pem` | Ed25519 private key | File 0600, containing directory 0700 | PKCS#8 PEM |
| `<data dir>/pairing/clients.json` | Client records (public keys + metadata, §8.1) | File 0600, containing directory 0700 | JSON, version field |
| `<data dir>/pairing/clients.json.lock` | Exclusive-lock target for headless admin ops | 0600 | lock file |

`<data dir>` is the platform-standard per-user application-data location of the Server. On Windows, the equivalent location is used and permissions are enforced via ACLs limited to the running user (plus local Administrators).

**Start-time permission enforcement (decidable):** on POSIX, if the identity key file is readable by group or other (or its containing directory is), Server Core refuses to enable v2 pairing/auth at startup — v1 and capture are unaffected — and logs the reason. An explicit `--allow-loose-key-permissions` flag overrides this (discouraged; logged). On Windows, a wider-than-expected ACL produces a warning, not a refusal (Windows ACL enumeration is advisory here; recorded as a decision in §12).

### 10.3 Concurrent access and corruption handling

| Concern | Rule |
|---|---|
| Ownership | The running Server Core owns the identity key and the authorization state. The Server GUI and `as-cmd` share **one pairing Core** and one identity/authorization state by operating that Core (design baseline §Key Lifecycle and Local Administration); they never write the files directly while a Core is running |
| Headless writes | When no Core is running, `as-cmd` admin operations take an exclusive advisory lock on `clients.json.lock` for the operation's duration. Two direct writers are therefore impossible; blocking on the lock, not corrupting, is the defined behavior |
| Write discipline | Every write is atomic: write to a temp file, fsync, rename over the target, keep the previous version as `clients.json.bak` (one backup generation) |
| Corruption | If the store fails JSON/schema validation at load, Server Core **refuses to enable v2 pairing/auth** (fail-closed: no Client is accepted), logs the failure, and leaves v1 unaffected. It never regenerates or silently truncates the store. Recovery: restore `clients.json.bak` or replace the file manually, then restart the Core |
| Key file corruption | Same fail-closed treatment for the identity key: v2 stays disabled; the operator resets the identity explicitly (a reset is the designed response; §8.3) |

### 10.4 Backup guidance

| Item | Guidance |
|---|---|
| `server-identity.pem` | Never backed up automatically; never included in config export or diagnostics. Manual backup is permitted and MUST be warned about: the file is the Server identity — treat it like a password. Losing it is an identity reset; all Clients re-pair (§8.3) — the designed recovery path |
| `clients.json` | Low sensitivity (public keys + metadata). Backup restores the list of paired devices; stale entries simply fail auth |
| Client identity key | Non-exportable by design (§10.1); no backup exists |

## 11. Administration surfaces

### 11.1 Shared pairing Core

The Server GUI and `as-cmd` both drive the **same pairing Core** inside Server Core and observe the same identity/authorization state (design baseline §Key Lifecycle and Local Administration). There is no second source of truth and no GUI-only or as-cmd-only pairing state.

### 11.2 Headless pairing

`as-cmd` provides local pairing commands:

| Invocation | Behavior |
|---|---|
| `as-cmd pair` | Generates a nonce, prints the QR payload URI, the SAS, and the SPKI fingerprint, and renders the QR as UTF-8/ANSI terminal art **where the terminal supports it** ("where practical"); otherwise the printed URI can be rendered as a QR by any tool |
| `as-cmd pair --sas` | Omits QR rendering; prints URI + SAS + fingerprint — the SAS/fingerprint path is always available |
| `as-cmd pair --accept` / interactive confirm | Completes pairing after the Client's `PairingConfirm`, mirroring the GUI's "Accept device" |
| `as-cmd clients list|revoke|delete|replace` | Authorization administration over the same Core state |

Pairing is a local operation: the as-cmd instance must run on the Server computer (TB3); there is no remote pairing in v2.0.

### 11.3 Test-only bypasses

Binding rule (design baseline §Key Lifecycle and Local Administration): **test-only authentication bypasses stay in test builds or isolated test configuration.**

- Any bypass (fixed test credential, skipped SPKI pin, deterministic keys) lives behind a compile-time test feature of the protocol crates. Production binaries contain **no runtime flag, environment variable, or configuration key** that can enable a bypass.
- Test configuration that exercises bypasses lives with the test code/isolated test config, never in the default configuration path.

## 12. Decisions proposed

Signed off by maintainer adjudication 2026-11-12; see tickets 03/04 Comments.

| ID | Decision | Proposed value | Where | Maintainer sign-off |
|---|---|---|---|---|
| PR-1 | Server identity key | Ed25519, self-signed certificate; pin = SHA-256(SPKI DER) | §2, §6.1 | ✅ accepted (2026-11-12) |
| PR-2 | Client identity key | ECDSA P-256 + SHA-256, Android Keystore (StrongBox when available), non-exportable; software fallback marked in the record | §2, §10.1 | ✅ accepted (2026-11-12) |
| PR-3 | QR payload encoding | Text URI `audioshare-pair://p1?…`, QR byte mode, ECC M (L fallback); unknown query fields rejected | §4.1 | ✅ accepted (2026-11-12) |
| PR-4 | QR fields | `pm` (major), `addr` (host:port hint), `key` (raw 32-byte Ed25519 public key, base64url), `nonce` (16 bytes, base64url) | §4.2 | ✅ accepted (2026-11-12) |
| PR-5 | Pairing nonce | 128-bit CSPRNG; 10 min lifetime; single-use (burned by the first well-formed PairingRequest); ≤ 8 outstanding; memory-only | §5 | ✅ accepted (2026-11-12) |
| PR-6 | Transcript binding + Client proof of possession | SHA-256 over the TLV canonicalization of §6.3, with `capabilities_canonical` = numeric-ID-ascending, unique encoding; TLV value encodings normative — `mode` = lowercase ASCII `"qr"`/`"sas"`, `protocol_major` = fixed 4-byte big-endian (v2 = `00 00 00 02`), one rule covering the pairing and auth transcripts; TLS-Exporter labels `EXPORTER audio-share v2 pairing` (48 B) and `EXPORTER audio-share v2 sas` (32 B); PairingRequest.capabilities = the Client's **offered** set, echoed by PairingResponse for transcript binding (NegotiateResponse is the only session-authoritative intersection); `client_proof` = one ECDSA-P256-SHA256 signature (64 B r\|\|s) by the claimed identity key over the pairing proof input (§6.3.1), verified by the Server before persisting the identity or sending PairingResult(OK); `binding_echo`-only possession semantics superseded | §6.2, §6.3 | ✏️ accepted as revised (2026-11-12) |
| PR-7 | SAS | 6 digits, `big-endian u32(sas_seed[0..4]) mod 1_000_000`, zero-padded, displayed `123 456`; explicit confirm on both ends; ≈ 19.93 bits — single-attempt random match ≈ 10⁻⁶, not 2⁻³⁰; undetected-MITM probability bounded by guess attempts under the pairing rate limits; compensating controls per §7.2; SAS-mode admission gate = the admin-enabled local pairing window (§7.3) | §7.1–§7.3 | ✏️ accepted as revised (2026-11-12) |
| PR-8 | Fingerprint presentation | SHA-256(SPKI), 8 groups of 8 uppercase hex | §7.4 | ✅ accepted (2026-11-12) |
| PR-9 | Auth scheme | Signed challenge (no bearer token): AuthInit/Challenge(32 B random)/Proof(one ECDSA-P256-SHA256 signature, r\|\|s 64 B, over the RAW auth_context bytes — exactly one hash, never pre-hashed-then-SHA256withECDSA — incl. exporter component)/Result | §9.1 | ✏️ accepted as revised (2026-11-12) |
| PR-10 | Authorization store | JSON `pairing/clients.json`, public data only; `client_id` = first 8 bytes of SHA-256(public_key); states active/revoked | §8.1 | ✅ accepted (2026-11-12) |
| PR-11 | Server file permissions | POSIX 0600 files / 0700 dirs; refuse v2 enablement on loose permissions with `--allow-loose-key-permissions` override; Windows: warn-only on wide ACLs | §10.2 | ✅ accepted (2026-11-12) |
| PR-12 | Concurrency/corruption | Core-owned state + exclusive lock for headless writes; atomic write + 1 backup generation; corruption ⇒ v2 fail-closed, never auto-regenerated | §10.3 | ✅ accepted (2026-11-12) |
| PR-13 | Revocation timing | New connections immediate; active sessions closed within ≤ 5 s | §9.2 | ✅ accepted (2026-11-12) |
| PR-14 | Test bypasses | Compile-time test feature only; no runtime bypass in production binaries | §11.3 | ✅ accepted (2026-11-12) |

## 13. References

- Design baseline: [`../../.scratch/wire-protocol-v2/spec.md`](../../.scratch/wire-protocol-v2/spec.md) — §Security and Pairing Model (Identity and Authorization; First Pairing; Key Lifecycle and Local Administration; Resource and Privacy Controls), §Connection Rules, §Control-Plane Semantics, §Dual-Protocol Migration, §Android Shape.
- ADR-0004: [`../adr/0004-wire-protocol-v2-transport.md`](../adr/0004-wire-protocol-v2-transport.md) — accepted transport and security posture.
- Threat model: [threat-model.md](threat-model.md) (this directory) — assets, actors, trust boundaries, attack/limit mapping.
- Prototype report: [`../../.scratch/wire-protocol-v2/prototype-report.md`](../../.scratch/wire-protocol-v2/prototype-report.md) — SPKI-pinning rejection and credential authorization proven on device; production pairing explicitly listed as not yet validated.
- Normative wire specification (message numbering, error-code registry, Protobuf field numbers; path referenced only): `docs/wire-protocol-v2/spec.md` (ticket 03).
- External: RFC 2119, RFC 4648 §5 (base64url), RFC 8446 §7.5 (TLS exporters), RFC 6189 and RFC 9383 (SAS design background), NIST FIPS 186-5 (ECDSA P-256), RFC 8032 (Ed25519).
