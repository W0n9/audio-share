//! Per-connection statistics for the prototype server.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

/// CLOSE reason values (mirror of `proto_core::frame::CloseReason`); used as
/// the QUIC application-error code on transport close and as an "unset"
/// sentinel base for atomic slots.
pub const CLOSE_REASON_NORMAL: u32 = 0;
pub const CLOSE_REASON_SERVER_STOP: u32 = 1;
pub const CLOSE_REASON_PROTOCOL_ERROR: u32 = 2;
pub const CLOSE_REASON_AUTH_FAILED: u32 = 3;
pub const CLOSE_REASON_IDLE_TIMEOUT: u32 = 4;
pub const CLOSE_REASON_INTERNAL_ERROR: u32 = 5;

/// Sentinel meaning "no CLOSE recorded in this direction yet".
const CLOSE_UNSET: u8 = 255;

/// Session state machine positions.
pub const STATE_CONNECTING: u8 = 0;
pub const STATE_AUTHENTICATING: u8 = 1;
pub const STATE_READY: u8 = 2;
pub const STATE_PLAYING: u8 = 3;
pub const STATE_CLOSING: u8 = 4;
pub const STATE_CLOSED: u8 = 5;

pub fn state_name(state: u8) -> &'static str {
    match state {
        STATE_CONNECTING => "CONNECTING",
        STATE_AUTHENTICATING => "AUTHENTICATING",
        STATE_READY => "READY",
        STATE_PLAYING => "PLAYING",
        STATE_CLOSING => "CLOSING",
        STATE_CLOSED => "CLOSED",
        _ => "UNKNOWN",
    }
}

/// Lock-free per-session counters shared between the session task, media
/// tasks, and test snapshots.
#[derive(Debug, Default)]
pub struct SessionStats {
    pub session_id: AtomicU64,
    pub remote_addr: std::sync::Mutex<String>,
    pub state: AtomicU8,
    pub frames_auth_in: AtomicU64,
    pub frames_subscribe_in: AtomicU64,
    pub frames_unsubscribe_in: AtomicU64,
    pub frames_close_in: AtomicU64,
    pub auth_ok_out: AtomicU64,
    pub auth_err_out: AtomicU64,
    pub auth_err_bad_credential: AtomicU64,
    pub auth_err_not_authenticated: AtomicU64,
    pub auth_err_malformed: AtomicU64,
    pub media_start_out: AtomicU64,
    pub protocol_errors_out: AtomicU64,
    pub close_out: AtomicU64,
    pub media_produced: AtomicU64,
    pub media_queued: AtomicU64,
    pub media_queue_dropped: AtomicU64,
    pub media_sendbuf_dropped: AtomicU64,
    pub media_sent: AtomicU64,
    pub media_bytes_sent: AtomicU64,
    pub media_batches: AtomicU64,
    pub media_unsubscribes: AtomicU64,
    /// Monotonic media instance counter; instance ids start at 1.
    pub media_instances: AtomicU64,
    /// CLOSE reason sent by the server (u8 value or CLOSE_UNSET).
    pub close_sent_reason: AtomicU8,
    /// CLOSE reason received from the client (u8 value or CLOSE_UNSET).
    pub close_received_reason: AtomicU8,
    pub closed: AtomicBool,
}

impl SessionStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    pub fn set_state(&self, state: u8) {
        self.state.store(state, Ordering::Relaxed);
    }

    pub fn mark_close_sent(&self, reason: u8) {
        self.close_sent_reason.store(reason, Ordering::Relaxed);
    }

    pub fn mark_close_received(&self, reason: u8) {
        self.close_received_reason.store(reason, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> SessionStatsSnapshot {
        SessionStatsSnapshot {
            session_id: self.session_id.load(Ordering::Relaxed),
            remote_addr: self
                .remote_addr
                .lock()
                .expect("remote addr mutex")
                .clone(),
            state: state_name(self.state.load(Ordering::Relaxed)).to_string(),
            frames_auth_in: self.frames_auth_in.load(Ordering::Relaxed),
            frames_subscribe_in: self.frames_subscribe_in.load(Ordering::Relaxed),
            frames_unsubscribe_in: self.frames_unsubscribe_in.load(Ordering::Relaxed),
            frames_close_in: self.frames_close_in.load(Ordering::Relaxed),
            auth_ok_out: self.auth_ok_out.load(Ordering::Relaxed),
            auth_err_out: self.auth_err_out.load(Ordering::Relaxed),
            auth_err_bad_credential: self.auth_err_bad_credential.load(Ordering::Relaxed),
            auth_err_not_authenticated: self
                .auth_err_not_authenticated
                .load(Ordering::Relaxed),
            auth_err_malformed: self.auth_err_malformed.load(Ordering::Relaxed),
            media_start_out: self.media_start_out.load(Ordering::Relaxed),
            protocol_errors_out: self.protocol_errors_out.load(Ordering::Relaxed),
            close_out: self.close_out.load(Ordering::Relaxed),
            media_produced: self.media_produced.load(Ordering::Relaxed),
            media_queued: self.media_queued.load(Ordering::Relaxed),
            media_queue_dropped: self.media_queue_dropped.load(Ordering::Relaxed),
            media_sendbuf_dropped: self.media_sendbuf_dropped.load(Ordering::Relaxed),
            media_sent: self.media_sent.load(Ordering::Relaxed),
            media_bytes_sent: self.media_bytes_sent.load(Ordering::Relaxed),
            media_batches: self.media_batches.load(Ordering::Relaxed),
            media_unsubscribes: self.media_unsubscribes.load(Ordering::Relaxed),
            media_instances: self.media_instances.load(Ordering::Relaxed),
            close_sent_reason: decode_close_reason(self.close_sent_reason.load(Ordering::Relaxed)),
            close_received_reason: decode_close_reason(
                self.close_received_reason.load(Ordering::Relaxed),
            ),
        }
    }
}

fn decode_close_reason(v: u8) -> Option<String> {
    if v == CLOSE_UNSET {
        return None;
    }
    proto_core::frame::CloseReason::from_u8(v).map(|r| format!("{r:?}"))
}

/// Plain snapshot of `SessionStats` for tests and logging.
#[derive(Debug, Clone, Default)]
pub struct SessionStatsSnapshot {
    pub session_id: u64,
    pub remote_addr: String,
    pub state: String,
    pub frames_auth_in: u64,
    pub frames_subscribe_in: u64,
    pub frames_unsubscribe_in: u64,
    pub frames_close_in: u64,
    pub auth_ok_out: u64,
    pub auth_err_out: u64,
    pub auth_err_bad_credential: u64,
    pub auth_err_not_authenticated: u64,
    pub auth_err_malformed: u64,
    pub media_start_out: u64,
    pub protocol_errors_out: u64,
    pub close_out: u64,
    pub media_produced: u64,
    pub media_queued: u64,
    pub media_queue_dropped: u64,
    pub media_sendbuf_dropped: u64,
    pub media_sent: u64,
    pub media_bytes_sent: u64,
    pub media_batches: u64,
    pub media_unsubscribes: u64,
    pub media_instances: u64,
    pub close_sent_reason: Option<String>,
    pub close_received_reason: Option<String>,
}

/// Endpoint-level counters (handshake-stage, not tied to a session).
#[derive(Debug, Clone, Copy, Default)]
pub struct EndpointStatsSnapshot {
    pub accepted: u64,
    pub handshake_failures: u64,
}
