//! Control framing (spec §3): `u32 BE length | Frame`, 64 KiB cap enforced
//! before allocation, streaming decode across chunk boundaries, and one
//! encode path. The Frame message itself is prost-generated from the frozen
//! schema — this module owns only the length prefix and the size rules.
//!
//! Tautology guard: every test expected-byte value comes from
//! `fixtures/wire-protocol-v2/` golden vectors (hand-derived from the spec),
//! never from this module's own encoder.

use crate::proto::Frame;
use prost::Message as _;

/// Spec §3 / §10: control frame cap. Violation → Error(401)+Close(401),
/// before any allocation.
pub const CONTROL_FRAME_CAP: usize = 65536;

/// A framed `Frame`: length prefix + serialized message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FramedError(pub String);

impl std::fmt::Display for FramedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Serialize `frame` with the `u32 BE` length prefix (spec §3).
pub fn encode_frame(frame: &Frame) -> Result<Vec<u8>, FramedError> {
    let body = frame.encode_to_vec();
    let len = body.len();
    if len == 0 || len > CONTROL_FRAME_CAP {
        // a sender can never produce either: an empty Frame is unset-body (401)
        return Err(FramedError(format!("frame length {len} violates the §3 cap")));
    }
    let mut out = Vec::with_capacity(4 + len);
    out.extend_from_slice(&(len as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Serialize the Frame message alone (no length prefix) — the fixtures'
/// `body_hex` comparison target.
pub fn encode_body(frame: &Frame) -> Vec<u8> {
    frame.encode_to_vec()
}

/// Streaming decoder: accepts arbitrary byte chunks, emits complete Frames.
/// Length validation happens before any body allocation (spec §3).
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push raw control-stream bytes; returns every complete frame in order.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Frame>, FramedError> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            if self.buf.len() < 4 {
                return Ok(out);
            }
            let len = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]])
                as usize;
            // §3: length == 0 or length > 65536 → 401, before any allocation
            if len == 0 || len > CONTROL_FRAME_CAP {
                self.buf.clear();
                return Err(FramedError(format!("frame length {len} violates §3 (0 or > 64 KiB)")));
            }
            if self.buf.len() < 4 + len {
                return Ok(out);
            }
            let body = self.buf[4..4 + len].to_vec();
            self.buf.drain(..4 + len);
            let frame = Frame::decode(body.as_slice())
                .map_err(|e| FramedError(format!("frame decode: {e}")))?;
            out.push(frame);
        }
    }

    /// Bytes buffered waiting for a complete frame.
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::frames::close_frame;
    use crate::proto::frame::Body;

    /// Golden vector control.error-701-client-stopping, body_hex — the
    /// expected encoding is the fixtures' hand-derived bytes, not ours.
    #[test]
    fn encode_matches_fixture_close_body() {
        let frame = close_frame(701);
        let body = encode_body(&frame);
        assert_eq!(hex::encode(&body), "320308bd05");
        let framed = encode_frame(&frame).unwrap();
        assert_eq!(hex::encode(&framed), "00000005320308bd05");
    }

    /// Golden vector control.error-401-frame-length-zero: a 0 length prefix is
    /// rejected before allocation, error mentions the rule.
    #[test]
    fn decoder_rejects_zero_length_prefix() {
        let mut d = FrameDecoder::new();
        let err = d.push(&hex::decode("00000000").unwrap()).unwrap_err();
        assert!(err.0.contains("0"), "got: {err}");
        assert_eq!(d.buffered(), 0);
    }

    /// Golden vector control.error-401-frame-length-oversize: length 65537
    /// (0x00010001) is rejected before allocation.
    #[test]
    fn decoder_rejects_oversize_length_prefix() {
        let mut d = FrameDecoder::new();
        let err = d.push(&hex::decode("00010001").unwrap()).unwrap_err();
        assert!(err.0.contains("65537"), "got: {err}");
    }

    /// Exactly 65536 is the cap boundary (limits.frame-cap-65536-accepted):
    /// the decoder must accept a well-formed frame at the cap, only the prefix
    /// above it is 401. Use a minimal body plus padding-free length: we
    /// synthesize a 65536-byte *declared* length only to prove the boundary
    /// logic — the real cap-acceptance replay lives in the fixture runner.
    #[test]
    fn decoder_accepts_length_exactly_at_cap() {
        // A Subscribe with unknown-field padding that sums to 65536 would be
        // the fixture case; here we only prove 65536 is not rejected at the
        // length check by pushing a 65536-byte frame of a valid Subscribe.
        let mut body = vec![0x82, 0x02, 0x09];
        body.extend_from_slice(&1u64.to_le_bytes()); // request_id 1 (fixed64 f1)
        // pad with unknown field 1000 (varint, wire type 0): tag 0xa0 0x3e …
        // Simpler: pad via an unknown field 15 varint of zeros to reach size.
        // Build padding with field 1000 wiretype 2 (bytes): tag c2 3e, len …
        // We construct the exact fixture body instead:
        let fixture_body = {
            let h = hex::decode(
                "820209090100000000000000c23eefff03000000000000000000000000000000000000000000000000000000000000000000",
            ).unwrap();
            // fixture body is 65536 bytes total: rebuild by extending zeros
            let mut v = h;
            v.resize(65536, 0);
            v
        };
        assert_eq!(fixture_body.len(), 65536);
        let mut d = FrameDecoder::new();
        let mut bytes = Vec::with_capacity(4 + 65536);
        bytes.extend_from_slice(&(65536u32).to_be_bytes());
        bytes.extend_from_slice(&fixture_body);
        let frames = d.push(&bytes).unwrap();
        assert_eq!(frames.len(), 1);
        match frames[0].body.as_ref().unwrap() {
            Body::Subscribe(s) => assert_eq!(s.request_id, 1),
            other => panic!("expected subscribe, got {other:?}"),
        }
    }

    /// Streaming: a frame split across three pushes decodes once complete.
    #[test]
    fn decoder_reassembles_across_push_boundaries() {
        let mut d = FrameDecoder::new();
        let framed = encode_frame(&close_frame(602)).unwrap();
        assert!(d.push(&framed[..2]).unwrap().is_empty());
        assert!(d.push(&framed[2..5]).unwrap().is_empty());
        let out = d.push(&framed[5..]).unwrap();
        assert_eq!(out.len(), 1);
        match out[0].body.as_ref().unwrap() {
            Body::Close(c) => assert_eq!(c.code, 602),
            other => panic!("expected close, got {other:?}"),
        }
    }

    /// Undecodable body bytes (fixture control.error-401-undecodable:
    /// `0aff01` — a length-delimited field with no terminator) → decode error.
    #[test]
    fn decoder_rejects_undecodable_body() {
        let mut d = FrameDecoder::new();
        let mut bytes = vec![0, 0, 0, 3];
        bytes.extend_from_slice(&hex::decode("0aff01").unwrap());
        assert!(d.push(&bytes).is_err());
    }
}

// hex is a test-only helper; keep it out of the shipped crate.
#[cfg(test)]
pub(crate) mod hex {
    pub fn encode(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }
    pub fn decode(s: &str) -> Result<Vec<u8>, String> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|e| e.to_string()))
            .collect()
    }
}
