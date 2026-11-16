//! Per-Client media send queue and the §7.4 send-ahead pacer.
//!
//! The queue is bounded at 256 datagrams and drops the newest on overflow
//! (spec §7.4, §10 TM-8), so a stalled or congested Client can never grow
//! memory or block capture: enqueueing and popping are both non-blocking.
//!
//! The pacer enforces the 100 ms send-ahead window (D12) as a token bucket on
//! an injected clock. Time is passed in rather than read, so the invariant is
//! testable without sleeping, and media seconds — not wall milliseconds — are
//! what a burst is measured in at the published sample rate.

use std::time::Instant;

/// Per-Client queue capacity in datagrams (spec §10, TM-8).
pub const MAX_QUEUED_DATAGRAMS: usize = 256;

/// Send-ahead window in milliseconds (spec §7.4, D12: ≤ 100 ms).
pub const SEND_AHEAD_MS: u32 = 100;

/// Why media may not be sent now. Never a reason to drop a datagram.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacingViolation {
    /// The burst would exceed the 100 ms send-ahead window.
    WindowExceeded,
}

/// A bounded FIFO of encoded datagrams for one session.
#[derive(Debug)]
pub struct MediaSendQueue {
    datagrams: std::collections::VecDeque<Vec<u8>>,
    dropped_newest: u64,
}

impl MediaSendQueue {
    /// An empty queue.
    pub fn new() -> Self {
        Self {
            datagrams: std::collections::VecDeque::with_capacity(MAX_QUEUED_DATAGRAMS),
            dropped_newest: 0,
        }
    }

    /// Enqueue in media order. Beyond the bound the newest items are dropped
    /// and counted (§7.4); returns how many were accepted.
    pub fn enqueue(&mut self, incoming: impl IntoIterator<Item = Vec<u8>>) -> usize {
        let mut accepted = 0;
        for datagram in incoming {
            if self.datagrams.len() >= MAX_QUEUED_DATAGRAMS {
                self.dropped_newest += 1;
                continue;
            }
            self.datagrams.push_back(datagram);
            accepted += 1;
        }
        accepted
    }

    /// Datagrams dropped because the queue was full (drop-newest, §7.4).
    pub const fn dropped_newest(&self) -> u64 {
        self.dropped_newest
    }

    /// Datagrams currently queued.
    pub fn len(&self) -> usize {
        self.datagrams.len()
    }

    /// Whether nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.datagrams.is_empty()
    }

    /// The next datagram in media order (never blocks).
    pub fn pop_front(&mut self) -> Option<Vec<u8>> {
        self.datagrams.pop_front()
    }

    /// Put back a datagram whose non-waiting send was refused (§7.4:
    /// `send_datagram(TooLarge)` is not a media drop — the next tick queries
    /// a smaller path limit). This is not a queue-overflow event.
    pub fn push_front(&mut self, datagram: Vec<u8>) {
        self.datagrams.push_front(datagram);
    }

    /// Borrow the next datagram without removing it (pacing decisions).
    pub fn front(&self) -> Option<&Vec<u8>> {
        self.datagrams.front()
    }

    /// Release all pending media (session teardown, §5.4 invariant 6). This
    /// is not a drop-accounting event.
    pub fn clear(&mut self) {
        self.datagrams.clear();
    }
}

impl Default for MediaSendQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// The §7.4 send-ahead window as a token bucket on an injected clock.
#[derive(Debug)]
pub struct MediaPacer {
    sample_rate: u32,
    window_frames: u64,
    available: u64,
    last_refill: Option<Instant>,
    consumed_frames: u64,
}

impl MediaPacer {
    /// A full window available at construction; the first `try_consume` fixes
    /// the clock origin.
    pub fn new(sample_rate: u32) -> Self {
        let window_frames = u64::from(sample_rate) * u64::from(SEND_AHEAD_MS) / 1_000;
        Self {
            sample_rate,
            window_frames,
            available: window_frames,
            last_refill: None,
            consumed_frames: 0,
        }
    }

    /// Window length in sample frames (100 ms at the published rate).
    pub const fn window_frames(&self) -> u64 {
        self.window_frames
    }

    /// Frames consumed by successful sends on this session.
    pub const fn consumed_frames(&self) -> u64 {
        self.consumed_frames
    }

    /// Refill at the media rate up to one window, then consume `frames`.
    /// Refill is capped at the window so idle time can never be banked into a
    /// later burst (§7.4: never an unbounded whole-callback burst).
    pub fn try_consume(&mut self, now: Instant, frames: u64) -> Result<(), PacingViolation> {
        match self.last_refill {
            None => self.last_refill = Some(now),
            Some(last) => {
                if now > last {
                    let elapsed = now.saturating_duration_since(last);
                    let accrued = elapsed.as_nanos() * u128::from(self.sample_rate) / 1_000_000_000;
                    let accrued = u64::try_from(accrued).unwrap_or(u64::MAX);
                    self.available = self
                        .available
                        .saturating_add(accrued)
                        .min(self.window_frames);
                    self.last_refill = Some(now);
                }
            }
        }
        if frames > self.available {
            return Err(PacingViolation::WindowExceeded);
        }
        self.available -= frames;
        self.consumed_frames = self.consumed_frames.saturating_add(frames);
        Ok(())
    }
}
