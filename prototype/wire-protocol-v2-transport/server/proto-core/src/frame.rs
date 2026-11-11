//! Control-plane frame codec for the throwaway wire-protocol-v2 prototype.
//!
//! Wire shape (fixed for the prototype, NOT the v2 normative layout):
//!
//! ```text
//! u32 BE frame_len | u8 msg_type | payload     # frame_len = 1 + payload.len(); cap 64 KiB
//! ```
//!
//! One reliable bidirectional QUIC stream carries a sequence of these frames.

/// ALPN identifier; a major-version mismatch must fail before any app data.
pub const ALPN: &[u8] = b"audio-share/v2";
/// Upper bound for one control frame (length field inclusive).
pub const MAX_FRAME_LEN: u32 = 64 * 1024;

/// Message type byte values (prototype throwaway numbering).
pub mod msg_type {
    pub const AUTH: u8 = 0x01;
    pub const AUTH_OK: u8 = 0x02;
    pub const AUTH_ERR: u8 = 0x03;
    pub const SUBSCRIBE: u8 = 0x04;
    pub const MEDIA_START: u8 = 0x05;
    pub const UNSUBSCRIBE: u8 = 0x06;
    pub const CLOSE: u8 = 0x07;
}

/// AUTH_ERR payload codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AuthErrCode {
    BadCredential = 1,
    NotAuthenticated = 2,
    Malformed = 3,
    RateLimited = 4,
}

impl AuthErrCode {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::BadCredential),
            2 => Some(Self::NotAuthenticated),
            3 => Some(Self::Malformed),
            4 => Some(Self::RateLimited),
            _ => None,
        }
    }
}

/// CLOSE payload reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CloseReason {
    Normal = 0,
    ServerStop = 1,
    ProtocolError = 2,
    AuthFailed = 3,
    IdleTimeout = 4,
    InternalError = 5,
}

impl CloseReason {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Normal),
            1 => Some(Self::ServerStop),
            2 => Some(Self::ProtocolError),
            3 => Some(Self::AuthFailed),
            4 => Some(Self::IdleTimeout),
            5 => Some(Self::InternalError),
            _ => None,
        }
    }
}

/// MEDIA_START payload: the media configuration the server will send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaStartPayload {
    pub stream_instance_id: u32,
    pub config_generation: u32,
    pub sample_rate: u32,
    pub channels: u8,
    /// 1 = f32le (the only format the prototype sends).
    pub sample_format: u8,
}

/// One decoded control frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Auth { token: String },
    AuthOk { session_id: u32 },
    AuthErr(AuthErrCode),
    Subscribe,
    MediaStart(MediaStartPayload),
    Unsubscribe,
    Close(CloseReason),
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EncodeError {
    #[error("frame too large: {len} > {cap}")]
    TooLarge { len: u32, cap: u32 },
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum FrameParseError {
    #[error("unknown msg_type 0x{0:02x}")]
    UnknownType(u8),
    #[error("bad payload length {got} for msg_type 0x{ty:02x} (want {want})")]
    BadPayloadLen { ty: u8, got: usize, want: &'static str },
    #[error("invalid utf-8 in AUTH payload")]
    BadUtf8,
    #[error("unknown auth error code {0}")]
    UnknownAuthErrCode(u8),
    #[error("unknown close reason {0}")]
    UnknownCloseReason(u8),
}

pub fn encode(frame: &Frame) -> Result<Vec<u8>, EncodeError> {
    let (ty, payload): (u8, Vec<u8>) = match frame {
        Frame::Auth { token } => (msg_type::AUTH, token.as_bytes().to_vec()),
        Frame::AuthOk { session_id } => (msg_type::AUTH_OK, session_id.to_be_bytes().to_vec()),
        Frame::AuthErr(code) => (msg_type::AUTH_ERR, vec![*code as u8]),
        Frame::Subscribe => (msg_type::SUBSCRIBE, Vec::new()),
        Frame::MediaStart(m) => {
            let mut payload = Vec::with_capacity(14);
            payload.extend_from_slice(&m.stream_instance_id.to_be_bytes());
            payload.extend_from_slice(&m.config_generation.to_be_bytes());
            payload.extend_from_slice(&m.sample_rate.to_be_bytes());
            payload.push(m.channels);
            payload.push(m.sample_format);
            (msg_type::MEDIA_START, payload)
        }
        Frame::Unsubscribe => (msg_type::UNSUBSCRIBE, Vec::new()),
        Frame::Close(reason) => (msg_type::CLOSE, vec![*reason as u8]),
    };
    let frame_len = 1u32.checked_add(payload.len() as u32).expect("payload length fits u32");
    if frame_len > MAX_FRAME_LEN {
        return Err(EncodeError::TooLarge { len: frame_len, cap: MAX_FRAME_LEN });
    }
    let mut out = Vec::with_capacity(4 + frame_len as usize);
    out.extend_from_slice(&frame_len.to_be_bytes());
    out.push(ty);
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Requires `payload` to hold exactly `n` bytes, else a length error.
fn want_exact(ty: u8, payload: &[u8], n: usize, want: &'static str) -> Result<(), FrameParseError> {
    if payload.len() == n {
        Ok(())
    } else {
        Err(FrameParseError::BadPayloadLen { ty, got: payload.len(), want })
    }
}

/// Decodes the payload of one frame whose 4-byte length prefix and 1-byte type
/// have already been consumed from the wire.
pub fn parse(msg_type: u8, payload: &[u8]) -> Result<Frame, FrameParseError> {
    match msg_type {
        msg_type::AUTH => {
            let token = std::str::from_utf8(payload).map_err(|_| FrameParseError::BadUtf8)?;
            Ok(Frame::Auth { token: token.to_string() })
        }
        msg_type::AUTH_OK => {
            want_exact(msg_type::AUTH_OK, payload, 4, "u32 BE session_id")?;
            let session_id = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
            Ok(Frame::AuthOk { session_id })
        }
        msg_type::AUTH_ERR => {
            want_exact(msg_type::AUTH_ERR, payload, 1, "u8 code")?;
            AuthErrCode::from_u8(payload[0])
                .map(Frame::AuthErr)
                .ok_or(FrameParseError::UnknownAuthErrCode(payload[0]))
        }
        msg_type::SUBSCRIBE => {
            want_exact(msg_type::SUBSCRIBE, payload, 0, "empty")?;
            Ok(Frame::Subscribe)
        }
        msg_type::MEDIA_START => {
            want_exact(msg_type::MEDIA_START, payload, 14, "u32,u32,u32,u8,u8 all BE")?;
            Ok(Frame::MediaStart(MediaStartPayload {
                stream_instance_id: u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]),
                config_generation: u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]),
                sample_rate: u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]),
                channels: payload[12],
                sample_format: payload[13],
            }))
        }
        msg_type::UNSUBSCRIBE => {
            want_exact(msg_type::UNSUBSCRIBE, payload, 0, "empty")?;
            Ok(Frame::Unsubscribe)
        }
        msg_type::CLOSE => {
            want_exact(msg_type::CLOSE, payload, 1, "u8 reason")?;
            CloseReason::from_u8(payload[0])
                .map(Frame::Close)
                .ok_or(FrameParseError::UnknownCloseReason(payload[0]))
        }
        unknown => Err(FrameParseError::UnknownType(unknown)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(frame: &Frame) {
        let bytes = encode(frame).expect("encode");
        let len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        assert_eq!(len, bytes.len() - 4, "frame_len must equal 1 + payload.len()");
        assert_eq!(len >= 1, true, "frame_len covers the type byte");
        let parsed = parse(bytes[4], &bytes[5..]).expect("parse");
        assert_eq!(&parsed, frame);
    }

    #[test]
    fn auth_roundtrips() {
        roundtrip(&Frame::Auth {
            token: "test-credential-do-not-use".into(),
        });
        roundtrip(&Frame::Auth { token: String::new() });
    }

    #[test]
    fn auth_ok_roundtrips() {
        roundtrip(&Frame::AuthOk { session_id: 7 });
        roundtrip(&Frame::AuthOk { session_id: u32::MAX });
    }

    #[test]
    fn auth_err_roundtrips_all_codes() {
        for code in [
            AuthErrCode::BadCredential,
            AuthErrCode::NotAuthenticated,
            AuthErrCode::Malformed,
            AuthErrCode::RateLimited,
        ] {
            roundtrip(&Frame::AuthErr(code));
        }
    }

    #[test]
    fn subscribe_unsubscribe_are_empty_payloads() {
        let sub = encode(&Frame::Subscribe).expect("encode");
        assert_eq!(sub.len(), 5, "4-byte len + 1-byte type, empty payload");
        assert_eq!(u32::from_be_bytes([sub[0], sub[1], sub[2], sub[3]]), 1);
        assert_eq!(sub[4], msg_type::SUBSCRIBE);
        let unsub = encode(&Frame::Unsubscribe).expect("encode");
        assert_eq!(unsub.len(), 5);
        assert_eq!(unsub[4], msg_type::UNSUBSCRIBE);
    }

    #[test]
    fn media_start_roundtrips_and_layout_is_be() {
        let frame = Frame::MediaStart(MediaStartPayload {
            stream_instance_id: 1,
            config_generation: 1,
            sample_rate: 48000,
            channels: 2,
            sample_format: 1,
        });
        let bytes = encode(&frame).expect("encode");
        assert_eq!(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]), 15, "1 type byte + 14 payload bytes");
        assert_eq!(bytes[4], msg_type::MEDIA_START);
        // u32 BE stream_instance_id
        assert_eq!(&bytes[5..9], &1u32.to_be_bytes());
        // u32 BE config_generation
        assert_eq!(&bytes[9..13], &1u32.to_be_bytes());
        // u32 BE sample_rate = 48000
        assert_eq!(&bytes[13..17], &48000u32.to_be_bytes());
        // u8 channels
        assert_eq!(bytes[17], 2);
        // u8 sample_format (1 = f32le)
        assert_eq!(bytes[18], 1);
        assert_eq!(bytes.len(), 19, "4 len + 1 type + 14 payload");
        roundtrip(&frame);
    }

    #[test]
    fn close_roundtrips_all_reasons() {
        for reason in [
            CloseReason::Normal,
            CloseReason::ServerStop,
            CloseReason::ProtocolError,
            CloseReason::AuthFailed,
            CloseReason::IdleTimeout,
            CloseReason::InternalError,
        ] {
            roundtrip(&Frame::Close(reason));
        }
    }

    #[test]
    fn unknown_msg_type_is_rejected() {
        assert_eq!(parse(0x99, &[]), Err(FrameParseError::UnknownType(0x99)));
    }

    #[test]
    fn wrong_payload_lengths_are_rejected() {
        assert!(parse(msg_type::AUTH_OK, &[1, 2, 3]).is_err(), "AUTH_OK needs 4 bytes");
        assert!(parse(msg_type::AUTH_ERR, &[]).is_err(), "AUTH_ERR needs 1 byte");
        assert!(parse(msg_type::SUBSCRIBE, &[0]).is_err(), "SUBSCRIBE must be empty");
        assert!(parse(msg_type::UNSUBSCRIBE, &[0, 0]).is_err());
        assert!(parse(msg_type::CLOSE, &[0, 0]).is_err());
        assert!(parse(msg_type::MEDIA_START, &[0u8; 13]).is_err());
        assert!(parse(msg_type::MEDIA_START, &[0u8; 15]).is_err());
    }

    #[test]
    fn bad_utf8_auth_payload_is_rejected() {
        assert_eq!(parse(msg_type::AUTH, &[0xff, 0xfe]), Err(FrameParseError::BadUtf8));
    }

    #[test]
    fn unknown_auth_err_code_and_close_reason_are_rejected() {
        assert_eq!(parse(msg_type::AUTH_ERR, &[0]), Err(FrameParseError::UnknownAuthErrCode(0)));
        assert_eq!(parse(msg_type::AUTH_ERR, &[9]), Err(FrameParseError::UnknownAuthErrCode(9)));
        assert_eq!(parse(msg_type::CLOSE, &[9]), Err(FrameParseError::UnknownCloseReason(9)));
    }

    #[test]
    fn oversized_frames_reject_at_encode() {
        let token = "x".repeat(MAX_FRAME_LEN as usize);
        assert_eq!(
            encode(&Frame::Auth { token }),
            Err(EncodeError::TooLarge { len: MAX_FRAME_LEN + 1, cap: MAX_FRAME_LEN })
        );
    }
}
