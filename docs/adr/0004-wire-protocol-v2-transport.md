# ADR-0004: Wire Protocol v2 transport uses quinn (QUIC + TLS 1.3) with one reliable control stream and unreliable DATAGRAM media

Status: proposed (2026-11-11)

## Context

The Wire Protocol v2 effort ([`.scratch/wire-protocol-v2/spec.md`](../../.scratch/wire-protocol-v2/spec.md)) reopens the UDP-header/reorder work explicitly deferred by [ADR-0001](0001-server-rewrite-baseline.md), on a separately scoped port and connection lifecycle: the frozen v1 contract ([`docs/protocol.md`](../protocol.md)) and the released v0.3.4 Android Client are untouched, and this ADR does not revise ADR-0001.

Issue 01 ([prototype report](../../.scratch/wire-protocol-v2/prototype-report.md), rev 3) validated the primary transport candidate end-to-end on a real arm64 Android device (Android 12 / API 31): QUIC handshake with ALPN `audio-share/v2`, pinned-SPKI Server identity, credential authorization over an encrypted control stream, byte-exact DATAGRAM media through a native ring → direct `ByteBuffer` → `AudioTrack` path (65,486 datagrams / 62.9 MB, zero byte errors, zero control errors), screen-off playback via a foreground Service, clean two-sided close, a desktop impairment matrix (loss/reorder/duplicate/jitter/congestion) with reliable control throughout, dynamic datagram sizing under a reduced 1280-byte path MTU, and a +2.20 MiB stripped arm64 `.so` against the provisional ~5 MiB gate. The maintainer waived the two hardware-condition items (API 26 device runtime; Wi-Fi↔cellular migration — no such scenario exists in their environment); ordinary authenticated reconnect, the shipped requirement when migration is unavailable, is proven on device. Verdict: **GO**.

The design baseline's requirements that bind this decision: explicit version negotiation (ALPN major), secure by default (mandatory capabilities cannot be negotiated off; a protocol or authentication failure never downgrades to v1 or plaintext), cross-language schemas and vectors, drop-eligible media that must not be retransmitted, and one shared capture feeding independently authenticated Client sessions.

## Decision

1. **Transport stack**: quinn 0.11.x + rustls (TLS 1.3 only) on a single UDP endpoint, separate from v1's port. ALPN `audio-share/v2` carries the Wire Protocol major; a major mismatch fails explicitly before any application data. Capabilities negotiate optional features only within a compatible major.
2. **Two planes, one connection**: one reliable bidirectional QUIC stream carries length-delimited Protobuf control messages (default max 64 KiB); unreliable QUIC DATAGRAMs (RFC 9221) carry media with an application-level header providing at least `stream_instance_id`, `config_generation`, ≥32-bit sequence, and a 64-bit capture-clock sample-position timestamp. The exact header layout, port, error codes, and limits are normative-specification decisions (issue 03), not library defaults.
3. **Security posture**: the Server holds a persistent identity key; Android Clients pin its SPKI/public key through pairing. Client authorization rides the encrypted control stream as a separate check — encrypted transport alone never authorizes. 0-RTT is disabled for this major; no pairing, authorization, or control operation is accepted as early data.
4. **Migration posture**: QUIC connection migration is attempted (quinn `Endpoint::rebind`, socket bound to the selected Android `Network`); ordinary teardown + bounded-backoff authenticated reconnect is the guaranteed fallback and the only production promise. A network switch that kills the connection behaves like any other loss: reconnect, never silent degradation.
5. **Android integration**: a native Rust `cdylib` with a low-frequency JNI surface (session lifecycle, state, statistics) and exactly one direct `ByteBuffer` ring for media — no per-datagram JNI callbacks, no UniFFI/JNA on the hot path. Baseline stays Android 8 / API 26; no dependency may raise it.
6. **Resource posture**: non-waiting datagram sends into application-level bounded queues; a congested or stalled Client never blocks capture or another Client; drop accounting lives at the application layer by construction.

## Considered Options

- **TLS 1.3 TCP control + DTLS/UDP media**: rejected — two handshakes, two connection lifecycles, and two TLS stacks (rustls plus a C DTLS library), defeating the goal of shared protocol logic between the Rust Server and the Android Client's native crate.
- **Netty QUIC (self-built Android artifact)**: rejected — no official Android validation, and it introduces a Java-side QUIC stack beside the Rust protocol logic instead of sharing it.
- **WebRTC data channels**: rejected — its media engine, Opus/NetEq, ICE/STUN/TURN machinery serve product scopes (browser clients, NAT traversal) outside this project's accepted scope; reconsider only if that scope changes.
- **Cronet**: rejected — not a candidate for arbitrary control streams and application media datagrams.
- **Evolve v1's TCP/UDP scheme in place**: rejected — the defects v2 exists to fix (no version negotiation, unauthenticated UDP rebinding, implicit connection state, undiagnosable disconnects) are architectural, and ADR-0001 freezes v1 for the released APK. v1 remains a frozen compatibility adapter.

## Consequences

- The normative specification (issue 03) must decide, explicitly and with recorded values — prototype observations are evidence, not decisions:
  - **MTUD/DPLPMTUD policy**: quinn enables DPLPMTUD by default; the prototype observed it racing cross-end datagram-size assertions and pinned a stable value with it disabled.
  - **Idle timeout and keep-alive**: quinn's default 30 s idle timeout silently kills connected-but-idle sessions (observed on both desktop and device); the foreground-Service idle design depends on explicit values.
  - **Default v2 port** (prototype used 65531, never v1's 65530), error-code registry numbering, resource-limit defaults, buffer tuning.
- quinn's `send_datagram` exposes no full-buffer state (its internal queue evicts oldest silently); application-level bounded queues are therefore mandatory for honest drop accounting.
- JNI functions must receive `JNIEnv` **by value** (jni 0.21 contract); the reference form failed on device and had zero diagnostics until errors were logged — the normative implementation carries that lesson.
- The migration scaffold must reconcile `Network`-local socket binding with VPN-overlay routes (device finding: pinning the socket to wlan0 broke routing to a tailscale destination; a discovered address is not a route).
- The deterministic-vector strategy is viable across architectures: the waveform formula proved byte-exact between x86_64 and aarch64.
- Size cost accepted: +2.20 MiB stripped arm64 `.so` (44% of the provisional ~5 MiB gate), measured on NDK 27.2 with the API-26 clang wrapper.
- v2 starts experimental but enabled by default alongside v1; stable status requires the separate release gate from the design baseline. No removal date is assigned to v1.
