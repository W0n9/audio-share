//! Control-plane framing for the Android client, single-sourced on
//! `proto_core::frame`.
//!
//! The wire shape — `u32 BE frame_len | u8 msg_type | payload`,
//! `frame_len = 1 + payload.len()`, 64 KiB cap, all seven message types and
//! their payload layouts — is owned by `proto_core::frame` (the same crate
//! the server half uses) and is NOT re-declared here. This module keeps only
//! client-side extras: the streaming decoder across push boundaries, the
//! string-typed error used at the JNI boundary, and the message type
//! constants as re-exports for readability. NOT the normative v2 layout.

pub use proto_core::frame::{Frame, MAX_FRAME_LEN};

use proto_core::frame;

pub const TYPE_AUTH: u8 = frame::msg_type::AUTH;
pub const TYPE_AUTH_OK: u8 = frame::msg_type::AUTH_OK;
pub const TYPE_AUTH_ERR: u8 = frame::msg_type::AUTH_ERR;
pub const TYPE_SUBSCRIBE: u8 = frame::msg_type::SUBSCRIBE;
pub const TYPE_MEDIA_START: u8 = frame::msg_type::MEDIA_START;
pub const TYPE_UNSUBSCRIBE: u8 = frame::msg_type::UNSUBSCRIBE;
pub const TYPE_CLOSE: u8 = frame::msg_type::CLOSE;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecError(pub String);

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "codec error: {}", self.0)
    }
}
impl std::error::Error for CodecError {}

pub type CodecResult<T> = Result<T, CodecError>;

/// Encode one control message with the shared proto-core codec. The client
/// never sends a frame large enough to hit the 64 KiB cap (the credential is
/// size-capped at the JNI boundary), but the shared codec is fallible so the
/// error is propagated instead of assumed impossible.
pub fn encode_frame(msg: &Frame) -> CodecResult<Vec<u8>> {
    frame::encode(msg).map_err(|e| CodecError(e.to_string()))
}

/// Streaming frame decoder: feed raw bytes off the QUIC control stream,
/// pop whole frames as they complete. The length-prefix state machine is
/// client-side; payload decoding delegates to `proto_core::frame::parse`.
#[derive(Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append raw bytes, then drain any complete frames out.
    /// Errors are sticky: once a malformed frame is seen the stream is unusable.
    pub fn push(&mut self, bytes: &[u8]) -> CodecResult<Vec<Frame>> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            if self.buf.len() < 4 {
                return Ok(out);
            }
            let frame_len =
                u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
            if frame_len < 1 {
                return Err(CodecError(format!("frame_len {frame_len} < 1")));
            }
            if frame_len > MAX_FRAME_LEN {
                return Err(CodecError(format!(
                    "frame_len {frame_len} exceeds cap {MAX_FRAME_LEN}"
                )));
            }
            let total = 4 + frame_len as usize;
            if self.buf.len() < total {
                return Ok(out);
            }
            let frame = self.buf.drain(..total).collect::<Vec<u8>>();
            out.push(
                frame::parse(frame[4], &frame[5..]).map_err(|e| CodecError(e.to_string()))?,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto_core::frame::{AuthErrCode, CloseReason, MediaStartPayload};

    fn media_start() -> Frame {
        Frame::MediaStart(MediaStartPayload {
            stream_instance_id: 1,
            config_generation: 1,
            sample_rate: 48000,
            channels: 2,
            sample_format: proto_core::media::SAMPLE_FORMAT_F32LE,
        })
    }

    #[test]
    fn auth_roundtrip() {
        let msg = Frame::Auth {
            token: "test-credential-do-not-use".to_string(),
        };
        let bytes = encode_frame(&msg).unwrap();
        // length prefix = 1 + token len
        let expect_len = 1 + "test-credential-do-not-use".len() as u32;
        assert_eq!(&bytes[..4], &expect_len.to_be_bytes());
        assert_eq!(bytes[4], TYPE_AUTH);
        let mut dec = FrameDecoder::new();
        let out = dec.push(&bytes).unwrap();
        assert_eq!(out, vec![msg]);
    }

    #[test]
    fn all_message_types_roundtrip() {
        let msgs = vec![
            Frame::Auth { token: String::new() },
            Frame::Auth { token: "tok".into() },
            Frame::AuthOk { session_id: 0xdead_beef },
            Frame::AuthErr(AuthErrCode::RateLimited),
            Frame::Subscribe,
            media_start(),
            Frame::Unsubscribe,
            Frame::Close(CloseReason::ProtocolError),
        ];
        for msg in msgs {
            let mut dec = FrameDecoder::new();
            assert_eq!(dec.push(&encode_frame(&msg).unwrap()).unwrap(), vec![msg.clone()]);
        }
    }

    #[test]
    fn pipelined_frames_decode_in_order() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&encode_frame(&Frame::Auth { token: "abc".into() }).unwrap());
        bytes.extend_from_slice(&encode_frame(&Frame::Subscribe).unwrap());
        bytes.extend_from_slice(&encode_frame(&Frame::AuthOk { session_id: 9 }).unwrap());
        let mut dec = FrameDecoder::new();
        assert_eq!(
            dec.push(&bytes).unwrap(),
            vec![
                Frame::Auth { token: "abc".into() },
                Frame::Subscribe,
                Frame::AuthOk { session_id: 9 }
            ]
        );
    }

    #[test]
    fn split_across_pushes() {
        let bytes = encode_frame(&media_start()).unwrap();
        let mut dec = FrameDecoder::new();
        // byte-by-byte: no frame may appear until the last byte lands
        for (i, b) in bytes.iter().enumerate() {
            let out = dec.push(&[*b]).unwrap();
            if i + 1 < bytes.len() {
                assert!(out.is_empty(), "frame decoded early at byte {i}");
            } else {
                assert_eq!(out.len(), 1);
            }
        }
    }

    #[test]
    fn zero_frame_len_rejected() {
        let mut dec = FrameDecoder::new();
        assert!(dec.push(&[0, 0, 0, 0, 1]).is_err());
    }

    #[test]
    fn oversized_frame_rejected() {
        // frame_len = cap + 1, valid-looking so far
        let big = (MAX_FRAME_LEN + 1).to_be_bytes();
        let mut dec = FrameDecoder::new();
        assert!(dec.push(&big).is_err());
    }

    #[test]
    fn unknown_type_rejected() {
        // frame_len = 1, type byte = 0x99
        let mut dec = FrameDecoder::new();
        let err = dec.push(&[0, 0, 0, 1, 0x99]).unwrap_err();
        assert!(
            err.0.contains("unknown msg_type"),
            "got: {err}"
        );
    }

    #[test]
    fn malformed_payload_length_rejected() {
        // AUTH_OK needs exactly 4 payload bytes; give 3
        let mut dec = FrameDecoder::new();
        let err = dec
            .push(&[0, 0, 0, 4, TYPE_AUTH_OK, 1, 2, 3])
            .unwrap_err();
        assert!(err.0.contains("payload length"), "got: {err}");
        // SUBSCRIBE must be empty; give 1 byte
        let mut dec = FrameDecoder::new();
        assert!(dec.push(&[0, 0, 0, 2, TYPE_SUBSCRIBE, 0x00]).is_err());
        // MEDIA_START needs exactly 14 payload bytes
        let mut dec = FrameDecoder::new();
        assert!(dec
            .push(&[0, 0, 0, 5, TYPE_MEDIA_START, 1, 2, 3, 4, 5])
            .is_err());
    }

    #[test]
    fn max_cap_frame_accepted() {
        // exactly at the cap: frame_len = 1 + payload, huge utf-8 AUTH token
        let payload_len = MAX_FRAME_LEN as usize - 1;
        let mut dec = FrameDecoder::new();
        let mut bytes = (MAX_FRAME_LEN).to_be_bytes().to_vec();
        bytes.push(TYPE_AUTH);
        bytes.extend(std::iter::repeat(b'a').take(payload_len));
        assert_eq!(
            dec.push(&bytes).unwrap(),
            vec![Frame::Auth {
                token: "a".repeat(payload_len)
            }]
        );
    }

    // ---- cross-end contract (single-sourced on proto-core) ----------------

    /// The server half encodes with `proto_core::frame::encode`; the client
    /// decoder must accept those exact bytes for every message type.
    #[test]
    fn server_encoded_frames_decode_on_client() {
        let frames = vec![
            Frame::Auth {
                token: "test-credential-do-not-use".into(),
            },
            Frame::AuthOk { session_id: 0xdead_beef },
            Frame::AuthErr(AuthErrCode::RateLimited),
            Frame::Subscribe,
            media_start(),
            Frame::Unsubscribe,
            Frame::Close(CloseReason::InternalError),
        ];
        for frame in &frames {
            let bytes = proto_core::frame::encode(frame).expect("server-side encode");
            let mut dec = FrameDecoder::new();
            assert_eq!(dec.push(&bytes).unwrap(), vec![frame.clone()]);
        }
    }

    /// The client encoder's bytes must parse with the server half's
    /// `proto_core::frame::parse` (length prefix stripped).
    #[test]
    fn client_encoded_frames_parse_on_server() {
        let frames = vec![
            Frame::Auth { token: "tok".into() },
            Frame::Subscribe,
            media_start(),
            Frame::Unsubscribe,
            Frame::Close(CloseReason::Normal),
            Frame::AuthErr(AuthErrCode::BadCredential),
        ];
        for frame in &frames {
            let bytes = encode_frame(frame).expect("client-side encode");
            let parsed =
                proto_core::frame::parse(bytes[4], &bytes[5..]).expect("server-side parse");
            assert_eq!(&parsed, frame);
        }
    }
}
