# Ticket 33 — Server log v2 session events

Status: agent-side implementation PASS; Windows host rerun **NOT RUN** (human gate).

Branch: `ticket/33-server-log-events`
Commits: `233c1a7` — feature; `2695443` — two-axis review fixes
Merge: `de83686` (`--no-ff` into `integration/wire-protocol-v2-beta`); follow-up `362d023` (random-key redaction assertion)
Base: `89f84b0`

## Root cause

`v2-transport` already emitted `tracing` events, but the Server GUI owns its own
`logging::Logger` (a plain file writer) and never installed a `tracing` subscriber.
Every library event was therefore dropped inside the GUI process — and the Windows
acceptance run (ticket 25) used the GUI. `as-cmd` did install a subscriber, which is why
earlier headless evidence showed `v2 session ended: ClientClosed(701)` while the GUI
`server.log` had startup/stop only.

## Changes

- `crates/v2-transport/src/session_log.rs` — one seam for the structured events:
  `pairing.presented`, `pairing.accepted`, `pairing.declined`, `session.accepted`,
  `session.closed` (with `code`), `client.revoked`, `identity.reset`.
  Only public `client_id`, pairing mode, §6 code, and the public SPKI fingerprint are
  accepted.
- `driver.rs` — emits presented at the operator view, accepted at first registry
  registration, closed once in `terminate()` (covers 701/204/303/601/loss).
- `pairing.rs` — accepted/declined after the decision callback is delivered; revoked in
  `revoke`; identity reset in `reset_identity`.
- `gui/src-tauri/src/tracing_bridge.rs` — forwards only `v2_transport::session_log` INFO
  events into the existing `server.log` writer; transport chatter and third-party
  QUIC/TLS events are filtered out. Installed in `main()` after `logging::init`; an
  install failure is written as an error line rather than swallowed.
- `crates/as-cmd/src/main.rs` — offline `clients revoke` / `identity reset` append one
  audit line to `server.log` (never call the truncating `init_logging`). The line repeats
  the `session_log` vocabulary by design: there is no subscriber in these offline
  processes, and installing one would truncate the file.
- `auth::client_id_hex` is the one canonical `${client_id}` formatter; the store, pairing,
  state machine, and session log share it. `v2-runtime`'s existing `v2 session ended` INFO
  line is unchanged (it also covers connections that never establish a session).

## Evidence

Test seam: live QUIC (`SessionServer` + `PairingCore`) with a `tracing` capture sink.

| Step | Command | Result |
|---|---|---|
| RED | `cargo test -p v2-transport --test session_log` (before implementation) | **FAIL** — 0 passed, 4 failed; capture had only transport accept lines |
| GREEN | `cargo test -p v2-transport --test session_log` | **PASS** — 4 passed |
| v2-runtime | `cargo test -p v2-runtime` | **PASS** — 11 passed |
| as-cmd unit | `cargo test -p as-cmd --bin as-cmd` | **PASS** — 9 passed |
| GUI lib | `cargo test -p audio-share-server-gui --lib` | **PASS** — 112 passed, 1 ignored |
| workspace | `cargo test --workspace` | **PASS** — 621 passed, 4 ignored |
| clippy | `cargo clippy -p v2-transport -p v2-runtime -p as-cmd -p audio-share-server-gui --all-targets -- -D warnings` | **PASS** — no issues |
| fmt | `cargo fmt --all -- --check` | **PASS** |

Covered behaviours:

1. Session establishment logs `event=session.accepted client_id=… session_id=…`.
2. Client stop logs `event=session.closed client_id=… code=701`.
3. Revocation logs `event=client.revoked`, then `event=session.closed … code=204`.
4. Pairing logs `event=pairing.presented … mode=qr`, `event=pairing.declined`,
   `event=pairing.accepted`.
5. Identity reset logs `event=identity.reset fingerprint=…`.
6. Redaction: the SAS string, pairing nonce hex, binding bytes hex, client proof hex, the
   Client private-key scalar, and any binary (raw-audio-like) bytes never appear in the
   captured log during a live SAS-capable pairing.

Two-axis review fixes (`2695443`): strengthened the redaction test as above; consolidated
`client_id` hex formatting; narrowed the GUI bridge to `session_log`; logged pairing
decisions after delivery; restored the runtime's INFO termination line; corrected the
module red-line docs.

Known follow-ups not taken: the test helpers duplicate `redaction.rs` / `pairing_live.rs`
(extractable into `tests/common/`), and the GUI logger has no WARN level, so WARN events
map to `[info]`.

## NOT RUN (human gate)

- Windows host: rebuild `AudioShareServer.exe` + `as-cmd.exe`, run a GUI session, stop the
  Client, and confirm `event=session.closed … code=701` appears in `server.log`.
- A real-device pairing/revoke rerun is not required by this ticket but would exercise the
  same lines.
- This ticket therefore ends `ready-for-human`, not `resolved`.
