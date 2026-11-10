//! Wire Protocol v0.3.4 (frozen).
//!
//! The Android Client sends `u32` little-endian commands on the TCP control
//! channel. `GET_FORMAT` and `START_PLAY` replies add payload bytes; PCM is
//! sent over UDP after the Android Client binds its UDP address with the play
//! id.

pub mod format;
pub mod framing;
pub mod mtu;

pub use format::{AudioFormat, Encoding};
pub use framing::{
    decode_command, decode_get_format_response, decode_heartbeat, decode_start_play_response,
    decode_udp_bind, encode_command, encode_get_format_response, encode_heartbeat,
    encode_start_play_response, encode_udp_bind, Command, ProtocolError,
};
pub use mtu::{
    max_segment_payload, segment_audio, IP_HEADER_BYTES, MAX_PAYLOAD_BYTES, MTU, UDP_HEADER_BYTES,
};

use std::time::Duration;

/// The Server sends a heartbeat every three seconds.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(3);

/// A session with no client heartbeat for five seconds is closed.
pub const SESSION_TIMEOUT: Duration = Duration::from_secs(5);
