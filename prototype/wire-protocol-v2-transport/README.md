# wire-protocol-v2 transport prototype (ticket 01)

Throwaway prototype answering one question: can quinn + rustls/TLS 1.3 carry
this project's v2 transport on a real arm64 Android client at API 26, within
the accepted native-integration and binary-size boundaries?

It does NOT freeze the v2 wire layout, implement production pairing, or
authorize merging anything into production code.

## Layout

```
prototype/wire-protocol-v2-transport/
  README.md          # this file
  server/            # Rust test server + client bins + integration tests (server ticket)
  android-client/    # standalone gradle project (app/ Kotlin + native/ Rust cdylib)
  scripts/           # e2e helper scripts
  evidence/          # committed run logs with command/version headers
```

## Android client (this half of ticket 01)

- Build + size evidence: [`android-client/BUILD.md`](android-client/BUILD.md)
- Branch: `ticket/01-transport-android`, merged into
  `prototype/wire-protocol-v2-transport`
- `cargo test` (host, in `android-client/native/`): **54 tests green** —
  frame codec, client control state machine, gap/dup/reorder accounting,
  deterministic byte verification, SPKI-pinned verifier (rcgen certs), ring
  buffer, stats JSON contract.
- Measured sizes (arm64-v8a): raw `.so` 8,761,880 B → stripped 2,303,408 B
  (2.20 MiB); release APK 2,337,519 B vs empty baseline 33,429 B —
  **native increment 2.20 MiB, within the ~5 MiB gate**.
- minSdk 26 gate holds; zero external Android dependencies; R8 rule required:
  `-keep class dev.prototype.wireproto.ProtoNative { *; }`.
- JNI surface: 9 low-frequency functions (createSession/connect/subscribe/
  unsubscribe/stop/destroy/getState/getStats/attachBuffer). Media crosses JNI
  exactly once, as one attached direct ByteBuffer; no per-datagram calls.

Device-gated and NOT RUN from this host (no device attached): real-device
handshake, ALPN/0-RTT on-wire assertions, foreground-service lifecycle,
screen-off/process death, Wi-Fi↔cellular migration (scaffolded, see
BUILD.md), impairment matrix. The full GO/NO-GO verdict is the ticket
reporter's job once device runs exist.

## Prototype protocol conventions this client implements

Binding for both implementers (from `.scratch/wire-protocol-v2/prototype-brief.md`):

- Control plane: one QUIC bidirectional stream, `u32 BE frame_len | u8 type |
  payload`, `frame_len = 1 + payload.len()`, cap 64 KiB; types 0x01–0x07
  (AUTH, AUTH_OK, AUTH_ERR, SUBSCRIBE, MEDIA_START, UNSUBSCRIBE, CLOSE).
- Media plane: QUIC DATAGRAM, fixed 20-byte BE header
  `u32 stream_instance_id | u32 config_generation | u32 sequence | u64 timestamp`,
  payload raw f32le interleaved, frame-aligned.
- **Deterministic content convention (client-side interpretation):** the
  sample value at absolute sample-frame position `p` is
  `(2π·440·p/48000).sin() as f32` computed in f64, and **every channel of
  frame p carries that same value** (`p` is the frame position carried by the
  header `timestamp`, not a per-channel sample index). The server must use the
  identical formula and convention or the client's byte verification will
  report byte_errors uniformly — that itself is diagnosable evidence, but it
  would not be a transport failure.
- ALPN exactly `audio-share/v2`; TLS 1.3 only; 0-RTT never configured.
- SPKI pinning: server self-signed ECDSA P-256 cert; client verifies
  sha256(cert SPKI DER) against the operator-supplied 64-hex pin; no CA, no
  name/expiry checking — the pin IS the identity check for the prototype.
- Default test credential: `test-credential-do-not-use`.
- Client buffer: 4 MiB overwrite-oldest ring (~10.9 s of stereo f32 @48 kHz),
  9,600 B (50 ms) prefetch target before playback starts.
- A capture restart bumps `stream_instance_id`; seq accounting resets per
  instance. Gap counters are arrival-time evidence (late fill-ins count as
  `reordered` and do not decrement `gaps`).

## Run

- Android client build: see `android-client/BUILD.md`.
- Server + end-to-end: see `server/` (server ticket) once merged.
