//! Shared-memory media ring buffer (ticket 08; the prototype's validated
//! skeleton carried over essentially unchanged — the media path stays
//! SKELETON-ONLY in this ticket, not activated).
//!
//! Native writes PCM payloads into memory Kotlin owns (a direct ByteBuffer
//! attached once over JNI); Kotlin reads at its own pace using the monotonic
//! `write_pos` cursor. Media never crosses JNI per datagram.
//!
//! Single-producer overwrite ring: when a write would lap an unknown reader,
//! oldest data is overwritten; the reader detects it because
//! `write_pos - local_read_pos > capacity` and resyncs to `write_pos`. Native
//! never blocks and never knows the read cursor.

pub struct MediaRing {
    /// Base address of the writable region (Kotlin's direct ByteBuffer).
    base: *mut u8,
    pub capacity: usize,
    /// Monotonic total bytes ever written.
    write_pos: u64,
    pub writes: u64,
    pub oversize_drops: u64,
    /// Payload bytes dropped because a chunk exceeded the whole ring.
    pub dropped_bytes: u64,
}

// The ring is used from one native media task only (JNI attach happens on the
// JNI thread before the media task starts; guarded by the session mutex).
unsafe impl Send for MediaRing {}

impl MediaRing {
    /// # Safety
    /// `base` must point to `capacity` writable bytes owned by the attached
    /// ByteBuffer for the lifetime of this ring. Callers (session.rs) take
    /// the pointer via `env.get_direct_buffer_address`.
    pub unsafe fn attach(base: *mut u8, capacity: usize) -> Self {
        debug_assert!(!base.is_null());
        debug_assert!(capacity > 0);
        MediaRing {
            base,
            capacity,
            write_pos: 0,
            writes: 0,
            oversize_drops: 0,
            dropped_bytes: 0,
        }
    }

    pub fn write_pos(&self) -> u64 {
        self.write_pos
    }

    pub fn write_offset(&self) -> u64 {
        self.write_pos % self.capacity as u64
    }

    /// Append payload at the write cursor, wrapping. A chunk larger than the
    /// whole ring is dropped entirely (can never be read coherently).
    pub fn write(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() > self.capacity {
            self.oversize_drops += 1;
            self.dropped_bytes += bytes.len() as u64;
            return false;
        }
        if bytes.is_empty() {
            return true;
        }
        unsafe {
            let cap = self.capacity;
            let mut off = (self.write_pos % cap as u64) as usize;
            let mut rest = bytes;
            // first segment up to the wrap point
            let first = rest.len().min(cap - off);
            std::ptr::copy_nonoverlapping(rest.as_ptr(), self.base.add(off), first);
            rest = &rest[first..];
            off = 0;
            if !rest.is_empty() {
                std::ptr::copy_nonoverlapping(rest.as_ptr(), self.base.add(off), rest.len());
            }
        }
        self.write_pos += bytes.len() as u64;
        self.writes += 1;
        true
    }

    /// Read a contiguous run of bytes for tests/inspection: copies from
    /// (offset % capacity). Not used on the media path (Kotlin reads memory
    /// directly); used by host tests to verify layout.
    #[cfg(test)]
    fn read_for_test(&self, start: u64, len: usize) -> Vec<u8> {
        debug_assert!(len <= self.capacity);
        debug_assert!(
            start + len as u64 <= self.write_pos,
            "reading unwritten data"
        );
        let mut out = vec![0u8; len];
        unsafe {
            let cap = self.capacity;
            let mut off = (start % cap as u64) as usize;
            let mut i = 0;
            while i < len {
                let run = (cap - off).min(len - i);
                out[i..i + run]
                    .copy_from_slice(std::slice::from_raw_parts(self.base.add(off), run));
                i += run;
                off = (off + run) % cap;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_ring(cap: usize, f: impl FnOnce(&mut Vec<u8>, MediaRing)) {
        let mut backing = vec![0u8; cap];
        let ptr = backing.as_mut_ptr();
        let ring = unsafe { MediaRing::attach(ptr, cap) };
        f(&mut backing, ring);
    }

    #[test]
    fn in_order_appends_are_contiguous() {
        with_ring(64, |backing, mut ring| {
            assert!(ring.write(&[1, 2, 3]));
            assert!(ring.write(&[4, 5]));
            assert_eq!(ring.write_pos(), 5);
            assert_eq!(ring.write_offset(), 5);
            assert_eq!(&backing[..5], &[1, 2, 3, 4, 5]);
        });
    }

    #[test]
    fn wraparound_writes_both_segments() {
        with_ring(8, |backing, mut ring| {
            // fill to offset 6, then a 5-byte write must split 2+3
            assert!(ring.write(&[0xAA; 6]));
            assert!(ring.write(&[1, 2, 3, 4, 5]));
            assert_eq!(ring.write_pos(), 11);
            assert_eq!(ring.write_offset(), 3);
            // bytes 0..3 hold the wrapped tail; 6..8 the first segment; 3..6 untouched 0xAA
            assert_eq!(&backing[0..3], &[3, 4, 5]);
            assert_eq!(&backing[3..8], &[0xAA, 0xAA, 0xAA, 1, 2]);
        });
    }

    #[test]
    fn reader_detects_lapping_via_cursor_arithmetic() {
        // capacity 8; reader saw 4 bytes; writer pushes 10 → write_pos=14;
        // reader lag 14-4=10 > capacity ⇒ must resync to write_pos.
        with_ring(8, |_, mut ring| {
            let mut read_pos = 0u64;
            assert!(ring.write(&[0u8; 4]));
            read_pos += 4;
            // chunks never exceed capacity (oversize would be dropped instead)
            assert!(ring.write(&[1u8; 8]));
            assert!(ring.write(&[2u8; 4]));
            let lag = ring.write_pos() - read_pos;
            assert!(lag > ring.capacity as u64, "reader lapped: {lag}");
            // resync: reader jumps to the writer cursor
            read_pos = ring.write_pos();
            assert_eq!(read_pos, 16);
        });
    }

    #[test]
    fn oversize_chunk_dropped_entirely() {
        with_ring(8, |_, mut ring| {
            assert!(!ring.write(&[0u8; 9]));
            assert_eq!(ring.oversize_drops, 1);
            assert_eq!(ring.dropped_bytes, 9);
            assert_eq!(ring.write_pos(), 0, "nothing written for oversize chunk");
        });
        with_ring(8, |_, mut ring| {
            assert!(ring.write(&[0u8; 8]), "chunk exactly at capacity fits");
            assert_eq!(ring.write_pos(), 8);
        });
    }

    #[test]
    fn empty_write_is_noop_success() {
        with_ring(8, |_, mut ring| {
            assert!(ring.write(&[]));
            assert_eq!(ring.write_pos(), 0);
            assert_eq!(ring.writes, 0);
        });
    }

    #[test]
    fn long_stream_keeps_cursor_consistent() {
        with_ring(1000, |_, mut ring| {
            let mut expect_pos = 0u64;
            for i in 0..10_000u32 {
                let chunk = (i % 371 + 1) as usize;
                let data = vec![(i % 251) as u8; chunk];
                assert!(ring.write(&data));
                expect_pos += chunk as u64;
                assert_eq!(ring.write_pos(), expect_pos);
                assert_eq!(
                    ring.write_offset(),
                    expect_pos % 1000,
                    "cursor drift at iteration {i}"
                );
            }
        });
    }

    #[test]
    fn read_for_test_roundtrips_across_wrap() {
        with_ring(16, |_, mut ring| {
            let payload: Vec<u8> = (0..40u8).collect();
            // stream in chunks of at most the ring capacity
            for chunk in payload.chunks(16) {
                assert!(ring.write(chunk));
            }
            assert_eq!(ring.write_pos(), 40);
            // last 16 bytes are fully readable, in ring order
            let got = ring.read_for_test(24, 16);
            assert_eq!(got, payload[24..40]);
        });
    }
}
