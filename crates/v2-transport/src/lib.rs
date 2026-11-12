//! Wire Protocol v2 transport for the Audio Share Server.
//!
//! This crate is the server-side half of the v2 control channel's transport
//! layer (ticket 06): the QUIC endpoint (quinn 0.11 + rustls TLS 1.3, ALPN
//! `audio-share/v2`, 0-RTT disabled), the persistent Server identity key
//! (pairing.md PR-1), control framing with the 64 KiB pre-allocation cap,
//! resource limits (threat-model TM-1–TM-8, spec §10), and the golden-vector
//! replay harness against `fixtures/wire-protocol-v2/`.
//!
//! Scope boundary: authentication and the full §5 state machine are ticket 07.
//! This crate delivers "accept compliant connections, refuse non-compliant
//! ones" — ALPN/version failure, capacity rejection, connection lifecycle,
//! and close codes from the §6 error registry.
//!
//! It lives in its own crate (not a module of `audio-share-core`) so the v1
//! crate's dependency tree stays frozen-v1-shaped: `quinn`/`rustls`/`ring`/
//! `p256`/`rcgen` are ~5 MiB of added binary and nothing in the v1 crates,
//! the Server GUI, or `protocol-harness` needs them until it opts into v2.

#![warn(missing_docs)]

/// Authentication primitives: auth_context TLV, ECDSA-P256-SHA256 proof
/// verification over the raw bytes (PR-9), client_id, capability-list rules.
pub mod auth;
/// The server-side authorization store: `pairing/clients.json` (§8.1/§10).
pub mod auth_store;
/// Normative v2 constants: ports, ALPN, frame caps, §6 error registry.
pub mod codes;
/// The per-connection driver: QUIC connection + control framing + §5.2 state
/// machine on one live session (ticket 07).
pub mod driver;
/// Control-stream framing: `u32 BE length | Frame` with the §3 validation.
pub mod framing;
/// Server identity key lifecycle: Ed25519, PKCS#8 PEM, 0600/0700, fail-closed.
pub mod identity;
/// Resource limits (spec §10 / threat-model §6): guards and rate windows.
pub mod limits;
/// Hand-written prost transcription of `protos/wire-protocol-v2/control.proto`.
pub mod schema;
/// The §5.2 Server session state machine (ticket 07) — pure domain logic.
pub mod state_machine;
/// Connection lifecycle state exposed through the domain API (spec §5.1).
pub mod session;
/// The quinn endpoint: TLS 1.3, ALPN, 0-RTT off, accept loop, close codes.
pub mod transport;

/// Golden-vector replay harness for `fixtures/wire-protocol-v2/` (spec §11.1).
///
/// Test-only: the harness is compiled behind the `test-support` feature so
/// production binaries carry no path to the fixture-driven bypass material
/// (pairing.md §11.3).
#[cfg(feature = "test-support")]
pub mod vectors;

/// QUIC transport parameters chosen for v2 (spec §3 table, D3).
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Keep-alive interval, both sides (D3).
pub const KEEP_ALIVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
