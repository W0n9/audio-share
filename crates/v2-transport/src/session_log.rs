//! Structured, secret-free v2 session lifecycle events (ticket 33).
//!
//! Support must be able to verify pairing, session establishment,
//! `ClientClosed(701)`, revocation, and identity reset from `server.log`.
//! Events carry only the public identifiers each one needs: the canonical
//! `client_id` (first 8 bytes of SHA-256 of the Client public key, lowercase
//! hex, pairing.md §8.1) for pairing and sessions, the §6 `code` for a closed
//! session, and the public SPKI `fingerprint` for an identity reset.
//!
//! # Red line
//!
//! These helpers accept only public values. They must never be given the SAS,
//! a nonce, the binding context, a client proof, key material, or raw audio;
//! the test suite drives a live SAS pairing and asserts none of it appears in
//! the captured log (`tests/session_log.rs`).

use crate::schema::PairingMode;

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

/// A revoked Client record was deleted (permanent; ADR-0006).
pub fn client_deleted(client_id: &str) {
    tracing::info!(
        event = %"client.deleted",
        client_id = %client_id,
        "v2 client record deleted"
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

/// Server-side media drops at the bounded cadence (ticket M5-05). Distinguishes
/// pre-packetize capture drops (`MediaPump.dropped_chunks`) and encoded-datagram
/// queue drops (`MediaSendQueue.dropped_newest`) from network loss, which is a
/// Client-side §9 counter and never appears here. Public counters only — no key
/// material or audio.
pub fn media_drops(
    session_id: u64,
    stream_instance_id: u32,
    pump_dropped_chunks: u64,
    queue_dropped_newest: u64,
    pending_frames: u64,
) {
    tracing::info!(
        event = %"media.drops",
        session_id = session_id,
        instance = stream_instance_id,
        pump_dropped_chunks = pump_dropped_chunks,
        queue_dropped_newest = queue_dropped_newest,
        pending_frames = pending_frames,
        "v2 server-side media drops"
    );
}

/// Periodic (5 s) media-pump snapshot (M5-08). Unlike `media.drops` this is
/// emitted whether or not a drop occurred, so `pending_frames` is an unbiased
/// sample and the pacer's delivered-frame rate and call rate are measurable.
/// Public counters only — no key material or audio.
pub fn media_stats(
    session_id: u64,
    stream_instance_id: u32,
    pending_frames: u64,
    consumed_frames: u64,
    pacer_calls: u64,
    pacer_calls_per_sec: u64,
) {
    tracing::info!(
        event = %"media.stats",
        session_id = session_id,
        instance = stream_instance_id,
        pending_frames = pending_frames,
        consumed_frames = consumed_frames,
        pacer_calls = pacer_calls,
        pacer_calls_per_sec = pacer_calls_per_sec,
        "v2 server-side media pump stats"
    );
}
