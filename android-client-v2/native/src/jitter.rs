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

/// The four §8 concealment interventions. M5-03 emits `Gap` and `Underrun`
/// (underrun fill, recorded by the playback loop); M5-04 emits `Drift` (frame
/// insert/delete) and `ReAnchor` (hard-limit deletion). The counter API accepts
/// all four so no schema change is needed.
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

/// Arrival facts for one datagram, bundled so the submit path keeps a small,
/// readable argument list (and stays within clippy's budget).
struct Arrival {
    now_ms: u64,
    extended: u64,
    timestamp: u64,
    frames: u64,
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
    /// Capture-clock → session-clock anchor for the playout deadline: the
    /// `(session now_ms, capture position just past the newest accepted
    /// frame)` of the most recent *live* arrival. A datagram whose last frame
    /// is more than `target_ms` behind the newest arrival is late even when the
    /// frontier never advances (a sender that pauses).
    ///
    /// The anchor refreshes on EVERY live arrival, late ones included, and
    /// never moves its capture position backwards (`max`). That monotonic
    /// refresh is what stops a stalled capture clock plus one late datagram
    /// from wedging the deadline forever: the next arrival is judged against
    /// the refreshed session instant, not the stale one.
    playout_anchor: Option<(u64, u64)>,
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
            playout_anchor: None,
            jitter_frames: 0.0,
            last_arrival: None,
        }
    }

    /// The active §8 target/hard-limit configuration (exposed so the JVM can
    /// read the real values instead of duplicating constants).
    pub const fn config(&self) -> JitterConfig {
        self.config
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

    /// Forget the A.8 arrival history. Called on a timestamp re-base (a
    /// server-side drop): the position step is not interarrival jitter.
    ///
    /// Held datagrams released by a configuration change never reach
    /// [`Self::observe_arrival`] in the first place, so they need no reset —
    /// the replay path passes the original arrival instant and is exempt from
    /// the estimate entirely.
    pub fn reset_arrival(&mut self) {
        self.jitter_frames = 0.0;
        self.last_arrival = None;
    }

    /// The A.8 estimate converted to microseconds at the published rate.
    pub fn jitter_microseconds(&self) -> u32 {
        let micros = self.jitter_frames * 1_000_000.0 / f64::from(self.sample_rate);
        if !micros.is_finite() || micros <= 0.0 {
            return 0;
        }
        micros.min(u32::MAX as f64) as u32
    }

    /// Whether a datagram whose last frame ends at capture position `end`
    /// arrived after its playout deadline. Measured on the capture-clock →
    /// session-clock anchor so a stalled sender (no frontier advance) still
    /// trips it.
    ///
    /// The last frame (`timestamp + frames`) is the right probe: a datagram is
    /// still useful while any of its frames can be played, and a batched
    /// sender's header timestamp names the *first* frame of the batch.
    ///
    /// This must be called against the PRE-update anchor. The caller then
    /// refreshes the anchor, so the classification of *this* datagram is never
    /// softened by its own arrival.
    fn past_deadline(&self, now_ms: u64, end: u64) -> bool {
        let Some((anchor_now, anchor_end)) = self.playout_anchor else {
            return false;
        };
        let capture_delta_ms =
            (end as i128 - anchor_end as i128) * 1_000 / i128::from(self.sample_rate);
        let deadline = anchor_now as i128 + i128::from(self.config.target_ms) + capture_delta_ms;
        i128::from(now_ms) > deadline
    }

    /// Submit one newly observed, non-duplicate datagram at session-clock
    /// `now_ms`. `write` appends the given bytes to the ring in playout order
    /// (real payload or all-zero silence). Returns whether this was a late
    /// drop and any silence fills.
    pub fn submit(
        &mut self,
        now_ms: u64,
        extended: u64,
        timestamp: u64,
        frames: u64,
        payload: &[u8],
        write: &mut dyn FnMut(&[u8]),
    ) -> SubmitResult {
        self.submit_inner(
            Arrival {
                now_ms,
                extended,
                timestamp,
                frames,
            },
            payload,
            write,
            true,
        )
    }

    /// Submit media that is being replayed from the §7.6 hold. `now_ms` is the
    /// instant it really arrived, but the live arrival clock must not move
    /// backwards to it: the playout anchor stays at the newest live datagram.
    pub fn submit_replay(
        &mut self,
        arrival_ms: u64,
        extended: u64,
        timestamp: u64,
        frames: u64,
        payload: &[u8],
        write: &mut dyn FnMut(&[u8]),
    ) -> SubmitResult {
        self.submit_inner(
            Arrival {
                now_ms: arrival_ms,
                extended,
                timestamp,
                frames,
            },
            payload,
            write,
            false,
        )
    }

    fn submit_inner(
        &mut self,
        arrival: Arrival,
        payload: &[u8],
        write: &mut dyn FnMut(&[u8]),
        update_anchor: bool,
    ) -> SubmitResult {
        let Arrival {
            now_ms,
            extended,
            timestamp,
            frames,
        } = arrival;
        let end = timestamp.saturating_add(frames);
        // Judge lateness against the anchor as it stood BEFORE this arrival.
        let late = self.past_deadline(now_ms, end);
        if update_anchor {
            // Any live arrival — accepted, late, or a reordered slot — is the
            // newest live observation. The capture side never moves backwards
            // (`max`). Without this refresh, a stalled capture clock plus one
            // late datagram leaves the anchor parked and every later datagram
            // is judged late against that stale instant forever.
            let position = self
                .playout_anchor
                .map(|(_, anchor_end)| anchor_end.max(end))
                .unwrap_or(end);
            self.playout_anchor = Some((now_ms, position));
        }
        if late {
            // Arrived after its playout deadline (§8): late drop, never a
            // network loss and never ring data. This is the pause case where
            // the frontier has not advanced either.
            return SubmitResult {
                late: true,
                rebased: false,
                concealments: Vec::new(),
            };
        }
        let Some(frontier) = self.frontier else {
            // First datagram anchors the frontier. The playout clock was
            // already anchored above on the live path.
            self.frontier = Some(extended + 1);
            self.frontier_ts = end;
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
        while let Some(frontier) = self.frontier {
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
            let r = jb.submit(0, seq, seq * 64, 64, &[0x7f; 512], &mut |b| {
                written.extend_from_slice(b)
            });
            assert!(!r.late);
            assert!(r.concealments.is_empty());
        }
        assert_eq!(written.len(), 5 * 512);
    }

    #[test]
    fn a_timely_reorder_needs_no_concealment() {
        let mut jb = JitterBuffer::new(JitterConfig::default(), 48_000, 8);
        let mut written = Vec::new();
        jb.submit(0, 0, 0, 64, &[1; 512], &mut |b| {
            written.extend_from_slice(b)
        });
        jb.submit(40, 2, 128, 64, &[3; 512], &mut |b| {
            written.extend_from_slice(b)
        });
        // seq 1 arrives before the target delay elapses → fills the hole.
        jb.submit(50, 1, 64, 64, &[2; 512], &mut |b| {
            written.extend_from_slice(b)
        });
        assert_eq!(written.len(), 3 * 512);
        assert!(written.chunks(512).all(|c| c.iter().all(|&x| x != 0)));
    }

    #[test]
    fn a_late_frame_is_reported_not_written() {
        let mut jb = JitterBuffer::new(JitterConfig::default(), 48_000, 8);
        let mut written = Vec::new();
        for seq in 0..50u64 {
            jb.submit(0, seq, seq * 64, 64, &[0x7f; 512], &mut |b| {
                written.extend_from_slice(b)
            });
        }
        let r = jb.submit(0, 1, 64, 64, &[0x7f; 512], &mut |b| {
            written.extend_from_slice(b)
        });
        assert!(r.late);
    }

    /// RED before the deadline fix: with only a sequence-frontier check, a
    /// sender that pauses leaves the frontier parked, so the next datagram
    /// looked timely even minutes after its slot passed.
    #[test]
    fn an_arrival_after_a_sender_pause_is_late_by_deadline() {
        let mut jb = JitterBuffer::new(JitterConfig::default(), 48_000, 8);
        let mut written = Vec::new();
        // Anchor seq 0 (64 frames = 1.33 ms) at t=0; then the sender pauses
        // for five seconds and sends seq 1 with an unchanged capture clock.
        jb.submit(0, 0, 0, 64, &[0x7f; 512], &mut |b| {
            written.extend_from_slice(b)
        });
        let r = jb.submit(5_000, 1, 64, 64, &[0x7f; 512], &mut |b| {
            written.extend_from_slice(b)
        });
        assert!(r.late, "5 s after its 60 ms slot the datagram is late");
        assert_eq!(written.len(), 512, "late media never reaches the ring");
    }

    /// A paused sender that resumes with genuinely new capture positions is
    /// not late: the capture clock advanced with wall time.
    #[test]
    fn a_resumed_sender_with_advanced_capture_clock_is_not_late() {
        let mut jb = JitterBuffer::new(JitterConfig::default(), 48_000, 8);
        let mut written = Vec::new();
        jb.submit(0, 0, 0, 64, &[0x7f; 512], &mut |b| {
            written.extend_from_slice(b)
        });
        let r = jb.submit(5_000, 1, 240_064, 64, &[0x7f; 512], &mut |b| {
            written.extend_from_slice(b)
        });
        assert!(!r.late, "5 s of new capture audio is not a late datagram");
        assert_eq!(written.len(), 2 * 512);
    }

    /// RED before the anchor-refresh fix: a late datagram returned without
    /// refreshing `playout_anchor`, so with a stalled capture clock every
    /// subsequent arrival stayed late forever. The anchor must refresh on any
    /// live arrival while lateness is still judged against the pre-update one.
    #[test]
    fn a_late_datagram_refreshes_the_deadline_anchor() {
        let mut jb = JitterBuffer::new(JitterConfig::default(), 48_000, 8);
        let mut written = Vec::new();
        jb.submit(0, 0, 0, 64, &[0x7f; 512], &mut |b| {
            written.extend_from_slice(b)
        });
        // One datagram arrives 5 s after its slot: late, as it should be.
        assert!(
            jb.submit(5_000, 1, 64, 64, &[0x7f; 512], &mut |b| written
                .extend_from_slice(b))
                .late,
            "the first arrival after the stall is a late drop"
        );
        // The capture clock is still stalled, but the anchor refreshed to the
        // late arrival's session instant. A contiguous run is accepted (its
        // frames queue behind the seq-1 hole; the point is that none is late).
        let mut gaps = 0i64;
        for i in 0..45u32 {
            let sequence = 2 + u64::from(i);
            let timestamp = 128 + u64::from(i) * 64;
            let result = jb.submit(
                5_010 + u64::from(i) * 10,
                sequence,
                timestamp,
                64,
                &[0x7f; 512],
                &mut |b| written.extend_from_slice(b),
            );
            assert!(
                !result.late,
                "datagram {sequence} must be accepted once the anchor refreshed"
            );
            gaps += result
                .concealments
                .iter()
                .map(|(_, delta)| *delta)
                .sum::<i64>();
        }
        assert_eq!(
            gaps, 64,
            "the seq-1 hole is concealed once, not every frame"
        );
        assert_eq!(
            written.len(),
            47 * 512,
            "anchor frame + 45 live datagrams + one silence fill"
        );
    }

    #[test]
    fn a_contiguous_timestamp_jump_is_rebased_without_silence() {
        let mut jb = JitterBuffer::new(JitterConfig::default(), 48_000, 8);
        let mut written = Vec::new();
        jb.submit(0, 0, 0, 64, &[1; 512], &mut |b| {
            written.extend_from_slice(b)
        });
        // Server dropped frames: sequence contiguous but timestamp jumped by
        // 1000 frames. Must NOT insert silence (M5-05 timestamp offset).
        let r = jb.submit(0, 1, 1064, 64, &[2; 512], &mut |b| {
            written.extend_from_slice(b)
        });
        assert!(
            r.concealments.is_empty(),
            "no silence for a server-side drop"
        );
        assert_eq!(written.len(), 2 * 512);
    }

    /// A timestamp re-base must not leave a bogus interarrival step behind.
    #[test]
    fn a_rebase_resets_the_arrival_estimate() {
        let mut jb = JitterBuffer::new(JitterConfig::default(), 48_000, 8);
        // 1 ms of media presented 50 ms late is a large interarrival step.
        jb.observe_arrival(0, 0);
        jb.observe_arrival(50, 48);
        assert!(jb.jitter_microseconds() > 0);
        jb.reset_arrival();
        assert_eq!(jb.jitter_microseconds(), 0);
    }
}
