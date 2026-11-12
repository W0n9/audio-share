# Wire Protocol v2 golden fixtures

Cross-language golden vectors for Wire Protocol v2.0 (ticket 05, phase 2 of
`wire-protocol-v2`). They are hand-derived deterministic encodings from the
accepted normative documents — **not** from the prototype's throwaway
`BRIEF.md` wire format, and not produced by any implementation encoder:

| Source | Role |
| --- | --- |
| `docs/wire-protocol-v2/spec.md` (accepted 2026-11-12) | §3 framing · §4 control schema · §5 state machine · §6 error registry · §7 media plane · §9 diagnostics · §10 resource limits |
| `docs/wire-protocol-v2/pairing.md` (accepted 2026-11-12) | pairing/auth message sequences, transcript binding, `client_proof` |
| `protos/wire-protocol-v2/control.proto` | the control-plane schema (a transcription of spec §4); the media plane carries **no protobuf** — a fixed 24-byte header (spec §7.1) on QUIC DATAGRAMs |

The fixture set is the verification-harness expectation: any implementation
(Rust Server or Kotlin Android Client) replays the same files and reports
byte-level equality. No implementation-specific encoder participates.

## Layout

```
manifest.json     index of every case (id, file, category, spec cite, summary)
test-keys.json    TEST-ONLY keys and fixed constants — read the warning inside
control/*.json    happy paths, error-registry cases, negative space
media/*.json      media header/payload samples and negative media
limits/*.json     resource-limit boundary cases (§10 / TM rows)
```

## Case file format

Each case is one JSON object:

| Field | Meaning |
| --- | --- |
| `id` / `category` | stable identifiers; `manifest.json` indexes them |
| `spec_cite` | the normative sections the case is derived from |
| `description` | what the case pins, in prose |
| `preconditions` | state-machine position, store state, fixture constants in play |
| `steps` | ordered exchange: each step is a `frame` (control stream), `datagram` (media), `raw-stream` (framing-level bytes), or `transport-close` (QUIC application close code only, no frame) |
| `assertions` | the prose assertions the replay must confirm |
| `size_bounds` | optional spec size limits checked during replay (128 B pre-auth, 320 B PairingResponse, 512 B post-auth Error) |
| `derived` | optional recorded derivations (auth_context, binding_context, SAS code, …) for hand-verification |

Step fields:

- `frame` steps carry `frame_hex` (`u32 BE length | Frame`, the exact §3
  framing), `body_hex` (the serialized `Frame` message alone), `length` (the
  `Frame.length` value), and `decoded` (the logical message content).
- `datagram` steps carry `datagram_hex` — the full application payload of the
  QUIC DATAGRAM: 24-byte big-endian header + payload (no length prefix; the
  media plane is not framed).
- `raw-stream` steps carry `bytes_hex` — raw control-stream bytes that are
  deliberately NOT a decodable frame (framing violations).
- `transport-close` steps assert which peer closes and with which §6 registry
  code, and that **no** control frames are sent for that failure mode.
- `dir` is `C->S` / `S->C`; `channel` is `control-stream` / `datagram`.

## Replay / verification-harness contract

1. Read `manifest.json`; for each case, replay its steps in order against a
   session driven to the case's `preconditions`.
2. **Encoding direction**: for every `frame` step, construct the message from
   `decoded` (see the encoding policy below), serialize with the protobuf
   schema, prepend the `u32 BE` length, and the bytes MUST equal `frame_hex`
   (equivalently `body_hex` for the message alone). For `datagram` steps,
   build the 24-byte header from `decoded` and the bytes MUST equal
   `datagram_hex`.
3. **Decoding direction**: parse `body_hex` / `datagram_hex` with the schema;
   the result MUST match `decoded` exactly (no extra fields; unknown-field
   steps assert the documented ignore/body-unset behavior instead).
4. **State machine**: feeding the `C->S` steps and driving the implementation's
   state machine MUST make it emit the `S->C` steps byte-identically, in order,
   plus the listed `transport-close` behavior (or an absence of frames, where
   the case says so).
5. **Crypto**: signature-carrying cases (`auth-*`, `pairing-*`) VERIFY the
   recorded signatures against the recorded public keys and the inputs derived
   per the case's `derived` formulas (auth_context / proof_input TLV
   construction, exactly one SHA-256 hash). Implementations never reproduce
   signatures — verification is deterministic, signing is not replayed.
6. `size_bounds`, when present, are checked during replay.

A case passes only if every step matches at the byte level.

## Encoding policy (byte-exact reproduction)

The vectors use plain proto3 deterministic encoding; all listed cases observe:

- Fields are serialized in ascending field-number order (the protobuf norm).
- Implicit presence: scalars/strings/bytes equal to their proto3 default
  (`0`, `false`, `""`) and empty repeated fields are **not encoded**, even when
  `decoded` lists them (their listing documents the semantic value — e.g.
  `request_id: 0` = unsolicited, `protocol_minor: 0`).
- A submessage listed in `decoded` is encoded present — including a
  present-but-empty `Status` (`{}`) on success responses, encoded as
  tag + zero length (spec D5: field 2 of every response is `Status`).
- Repeated scalar fields use proto3 packed encoding (capabilities).
- `fixed64` fields are **little-endian** on the wire (protobuf rule) — in
  contrast to the §3 length prefix and the entire §7.1 media header, which are
  big-endian.
- `sint64` fields use zigzag varints; `fixed64` counters do not.
- protobuf.Enum fields are varints carrying the spec's enum values
  (`PAIRING_MODE_QR=1`, `PAIRING_MODE_SAS=2`, `CODEC_PCM=1`,
  `SAMPLE_FORMAT_F32LE=1`, … — SampleFormat numbers mirror v1
  `AudioFormat.Encoding`).

## Crypto / determinism note (TEST-ONLY material)

`test-keys.json` holds the fixture-fixed test material: a P-256 client
identity key (private scalar recorded, derived by a documented SHA-256
formula), the Ed25519 test server key (seed, raw public key, SPKI, pin), fixed
"randomness" constants, and fixed TLS-Exporter stand-ins. ECDSA signing
nonces (`k`) are pinned per signature by a recorded formula, so every recorded
`r||s` is reproducible; implementations still only ever VERIFY. All of it is
test-only material — never ship it (pairing.md §11.3: test bypasses stay behind
compile-time test features; production binaries contain no path to them).

The TLS-Exporter outputs (`binding`, `binding_echo`, the auth exporter
component, sas_seed) are connection-scoped TLS 1.3 secrets; the vectors replace
them with fixed constants so the recorded signatures and derivations are
verifiable without a TLS session. An implementation replaying the vectors
injects these constants where the exporter values would be.

## Coverage

`manifest.json` carries the authoritative per-code coverage map
(`error_registry_coverage`). Every band of §6 has at least one vector:

| Band | Codes vectorized | Notes |
| --- | --- | --- |
| 100 | 101 | 102 is unreachable in v2.0 — no capability ids are assigned; unknown ids are ignored (`control.negotiate-unknown-capability-ignored`) |
| 200 | 201, 202, 203, 204 | incl. the 202-vs-204 contrast, `client_proof` failure, and approval declined (no-detail PairingResult(201)); 205 has no wire bytes (local TLS classification); 206 is timing-only |
| 300 | 301, 302, 303 | 301 signals via transport close code only |
| 400 | 401 (×7 shapes), 402 (fatal ×3 + non-fatal), 403 | |
| 500 | 501 (×2), 502, 599 | 503 has no pinned wire sequence in the accepted spec (flagged, not invented) |
| 600 | 601, 602 | 601 signals via transport close code only |
| 700 | 701 | Close never echoed |

Media coverage: header/payload samples (all header fields), reserved-flag
ignore (bit 2 and bit 31), sequence wrap, `stream_instance_id` change (with the
control-stream pair), `config_generation` change mid-stream (with
stale-generation drop), reduced-MTU sizing and payload alignment, the 1176 B
interop floor, and malformed-media drop.

## Flagged spec gaps (for the maintainer)

Recorded in `test-keys.json` → `derivation_assumptions` and in the affected
case files:

1. **Oneof case numbers for Negotiate/NegotiateResponse** — spec §4.1's band
   table lists cases 1–6 for band 1–15, but §4.4 defines Negotiate and
   NegotiateResponse in that band without numbers. The schema assigns 7 and 8
   (next free in the band).
2. **Binding TLV encodings** — pairing.md §6.3 does not state how `mode` and
   `protocol_major` are encoded inside the TLV canonicalization. The vectors
   use the §6.2 ASCII mode string (`"qr"`/`"sas"`) and `protocol_major` as u32
   big-endian (consistent with `capabilities_canonical`'s u32 BE).
3. **Reserved media flag bits** — spec §7.5 lists "reserved flag bits set" as
   malformed media, while §7.1 and the §12 parked row (accepted with D4) say
   they are ignored. The vectors follow the adjudicated ignore rule
   (`media.flags-reserved-ignored`).
4. **PairingResult(201) detail** — spec §5.2 writes `detail "declined"` for the
   approval-declined row, but §4.2/TM-6 forbid `detail` in pre-auth responses.
   The declined case is deliberately not vectorized (both readings agree on
   201 + Close); a maintainer sentence is needed.
5. **CONNECTING stray-frame code** — spec §5.2's CONNECTING row says
   "Error(401 or 402)" without a decidable split; the state-violation vectors
   use the pinned NEGOTIATING/READY rows instead.
