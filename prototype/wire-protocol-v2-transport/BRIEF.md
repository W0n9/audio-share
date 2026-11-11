Frozen snapshot of the prototype contract; the scratch original stays untracked by convention.

# Prototype brief — wire-protocol-v2 transport validation (ticket 01)

Shared contract for all prototype implementer subagents. Read this file FIRST.

## Pointers (absolute paths, main checkout)

- Spec: `/home/w0n9/workspaces/audio-share/.scratch/wire-protocol-v2/spec.md`
- Ticket: `/home/w0n9/workspaces/audio-share/.scratch/wire-protocol-v2/issues/01-validate-quinn-android-transport.md`
- Frozen v1 protocol (do not modify): `/home/w0n9/workspaces/audio-share/docs/protocol.md`
- ADR-0001: `/home/w0n9/workspaces/audio-share/docs/adr/0001-server-rewrite-baseline.md`

## Scope guards

- All code lives under `prototype/wire-protocol-v2-transport/` on the integration branch. Never touch `crates/`, `android-app/`, `gui/`, `protos/`, `fixtures/`, `docs/protocol.md`, release workflows.
- Prototype only answers the transport question. It does NOT freeze the v2 wire layout, does NOT implement production pairing, does NOT write a transport ADR.
- Do NOT commit anything under `.scratch/` (the maintainer keeps it untracked).
- Commit messages: English conventional commits (`feat: ...`, `test: ...`, `docs: ...`). **No Co-Authored-By or other attribution trailers.**

## Branches and layout

- Integration branch: `prototype/wire-protocol-v2-transport` (created off `main`).
- Repo layout on the branch:

```
prototype/wire-protocol-v2-transport/
  README.md          # how to run everything, recorded versions
  server/            # cargo workspace: proto-server bin, proto-client bin, integration tests
  android-client/    # standalone gradle project (app/ + native/ Rust cdylib crate)
  scripts/           # run-e2e.sh, impairment helper scripts
  evidence/          # committed run logs and captured outputs
```

## Throwaway protocol shape (fixed for the prototype; NOT the v2 normative layout)

Control plane: one reliable bidirectional QUIC stream, length-delimited frames:

```
u32 BE frame_len | u8 msg_type | payload     # frame_len = 1 + payload.len(); cap 64 KiB
```

| type | name | payload |
|---|---|---|
| 0x01 | AUTH | utf-8 test credential token |
| 0x02 | AUTH_OK | u32 BE session_id |
| 0x03 | AUTH_ERR | u8 code: 1=BAD_CREDENTIAL 2=NOT_AUTHENTICATED 3=MALFORMED 4=RATE_LIMITED |
| 0x04 | SUBSCRIBE | empty |
| 0x05 | MEDIA_START | u32 stream_instance_id, u32 config_generation, u32 sample_rate, u8 channels, u8 sample_format (1=f32le) — all fields BE |
| 0x06 | UNSUBSCRIBE | empty |
| 0x07 | CLOSE | u8 reason: 0=NORMAL 1=SERVER_STOP 2=PROTOCOL_ERROR 3=AUTH_FAILED 4=IDLE_TIMEOUT 5=INTERNAL_ERROR |

Media plane: QUIC DATAGRAMs, fixed 20-byte header, all BE:

```
u32 stream_instance_id | u32 config_generation | u32 sequence | u64 timestamp
```

- `timestamp` = sample-frame position of the first PCM frame in the payload, on the capture clock (not Unix time).
- Payload: raw f32le interleaved PCM, frame-aligned (payload len % (4*channels) == 0).
- Deterministic content: sample value at absolute position `p` = `sin(2π * 440 * p / 48000) as f32` (bit-exact both ends; client verifies continuity).
- Default media config: 48000 Hz, 2 channels, f32le, `config_generation = 1`.
- A capture restart bumps `stream_instance_id` (monotonic from 1).

Transport rules:

- ALPN exactly `audio-share/v2`; incompatible ALPN major must fail before any app data.
- TLS 1.3 only (rustls). 0-RTT disabled on both sides; server never accepts early data; no control frame is processed before AUTH succeeds.
- Server identity: self-signed cert generated at startup (rcgen, ECDSA P-256); server prints/writes SPKI sha256 at startup; clients PIN that SPKI hash (custom rustls verifier) and must fail against any other cert.
- Client credential: single shared test token passed by CLI/env (`test-credential-do-not-use` default). Encrypted transport alone is NOT authorization.
- Datagram sizing: always `min(max_datagram_size() - 20, chosen_chunk)`; re-query `max_datagram_size()`; never assume 1464 or any fixed MTU.
- Sends: non-waiting `send_datagram()` into a bounded queue (cap 256 datagrams); on overflow drop newest + log; a slow client must never block the sender or another client.
- Pacing: send synthetic media at real-time rate (~5 ms of audio per datagram batch) driven by a ticker.
- Server state machine (prototype-simple): CONNECTING→AUTHENTICATING→READY→PLAYING→CLOSING/CLOSED; AUTH required before anything else; SUBSCRIBE only legal in READY; violations → CLOSE(PROTOCOL_ERROR) not silent ignore.

## Environment facts (this build machine)

- Linux x86_64, kernel 6.8, Rust 1.98.1, Java 17, docker, passwordless sudo (tc/netem usable).
- Android SDK at `/home/w0n9/Android/Sdk` (platforms android-35/36/37, build-tools 34/35/36, NDK 27.2.12479018 and 29.0.14033849, cmake). `ANDROID_HOME` env is NOT set — pass it explicitly.
- No Android device attached (`adb devices` empty) → all real-device gates will end NOT RUN; still build and record everything buildable.
- Network access available for crates.io / gradle dependencies.
- Reference for existing app's gradle setup: `/home/w0n9/workspaces/audio-share/android-app/` (read-only reference; do not modify).

## Evidence conventions

- Every scripted run appends to `prototype/wire-protocol-v2-transport/evidence/<name>.log` with a header line recording date, exact command, and versions.
- Record exact versions: Rust toolchain, quinn/rustls/ring/rcgen crates (from Cargo.lock), NDK rev, AGP/Kotlin versions, target SDK, kernel.
- Library defaults observed are evidence, never silently adopted protocol decisions — call them out explicitly.
