//! Media-plane primitives for the throwaway wire-protocol-v2 prototype.
//!
//! QUIC DATAGRAM layout (fixed for the prototype, NOT the v2 normative layout):
//!
//! ```text
//! u32 BE stream_instance_id | u32 BE config_generation | u32 BE sequence | u64 BE timestamp
//! ```
//!
//! followed by raw f32le interleaved PCM, frame-aligned. `timestamp` is the
//! sample-frame position of the first PCM frame in the payload on the capture
//! clock. Content is deterministic so any peer can verify continuity.

/// Fixed 20-byte media datagram header.
pub const MEDIA_HEADER_LEN: usize = 20;
/// Default transfer-format sample rate (prototype default config).
pub const DEFAULT_SAMPLE_RATE: u32 = 48000;
/// Default channel count (prototype default config).
pub const DEFAULT_CHANNELS: u8 = 2;
/// f32le sample format code carried in MEDIA_START.
pub const SAMPLE_FORMAT_F32LE: u8 = 1;
/// Prototype test tone frequency in Hz.
pub const TEST_TONE_HZ: f64 = 440.0;

/// Fixed media datagram header, all integers big-endian on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaHeader {
    pub stream_instance_id: u32,
    pub config_generation: u32,
    pub sequence: u32,
    /// Sample-frame position of the first PCM frame in the payload (capture clock).
    pub timestamp: u64,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MediaParseError {
    #[error("datagram shorter than the {MEDIA_HEADER_LEN}-byte media header: {0} bytes")]
    TooShort(usize),
}

impl MediaHeader {
    pub fn encode(&self) -> [u8; MEDIA_HEADER_LEN] {
        let mut out = [0u8; MEDIA_HEADER_LEN];
        out[0..4].copy_from_slice(&self.stream_instance_id.to_be_bytes());
        out[4..8].copy_from_slice(&self.config_generation.to_be_bytes());
        out[8..12].copy_from_slice(&self.sequence.to_be_bytes());
        out[12..20].copy_from_slice(&self.timestamp.to_be_bytes());
        out
    }

    /// Splits a datagram into header and PCM payload bytes.
    pub fn parse(buf: &[u8]) -> Result<(MediaHeader, &[u8]), MediaParseError> {
        if buf.len() < MEDIA_HEADER_LEN {
            return Err(MediaParseError::TooShort(buf.len()));
        }
        let header = MediaHeader {
            stream_instance_id: u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]),
            config_generation: u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]),
            sequence: u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]),
            timestamp: u64::from_be_bytes([
                buf[12], buf[13], buf[14], buf[15], buf[16], buf[17], buf[18], buf[19],
            ]),
        };
        Ok((header, &buf[MEDIA_HEADER_LEN..]))
    }
}

/// Deterministic sample value at absolute sample-frame position `p`.
///
/// `sin(2π * 440 * p / 48000) as f32` — the exact prototype-brief formula.
/// Computed in f64 then cast to f32; both ends use this one function so the
/// pattern is reproducible on the same machine.
pub fn sample_value(p: u64) -> f32 {
    ((2.0 * std::f64::consts::PI * TEST_TONE_HZ * p as f64) / DEFAULT_SAMPLE_RATE as f64).sin()
        as f32
}

/// Bytes of one interleaved f32le sample frame with `channels` channels.
pub fn frame_bytes(channels: u8) -> usize {
    channels as usize * 4
}

/// Payload length for one media datagram: `min(max_datagram_size - MEDIA_HEADER_LEN,
/// desired)`, floored to complete sample frames.
///
/// The transport limit is always re-queried by the caller and never assumed
/// (no fixed 1464 arithmetic).
pub fn chunk_payload_len(max_datagram_size: usize, frame_bytes: usize, desired: usize) -> usize {
    let cap = max_datagram_size
        .saturating_sub(MEDIA_HEADER_LEN)
        .min(desired);
    cap - (cap % frame_bytes.max(1))
}

/// Builds one media datagram: 20-byte header + `frames` interleaved f32le
/// sample frames starting at absolute position `sample_start`.
pub fn encode_datagram(
    header: &MediaHeader,
    sample_start: u64,
    frames: usize,
    channels: u8,
) -> bytes::Bytes {
    let fb = frame_bytes(channels);
    let mut buf = Vec::with_capacity(MEDIA_HEADER_LEN + frames * fb);
    buf.extend_from_slice(&header.encode());
    for frame in 0..frames as u64 {
        let value = sample_value(sample_start + frame).to_le_bytes();
        for _ in 0..channels as usize {
            buf.extend_from_slice(&value);
        }
    }
    bytes::Bytes::from(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        let h = MediaHeader {
            stream_instance_id: 3,
            config_generation: 1,
            sequence: 4294967295,
            timestamp: u64::MAX,
        };
        let bytes = h.encode();
        assert_eq!(bytes.len(), 20);
        assert_eq!(&bytes[0..4], &3u32.to_be_bytes());
        assert_eq!(&bytes[4..8], &1u32.to_be_bytes());
        assert_eq!(&bytes[8..12], &4294967295u32.to_be_bytes());
        assert_eq!(&bytes[12..20], &u64::MAX.to_be_bytes());
        let (parsed, rest) = MediaHeader::parse(&bytes).expect("parse");
        assert_eq!(parsed, h);
        assert!(rest.is_empty());
    }

    #[test]
    fn header_parse_preserves_payload_slice() {
        let h = MediaHeader { stream_instance_id: 1, config_generation: 1, sequence: 0, timestamp: 240 };
        let mut buf = h.encode().to_vec();
        buf.extend_from_slice(&[1u8, 2, 3, 4, 5, 6, 7, 8]);
        let (parsed, payload) = MediaHeader::parse(&buf).expect("parse");
        assert_eq!(parsed, h);
        assert_eq!(payload, &[1u8, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn header_parse_rejects_short_datagrams() {
        assert_eq!(MediaHeader::parse(&[0u8; 19]), Err(MediaParseError::TooShort(19)));
        assert_eq!(MediaHeader::parse(&[]), Err(MediaParseError::TooShort(0)));
    }

    /// Independent oracles computed with Python:
    /// `struct.pack('<f', math.sin(2*pi*440*p/48000))` bit patterns.
    /// Tolerance is 1 f32 ulp for possible libm rounding-boundary differences;
    /// the client/server continuity check itself is bit-exact (same function).
    fn expect_sample(p: u64, expected_bits: u32) {
        let actual = sample_value(p);
        let actual_bits = actual.to_bits();
        let diff = actual_bits.abs_diff(expected_bits);
        assert!(
            diff <= 1,
            "p={p}: got bits {actual_bits:#010x} ({actual}), expected {expected_bits:#010x}"
        );
    }

    #[test]
    fn sample_formula_matches_independent_oracle() {
        expect_sample(0, 0x0000_0000); // sin(0) = 0.0 exactly
        expect_sample(1, 0x3d6b_c842);
        expect_sample(2, 0x3deb_642b);
        expect_sample(55, 0xbcd6_710a);
        expect_sample(120, 0x3f16_7918); // sin(2π*0.1) = sin(36°) ≈ 0.5877853
        expect_sample(240, 0x3f73_7871); // sin(2π*0.2) = sin(72°) ≈ 0.9510565
        expect_sample(12345, 0x3f5a_46a0);
        expect_sample(999999, 0xbf55_f779);
    }

    #[test]
    fn frame_bytes_counts_all_channel_samples() {
        assert_eq!(frame_bytes(1), 4);
        assert_eq!(frame_bytes(2), 8);
    }

    #[test]
    fn chunk_payload_len_respects_brief_sizing_rule() {
        // Typical QUIC initial limit: 1200-byte datagrams, 20-byte header,
        // 8-byte stereo f32le frames → 1180 floored to 1176.
        assert_eq!(chunk_payload_len(1200, 8, 1920), 1176);
        // Desired chunk wins when the path allows more.
        assert_eq!(chunk_payload_len(65535, 8, 1920), 1920);
        // Never exceeds the datagram limit even with a large desired size.
        assert_eq!(chunk_payload_len(100, 8, 1920), 80);
        // Tiny limit still yields whole sample frames.
        assert_eq!(chunk_payload_len(28, 8, 1920), 8);
        // Absurdly small limit yields zero (caller must stop, not under-size).
        assert_eq!(chunk_payload_len(20, 8, 1920), 0);
        assert_eq!(chunk_payload_len(19, 8, 1920), 0);
    }

    #[test]
    fn datagrams_are_frame_aligned_and_within_limit() {
        let channels = DEFAULT_CHANNELS;
        let fb = frame_bytes(channels);
        for mds in [1200usize, 1300, 2000, 9000] {
            let payload = chunk_payload_len(mds, fb, 1920);
            assert!(payload > 0, "mds={mds}");
            let frames = payload / fb;
            let header = MediaHeader { stream_instance_id: 1, config_generation: 1, sequence: 7, timestamp: 0 };
            let dg = encode_datagram(&header, 0, frames, channels);
            assert!(dg.len() <= mds, "datagram {} exceeds queried max_datagram_size {mds}", dg.len());
            let (_, pcm) = MediaHeader::parse(&dg).expect("parse");
            assert_eq!(pcm.len() % (4 * channels as usize), 0, "PCM payload frame-aligned");
        }
    }

    #[test]
    fn datagram_payload_carries_expected_samples() {
        let channels = DEFAULT_CHANNELS;
        let fb = frame_bytes(channels);
        let header = MediaHeader { stream_instance_id: 1, config_generation: 1, sequence: 0, timestamp: 118 };
        let dg = encode_datagram(&header, 118, 3, channels);
        let (_, pcm) = MediaHeader::parse(&dg).expect("parse");
        assert_eq!(pcm.len(), 3 * fb);
        // Interleaved layout: L(p118) R(p118) L(p119) R(p119) L(p120) R(p120).
        for frame in 0..3u64 {
            for chan in 0..channels as usize {
                let off = (frame as usize * fb) + chan * 4;
                let v = f32::from_le_bytes([pcm[off], pcm[off + 1], pcm[off + 2], pcm[off + 3]]);
                let expect = sample_value(118 + frame);
                let diff = v.to_bits().abs_diff(expect.to_bits());
                assert!(diff <= 1, "frame {frame} chan {chan}: {v} vs {expect}");
            }
        }
    }
}
