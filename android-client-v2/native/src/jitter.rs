//! Bounded jitter buffer (M5-03; spec §8).
//!
//! The buffer reorders timely datagrams and gives up on a hole once enough
//! later media has accumulated to exceed the target delay, filling the hole
//! with silence. Playout order is the extended RFC 3550 A.1 sequence (the
//! reliable contiguity signal); the wire `timestamp` is the *capture-clock*
//! position used to measure how long a frame has waited and how many frames
//! a silence gap must cover.
//!
//! TIMESTAMP TRAP (M5-05 read-only analysis): the server packetizer advances
//! `timestamp` only by frames actually sent, and pre-packetize drops leave no
//! sequence hole. A contiguous sequence with a jumped timestamp is therefore
//! a server-side drop, NOT a wire gap: we re-base on the actual timestamp and
//! insert no silence. Silence is inserted only for a *sequence* hole whose
//! size is derived from the timestamp delta.

use std::collections::BTreeMap;

/// §8 / D12 normative jitter-buffer bounds (Q1 unresolved: use the spec value
/// until a maintainer rules otherwise with new evidence).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JitterConfig {
    /// Target playout delay in milliseconds (D12: 60 ms).
    pub target_ms: u32,
    /// Hard upper bound in milliseconds (§8: 200 ms).
    pub hard_limit_ms: u32,
}

impl Default for JitterConfig {
    fn default() -> Self {
        Self {
            target_ms: 60,
            hard_limit_ms: 200,
        }
    }
}

impl JitterConfig {
    /// Target delay expressed as a frame count at `sample_rate`.
    pub fn target_frames(self, sample_rate: u32) -> u64 {
        u64::from(self.target_ms) * u64::from(sample_rate) / 1_000
    }

    /// Hard-limit delay expressed as a frame count at `sample_rate`.
    pub fn hard_limit_frames(self, sample_rate: u32) -> u64 {
        u64::from(self.hard_limit_ms) * u64::from(sample_rate) / 1_000
    }
}

/// The four §8 concealment interventions. `Drift` and `ReAnchor` are reserved
/// for M5-04; this module emits only `Gap` but the counter API accepts all
/// four so M5-04 can increment its interventions without a schema change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConcealmentKind {
    Gap,
    Underrun,
    Drift,
    ReAnchor,
}

struct PendingFrame {
    timestamp: u64,
    frames: u64,
    payload: Vec<u8>,
}

/// What one submitted datagram did to the playout frontier.
#[derive(Debug, Default)]
pub struct SubmitResult {
    /// The datagram's slot had already been committed: a late drop, not a
    /// network loss and not a reorder.
    pub late: bool,
    /// A committed contiguous frame carried a jumped wire timestamp (the
    /// M5-05 server-side drop). The playout clock was re-based on the actual
    /// timestamp, so the capture-clock position stepped without a matching
    /// playback step. M5-04 resets its drift window on this signal.
    pub rebased: bool,
    /// Silence-fill interventions: (kind, signed frame delta).
    pub concealments: Vec<(ConcealmentKind, i64)>,
}

/// The playout buffer. `pending` is keyed by extended sequence so a hole is
/// unambiguous; `frontier_ts` is the expected capture-clock position of the
/// next frame to commit (re-based on every committed frame).
pub struct JitterBuffer {
    config: JitterConfig,
    sample_rate: u32,
    frame_bytes: usize,
    pending: BTreeMap<u64, PendingFrame>,
    frontier: Option<u64>,
    frontier_ts: u64,
    // RFC 3550 A.8 interarrival jitter, in frame units.
    jitter_frames: f64,
    last_arrival: Option<(u64, u64)>, // (arrival position in frames, timestamp)
}

impl JitterBuffer {
    pub fn new(config: JitterConfig, sample_rate: u32, frame_bytes: usize) -> Self {
        Self {
            config,
            sample_rate,
            frame_bytes,
            pending: BTreeMap::new(),
            frontier: None,
            frontier_ts: 0,
            jitter_frames: 0.0,
            last_arrival: None,
        }
    }

    /// RFC 3550 A.8 interarrival J, updated once per accepted datagram in
    /// arrival order. `now_ms` is the injected session clock; the difference
    /// is computed in frame units at the published sample rate.
    pub fn observe_arrival(&mut self, now_ms: u64, timestamp: u64) {
        let arrival = (u128::from(now_ms) * u128::from(self.sample_rate) / 1_000) as u64;
        if let Some((last_arrival, last_timestamp)) = self.last_arrival {
            let d = (timestamp as i128 - last_timestamp as i128)
                - (arrival as i128 - last_arrival as i128);
            let abs = d.unsigned_abs() as f64;
            self.jitter_frames += (abs - self.jitter_frames) / 16.0;
        }
        self.last_arrival = Some((arrival, timestamp));
    }

    /// The A.8 estimate converted to microseconds at the published rate.
    pub fn jitter_microseconds(&self) -> u32 {
        let micros = self.jitter_frames * 1_000_000.0 / f64::from(self.sample_rate);
        if !micros.is_finite() || micros <= 0.0 {
            return 0;
        }
        micros.min(u32::MAX as f64) as u32
    }

    /// Submit one newly observed, non-duplicate datagram. `write` appends the
    /// given bytes to the ring in playout order (real payload or all-zero
    /// silence). Returns whether this was a late drop and any silence fills.
    pub fn submit(
        &mut self,
        extended: u64,
        timestamp: u64,
        frames: u64,
        payload: &[u8],
        write: &mut dyn FnMut(&[u8]),
    ) -> SubmitResult {
        let Some(frontier) = self.frontier else {
            // First datagram anchors the frontier and re-bases the clock.
            self.frontier = Some(extended + 1);
            self.frontier_ts = timestamp.saturating_add(frames);
            write(payload);
            return SubmitResult::default();
        };
        if extended < frontier {
            // Its playout slot has already passed: late drop (§8), never a
            // network loss and never ring data.
            return SubmitResult {
                late: true,
                rebased: false,
                concealments: Vec::new(),
            };
        }
        self.pending.insert(
            extended,
            PendingFrame {
                timestamp,
                frames,
                payload: payload.to_vec(),
            },
        );
        self.drain(write)
    }

    /// Commit everything contiguous at the frontier; if the frontier stays
    /// missing but enough later media has queued (≥ target), fill the hole with
    /// silence and leap the frontier forward.
    fn drain(&mut self, write: &mut dyn FnMut(&[u8])) -> SubmitResult {
        let mut result = SubmitResult::default();
        let target = self.config.target_frames(self.sample_rate).max(1);
        let hard_limit = self.config.hard_limit_frames(self.sample_rate).max(target);
        loop {
            let Some(frontier) = self.frontier else {
                break;
            };
            if let Some(frame) = self.pending.remove(&frontier) {
                // Re-base on the actual timestamp: a contiguous sequence with
                // a jumped timestamp is a server-side drop (M5-05), not a gap.
                if frame.timestamp != self.frontier_ts {
                    result.rebased = true;
                }
                write(&frame.payload);
                self.frontier = Some(frontier + 1);
                self.frontier_ts = frame.timestamp.saturating_add(frame.frames);
                continue;
            }
            let Some((&earliest_seq, earliest)) = self.pending.iter().next() else {
                break;
            };
            let Some((_, latest)) = self.pending.iter().next_back() else {
                break;
            };
            let span = latest
                .timestamp
                .saturating_add(latest.frames)
                .saturating_sub(self.frontier_ts);
            if span < target {
                break; // still within the target delay: wait for the hole
            }
            // Give up on the hole. Fill from the expected position up to the
            // earliest queued frame, clamped to the §8 hard limit so a hostile
            // timestamp cannot allocate an unbounded buffer.
            let fill = earliest
                .timestamp
                .saturating_sub(self.frontier_ts)
                .min(hard_limit);
            if fill > 0 {
                let zeros = vec![0u8; (fill as usize).saturating_mul(self.frame_bytes)];
                write(&zeros);
                result
                    .concealments
                    .push((ConcealmentKind::Gap, fill as i64));
            }
            self.frontier = Some(earliest_seq);
            self.frontier_ts = earliest.timestamp;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_normative_and_frames_scale_with_rate() {
        let cfg = JitterConfig::default();
        assert_eq!(cfg.target_ms, 60);
        assert_eq!(cfg.hard_limit_ms, 200);
        assert_eq!(cfg.target_frames(48_000), 2_880);
        assert_eq!(cfg.hard_limit_frames(48_000), 9_600);
    }

    #[test]
    fn contiguous_frames_commit_immediately_without_silence() {
        let mut jb = JitterBuffer::new(JitterConfig::default(), 48_000, 8);
        let mut written = Vec::new();
        for seq in 0..5u64 {
            let r = jb.submit(seq, seq * 64, 64, &[0x7f; 512], &mut |b| written.extend_from_slice(b));
            assert!(!r.late);
            assert!(r.concealments.is_empty());
        }
        assert_eq!(written.len(), 5 * 512);
    }

    #[test]
    fn a_timely_reorder_needs_no_concealment() {
        let mut jb = JitterBuffer::new(JitterConfig::default(), 48_000, 8);
        let mut written = Vec::new();
        jb.submit(0, 0, 64, &[1; 512], &mut |b| written.extend_from_slice(b));
        jb.submit(2, 128, 64, &[3; 512], &mut |b| written.extend_from_slice(b));
        // seq 1 arrives before the target delay elapses → fills the hole.
        jb.submit(1, 64, 64, &[2; 512], &mut |b| written.extend_from_slice(b));
        assert_eq!(written.len(), 3 * 512);
        assert!(written.chunks(512).all(|c| c.iter().all(|&x| x != 0)));
    }

    #[test]
    fn a_late_frame_is_reported_not_written() {
        let mut jb = JitterBuffer::new(JitterConfig::default(), 48_000, 8);
        let mut written = Vec::new();
        for seq in 0..50u64 {
            jb.submit(seq, seq * 64, 64, &[0x7f; 512], &mut |b| written.extend_from_slice(b));
        }
        let r = jb.submit(1, 64, 64, &[0x7f; 512], &mut |b| written.extend_from_slice(b));
        assert!(r.late);
    }

    #[test]
    fn a_contiguous_timestamp_jump_is_rebased_without_silence() {
        let mut jb = JitterBuffer::new(JitterConfig::default(), 48_000, 8);
        let mut written = Vec::new();
        jb.submit(0, 0, 64, &[1; 512], &mut |b| written.extend_from_slice(b));
        // Server dropped frames: sequence contiguous but timestamp jumped by
        // 1000 frames. Must NOT insert silence (M5-05 timestamp offset).
        let r = jb.submit(1, 1064, 64, &[2; 512], &mut |b| written.extend_from_slice(b));
        assert!(r.concealments.is_empty(), "no silence for a server-side drop");
        assert_eq!(written.len(), 2 * 512);
    }
}
