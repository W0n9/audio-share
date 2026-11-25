# 02: Transport ADR — quinn + TLS 1.3 for Wire Protocol v2

**What to build:** `docs/adr/0004-wire-protocol-v2-transport.md` — record the accepted transport decision (quinn 0.11.x + rustls/TLS 1.3 + one reliable control stream + unreliable QUIC DATAGRAM media), the rejected alternatives with rationale, and the consequences the normative specification must honor.

**Blocked by:** 01

**Status:** resolved

**Type:** task

## Requirements

- Format follows `docs/adr/0003` (`Status:` line, Context / Decisions / Considered Options / Consequences); vocabulary follows `GLOSSARY.md` (Server, Android Client, Server Core, Transfer Format; avoid "client", "backend", "CLI").
- Evidence: `.scratch/wire-protocol-v2/prototype-report.md` (rev 3, GO) and branch `prototype/wire-protocol-v2-transport` @ `21e1d71` on the fork.
- Must record, with prototype evidence cited:
  - ALPN `audio-share/v2` carries the Wire Protocol major; incompatible major fails before application data; 0-RTT permanently disabled in this major.
  - Security posture: SPKI-pinned persistent Server identity + Android Client authorization over the encrypted control stream as separate checks; pairing itself is future ticket work (04/05).
  - Migration posture: rebind attempted (quinn `Endpoint::rebind`), ordinary authenticated reconnect is the guaranteed fallback; Wi-Fi↔cellular not exercised (maintainer waiver — no such scenario); ordinary reconnect proven on device.
  - Android integration shape: native cdylib, low-frequency JNI, one direct `ByteBuffer` ring, API 26 minSdk, +2.20 MiB stripped arm64 `.so` (≤ ~5 MiB gate).
  - Resource posture: non-waiting sends + app-level bounded queues; drop accounting lives application-side (quinn's internal datagram queue is invisible).
- Must enumerate the knobs the normative spec (03) must decide explicitly, with the prototype observation for each: MTUD/DPLPMTUD policy (quinn default on; cross-end size-assertion races observed), idle timeout + keep-alive (quinn default 30 s kills idle sessions), default v2 port (prototype used 65531, never v1's 65530), error-code registry, resource-limit defaults.
- Must NOT revise ADR-0001 or the frozen v1 contract (`docs/protocol.md`); v2 is separately scoped on its own port and connection lifecycle.

## Definition of done

Draft committed on a reviewable branch (not `main`); `Status: proposed` until the maintainer accepts it. This ticket resolves when the maintainer accepts the ADR — 03 and 04 stay blocked until then.

## Comments

### 2026-11-12 — ADR-0004 drafted, awaiting maintainer acceptance

Draft at `docs/adr/0004-wire-protocol-v2-transport.md` on branch `phase-2/transport-adr` @ `923ca49` (off main `10e5a28`, worktree removed; `git checkout phase-2/transport-adr` to review). `Status: proposed (2026-11-11)`.

Covers all ticket requirements: ALPN major + 0-RTT-off posture, separate identity/authorization checks, rebind-attempted migration with ordinary reconnect as the only production promise (waiver recorded), Android integration shape (+2.20 MiB, JNI-by-value lesson, VPN-overlay bind finding), app-level bounded queues, and the explicit knob list handed to issue 03 (MTUD policy, idle timeout/keep-alive, default port, error registry, resource limits). Does not revise ADR-0001.

Accepting this ADR (edit Status to `accepted`) resolves this ticket and unblocks 03 + 04.

### 2026-11-12 — maintainer accepted; resolved

ADR-0004 `Status: accepted (2026-11-12)` committed on `phase-2/wire-protocol-v2` @ `0dda7aa` (branch renamed from `phase-2/transport-adr` — it now carries the whole phase-2 line). Issues 03 and 04 are unblocked.
