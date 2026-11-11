# wire-protocol-v2 transport prototype (ticket 01 — server side)

Throwaway code validating the candidate transport — **quinn + rustls/TLS 1.3 +
one reliable bidirectional control stream + unreliable QUIC DATAGRAM media** —
against the shape fixed in `.scratch/wire-protocol-v2/prototype-brief.md`.

This directory answers a transport question only. It does NOT freeze the v2
wire layout, does NOT implement production pairing, and is NOT production
code. The desktop `proto-client` here is a test tool; the real Android client
is built separately under `android-client/` on the same integration branch.

## Layout

```
server/
  Cargo.toml          cargo workspace
  proto-core/         shared: frame codec, synthetic PCM, SPKI helpers
  proto-server/       server core library + `proto-server` bin
  proto-client/       client core library + `proto-client` bin
  e2e/                integration tests (in-process, ephemeral ports)
scripts/
  run-e2e.sh          binary-level smoke run; appends to evidence/
  impairment.sh       idempotent tc netem / veth helper (needs sudo)
evidence/
  desktop-e2e.log     captured runs (header records date, command, versions)
```

## Recorded versions

| component | version | source |
|---|---|---|
| Linux kernel | 6.8.0-139-generic | `uname -r` (build machine) |
| rustc / cargo | 1.98.1 (48a229cea 2026-09-01) / 1.98.1 | toolchain on PATH |
| quinn | 0.11.12 | `server/Cargo.lock` |
| quinn-proto | 0.11.19 | `server/Cargo.lock` |
| quinn-udp | 0.5.16 | `server/Cargo.lock` |
| rustls | 0.23.46 | `server/Cargo.lock` |
| ring | 0.17.14 | `server/Cargo.lock` |
| rcgen | 0.13.2 | `server/Cargo.lock` |
| tokio | 1.54.1 | `server/Cargo.lock` |

## Build and run

```bash
cd server
cargo build --release          # bins in target/release/
cargo test --workspace         # 46 tests (units + 21 e2e cases)
```

Start a server (defaults: bind 127.0.0.1:**65531** — never the v1 port 65530 —
credential `test-credential-do-not-use`, 60 s idle timeout):

```bash
./target/release/proto-server --bind 127.0.0.1:65531 --spki-out /tmp/spki.hex
# prints: PROTO_SERVER_READY addr=127.0.0.1:65531 spki=<64 hex chars>
```

Flags: `--credential TOKEN` (or `$PROTO_TEST_CREDENTIAL`), `--idle-timeout-secs N`
(0 disables), `--chunk-ms` (media batch size in ms of audio, default 5),
`--queue-capacity` (default 256), `--mtud` (enable DPLPMTUD, default off —
see below), `-v/-vv`. SIGINT stops the server with `CLOSE(SERVER_STOP)` to all
sessions and prints per-session stat lines. The server log never contains the
credential or private key material.

Run a client (happy path, ~1 s of media):

```bash
./target/release/proto-client --addr 127.0.0.1:65531 --pin "$(cat /tmp/spki.hex)"
```

Client flags/modes: `--pin-file F`, `--credential TOKEN`, `--alpn S`
(default `audio-share/v2`), `--wrong-alpn`, `--wrong-pin`, `--wrong-credential`
(negative tests), `--duration-ms`, `--min-datagrams`, `--stop-reading-ms`
(slow-reader demonstration), `--mtud`, `-v/-vv`. Modes: `happy` (default),
`hold` (subscribe, then wait for a server-initiated close — used for the
server-stop case), `no-auth-subscribe`, `bad-msg-type`. Every run prints one
`RESULT ...` line with counters and exits 0 when the intended flow completed
(intended negative outcomes included).

Scripted smoke run (all 8 cases, PASS/FAIL each, appends to
`evidence/desktop-e2e.log`):

```bash
scripts/run-e2e.sh
```

## Protocol shape (throwaway; NOT the v2 normative layout)

- Control plane: one bidirectional QUIC stream (opened by the client),
  `u32 BE frame_len | u8 msg_type | payload`, `frame_len = 1 + payload.len()`,
  cap 64 KiB. Frame types 0x01–0x07 per the brief's table.
- State machine: `CONNECTING → AUTHENTICATING → READY → PLAYING → CLOSING/CLOSED`.
  AUTH required first; SUBSCRIBE only in READY; UNSUBSCRIBE only in PLAYING;
  violations → explicit `CLOSE(PROTOCOL_ERROR)` (with an
  `AUTH_ERR(NOT_AUTHENTICATED)` prelude pre-auth), never a silent ignore.
- Media: QUIC DATAGRAMs, 20-byte BE header
  (`stream_instance_id | config_generation | sequence | timestamp`), payload
  raw f32le interleaved PCM; sample at absolute position `p` is
  `sin(2π·440·p/48000) as f32` — one shared function, bit-exact both ends.
  New SUBSCRIBE (after UNSUBSCRIBE) bumps `stream_instance_id` from 1.
- Close reasons ride both the CLOSE frame and the QUIC application error code
  (`Normal=0 ServerStop=1 ProtocolError=2 AuthFailed=3 IdleTimeout=4
  InternalError=5`) plus an ASCII reason string on the transport close.
- A client `CLOSE(NORMAL)` is acknowledged with the server's own
  `CLOSE(NORMAL)` before the transport closes (prototype-simple goodbye).

## Security and version assertions

| assertion | where | result |
|---|---|---|
| correct pin handshakes, rogue SPKI fails | `e2e/tests/security.rs`, client pin verifier | PASS |
| `audio-share/v2` succeeds, incompatible ALPN fails before app data | `e2e/tests/security.rs` (server accepts 0 sessions) | PASS |
| valid credential succeeds; wrong/empty credential → `AUTH_ERR(BAD_CREDENTIAL)` + `CLOSE(AUTH_FAILED)` | `e2e/tests/security.rs` | PASS |
| SUBSCRIBE without AUTH → `AUTH_ERR(NOT_AUTHENTICATED)` + `CLOSE(PROTOCOL_ERROR)` | `e2e/tests/protocol.rs` | PASS |
| unknown msg_type / oversized length / bad payload length / unknown close reason → `CLOSE(PROTOCOL_ERROR)` | `e2e/tests/protocol.rs` | PASS |
| non-UTF-8 AUTH → `AUTH_ERR(MALFORMED)` + `CLOSE(AUTH_FAILED)` | `e2e/tests/protocol.rs` | PASS |
| no secrets in logs/errors | logging audit: credential and keys never traced | by construction |

### 0-RTT evidence path (honest limits)

quinn 0.11 offers **no runtime API to observe the negotiated 0-RTT state**, so
the strongest available evidence is configuration-level:

1. rustls only accepts early data when `ServerConfig::max_early_data_size > 0`;
   this server builds its config with `max_early_data_size = 0` explicitly, and
   `proto-server/src/lib.rs::tests::server_tls_config_disables_0rtt_and_sets_alpn`
   asserts it at runtime against the exact config object the server runs.
2. quinn additionally requires opt-in at two more places —
   `ServerConfig::enable_0rtt()` (transport setup) and
   `Incoming::accept_0rtt()` (per connection). Neither appears anywhere under
   `server/` (grep-verifiable: `rg 'accept_0rtt|enable_0rtt' server/`).
   The application also only reads the control stream after `connect()`
   completes, so no frame can be processed before the handshake.
3. Client side: quinn 0.11 has no client-side 0-RTT API at all (early data can
   only be produced by a rustls client session explicitly configured for
   resumption with early data, which this client never builds), and the
   client's control stream is opened after the handshake returns.

Consequence: 0-RTT is disabled by construction and audit, not by a runtime
probe. This is recorded as evidence-grade configuration, not a transport-level
runtime assertion.

## Datagram sizing (dynamic; no fixed 1464)

The server re-queries `connection.max_datagram_size()` every 5 ms tick and
sizes datagrams as `min(max_datagram_size() - 20, chosen_chunk)` floored to
whole sample frames, per the brief. With DPLPMTUD off, quinn reports **1162**
here (1200-byte initial MTU minus QUIC/AEAD overhead), yielding 1136-byte
payloads (142 frames ≈ 2.96 ms) — two datagrams per 5 ms batch. Test (h)
(`e2e/tests/flow_control.rs::every_datagram_respects_queried_max_datagram_size`)
fails for any server that assumed the v1 fixed 1464-byte path. The later
reduced-MTU runs should start both bins with `--mtud` so sizing reacts to
path-limit changes.

## Bounded sends and slow clients

Per session: a producer task paces 5 ms media batches into a **bounded queue**
(capacity 256) with non-waiting `try_send`; on overflow the **newest** datagram
is dropped and counted (`media_queue_dropped`). A forwarder task drains the
queue into non-waiting `send_datagram()`. Tests cover: a client that stops
reading never blocks the producer or the control stream; a deliberately slowed
outbound path (test-only `forwarder_delay` hook) drives counted drop-newest
while the session stays responsive; one slow client does not degrade another;
UNSUBSCRIBE detaches the pump without stalling the control loop.

## Library defaults observed (evidence, not adopted decisions)

All read from `quinn-proto 0.11.19` `TransportConfig::new()` / source unless
noted; each row states what the prototype did about it.

- **DPLPMTUD is ON by default** (`mtu_discovery_config: Some(default)`) and
  grows `max_datagram_size()` past 1200 on unimpaired paths, which makes
  cross-endpoint size assertions racy (the server's limit grows faster than
  the client's). The prototype disables it on both sides (`--mtud` re-enables).
- `Connection::max_datagram_size()` = path MTU minus QUIC/AEAD overhead →
  **1162** bytes at the 1200-byte initial MTU on this machine.
- `Connection::send_datagram()` **never reports a full send buffer** in
  0.11.12 (proto-level `Blocked` is `unreachable!()`): quinn's internal 1 MiB
  datagram send queue silently evicts oldest-first. Counted drops therefore
  live in the application's own bounded queue (ours), not in quinn.
- Default datagram **receive** buffer = `STREAM_RWND` = 1.25 MiB
  (12.5 MB/s × 100 ms). Consequence observed in CASE 7 of the e2e log: a
  500 ms stop-reading pause at ~460 KB/s was absorbed entirely (`gaps=0`);
  client-endpoint drops need longer pauses, higher rates, or a smaller
  `datagram_receive_buffer_size` — relevant for later impairment runs.
- `max_idle_timeout` advertised by default: 30 s; `keep_alive_interval`: none.
  The prototype adds its own application-level idle timeout (60 s default,
  exempt while media flows).
- Congestion controller default: **Cubic**.
- `stream_receive_window` 1.25 MB, `send_window` 10 MB, bidi/uni stream
  limits 100/100, `crypto_buffer_size` 16 KiB, `allow_spin` true.
- tokio 1.54 bounded mpsc empirically holds **capacity + 1** messages
  (257 in flight with a 256-slot channel — observed in the bounded-queue
  accounting test).
- rustls 0.23 with the **ring** provider; the prototype pins TLS 1.3 only via
  `with_protocol_versions(&[&rustls::version::TLS13])`.
- Client-side SPKI pinning ignores the certificate name/chain by design (the
  pin is the identity, mirroring post-pairing Android behavior); handshake
  signature verification still uses rustls' standard algorithms.

## Prototype decisions worth recording

- AUTH uses a plain comparison of a throwaway LAN token — not a hardened
  constant-time primitive; production needs a real credential model.
- `AUTH_ERR(MALFORMED)` is used for non-UTF-8 AUTH payloads; `RATE_LIMITED`
  exists in the code registry but is not triggered (no rate limiting yet).
- Idle timeout applies in CONNECTING/READY only; PLAYING counts as alive.
- Media `timestamp` restarts at 0 per `stream_instance_id`; `sequence` likewise.
- `MEDIA_START` always carries the default transfer format
  (48000 Hz, 2 channels, f32le, `config_generation = 1`); no negotiation.

## What this directory does NOT cover

NOT RUN here, handled by other agents/gates on the same integration branch:
the Android client and all real-device gates, IPv4/IPv6 path matrix, actual
impairment runs (the commands in `scripts/impairment.sh` were verified against
a live veth pair on this host, but no impairment measurement was executed),
reduced-MTU sizing measurements, migration/rebinding, release-size budget.
