//! Media plane receiver rules (spec §7) — header parse (§7.1 D4, 24-byte BE),
//! receiver state accounting (§7.5), and the unknown-config hold rule (§7.6
//! constants). Fixture-driven: `fixtures/wire-protocol-v2/media/*.json` pins
//! every header field and the reserved-flag ignore rule at byte level.
//!
//! The media path itself is NOT activated in this ticket (Tracer-Bullet
//! step 4); this module is the host-tested receiver half the session will
//! wire to the datagram loop when media is activated.

pub const HEADER_SIZE: usize = 24;

/// §7.1 flag bits. Bits 2–31 are reserved: senders MUST zero, receivers MUST
/// ignore (adjudication 2026-11-12) — never malformed.
pub const FLAG_STREAM_START: u32 = 0x1;
pub const FLAG_CONFIG_CHANGE: u32 = 0x2;
pub const FLAG_SPARE: u32 = 0x4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaHeader {
    pub stream_instance_id: u32,
    pub config_generation: u32,
    pub sequence: u32,
    pub timestamp: u64,
    pub flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderError {
    /// Fewer than HEADER_SIZE bytes.
    Truncated,
    /// stream_instance_id == 0 or config_generation == 0 (§7.1: 0 invalid).
    ImpossibleField,
}

impl MediaHeader {
    /// Parse the fixed 24-byte big-endian header (§7.1). Every header integer
    /// is network byte order — unlike protobuf's little-endian fixed64.
    pub fn parse(datagram: &[u8]) -> Result<(MediaHeader, &[u8]), HeaderError> {
        if datagram.len() < HEADER_SIZE {
            return Err(HeaderError::Truncated);
        }
        let b = &datagram[..HEADER_SIZE];
        let h = MediaHeader {
            stream_instance_id: u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            config_generation: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
            sequence: u32::from_be_bytes([b[8], b[9], b[10], b[11]]),
            timestamp: u64::from_be_bytes([b[12], b[13], b[14], b[15], b[16], b[17], b[18], b[19]]),
            flags: u32::from_be_bytes([b[20], b[21], b[22], b[23]]),
        };
        if h.stream_instance_id == 0 || h.config_generation == 0 {
            return Err(HeaderError::ImpossibleField);
        }
        Ok((h, &datagram[HEADER_SIZE..]))
    }

    /// Build a header into `out` (big-endian), for tests and replay tooling.
    pub fn write_be(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.stream_instance_id.to_be_bytes());
        out.extend_from_slice(&self.config_generation.to_be_bytes());
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.extend_from_slice(&self.timestamp.to_be_bytes());
        out.extend_from_slice(&self.flags.to_be_bytes());
    }
}

/// §7.5 per-instance receiver state: RFC 3550 A.1 extended sequence, dedup
/// window, counters. Fixture media.instance-change and media.sequence-wrap
/// pin the reset and cycle-fold behavior.
#[derive(Debug, Default)]
pub struct MediaReceiver {
    /// Current instance (None = no datagram observed yet).
    pub instance: Option<u32>,
    /// RFC 3550 A.1 extended highest sequence (cycles folded in).
    pub extended_highest: u64,
    cycles: u32,
    /// No datagram of the current epoch observed yet (first seeds seq space).
    seeded: bool,
    /// Dedup bitmap over a reorder window; lap-tagged (16 KiB class).
    seen_lap: Vec<u32>,
    seen_bits: Vec<u64>,
    /// Counters (§9 names).
    pub cumulative_received: u64,
    pub cumulative_duplicates: u64,
    pub cumulative_reordered: u64,
    pub cumulative_lost: i64,
    pub max_reorder_distance: u32,
    /// Unknown-config or stale-generation drops (§7.5/§7.6).
    pub unknown_config_drops: u64,
    /// Malformed media (bad alignment, impossible fields) — drop + count.
    pub malformed_drops: u64,
}

/// Reorder/dedup window in bits.
const REORDER_WINDOW: u64 = 8192;

impl MediaReceiver {
    pub fn new() -> Self {
        let words = (REORDER_WINDOW / 64) as usize;
        MediaReceiver {
            seen_lap: vec![u32::MAX; words],
            seen_bits: vec![0u64; words],
            ..MediaReceiver::default()
        }
    }

    /// §7.5: a new instance id begins a new receiver epoch — reset playback
    /// and statistics state (fixture media.instance-change).
    fn begin_epoch(&mut self, instance: u32) {
        self.instance = Some(instance);
        self.extended_highest = 0;
        self.cycles = 0;
        self.seeded = false;
        self.seen_lap.iter_mut().for_each(|w| *w = u32::MAX);
        self.seen_bits.iter_mut().for_each(|w| *w = 0);
        self.cumulative_received = 0;
        self.cumulative_duplicates = 0;
        self.cumulative_reordered = 0;
        self.cumulative_lost = 0;
        self.max_reorder_distance = 0;
    }

    /// Fold a raw u32 sequence into the extended 64-bit sequence space
    /// (RFC 3550 A.1 cycle counting; fixture media.sequence-wrap). The first
    /// observation of an epoch seeds the sequence space — it is neither a gap
    /// nor a reorder, whatever its value.
    fn extend(&mut self, seq: u32) -> u64 {
        if !self.seeded {
            self.seeded = true;
            return u64::from(seq);
        }
        let current_low = (self.extended_highest & 0xffff_ffff) as u32;
        let delta = (seq.wrapping_sub(current_low)) as i32;
        if delta > 0 {
            // forward: maybe cross a cycle
            if current_low > seq {
                self.cycles += 1;
            }
            ((self.cycles as u64) << 32) | u64::from(seq)
        } else if delta < 0 {
            // backwards: seq is below current_low → it belongs to the previous
            // cycle if current_low is small (just wrapped)
            if current_low < 0x1_0000 && seq > 0x8000_0000 {
                return ((self.cycles.saturating_sub(1)) as u64) << 32 | u64::from(seq);
            }
            ((self.cycles as u64) << 32) | u64::from(seq)
        } else {
            self.extended_highest
        }
    }

    /// Observe one datagram's header (payload alignment already checked by
    /// the caller). Returns the verdict; payload handling is upstream.
    pub fn observe(&mut self, header: &MediaHeader) -> ObserveVerdict {
        // §7.5: new instance id → new epoch (STREAM_START/instance field is
        // authoritative)
        if self.instance != Some(header.stream_instance_id) {
            self.begin_epoch(header.stream_instance_id);
        }
        let first_of_epoch = !self.seeded;
        let extended = self.extend(header.sequence);

        // dedup within the window (§7.5: duplicate → drop, count)
        if self.already_seen(header.sequence) {
            self.cumulative_duplicates += 1;
            return ObserveVerdict::Duplicate;
        }
        self.mark_seen(header.sequence);

        if first_of_epoch {
            // §7.5 epoch anchor: the first datagram seeds the sequence space
            // (RFC 3550 A.1 base_seq) — in order, no gap, no reorder.
            self.extended_highest = extended;
            self.cumulative_received += 1;
            return ObserveVerdict::Ok { gap: 0 };
        }

        if extended > self.extended_highest {
            // in-order or a forward jump (loss gap)
            let gap = extended - self.extended_highest - 1;
            self.cumulative_lost += gap as i64;
            self.extended_highest = extended;
            self.cumulative_received += 1;
            ObserveVerdict::Ok { gap: gap as u32 }
        } else {
            // below the high-water mark but not seen before → reordered
            let distance = (self.extended_highest - extended) as u32;
            self.max_reorder_distance = self.max_reorder_distance.max(distance);
            self.cumulative_reordered += 1;
            self.cumulative_received += 1;
            // §9: duplicates may drive lost negative — RFC 3550 A.3 accounting
            self.cumulative_lost -= 1;
            ObserveVerdict::Reordered { distance }
        }
    }

    /// §7.5: payload not frame-aligned → malformed: drop, count locally.
    pub fn check_alignment(
        &mut self,
        payload_len: usize,
        channels: u32,
        sample_bytes: u32,
    ) -> bool {
        let frame_bytes = (sample_bytes * channels) as usize;
        if frame_bytes == 0 || !payload_len.is_multiple_of(frame_bytes) {
            self.malformed_drops += 1;
            return false;
        }
        true
    }

    /// §7.5: generation older than the current one (stale reorder across a
    /// change) → drop; count in unknown_config_drops.
    pub fn note_stale_generation(&mut self) {
        self.unknown_config_drops += 1;
    }

    fn already_seen(&self, seq: u32) -> bool {
        let idx = u64::from(seq) % REORDER_WINDOW;
        let w = (idx / 64) as usize;
        let lap = (u64::from(seq) / REORDER_WINDOW) as u32;
        self.seen_lap[w] == lap && (self.seen_bits[w] >> (idx % 64)) & 1 == 1
    }

    fn mark_seen(&mut self, seq: u32) {
        let idx = u64::from(seq) % REORDER_WINDOW;
        let w = (idx / 64) as usize;
        let lap = (u64::from(seq) / REORDER_WINDOW) as u32;
        self.seen_lap[w] = lap;
        self.seen_bits[w] |= 1 << (idx % 64);
    }
}

/// What happened to one observed datagram (header-level).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserveVerdict {
    /// In order or filling forward; `gap` = sequence holes seen before this
    /// datagram.
    Ok { gap: u32 },
    /// Sequence already seen inside the reorder window. Payload not forwarded.
    Duplicate,
    /// Late arrival below the high-water mark but not seen before.
    Reordered { distance: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    /// Golden vector media.header-happy-path step 1: full header parse of the
    /// fixture datagram_hex, all fields.
    #[test]
    fn parses_fixture_header_happy_path() {
        let d = unhex("000000010000000100000000000000000000000000000001000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f");
        let (h, payload) = MediaHeader::parse(&d).unwrap();
        assert_eq!(h.stream_instance_id, 1);
        assert_eq!(h.config_generation, 1);
        assert_eq!(h.sequence, 0);
        assert_eq!(h.timestamp, 0);
        assert_eq!(h.flags, 0x00000001);
        assert_eq!(h.flags & FLAG_STREAM_START, FLAG_STREAM_START);
        assert_eq!(payload.len(), 64, "payload_bytes 64, frame_bytes 8");
    }

    /// Golden vector media.header-happy-path step 2: sequence 1, timestamp 8,
    /// flags 0 — ts(next) = ts(prev) + frames(prev) continuity (§7.2).
    #[test]
    fn parses_fixture_header_second_datagram() {
        let d = unhex("000000010000000100000001000000000000000800000000404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f");
        let (h, payload) = MediaHeader::parse(&d).unwrap();
        assert_eq!(h.sequence, 1);
        assert_eq!(h.timestamp, 8);
        assert_eq!(h.flags, 0);
        assert_eq!(payload.len(), 64);
    }

    /// Header round-trip: write_be produces the exact fixture bytes.
    #[test]
    fn header_write_be_roundtrips_fixture_bytes() {
        let d = unhex("000000010000000100000000000000000000000000000001000102030405060708090a0b");
        let (h, payload) = MediaHeader::parse(&d).unwrap();
        let mut out = Vec::new();
        h.write_be(&mut out);
        out.extend_from_slice(payload);
        assert_eq!(out, d);
    }

    /// Golden vector media.malformed-drop step 1: stream_instance_id 0 is
    /// invalid (§7.1) — impossible field.
    #[test]
    fn zero_instance_id_is_malformed() {
        let d = unhex("000000000000000100000000000000000000000000000001000102030405060708090a0b");
        assert_eq!(MediaHeader::parse(&d), Err(HeaderError::ImpossibleField));
    }

    /// Golden vector media.malformed-drop step 2: 5-byte payload with
    /// frame_bytes 8 — 5 % 8 != 0, not frame-aligned.
    #[test]
    fn misaligned_payload_is_malformed() {
        let d = unhex("0000000100000001000000010000000000000008000000000001020304");
        let (_h, payload) = MediaHeader::parse(&d).unwrap();
        assert_eq!(payload.len(), 5);
        let mut r = MediaReceiver::new();
        assert!(!r.check_alignment(payload.len(), 2, 4));
        assert_eq!(r.malformed_drops, 1);
    }

    /// Golden vector media.flags-reserved-ignored: bit 2 (0x4 SPARE) set —
    /// processed exactly as if the bits were zero (§7.1/§7.5).
    #[test]
    fn reserved_bit2_is_ignored_not_malformed() {
        let d = unhex("000000010000000100000002000000000000001000000004000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f");
        let (h, payload) = MediaHeader::parse(&d).unwrap();
        assert_eq!(h.flags, 0x00000004);
        let _ = &h;
        assert_eq!(payload.len(), 64);
        // the reserved bits simply carry no meaning; the datagram processes
        let mut r = MediaReceiver::new();
        let v = r.observe(&h);
        assert!(
            matches!(v, ObserveVerdict::Ok { gap: 0 }),
            "first datagram anchors the epoch: no gap accounting precedes it"
        );
    }

    /// Golden vector media.flags-reserved-bit31-ignored: 0x80000000.
    #[test]
    fn reserved_bit31_is_ignored_not_malformed() {
        let d = unhex("000000010000000100000002000000000000001080000000000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f");
        let (h, payload) = MediaHeader::parse(&d).unwrap();
        assert_eq!(h.flags, 0x80000000);
        assert_eq!(payload.len(), 64);
        let mut r = MediaReceiver::new();
        assert!(
            matches!(r.observe(&h), ObserveVerdict::Ok { gap: 0 }),
            "first datagram anchors the epoch: no gap accounting precedes it"
        );
    }

    /// Golden vector media.interop-floor-1176: payload up to 1176 bytes is
    /// accepted (§7.3 interop floor).
    #[test]
    fn interop_floor_1176_accepted() {
        let prefix = unhex("000000090000000100000000000000000000000000000001");
        let mut d = prefix.clone();
        d.extend_from_slice(&vec![0x11u8; 1176]);
        assert_eq!(d.len(), 24 + 1176);
        let (_, payload) = MediaHeader::parse(&d).unwrap();
        assert_eq!(payload.len(), 1176);
        // 1176 / 8 = 147 frames exactly (fixture: payload_frames 147)
        let mut r = MediaReceiver::new();
        assert!(r.check_alignment(payload.len(), 2, 4));
    }

    /// Golden vector media.instance-change: StreamEnded(502)+StreamStarted(2)
    /// on the control stream, then instance 2 seq 0 — receiver epoch resets.
    #[test]
    fn instance_change_resets_receiver_epoch() {
        let mut r = MediaReceiver::new();
        // first instance: 3 datagrams
        for seq in 0..3u32 {
            let h = MediaHeader {
                stream_instance_id: 1,
                config_generation: 1,
                sequence: seq,
                timestamp: u64::from(seq) * 8,
                flags: if seq == 0 { FLAG_STREAM_START } else { 0 },
            };
            let v = r.observe(&h);
            assert!(matches!(v, ObserveVerdict::Ok { gap: 0 }), "seq {seq}");
        }
        assert_eq!(r.cumulative_received, 3);
        // new instance: counters reset, first datagram observed as instance 2
        let h2 = MediaHeader {
            stream_instance_id: 2,
            config_generation: 1,
            sequence: 0,
            timestamp: 0,
            flags: FLAG_STREAM_START,
        };
        let v = r.observe(&h2);
        assert!(matches!(v, ObserveVerdict::Ok { gap: 0 }));
        assert_eq!(r.instance, Some(2));
        assert_eq!(r.cumulative_received, 1, "counters reset on epoch");
        assert_eq!(r.extended_highest, 0);
    }

    /// Golden vector media.sequence-wrap: 0xffffffff → 0 wraps with the cycle
    /// folded into the extended sequence (RFC 3550 A.1).
    #[test]
    fn sequence_wrap_folds_cycle() {
        let mut r = MediaReceiver::new();
        let h1 = MediaHeader {
            stream_instance_id: 3,
            config_generation: 1,
            sequence: 0xffffffff,
            timestamp: 3_840_000,
            flags: 0,
        };
        let v = r.observe(&h1);
        assert!(matches!(v, ObserveVerdict::Ok { gap: 0 }));
        assert_eq!(r.extended_highest, 0xffffffff);
        let h2 = MediaHeader {
            stream_instance_id: 3,
            config_generation: 1,
            sequence: 0,
            timestamp: 3_840_080,
            flags: 0,
        };
        let v = r.observe(&h2);
        assert!(
            matches!(v, ObserveVerdict::Ok { gap: 0 }),
            "wrap is in-order, not reorder"
        );
        assert_eq!(r.extended_highest, 0x1_0000_0000, "cycle folded in");
        assert_eq!(r.cumulative_duplicates, 0);
    }

    /// Duplicate (extended sequence already seen within the window) → drop.
    #[test]
    fn duplicate_drops_and_counts() {
        let mut r = MediaReceiver::new();
        let h = MediaHeader {
            stream_instance_id: 1,
            config_generation: 1,
            sequence: 5,
            timestamp: 40,
            flags: 0,
        };
        assert!(
            matches!(r.observe(&h), ObserveVerdict::Ok { gap: 0 }),
            "first datagram anchors the epoch"
        );
        assert_eq!(r.observe(&h), ObserveVerdict::Duplicate);
        assert_eq!(r.cumulative_duplicates, 1);
    }

    /// Reordered arrival (below high-water, unseen) → reorder accounting.
    #[test]
    fn reordered_counts_and_tracks_distance() {
        let mut r = MediaReceiver::new();
        for seq in [0u32, 1, 2, 3] {
            let h = MediaHeader {
                stream_instance_id: 1,
                config_generation: 1,
                sequence: seq,
                timestamp: u64::from(seq) * 8,
                flags: 0,
            };
            let _ = r.observe(&h);
        }
        // out-of-order arrival of 5 then 4: 5 extends the window, 4 is reordered
        let h5 = MediaHeader {
            stream_instance_id: 1,
            config_generation: 1,
            sequence: 5,
            timestamp: 40,
            flags: 0,
        };
        let _ = r.observe(&h5);
        let h4 = MediaHeader {
            stream_instance_id: 1,
            config_generation: 1,
            sequence: 4,
            timestamp: 32,
            flags: 0,
        };
        let v = r.observe(&h4);
        assert_eq!(v, ObserveVerdict::Reordered { distance: 1 });
        assert_eq!(r.cumulative_reordered, 1);
        assert_eq!(r.max_reorder_distance, 1);
    }

    /// Stale generation (older than current) counts unknown_config_drops
    /// (§7.5: never decoded with the previous generation).
    #[test]
    fn stale_generation_counts_as_unknown_config_drop() {
        let mut r = MediaReceiver::new();
        r.note_stale_generation();
        assert_eq!(r.unknown_config_drops, 1);
    }
}
