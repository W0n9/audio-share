# Wire Protocol v2 — Design Baseline

Status: design-baseline; transport validation pending
Feature: wire-protocol-v2
Date: 2026-11-11

## Decision Status

This file records the product and architecture decisions accepted in the design interview. It is **not yet a normative Wire Protocol specification**: no v2 field layout, schema, port number, cryptographic suite, or library version is frozen by this document.

Issue `issues/01-validate-quinn-android-transport.md` must answer whether the proposed transport works on a real Android Client. A GO result allows the transport ADR and normative protocol specification to proceed. A NO-GO result reopens the transport and Android-integration decisions; it does not silently activate a fallback.

This is a new, separately scoped protocol-evolution project. It reopens the future UDP-header/reorder work explicitly deferred by ADR-0001, but **does not revise ADR-0001 or the frozen v0.4.0 compatibility contract**.

## Current Baseline

The released compatibility baseline is v0.3.4, not an online-negotiated protocol version:

- TCP control and UDP media use the same host/port, default `65530`; control integers and PCM are little-endian. See `docs/protocol.md:36-69`.
- TCP commands are `GET_FORMAT=1`, `START_PLAY=2`, and `HEARTBEAT=3`; `AudioFormat` is Protobuf. UDP bind is a signed 32-bit id and audio datagrams contain only raw PCM. See `docs/protocol.md:71-128` and `docs/protocol.md:163-185`.
- There is no online Wire Protocol version, authentication, encryption, media sequence number, media timestamp, structured close reason, or safe UDP session binding.
- The v0.3.4 Android APK is a frozen acceptance client for v1. It must continue to work without an APK update. See `.scratch/server-rewrite/spec.md:18` and `docs/adr/0001-server-rewrite-baseline.md`.
- Existing protocol fixtures and harnesses are compatibility assets. v2 work must extend the test strategy without weakening v1 replay or real-APK acceptance.

The main defects to address are therefore architectural, not a missing fashionable transport: absent version negotiation, implicit connection state, unauthenticated UDP rebinding, undiagnosable disconnects, no loss/reorder metadata, fixed IPv4 MTU arithmetic, and no path from capture failure to Client session state.

## Goals and Priorities

1. Preserve real-time playback and the existing v1 compatibility contract.
2. Make v2 explicitly versioned, evolvable, cross-language, secure by default, and diagnosable.
3. Keep implementation and bandwidth costs proportionate; do not replace working technology merely to adopt a newer name.

The security requirements below are mandatory for v2 even though realtime behavior has first priority. A slow or failed connection may degrade playback; it may not silently disable authentication or encryption.

### Measurable Initial Targets

These are initial engineering budgets to calibrate against real devices, not claims about the current implementation:

- Default Client jitter-buffer target: **60 ms**.
- Media-path added latency, measured from captured audio entering the protocol send path to audio entering the Client playback buffer: **p95 <= 80 ms, p99 <= 120 ms** under the agreed normal-LAN test scenario.
- A 60-minute playback run must not develop deadlock, a falsely live session, unrecoverable reordering, or continually increasing latency.
- Loss, reorder, late-drop, concealment, buffer underrun, and end-to-end audible latency are reported separately. A network arrival metric is not an end-to-end audible-latency metric.
- If measurements invalidate a number, change the budget explicitly with new evidence; do not relax a test while retaining the old claim.

## Accepted Product and Deployment Decisions

### Dual-Protocol Migration

- Keep v1 frozen on its current port and behavior for the released v0.3.4 APK.
- Add v2 on a separate UDP port; do not require same-port protocol detection for correctness.
- Initially enable v1 and v2 by default on Server versions that support both. v1 is labeled **trusted networks only; no encryption or authentication**, and can be disabled independently.
- v2 starts as experimental but is runnable by default. Stable status requires a separate release gate.
- New Android Clients prefer v2 and never automatically fall back to v1 after a v2 failure.
- Do not assign a removal date to v1. Revisit its default only after a real v2 Android release, migration guide, and rollback path exist.
- Product version, Android app version, and Wire Protocol major version are independent values.

### Network Scope

- The first v2 release requires a directly reachable Server address: LAN, VPN, or user-configured port forwarding.
- STUN/TURN, NAT hole punching, public rendezvous, relays, and an account system are outside the initial scope.
- Saved/manual Server addresses are the reliable baseline. mDNS may be added as optional discovery, but a discovered name is not a trusted identity.
- Wi-Fi/cellular connection migration is a prototype capability gate, not an initial promise of seamless recovery under every Android/network condition. Ordinary reconnect remains required.

### Client and Role Scope

- First implementations are Rust Server and a new Android Client. Schemas, test vectors, and the normative specification remain language-neutral.
- Browser, iOS, and additional clients are deferred.
- The only initial remote role is **player**: connect, subscribe to audio, receive required state, report playback diagnostics, and disconnect.
- Endpoint selection, capture configuration, pairing administration, and Server administration remain local GUI/`as-cmd` operations.
- One shared capture feeds independently authenticated Client sessions. Sample-accurate playback synchronization across phones is not promised.

## Candidate Transport and Android Integration

### Primary Candidate

The candidate is **quinn + rustls/TLS 1.3 + one reliable QUIC bidirectional control stream + unreliable QUIC DATAGRAM media**, with a narrow Android JNI boundary.

Reasons to test this candidate first:

- QUIC DATAGRAM matches live media that may be dropped or reordered and must not be retransmitted.
- One QUIC connection avoids the split TCP-control/UDP-media session lifecycle.
- quinn exposes application datagrams, dynamic `max_datagram_size()`, non-waiting `send_datagram()`, and endpoint rebinding.
- Rust Server and Android native code can share protocol logic without wrapping a second C QUIC stack.

This is not yet a validated Android product choice. quinn upstream Android CI does not replace arm64 real-device, API, ABI-size, certificate, foreground-Service, or network-switch acceptance.

### Connection Rules

- ALPN carries the Wire Protocol major, using the form `audio-share/v2`.
- A major-version mismatch fails explicitly. Capabilities negotiate optional features within a compatible major.
- Mandatory security capabilities cannot be negotiated off.
- Disable 0-RTT completely in the initial version. Pairing, authorization, and control operations are never accepted as early data.
- Server accepts QUIC connection migration. Android tests `ConnectivityManager.NetworkCallback`, network-bound sockets, and `Endpoint::rebind()`; lack of migration still falls back to a new authenticated session.
- Use non-waiting datagram sends and bounded queues. A congested Client must not block capture or other Clients.

### Android Shape

- New Android Client baseline is **Android 8/API 26**. Any dependency requiring a higher minimum reopens that decision.
- Keep the existing v0.3.4 APK and its support range unchanged.
- Kotlin owns a native session handle; QUIC/Tokio execution stays on the native side.
- Control operations and state events use low-frequency JNI or UniFFI. Media does not cross JNI as one callback or JNA/UniFFI call per packet.
- Media uses a direct `ByteBuffer` or native ring buffer consumed by the playback side.
- Playback uses a foreground media-playback Service. Process death, force stop, and revoked permissions are not represented as continued playback.
- Reconnection uses bounded exponential backoff with jitter. Persistent wake locks are not a substitute for correct Android lifecycle behavior.

### Fallback Order

If the primary candidate fails and cannot be repaired within its prototype scope:

1. Evaluate TLS 1.3 TCP control + DTLS/UDP media, accounting for two handshakes, two lifecycles, and two TLS stacks.
2. Evaluate a self-built Netty QUIC Android artifact and its missing official Android validation.
3. Reconsider WebRTC only if the product scope changes to need its media engine, Opus/PLC/NetEq, or NAT traversal.

Cronet is not a candidate for arbitrary control streams and media datagrams. A fallback requires an explicit decision update; it is not an automatic runtime or implementation fallback.

## Security and Pairing Model

### Identity and Authorization

- Server has a persistent identity key/certificate. Android pins the Server SPKI or public key obtained through pairing; an anonymous self-signed certificate is insufficient.
- Client authorization is carried over the encrypted control channel using a pairing credential or signed challenge. The first version does not require Android Keystore keys to act as rustls TLS client-certificate keys.
- Encryption and peer authorization are separate checks. Tests must reject both an unpinned Server and an unauthorized Client.
- A protocol or authentication failure never downgrades to v1, plaintext, anonymous encryption, or a weaker security mode.

### First Pairing

- QR pairing is preferred. Its payload includes a protocol major, Server address hint, full Server identity public key or long fingerprint, and a random **at-least-128-bit**, short-lived, single-use pairing nonce.
- The live handshake proves possession of the corresponding private key and binds identities, nonce, protocol major, and capabilities into the authenticated transcript.
- SAS comparison is the no-camera fallback. Use ephemeral key exchange with hash commitment; an unverified comparison does not claim protection against an active first-connection MITM.
- A six-digit code is never directly stretched into a session key. The first version does not introduce a custom password PAKE.
- After pairing, persist both identities for automatic authentication. Key changes require explicit re-pairing or confirmation, never silent replacement.

### Key Lifecycle and Local Administration

- GUI and `as-cmd` share one pairing Core and the same identity/authorization state.
- Headless pairing provides a local command with QR output where practical and an SAS/fingerprint path in all cases.
- Android private keys use Android Keystore where available. Server key and authorization files have explicit permissions, concurrent-access behavior, corruption handling, and backup guidance.
- Revocation affects new connections immediately and terminates affected active sessions promptly.
- Resetting a Server identity invalidates old pairings. There is no universal recovery backdoor.
- Test-only authentication bypasses stay in test builds or isolated test configuration.

### Resource and Privacy Controls

- Validate message and datagram lengths before allocation; bound queues, incomplete handshakes, pairing attempts, sessions per Client, and total Clients.
- Rate-limit authentication and pairing failures. Small unauthenticated requests must not produce large error responses.
- Duplicate, replayed, malformed, and unknown-generation media consume bounded resources.
- Error payloads omit secrets, internal paths, stack traces, and unnecessary implementation detail.
- Diagnostics stay local and exportable with redaction. Do not add external telemetry, raw PCM recording, or secret logging.

## Control-Plane Semantics

- Use length-delimited Protobuf messages on the reliable ordered control stream. The default maximum control-message size is **64 KiB** unless a specific message justifies a separate reviewed limit.
- Define an explicit state machine: `CONNECTING`, `AUTHENTICATING`, `NEGOTIATING`, `READY`, `PLAYING`, `CLOSING`, and `CLOSED`, including all legal transitions and terminal behavior.
- Messages have explicit types. Requests/responses carry opaque request ids; behavior never depends on Kotlin enum ordinals, Rust layout, or the position of a four-byte command.
- Pairing, authentication, version/capability negotiation, and configuration complete before media subscription.
- Server can publish configuration, stream-instance lifecycle, errors, and close events.
- Invalid state transitions produce a defined protocol error or close action rather than silent ignore.
- Stable error codes distinguish version/capability mismatch, pairing/authentication/authorization failure, busy/rate limit, malformed message/state, unsupported media configuration, capture failure, stream end, Server stop, normal Client stop, and temporary network failure.
- Errors include retryability and whether re-pairing is required. Transport timeout remains the fallback when a close message cannot be delivered.

## Media-Plane Semantics

### Encoding and Packet Header

- PCM is the required first codec. The model includes explicit codec and framing descriptions so Opus can be added later by capability negotiation.
- Use a compact, independently specified v2 binary header, not a claim of RTP wire compatibility.
- Reuse RTP semantics for packet sequence, sampling timestamps, and loss/jitter calculations.
- Carry at least `stream_instance_id`, `config_generation`, sequence, timestamp, and required flags. Datagram boundaries determine payload length.
- Use at least a 32-bit packet sequence and a 64-bit sample-position timestamp; specify initial values and wrap behavior in the normative specification.
- Fixed-header integers use network byte order. v1 little-endian behavior remains unchanged.
- Timestamp identifies the first PCM sample frame in the payload on the capture sampling clock. It is not Unix time, send time, or a count of interleaved channel samples.
- A capture reset, process restart, or discontinuous sampling clock creates a new `stream_instance_id` and resets Client playback/statistics state.
- Every decode-affecting configuration change increments `config_generation`. Media carries its generation because reliable control and DATAGRAM delivery have no cross-channel ordering guarantee.
- Media with unknown configuration is dropped or held in a strictly bounded queue while configuration is requested; it is never decoded with the previous generation.

### Live Playback and Recovery

- The first version has no application-layer retransmission and no PCM FEC.
- Reorder timely packets in a bounded jitter buffer. Drop late packets and count `late_dropped` separately from network loss.
- Fill gaps with defined silence, comfort noise, or PCM concealment and count the intervention.
- Pace sends against the media timeline with a bounded send-ahead window; do not burst an entire capture callback without limit.
- Measure capture-clock versus playback-clock drift. Correct it with small rate/resampling adjustments, or accounted insertion/deletion when smooth correction is insufficient.
- A buffer hard limit re-anchors to the live position rather than accumulating indefinite latency.
- Each Client owns independent session, sequence, buffer, pacing/backpressure, and statistics state. One slow Client cannot block capture or another Client.
- Opus, FEC, DTX, and codec-specific PLC are future capabilities, not hidden first-version requirements.

### IPv4, IPv6, and Datagram Size

- Support IPv4 and IPv6 without assuming a 1492-byte path or fixed IPv4 header size.
- Determine the current transport DATAGRAM limit, subtract the v2 media header, and align PCM payloads to complete sample frames.
- React to path-limit reductions and avoid relying on IP fragmentation.
- Small-MTU and unsupported-datagram cases have explicit tests and errors.

## Playback Diagnostics

Android reports playback-oriented statistics periodically, initially every five seconds, and best-effort on close:

- Extended highest sequence, cumulative/interval loss, duplicates, and reorder statistics.
- RTP/RTCP-style interarrival jitter.
- Late drops, unknown-configuration drops, concealment, and buffer underruns.
- Current/target buffer, active stream instance and configuration generation.
- Media position actually submitted to the playback system.

A QUIC ACK or socket receive is not proof of decode or playback. Statistics failures never block media delivery.

## Implementation Boundaries

- v1 framing, commands, UDP behavior, and session state remain a frozen compatibility adapter.
- v2 handshake, control, media, and sessions live in independent modules.
- Capture format descriptions, device failures, and lifecycle events cross an internal domain API rather than exposing v1 Protobuf as the Server Core API.
- GUI and `as-cmd` observe common v1/v2 lifecycle state without parsing wire bytes.
- Any v1 defect fix requires its own frozen-protocol compatibility justification and regression evidence.

## Verification and Delivery

### Transport Prototype Gate

Issue 01 must provide PASS/FAIL/NOT RUN evidence for every hard gate: arm64 real device, API 26, control stream, DATAGRAM, pinned identity and authorization, ALPN/0-RTT behavior, IPv4/IPv6, dynamic MTU, impairment behavior, foreground Service, migration, native hot path, and release binary size.

A completed prototype may conclude GO, NO-GO, or BLOCKED. **Resolved investigation is not equivalent to a GO decision.** No transport ADR may claim validation while a required gate is NOT RUN.

### Normative Specification Gate

After a GO result, produce and review:

- Transport ADR and rejected-alternative rationale.
- Normative wire specification, state-machine tables, schema, media layout, error registry, and capability rules.
- Threat model and pairing/key-lifecycle specification.
- Cross-language golden vectors for control, media, authentication, and errors.
- Resource limits and privacy/diagnostic behavior.

### Later Implementation Acceptance

- Preserve all v1 fixtures, replay, and real v0.3.4 APK acceptance.
- Test Rust and Android against each other, not only against their own encoders.
- Cover malformed lengths, unknown fields/messages, version/capability conflicts, state violations, and authentication failures.
- Deterministically inject loss, reorder, duplication, jitter, congestion, MTU changes, and clock drift.
- Fuzz control and media parsers.
- Run at least two real Android Clients concurrently for the 60-minute scenario and a longer Server soak for leak/session accumulation.
- Keep v1 buildable and releasable at every implementation stage.

### Planned Tracer-Bullet Sequence

1. `01` — Android transport prototype and GO/NO-GO report.
2. Transport ADR, normative specification, schema, vectors, and threat model.
3. Minimal authenticated v2 control channel between Server and Android.
4. Single-Client PCM media, playback statistics, and explicit close.
5. Jitter buffer, drift correction, impairment behavior, and failure recovery.
6. Multi-Client isolation, headless pairing, GUI/CLI state, and diagnostics.
7. Full acceptance, v1 regression, release, and migration documentation.

Only issue 01 is created at this stage. Later issues must carry their own blocking edges after the prototype and specification decisions are available.

## Engineering Details Left Open

The normative specification and prototype may choose concrete values without changing the accepted semantics above: exact header layout beyond minimum widths, default v2 port, Protobuf field numbers, error-code numbers, reviewed cryptographic suites, QR binary/text encoding, SAS presentation, resource-limit defaults, impairment profiles, and buffer tuning. Record each choice in a reviewable artifact and test vector rather than letting library defaults define the protocol.

## Primary Sources

Repository sources are cited inline above. External sources below were checked on 2026-11-11:

- QUIC transport and TLS: [RFC 9000](https://www.rfc-editor.org/rfc/rfc9000), [RFC 9001](https://www.rfc-editor.org/rfc/rfc9001), [RFC 8446](https://www.rfc-editor.org/rfc/rfc8446).
- QUIC DATAGRAM: [RFC 9221](https://www.rfc-editor.org/rfc/rfc9221).
- RTP/RTCP semantics: [RFC 3550](https://www.rfc-editor.org/rfc/rfc3550), [RFC 3551](https://www.rfc-editor.org/rfc/rfc3551).
- Pairing boundaries: [RFC 6189](https://www.rfc-editor.org/rfc/rfc6189), [RFC 7435](https://www.rfc-editor.org/rfc/rfc7435), [RFC 9383](https://www.rfc-editor.org/rfc/rfc9383).
- quinn API and Android implementation: [quinn repository](https://github.com/quinn-rs/quinn), [Connection datagram API](https://docs.rs/quinn/0.11.12/quinn/struct.Connection.html), [Endpoint rebind](https://docs.rs/quinn/0.11.12/quinn/struct.Endpoint.html#method.rebind).
- Android integration: [JNI performance](https://developer.android.com/training/articles/perf-jni), [NDK networking](https://developer.android.com/ndk/reference/group/networking), [foreground Service types](https://developer.android.com/develop/background-work/services/fg-service-types), [Rust Android targets](https://doc.rust-lang.org/rustc/platform-support/android.html).
- Alternative transport evidence: [Cronet](https://developer.android.com/develop/connectivity/cronet), [Netty QUIC](https://netty.io/4.2/api/io/netty/handler/codec/quic/QuicChannel.html), [quiche](https://github.com/cloudflare/quiche), [MsQuic platforms](https://github.com/microsoft/msquic/blob/main/docs/Release.md), [WebRTC data channels](https://www.rfc-editor.org/rfc/rfc8831).

## Comments

2026-11-11 — The maintainer accepted the design-tree recommendations through Q40 and confirmed the consolidated shared understanding. Issue 01 is the only authorized implementation preparation at this stage; production protocol code and the transport ADR remain pending prototype evidence.
