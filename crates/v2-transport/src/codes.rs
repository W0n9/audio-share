//! Normative v2 constants and the §6 error registry.
//!
//! Every value here is a recorded decision of the accepted spec
//! (`docs/wire-protocol-v2/spec.md`), never a library default.

/// Default v2 listen port, UDP (spec D1; v1 keeps its own 65530).
pub const DEFAULT_V2_PORT: u16 = 65_531;

/// The control-stream ALPN token (spec §3). Exactly this byte string; any
/// other ALPN fails the handshake before application data.
pub const ALPN_PROTOCOL: &[u8] = b"audio-share/v2";

/// Wire Protocol major carried by ALPN and validated in-band (spec §4.4).
pub const PROTOCOL_MAJOR: u32 = 2;

/// Server minor within major 2 for v2.0 (spec §4.4).
pub const PROTOCOL_MINOR: u32 = 0;

/// Control-frame length cap in bytes: 64 KiB (spec §3, §10).
///
/// `length == 0` or `length > MAX_CONTROL_FRAME` → Error(401)+Close(401),
/// before any allocation. Only a length *strictly greater* than this constant
/// is a violation; 65536 itself is accepted (spec §3, limits fixture).
pub const MAX_CONTROL_FRAME_BYTES: usize = 65_536;

/// Length-prefix width in bytes (spec §3).
pub const LENGTH_PREFIX_BYTES: usize = 4;

/// Total established sessions the Server admits by default, range 1–256
/// (spec §10, TM-2).
pub const DEFAULT_MAX_SESSIONS: u32 = 32;

/// Concurrent handshakes in progress (spec §10, TM-1).
pub const MAX_CONCURRENT_HANDSHAKES: u32 = 16;

/// Outstanding requests per session (spec §10); over → 302.
pub const MAX_OUTSTANDING_REQUESTS: u32 = 8;

/// Per-Client media send queue in datagrams, drop-newest (spec §10, TM-8).
pub const MEDIA_QUEUE_DATAGRAMS: usize = 256;

/// Maximum bytes of one datagram payload sent through [`crate::send_queue`];
/// media senders size datagrams from the current `max_datagram_size()` (D2)
/// and never exceed the QUIC packet limit (spec §7.3).
pub const MAX_DATAGRAM_BYTES: usize = 64 * 1024;

/// Detail cap in bytes for `Status.detail` and `Close.detail` (spec §4.2).
pub const MAX_DETAIL_BYTES: usize = 256;

/// Server identity key file name inside the data directory (pairing.md §10.2).
pub const IDENTITY_KEY_FILE: &str = "server-identity.pem";

/// The §6.2 error registry (spec §6, D6). Codes are `uint32`, frozen once
/// published, never renumbered; `0` is never sent.
///
/// `retryable` and `requires_repairing` are the registry defaults every v2.0
/// sender MUST equal (spec §4.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ErrorCode {
    /// The registry code (u32, §6.1).
    pub code: u32,
    /// The §6.2 `Retryable` column.
    pub retryable: bool,
    /// The §6.2 `Re-pairing` column (`yes` = `true`).
    pub requires_repairing: bool,
}

impl ErrorCode {
    /// Every code of the §6.2 registry, in band order.
    pub const ALL: [ErrorCode; 21] = [
        Self::VERSION_MISMATCH,
        Self::UNSUPPORTED_CAPABILITY,
        Self::PAIRING_REJECTED,
        Self::AUTHORIZATION_UNKNOWN,
        Self::AUTHENTICATION_FAILED,
        Self::AUTHORIZATION_DENIED,
        Self::SERVER_IDENTITY_MISMATCH,
        Self::PAIRING_PENDING_TIMEOUT,
        Self::BUSY,
        Self::RATE_LIMITED,
        Self::SESSION_SUPERSEDED,
        Self::MALFORMED_MESSAGE,
        Self::INVALID_STATE,
        Self::UNSUPPORTED_MEDIA_CONFIGURATION,
        Self::CAPTURE_FAILURE,
        Self::STREAM_ENDED,
        Self::SERVER_STOPPING,
        Self::INTERNAL_ERROR,
        Self::NETWORK_FAILURE,
        Self::DATAGRAM_UNSUPPORTED,
        Self::CLIENT_STOPPING,
    ];

    /// 101: major ≠ 2 or unsupported minor.
    pub const VERSION_MISMATCH: ErrorCode = ErrorCode { code: 101, retryable: false, requires_repairing: false };
    /// 102: a listed capability cannot be honored.
    pub const UNSUPPORTED_CAPABILITY: ErrorCode = ErrorCode { code: 102, retryable: false, requires_repairing: false };
    /// 201: nonce invalid/expired/replayed, binding mismatch, or declined.
    pub const PAIRING_REJECTED: ErrorCode = ErrorCode { code: 201, retryable: false, requires_repairing: true };
    /// 202: signature valid but the identity matches no Server record.
    pub const AUTHORIZATION_UNKNOWN: ErrorCode = ErrorCode { code: 202, retryable: false, requires_repairing: true };
    /// 203: signature verification failed (invalid proof).
    pub const AUTHENTICATION_FAILED: ErrorCode = ErrorCode { code: 203, retryable: false, requires_repairing: true };
    /// 204: known identity denied (revoked, insufficient role).
    pub const AUTHORIZATION_DENIED: ErrorCode = ErrorCode { code: 204, retryable: false, requires_repairing: false };
    /// 205: pinned SPKI ≠ presented certificate (local classification only).
    pub const SERVER_IDENTITY_MISMATCH: ErrorCode = ErrorCode { code: 205, retryable: false, requires_repairing: true };
    /// 206: no pairing result within the 10 min lifetime.
    pub const PAIRING_PENDING_TIMEOUT: ErrorCode = ErrorCode { code: 206, retryable: true, requires_repairing: false };
    /// 301: Server at capacity.
    pub const BUSY: ErrorCode = ErrorCode { code: 301, retryable: true, requires_repairing: false };
    /// 302: auth/pairing failure or outstanding-request rate exceeded.
    pub const RATE_LIMITED: ErrorCode = ErrorCode { code: 302, retryable: true, requires_repairing: false };
    /// 303: the same Client identity opened a new session.
    pub const SESSION_SUPERSEDED: ErrorCode = ErrorCode { code: 303, retryable: false, requires_repairing: false };
    /// 401: undecodable, unset body, oversized frame, zero code, invalid
    /// Configuration.
    pub const MALFORMED_MESSAGE: ErrorCode = ErrorCode { code: 401, retryable: false, requires_repairing: false };
    /// 402: request or push illegal for the current state (fatal except the
    /// duplicate-`Subscribe` reply).
    pub const INVALID_STATE: ErrorCode = ErrorCode { code: 402, retryable: false, requires_repairing: false };
    /// 403: receiver cannot decode the published Transfer Format.
    pub const UNSUPPORTED_MEDIA_CONFIGURATION: ErrorCode = ErrorCode { code: 403, retryable: false, requires_repairing: false };
    /// 501: Endpoint capture failed or absent.
    pub const CAPTURE_FAILURE: ErrorCode = ErrorCode { code: 501, retryable: true, requires_repairing: false };
    /// 502: stream instance ended normally.
    pub const STREAM_ENDED: ErrorCode = ErrorCode { code: 502, retryable: true, requires_repairing: false };
    /// 503: administrative Server stop.
    pub const SERVER_STOPPING: ErrorCode = ErrorCode { code: 503, retryable: true, requires_repairing: false };
    /// 599: unexpected Server error (bounded detail).
    pub const INTERNAL_ERROR: ErrorCode = ErrorCode { code: 599, retryable: false, requires_repairing: false };
    /// 601: temporary network failure, idle timeout, undeliverable Close,
    /// deadline.
    pub const NETWORK_FAILURE: ErrorCode = ErrorCode { code: 601, retryable: true, requires_repairing: false };
    /// 602: transport cannot carry media datagrams.
    pub const DATAGRAM_UNSUPPORTED: ErrorCode = ErrorCode { code: 602, retryable: true, requires_repairing: false };
    /// 701: normal Client stop.
    pub const CLIENT_STOPPING: ErrorCode = ErrorCode { code: 701, retryable: false, requires_repairing: false };

    /// Look up a code in the registry; `None` for values outside it.
    pub fn from_code(code: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|entry| entry.code == code)
    }

    /// The registry's stable name (§6.2).
    pub const fn name(self) -> &'static str {
        match self.code {
            101 => "VERSION_MISMATCH",
            102 => "UNSUPPORTED_CAPABILITY",
            201 => "PAIRING_REJECTED",
            202 => "AUTHORIZATION_UNKNOWN",
            203 => "AUTHENTICATION_FAILED",
            204 => "AUTHORIZATION_DENIED",
            205 => "SERVER_IDENTITY_MISMATCH",
            206 => "PAIRING_PENDING_TIMEOUT",
            301 => "BUSY",
            302 => "RATE_LIMITED",
            303 => "SESSION_SUPERSEDED",
            401 => "MALFORMED_MESSAGE",
            402 => "INVALID_STATE",
            403 => "UNSUPPORTED_MEDIA_CONFIGURATION",
            501 => "CAPTURE_FAILURE",
            502 => "STREAM_ENDED",
            503 => "SERVER_STOPPING",
            599 => "INTERNAL_ERROR",
            601 => "NETWORK_FAILURE",
            602 => "DATAGRAM_UNSUPPORTED",
            701 => "CLIENT_STOPPING",
            _ => "UNKNOWN",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.code, self.name())
    }
}

/// QUIC transport-error interpretation helpers over §6 codes (spec §3:
/// "every transport close carries the registry code").
///
/// quinn surfaces close codes as `ConnectionError`; application codes ride the
/// low bits. The helper converts a 60-bit application code back to the
/// registry entry when it is one.
pub fn registry_code_from_app_code(raw: u64) -> Option<ErrorCode> {
    ErrorCode::from_code(u32::try_from(raw).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_port_is_65531_never_v1_port() {
        assert_eq!(DEFAULT_V2_PORT, 65_531);
        assert_ne!(DEFAULT_V2_PORT, crate::codes::DEFAULT_V2_PORT - 1);
        // v1's port is defined in audio-share-core as 65530; v2 never shares it.
        assert_eq!(crate::codes::DEFAULT_V2_PORT - 1, 65_530);
    }

    #[test]
    fn alpn_is_exact_byte_string() {
        assert_eq!(ALPN_PROTOCOL, b"audio-share/v2");
    }

    #[test]
    fn frame_cap_is_64_kib() {
        assert_eq!(MAX_CONTROL_FRAME_BYTES, 65_536);
    }

    #[test]
    fn registry_lookup_round_trips() {
        for entry in ErrorCode::ALL {
            assert_eq!(ErrorCode::from_code(entry.code), Some(entry));
            assert_ne!(entry.name(), "UNKNOWN");
        }
    }

    #[test]
    fn registry_flags_match_spec_62() {
        // Spot-check the §6.2 columns: retryable / re-pairing.
        assert_eq!(ErrorCode::from_code(101).map(|e| (e.retryable, e.requires_repairing)), Some((false, false)));
        assert_eq!(ErrorCode::from_code(201).map(|e| (e.retryable, e.requires_repairing)), Some((false, true)));
        assert_eq!(ErrorCode::from_code(202).map(|e| (e.retryable, e.requires_repairing)), Some((false, true)));
        assert_eq!(ErrorCode::from_code(203).map(|e| (e.retryable, e.requires_repairing)), Some((false, true)));
        assert_eq!(ErrorCode::from_code(204).map(|e| (e.retryable, e.requires_repairing)), Some((false, false)));
        assert_eq!(ErrorCode::from_code(206).map(|e| (e.retryable, e.requires_repairing)), Some((true, false)));
        assert_eq!(ErrorCode::from_code(301).map(|e| (e.retryable, e.requires_repairing)), Some((true, false)));
        assert_eq!(ErrorCode::from_code(303).map(|e| (e.retryable, e.requires_repairing)), Some((false, false)));
        assert_eq!(ErrorCode::from_code(401).map(|e| (e.retryable, e.requires_repairing)), Some((false, false)));
        assert_eq!(ErrorCode::from_code(501).map(|e| (e.retryable, e.requires_repairing)), Some((true, false)));
        assert_eq!(ErrorCode::from_code(599).map(|e| (e.retryable, e.requires_repairing)), Some((false, false)));
        assert_eq!(ErrorCode::from_code(601).map(|e| (e.retryable, e.requires_repairing)), Some((true, false)));
        assert_eq!(ErrorCode::from_code(701).map(|e| (e.retryable, e.requires_repairing)), Some((false, false)));
    }

    #[test]
    fn unknown_code_is_none() {
        assert_eq!(ErrorCode::from_code(0), None);
        assert_eq!(ErrorCode::from_code(999), None);
        // 205 exists in the registry but is a local classification; 205's slot
        // is still resolvable (the Client uses it as a Close code).
        assert_eq!(ErrorCode::from_code(205), Some(ErrorCode::SERVER_IDENTITY_MISMATCH));
    }
}
