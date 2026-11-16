//! Shared Wire Protocol v2 Server runtime.
//!
//! One assembly of the Ed25519 identity, authorization store, Pairing Core,
//! QUIC listener, session driver, and the v1 capture/media adapters. Both
//! `as-cmd` and the Server GUI host this runtime, so pairing logic exists in
//! exactly one place (beta spec: "one Pairing Core").

pub mod admin;
pub mod capture;
pub mod qr;
pub mod runtime;

pub use admin::{
    ClientStateView, ClientView, CorePairingAdmin, InvitationView, PairingAdmin,
    PendingRequestView, RequestedPairingMode,
};
pub use capture::{
    capture_media_source, v2_capture_snapshot, v2_capture_source, CaptureMediaSource,
};
pub use runtime::{
    data_dir_for_config, resolve_bind_address, RuntimeConfig, RuntimeError, V2Runtime,
    REVOCATION_POLL_INTERVAL,
};
pub use v2_transport::codes::DEFAULT_V2_PORT;
