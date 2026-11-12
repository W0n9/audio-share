//! wire-protocol-v2 Android Client — native core (ticket 08).
//!
//! Evolved from the ticket-01 prototype's validated skeleton. What is kept
//! from the prototype (device-validated): the SPKI-pinned verifier, JNI by
//! value JNIEnv (the prototype's `&mut JNIEnv` failure), the direct
//! ByteBuffer ring skeleton (media path NOT activated in this ticket), the
//! bounded backoff with ±20% jitter, and the NetworkCallback rebind +
//! fallback. What replaces the prototype: prost codegen of the frozen control
//! schema (`protos/wire-protocol-v2/control.proto`) instead of the hand-written
//! codec, and the §5.3 Client state machine implemented row by row.
//!
//! Normative sources: `docs/wire-protocol-v2/spec.md` (§3 framing, §4 schema,
//! §5.3 Client table, §6 registry, §7 media header), `docs/wire-protocol-v2/pairing.md`
//! (§9.1 auth transcript, §6.3 pairing binding), and
//! `fixtures/wire-protocol-v2/` (byte-level replay contract).

pub mod auth;
pub mod control;
#[cfg(test)]
pub mod fixture;
pub mod frame;
pub mod jni;
pub mod media;
pub mod ring;
pub mod session;
pub mod verify;

/// prost-generated module for the frozen schema (package audio.share.v2).
pub mod proto {
    #![allow(clippy::all)]
    include!(concat!(env!("OUT_DIR"), "/audio.share.v2.rs"));
}

/// android_logger init (device only). Called from createSession.
#[cfg(target_os = "android")]
pub fn init_android_logger() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Info),
        );
    });
}
