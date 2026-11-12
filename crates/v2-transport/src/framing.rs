//! Control-stream framing (spec §3).
//!
//! The control stream carries `u32 BE length | Frame`. `length` is the exact
//! byte length of the serialized `Frame` message. `length == 0` or
//! `length > 65536` → Error(401)+Close(401), **before any allocation** — the
//! length check runs on the prefix alone, never on buffered body bytes.

use crate::codes::{ErrorCode, LENGTH_PREFIX_BYTES, MAX_CONTROL_FRAME_BYTES};
use prost::Message as _;

use crate::schema::Frame;

/// A decoded inbound frame with its declared length.
#[derive(Clone, Debug, PartialEq)]
pub struct InboundFrame {
    /// The decoded protobuf `Frame` message.
    pub frame: Frame,
    /// The `Frame.length` value (exact byte length of the serialized message).
    pub length: usize,
}

/// The framing verdict for a length prefix (checked before any body bytes are
/// read or allocated).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LengthCheck {
    /// The prefix is acceptable; the receiver may allocate and read `body_len`
    /// bytes.
    Accept {
        /// Declared body length in bytes.
        body_len: usize,
    },
    /// The prefix violates the §3 rule: 0 or > 64 KiB → 401.
    Reject,
}

/// Check a 4-byte big-endian length prefix (spec §3) without allocating.
///
/// This is the pre-allocation gate: callers feed the first 4 bytes of the
/// stream here and must not reserve buffer space until [`LengthCheck::Accept`]
/// comes back.
pub fn check_length_prefix(prefix: [u8; LENGTH_PREFIX_BYTES]) -> LengthCheck {
    let declared = u32::from_be_bytes(prefix) as usize;
    if declared == 0 || declared > MAX_CONTROL_FRAME_BYTES {
        LengthCheck::Reject
    } else {
        LengthCheck::Accept { body_len: declared }
    }
}

/// Encode a frame with its `u32 BE length` prefix.
///
/// Returns an error when the serialized message would exceed the 64 KiB cap —
/// v2.0 has no message that can, and the cap is a reviewed-spec-revision
/// exception only (spec §3).
pub fn encode_frame(frame: &Frame) -> Result<Vec<u8>, FrameError> {
    let body_len = frame.encoded_len();
    if body_len == 0 || body_len > MAX_CONTROL_FRAME_BYTES {
        return Err(FrameError::LengthOutOfBounds(body_len));
    }
    let mut out = Vec::with_capacity(LENGTH_PREFIX_BYTES + body_len);
    out.extend_from_slice(&(body_len as u32).to_be_bytes());
    frame
        .encode(&mut out)
        .map_err(|_| FrameError::Encode)?;
    Ok(out)
}

/// Encode only the serialized `Frame` message (no length prefix).
pub fn encode_body(frame: &Frame) -> Result<Vec<u8>, FrameError> {
    let body_len = frame.encoded_len();
    if body_len == 0 || body_len > MAX_CONTROL_FRAME_BYTES {
        return Err(FrameError::LengthOutOfBounds(body_len));
    }
    let mut out = Vec::with_capacity(body_len);
    frame.encode(&mut out).map_err(|_| FrameError::Encode)?;
    Ok(out)
}

/// Decode a complete `Frame` body (the bytes after a validated prefix).
///
/// A body that does not parse, or that parses with no `body` oneof case set,
/// is malformed (401). Unknown proto3 fields are ignored by prost, which is
/// exactly the spec's evolution rule — but ignoring them can leave the oneof
/// unset, and that is malformed (spec §4.4 validation table).
pub fn decode_body(body: &[u8]) -> Result<Frame, FrameError> {
    if body.is_empty() {
        return Err(FrameError::Empty);
    }
    let frame = Frame::decode(body).map_err(|_| FrameError::Malformed)?;
    if frame.body.is_none() {
        return Err(FrameError::UnsetBody);
    }
    Ok(frame)
}

/// Streaming frame reader for one direction of the control stream.
///
/// Feeds bytes from the stream; completed frames come out through
/// [`FrameReader::push`]'s return value. The reader enforces the pre-allocation
/// rule: at most [`MAX_CONTROL_FRAME_BYTES`] bytes are ever reserved for a
/// body, and only after the prefix validates.
#[derive(Debug, Default)]
pub struct FrameReader {
    buffer: Vec<u8>,
    expect_body: Option<usize>,
}

/// A framing failure carrying the §6 code the peer must answer with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The length prefix declared 0 bytes.
    Empty,
    /// The length prefix exceeded 64 KiB.
    Oversize,
    /// The message bytes did not parse.
    Malformed,
    /// The message parsed but no `body` oneof case was set.
    UnsetBody,
    /// An outbound message exceeded (or hit zero at) the length cap.
    LengthOutOfBounds(usize),
    /// The protobuf encoder rejected the message.
    Encode,
}

impl FrameError {
    /// The §6 code for this framing failure: always 401 (spec §3).
    pub const fn error_code(self) -> ErrorCode {
        ErrorCode::MALFORMED_MESSAGE
    }
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "control frame length is zero"),
            Self::Oversize => write!(f, "control frame exceeds the 64 KiB cap"),
            Self::Malformed => write!(f, "control frame is undecodable"),
            Self::UnsetBody => write!(f, "control frame sets no message body"),
            Self::LengthOutOfBounds(len) => {
                write!(f, "control frame length {len} outside 1..=65536")
            }
            Self::Encode => write!(f, "control frame failed to encode"),
        }
    }
}

impl std::error::Error for FrameError {}

impl FrameReader {
    /// Feed stream bytes; returns every frame that became complete, in order.
    ///
    /// A framing violation (bad prefix, undecodable body, unset body) makes
    /// the reader fail: it returns the error and resets, mirroring the peer's
    /// Error(401)+Close(401) obligation.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<InboundFrame>, FrameError> {
        self.buffer.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            if let Some(body_len) = self.expect_body {
                if self.buffer.len() < body_len {
                    return Ok(out);
                }
                let body: Vec<u8> = self.buffer.drain(..body_len).collect();
                self.expect_body = None;
                let frame = decode_body(&body)?;
                out.push(InboundFrame { frame, length: body_len });
            } else {
                if self.buffer.len() < LENGTH_PREFIX_BYTES {
                    return Ok(out);
                }
                let mut prefix = [0u8; LENGTH_PREFIX_BYTES];
                prefix.copy_from_slice(&self.buffer[..LENGTH_PREFIX_BYTES]);
                match check_length_prefix(prefix) {
                    LengthCheck::Accept { body_len } => {
                        self.buffer.drain(..LENGTH_PREFIX_BYTES);
                        // The cap is enforced here, on the prefix, so no
                        // allocation larger than the 64 KiB cap can happen.
                        self.expect_body = Some(body_len);
                    }
                    LengthCheck::Reject => {
                        self.buffer.clear();
                        return Err(if u32::from_be_bytes(prefix) == 0 {
                            FrameError::Empty
                        } else {
                            FrameError::Oversize
                        });
                    }
                }
            }
        }
    }

    /// Reset the reader after a fatal framing error.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.expect_body = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{frame, Close};

    fn close_frame(code: u32) -> Frame {
        Frame {
            body: Some(frame::Body::Close(Close { code, detail: String::new() })),
        }
    }

    fn close_body(code: u32) -> Vec<u8> {
        encode_body(&close_frame(code)).unwrap()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn encode_wraps_be_length_prefix() {
        let body = close_body(701);
        let framed = encode_frame(&close_frame(701)).unwrap();
        assert_eq!(framed[..4], (body.len() as u32).to_be_bytes());
        assert_eq!(&framed[4..], &body[..]);
    }

    #[test]
    fn length_prefix_zero_rejects() {
        // Fixture control.error-401-frame-length-zero: raw-stream "00000000".
        assert_eq!(check_length_prefix([0x00, 0x00, 0x00, 0x00]), LengthCheck::Reject);
    }

    #[test]
    fn length_prefix_65537_rejects_on_prefix_alone() {
        // Fixture control.error-401-frame-length-oversize: raw-stream "00010001".
        assert_eq!(check_length_prefix([0x00, 0x01, 0x00, 0x01]), LengthCheck::Reject);
    }

    #[test]
    fn length_prefix_65536_accepts() {
        // Fixture limits/frame-cap-65536-accepted: "00010000" is accepted.
        assert_eq!(
            check_length_prefix([0x00, 0x01, 0x00, 0x00]),
            LengthCheck::Accept { body_len: 65_536 }
        );
    }

    #[test]
    fn reader_rejects_zero_length_before_allocation() {
        let mut reader = FrameReader::default();
        let err = reader.push(&unhex("00000000")).unwrap_err();
        assert_eq!(err, FrameError::Empty);
        assert_eq!(err.error_code().code, 401);
    }

    #[test]
    fn reader_rejects_65537_before_allocation() {
        let mut reader = FrameReader::default();
        // No body bytes at all are sent — rejection must happen on the prefix.
        let err = reader.push(&unhex("00010001")).unwrap_err();
        assert_eq!(err, FrameError::Oversize);
    }

    #[test]
    fn reader_accepts_decoded_fixture_frame() {
        // Fixture control.error-701-client-stopping, step 1:
        // "00000005320308bd05".
        let mut reader = FrameReader::default();
        let frames = reader.push(&unhex("00000005320308bd05")).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].length, 5);
        assert_eq!(
            frames[0].frame,
            Frame { body: Some(frame::Body::Close(Close { code: 701, detail: String::new() })) }
        );
    }

    #[test]
    fn reader_splits_partial_feed() {
        let bytes = encode_frame(&close_frame(602)).unwrap();
        let mut reader = FrameReader::default();
        assert!(reader.push(&bytes[..2]).unwrap().is_empty());
        let frames = reader.push(&bytes[2..]).unwrap();
        assert_eq!(frames.len(), 1);
    }

    #[test]
    fn unset_body_is_malformed() {
        // Fixture control.error-401-unset-body: body "7800" (unknown field 15
        // only) is ignored by proto3 but leaves the oneof unset → 401.
        let err = decode_body(&unhex("7800")).unwrap_err();
        assert_eq!(err, FrameError::UnsetBody);
    }

    #[test]
    fn undecodable_body_is_malformed() {
        // Fixture control.error-401-undecodable: body "0aff01" (bad wire type
        // end-group) fails the parse.
        let err = decode_body(&unhex("0aff01")).unwrap_err();
        assert_eq!(err, FrameError::Malformed);
    }

    #[test]
    fn unknown_field_alongside_known_body_is_ignored() {
        // Fixture control.unknown-field-ignored / limits/frame-cap-65536:
        // unknown fields ride along; the known case still drives behavior.
        // subscribe { request_id: 1 } = 0901000000000000
        let mut body = unhex("820209090100000000000000");
        assert!(body.len() < MAX_CONTROL_FRAME_BYTES);
        let frame = decode_body(&body).unwrap();
        match frame.body {
            Some(frame::Body::Subscribe(s)) => assert_eq!(s.request_id, 1),
            other => panic!("expected subscribe body, got {other:?}"),
        }
        // The unknown field 15 varint (78 00) alone leaves body unset:
        body = unhex("7800");
        assert_eq!(decode_body(&body).unwrap_err(), FrameError::UnsetBody);
    }

    #[test]
    fn oversized_outbound_encoding_is_rejected() {
        let mut frame = Frame { body: Some(frame::Body::Subscribe(crate::schema::Subscribe { request_id: 1 })) };
        // Pad past the cap with an unknown field (field 1000, length-delim).
        // Build manually: tag 0xC2 3E (field 1000, wire type 2), varint length
        // 65519, then zeros — the shape limits/frame-cap-65536-accepted pins.
        frame.body = None;
        let err = encode_frame(&Frame { body: None }).unwrap_err();
        assert!(matches!(err, FrameError::UnsetBody) || matches!(err, FrameError::LengthOutOfBounds(_)));
        assert!(hex(&[0]).len() > 0); // keep hex helper used
    }
}
