//! Media-plane datagram header + deterministic-media verification.
//!
//! Header shape fixed by the prototype brief (20 bytes, all BE):
//! `u32 stream_instance_id | u32 config_generation | u32 sequence | u64 timestamp`
//! Payload: raw f32le interleaved PCM, frame-aligned.
//!
//! Deterministic content (prototype convention, shared with the Rust server):
//! the sample value at absolute sample-frame position `p` is
//! `sin(2*pi*440*p/48000) as f32`, identical for every channel of frame `p`.
//! `timestamp` in the header is the sample-frame position of the first PCM
//! frame in the payload, on the capture clock. Bit-exactness is asserted on
//! the host (x86_64 == x86_64); cross-architecture libm `sin` is a known
//! prototype risk recorded in BUILD.md.
//!
//! This module is host-testable: gaps, duplicates, reorder, byte verification.

/// u64 timestamps in the header; seq is u32.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaHeader {
    pub stream_instance_id: u32,
    pub config_generation: u32,
    pub sequence: u32,
    pub timestamp: u64,
}

pub const MEDIA_HEADER_LEN: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderError {
    TooShort(usize),
}

impl std::fmt::Display for HeaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HeaderError::TooShort(n) => write!(f, "media datagram {} bytes < header {MEDIA_HEADER_LEN}", n),
        }
    }
}

impl MediaHeader {
    pub fn parse(bytes: &[u8]) -> Result<Self, HeaderError> {
        if bytes.len() < MEDIA_HEADER_LEN {
            return Err(HeaderError::TooShort(bytes.len()));
        }
        Ok(MediaHeader {
            stream_instance_id: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            config_generation: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            sequence: u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            timestamp: u64::from_be_bytes([
                bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17], bytes[18],
                bytes[19],
            ]),
        })
    }

    pub fn encode(&self) -> [u8; MEDIA_HEADER_LEN] {
        let mut out = [0u8; MEDIA_HEADER_LEN];
        out[0..4].copy_from_slice(&self.stream_instance_id.to_be_bytes());
        out[4..8].copy_from_slice(&self.config_generation.to_be_bytes());
        out[8..12].copy_from_slice(&self.sequence.to_be_bytes());
        out[12..20].copy_from_slice(&self.timestamp.to_be_bytes());
        out
    }
}

/// Deterministic test sample: identical for every channel of frame `p`.
#[inline]
pub fn sample_at(p: u64) -> f32 {
    (2.0 * std::f64::consts::PI * 440.0 * p as f64 / 48000.0) as f32
}

/// Render the deterministic payload a server would send for frames [p0, p0+n).
pub fn render_payload(p0: u64, n_frames: u32, channels: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(n_frames as usize * 4 * channels as usize);
    for i in 0..u64::from(n_frames) {
        let s = sample_at(p0 + i).to_le_bytes();
        for _ in 0..channels {
            out.extend_from_slice(&s);
        }
    }
    out
}

/// What happened to one observed datagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObserveKind {
    /// In order or filling forward; `gap` = sequence holes seen before this datagram.
    Ok { gap: u32 },
    /// Sequence already seen inside the reorder window. Payload not forwarded.
    Duplicate,
    /// Late arrival below the high-water mark but not seen before. Payload forwarded.
    Reordered { gap: u32 },
    /// New stream_instance_id: seq accounting resets. Payload forwarded.
    InstanceChange { gap: u32 },
    /// Bytes did not match the deterministic waveform. Payload forwarded anyway
    /// (the counter is the evidence; first mismatch position recorded).
    ByteMismatch { gap: u32, first_mismatch_frame: u64 },
    /// Payload length is not frame-aligned for the configured channel count.
    MalformedPayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserveVerdict {
    pub forward: bool,
    pub kind: ObserveKind,
}

/// Sequence/gap/dup accounting + deterministic byte verification for one session.
pub struct MediaTracker {
    pub config: crate::control::MediaConfig,
    latest: Option<u32>,
    /// Exact dedup over all u32 seqs: bit `seq % REORDER_WINDOW` is valid only
    /// when its word's lap tag equals `seq / REORDER_WINDOW`. 16 KiB.
    seen_lap: Vec<u32>,
    seen_bits: Vec<u64>,
    pub datagrams_observed: u64,
    pub gaps: u64,
    pub duplicates: u64,
    pub reordered: u64,
    pub instance_changes: u64,
    pub byte_errors: u64,
    pub malformed_payloads: u64,
    pub config_mismatches: u64,
    pub forwarded_bytes: u64,
    pub forwarded_frames: u64,
}

/// Dedup window in bits (2 KiB of bits + lap tags).
const REORDER_WINDOW: u64 = 8192;

impl MediaTracker {
    pub fn new(config: crate::control::MediaConfig) -> Self {
        let words = (REORDER_WINDOW / 64) as usize;
        MediaTracker {
            config,
            latest: None,
            seen_lap: vec![u32::MAX; words],
            seen_bits: vec![0u64; words],
            datagrams_observed: 0,
            gaps: 0,
            duplicates: 0,
            reordered: 0,
            instance_changes: 0,
            byte_errors: 0,
            malformed_payloads: 0,
            config_mismatches: 0,
            forwarded_bytes: 0,
            forwarded_frames: 0,
        }
    }

    /// Called when MEDIA_START reconfigures the stream (new instance/gen).
    pub fn reconfigure(&mut self, config: crate::control::MediaConfig) {
        self.config = config;
        self.latest = None;
        self.seen_lap.iter_mut().for_each(|w| *w = u32::MAX);
        self.seen_bits.iter_mut().for_each(|w| *w = 0);
    }

    fn mark_seen(&mut self, seq: u32) {
        let idx = u64::from(seq) % REORDER_WINDOW;
        let w = (idx / 64) as usize;
        let lap = (u64::from(seq) / REORDER_WINDOW) as u32;
        if self.seen_lap[w] != lap {
            self.seen_lap[w] = lap;
            self.seen_bits[w] = 0;
        }
        self.seen_bits[w] |= 1u64 << (idx % 64);
    }

    fn was_seen(&self, seq: u32) -> bool {
        let idx = u64::from(seq) % REORDER_WINDOW;
        let w = (idx / 64) as usize;
        let lap = (u64::from(seq) / REORDER_WINDOW) as u32;
        self.seen_lap[w] == lap && (self.seen_bits[w] >> (idx % 64)) & 1 == 1
    }

    fn reset_seq_accounting(&mut self) {
        self.latest = None;
        self.seen_lap.iter_mut().for_each(|w| *w = u32::MAX);
        self.seen_bits.iter_mut().for_each(|w| *w = 0);
    }

    /// Observe one datagram: header + payload bytes (without the 20-byte header).
    pub fn observe(&mut self, header: MediaHeader, payload: &[u8]) -> ObserveVerdict {
        self.datagrams_observed += 1;

        if header.config_generation != self.config.config_generation {
            self.config_mismatches += 1;
        }

        let frame_bytes = self.config.frame_bytes();
        if payload.len() % frame_bytes != 0 {
            self.malformed_payloads += 1;
            return ObserveVerdict {
                forward: false,
                kind: ObserveKind::MalformedPayload,
            };
        }

        let is_new_instance = header.stream_instance_id != self.config.stream_instance_id;
        if is_new_instance {
            self.instance_changes += 1;
            // adopt the new instance id so subsequent datagrams of the same run
            // count normally; MEDIA_START reconfigure() updates the rest (gen, rate…)
            self.config.stream_instance_id = header.stream_instance_id;
            self.reset_seq_accounting();
        }

        let gap = match self.latest {
            None => 0,
            Some(l) if header.sequence > l => header.sequence - l - 1,
            Some(_) => 0,
        };

        let (forward, kind) = match self.latest {
            None => {
                let kind = if is_new_instance {
                    ObserveKind::InstanceChange { gap: 0 }
                } else {
                    ObserveKind::Ok { gap: 0 }
                };
                (true, kind)
            }
            Some(l) if header.sequence <= l => {
                if self.was_seen(header.sequence) {
                    self.duplicates += 1;
                    (false, ObserveKind::Duplicate)
                } else {
                    self.reordered += 1;
                    (true, ObserveKind::Reordered { gap })
                }
            }
            Some(_) => {
                self.gaps += u64::from(gap);
                (true, ObserveKind::Ok { gap })
            }
        };

        if self.latest.is_none() || header.sequence > self.latest.unwrap() {
            self.latest = Some(header.sequence);
        }
        self.mark_seen(header.sequence);

        if forward {
            // deterministic byte verification
            let n_frames = (payload.len() / frame_bytes) as u64;
            let mut mismatch_frame: Option<u64> = None;
            'outer: for i in 0..n_frames {
                let expect = sample_at(header.timestamp + i).to_le_bytes();
                let off = (i * frame_bytes as u64) as usize;
                for c in 0..self.config.channels as usize {
                    let got = &payload[off + c * 4..off + c * 4 + 4];
                    if got != expect {
                        mismatch_frame = Some(header.timestamp + i);
                        break 'outer;
                    }
                }
            }
            if let Some(frame) = mismatch_frame {
                self.byte_errors += 1;
                return ObserveVerdict {
                    forward: true,
                    kind: ObserveKind::ByteMismatch {
                        gap,
                        first_mismatch_frame: frame,
                    },
                };
            }
            self.forwarded_bytes += payload.len() as u64;
            self.forwarded_frames += n_frames;
        }

        ObserveVerdict { forward, kind }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::MediaConfig;

    fn config(instance: u32, gen: u32) -> crate::control::MediaConfig {
        MediaConfig {
            stream_instance_id: instance,
            config_generation: gen,
            sample_rate: 48000,
            channels: 2,
            sample_format: 1,
        }
    }

    fn hdr(instance: u32, gen: u32, seq: u32, ts: u64) -> MediaHeader {
        MediaHeader {
            stream_instance_id: instance,
            config_generation: gen,
            sequence: seq,
            timestamp: ts,
        }
    }

    #[test]
    fn header_roundtrip_be() {
        let h = hdr(0x11223344, 0x55667788, 0x99aabbcc, 0x1122334455667788);
        let bytes = h.encode();
        assert_eq!(bytes.len(), 20);
        assert_eq!(MediaHeader::parse(&bytes).unwrap(), h);
        // explicit BE layout check
        assert_eq!(&bytes[0..4], &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(&bytes[8..12], &[0x99, 0xaa, 0xbb, 0xcc]);
        assert_eq!(&bytes[12..20], &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
    }

    #[test]
    fn header_too_short_rejected() {
        assert_eq!(
            MediaHeader::parse(&[0u8; 19]),
            Err(HeaderError::TooShort(19))
        );
    }

    #[test]
    fn in_order_no_gaps() {
        let mut t = MediaTracker::new(config(1, 1));
        for seq in 0..100u32 {
            let payload = render_payload(u64::from(seq) * 240, 240, 2);
            let v = t.observe(hdr(1, 1, seq, u64::from(seq) * 240), &payload);
            assert_eq!(v.kind, ObserveKind::Ok { gap: 0 });
            assert!(v.forward);
        }
        assert_eq!(t.gaps, 0);
        assert_eq!(t.duplicates, 0);
        assert_eq!(t.reordered, 0);
        assert_eq!(t.byte_errors, 0);
        assert_eq!(t.forwarded_bytes, 100 * 240 * 8);
        assert_eq!(t.forwarded_frames, 100 * 240);
    }

    #[test]
    fn gap_counted_on_arrival() {
        let mut t = MediaTracker::new(config(1, 1));
        // 240 frames per datagram
        let mk = |seq: u32| (hdr(1, 1, seq, u64::from(seq) * 240), render_payload(u64::from(seq) * 240, 240, 2));
        let (h0, p0) = mk(0);
        t.observe(h0, &p0);
        // seq 1,2,3 lost; 4 arrives
        let (h4, p4) = mk(4);
        let v = t.observe(h4, &p4);
        assert_eq!(v.kind, ObserveKind::Ok { gap: 3 });
        assert_eq!(t.gaps, 3);
        // late seq 2 arrives (would normally be reordered)
        let (h2, p2) = mk(2);
        let v = t.observe(h2, &p2);
        assert_eq!(v.kind, ObserveKind::Reordered { gap: 0 });
        assert!(v.forward);
        assert_eq!(t.gaps, 3, "gap counter is arrival-time evidence, not adjusted on late fill");
        assert_eq!(t.reordered, 1);
    }

    #[test]
    fn duplicate_detected_and_not_forwarded() {
        let mut t = MediaTracker::new(config(1, 1));
        let payload = render_payload(0, 240, 2);
        t.observe(hdr(1, 1, 0, 0), &payload);
        let v = t.observe(hdr(1, 1, 0, 0), &payload);
        assert_eq!(v.kind, ObserveKind::Duplicate);
        assert!(!v.forward);
        assert_eq!(t.duplicates, 1);
        assert_eq!(t.forwarded_bytes, payload.len() as u64, "dup payload not double-counted");
    }

    #[test]
    fn old_seq_below_window_is_duplicate() {
        let mut t = MediaTracker::new(config(1, 1));
        let mk = |seq: u32| (hdr(1, 1, seq, u64::from(seq) * 240), render_payload(u64::from(seq) * 240, 240, 2));
        for seq in 0..3u32 {
            let (h, p) = mk(seq);
            t.observe(h, &p);
        }
        let (h1, p1) = mk(1);
        let v = t.observe(h1, &p1);
        assert_eq!(v.kind, ObserveKind::Duplicate);
        assert!(!v.forward);
    }

    #[test]
    fn reorder_within_window_forwarded() {
        let mut t = MediaTracker::new(config(1, 1));
        let mk = |seq: u32| (hdr(1, 1, seq, u64::from(seq) * 240), render_payload(u64::from(seq) * 240, 240, 2));
        for seq in [0u32, 2] {
            let (h, p) = mk(seq);
            t.observe(h, &p);
        }
        let (h1, p1) = mk(1);
        let v = t.observe(h1, &p1);
        assert_eq!(v.kind, ObserveKind::Reordered { gap: 0 });
        assert!(v.forward);
        assert_eq!(t.reordered, 1);
    }

    #[test]
    fn instance_change_resets_sequence_accounting() {
        let mut t = MediaTracker::new(config(1, 1));
        let mk = |inst: u32, seq: u32| {
            (
                hdr(inst, 1, seq, u64::from(seq) * 240),
                render_payload(u64::from(seq) * 240, 240, 2),
            )
        };
        // run 1 at seq 50
        let (h, p) = mk(1, 50);
        t.observe(h, &p);
        // capture restart: instance 2 starts at seq 0
        let (h, p) = mk(2, 0);
        let v = t.observe(h, &p);
        assert_eq!(v.kind, ObserveKind::InstanceChange { gap: 0 });
        assert!(v.forward);
        assert_eq!(t.instance_changes, 1);
        assert_eq!(t.gaps, 0, "no phantom gap across instance restart");
        // and it keeps counting inside instance 2
        let (h, p) = mk(2, 1);
        let v = t.observe(h, &p);
        assert_eq!(v.kind, ObserveKind::Ok { gap: 0 });
    }

    #[test]
    fn deterministic_payload_verifies() {
        let mut t = MediaTracker::new(config(1, 1));
        // nontrivial offset so timestamp arithmetic is exercised
        let p0 = 48_000 + 17;
        let payload = render_payload(p0, 240, 2);
        let v = t.observe(hdr(1, 1, 0, p0), &payload);
        assert_eq!(v.kind, ObserveKind::Ok { gap: 0 });
        assert_eq!(t.byte_errors, 0);
        assert_eq!(t.forwarded_bytes, payload.len() as u64);
        // channels carry the same value (prototype convention)
        let s0 = sample_at(p0).to_le_bytes();
        assert_eq!(&payload[0..4], &s0);
        assert_eq!(&payload[4..8], &s0);
    }

    #[test]
    fn byte_mismatch_detected_and_counted() {
        let mut t = MediaTracker::new(config(1, 1));
        let mut payload = render_payload(1000, 240, 2);
        // flip byte 500 => frame 1000 + 500/8 = 1062, channel 0 (byte 500 % 8 = 4 -> channel 0, byte 0 of the sample)
        payload[500] ^= 0xff;
        let v = t.observe(hdr(1, 1, 0, 1000), &payload);
        let expect_frame = 1000u64 + 500u64 / 8;
        assert!(matches!(
            v.kind,
            ObserveKind::ByteMismatch { gap: 0, .. }
        ), "expected ByteMismatch, got {:?}", v.kind);
        if let ObserveKind::ByteMismatch { first_mismatch_frame, .. } = v.kind {
            assert_eq!(first_mismatch_frame, expect_frame);
        }
        assert_eq!(t.byte_errors, 1);
        assert!(v.forward, "corrupt payload still forwarded; counter is the evidence");
    }

    #[test]
    fn misaligned_payload_rejected() {
        let mut t = MediaTracker::new(config(1, 1));
        let payload = vec![0u8; 7]; // 7 % 8 != 0 for stereo f32
        let v = t.observe(hdr(1, 1, 0, 0), &payload);
        assert_eq!(v.kind, ObserveKind::MalformedPayload);
        assert!(!v.forward);
        assert_eq!(t.malformed_payloads, 1);
    }

    #[test]
    fn config_generation_mismatch_counted() {
        let mut t = MediaTracker::new(config(1, 1));
        let payload = render_payload(0, 240, 2);
        let v = t.observe(hdr(1, 9, 0, 0), &payload);
        assert_eq!(v.kind, ObserveKind::Ok { gap: 0 });
        assert_eq!(t.config_mismatches, 1);
    }

    #[test]
    fn seq_window_slide_keeps_dup_detection() {
        let mut t = MediaTracker::new(config(1, 1));
        let mk = |seq: u32| (hdr(1, 1, seq, u64::from(seq) * 240), render_payload(u64::from(seq) * 240, 240, 2));
        for seq in 0..10_000u32 {
            let (h, p) = mk(seq);
            t.observe(h, &p);
        }
        // a duplicate from far back still identified as dup, not reorder
        let (h, p) = mk(9_000);
        let v = t.observe(h, &p);
        assert_eq!(v.kind, ObserveKind::Duplicate);
    }
}
