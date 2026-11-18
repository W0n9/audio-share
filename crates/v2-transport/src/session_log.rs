//! Structured, secret-free v2 session lifecycle events (ticket 33).
//!
//! Support must be able to verify pairing, session establishment,
//! `ClientClosed(701)`, revocation, and identity reset from `server.log`. Every
//! event carries the public `client_id` (first 8 bytes of SHA-256 of the Client
//! public key, lowercase hex, pairing.md §8.1) and the §6 code where one
//! applies.
//!
//! # Red line
//!
//! These helpers accept only public values. They must never be given the SAS,
//! a nonce, the binding context, a client proof, key material, or raw audio;
//! the test suite drives a live SAS pairing and asserts none of it appears in
//! the captured log (`tests/session_log.rs`).

use crate::auth;
use crate::schema::PairingMode;

/// The canonical public `client_id` for a Client public key, lowercase hex.
pub fn client_id_hex(public_key: &[u8]) -> String {
    auth::client_id(public_key)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The local spelling of a pairing mode (`qr` / `sas`); never a wire value.
pub fn pairing_mode(mode: PairingMode) -> &'static str {
    match mode {
        PairingMode::Qr => "qr",
        PairingMode::Sas => "sas",
        PairingMode::Unspecified => "unspecified",
    }
}

/// A verified pairing is being shown to the local operator.
pub fn pairing_presented(client_id: &str, mode: &str) {
    tracing::info!(
        event = %"pairing.presented",
        client_id = %client_id,
        mode = %mode,
        "v2 pairing presented"
    );
}

/// The local operator approved a pairing.
pub fn pairing_accepted(client_id: &str) {
    tracing::info!(
        event = %"pairing.accepted",
        client_id = %client_id,
        "v2 pairing accepted"
    );
}

/// The local operator declined a pairing.
pub fn pairing_declined(client_id: &str) {
    tracing::info!(
        event = %"pairing.declined",
        client_id = %client_id,
        "v2 pairing declined"
    );
}

/// A Client completed authentication and owns a live session.
pub fn session_accepted(client_id: &str, session_id: u64) {
    tracing::info!(
        event = %"session.accepted",
        client_id = %client_id,
        session_id = session_id,
        "v2 session accepted"
    );
}

/// A live session ended with its §6 close code.
pub fn session_closed(client_id: &str, code: u32) {
    tracing::info!(
        event = %"session.closed",
        client_id = %client_id,
        code = code,
        "v2 session closed"
    );
}

/// A paired Client was revoked (the record is kept for audit).
pub fn client_revoked(client_id: &str) {
    tracing::info!(
        event = %"client.revoked",
        client_id = %client_id,
        "v2 client revoked"
    );
}

/// The Server identity was reset; every pairing was invalidated.
pub fn identity_reset(fingerprint: &str) {
    tracing::info!(
        event = %"identity.reset",
        fingerprint = %fingerprint,
        "v2 identity reset"
    );
}
