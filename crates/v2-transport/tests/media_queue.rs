//! Per-Client media send queue and pacing (spec §7.4, §10 TM-8/D12).

use std::time::{Duration, Instant};
use v2_transport::send_queue::{MediaPacer, MediaSendQueue, PacingViolation};

/// §7.4/§10: at most 256 datagrams; overflow drops the NEWEST and is counted
/// locally. Order is preserved.
#[test]
fn send_queue_is_bounded_at_256_and_drops_the_newest() {
    let mut queue = MediaSendQueue::new();
    let datagrams: Vec<Vec<u8>> = (0..300u32).map(|i| vec![(i % 251) as u8; 4]).collect();
    let accepted = queue.enqueue(datagrams);
    assert_eq!(accepted, 256, "capacity is 256 datagrams");
    assert_eq!(queue.dropped_newest(), 44);
    assert_eq!(queue.len(), 256);
    assert_eq!(queue.pop_front(), Some(vec![0u8; 4]));
    assert_eq!(queue.pop_front(), Some(vec![1u8; 4]));
}

/// §7.4: a non-waiting `send_datagram(TooLarge)` refusal is not a media drop.
/// The datagram goes back at the head, in order, with no overflow accounting.
#[test]
fn a_too_large_send_is_requeued_in_order() {
    let mut queue = MediaSendQueue::new();
    queue.enqueue(vec![vec![1u8; 4], vec![2u8; 4], vec![3u8; 4]]);
    let first = queue.pop_front().expect("first datagram");
    queue.push_front(first);
    assert_eq!(queue.len(), 3);
    assert_eq!(queue.pop_front(), Some(vec![1u8; 4]));
    assert_eq!(queue.pop_front(), Some(vec![2u8; 4]));
    assert_eq!(queue.pop_front(), Some(vec![3u8; 4]));
    assert_eq!(queue.dropped_newest(), 0, "requeue is not an overflow");
}

/// A drained queue reports empty; popping never blocks.
#[test]
fn drained_queue_reports_empty() {
    let mut queue = MediaSendQueue::new();
    queue.enqueue(vec![vec![1u8; 4], vec![2u8; 4]]);
    assert!(queue.pop_front().is_some());
    assert!(queue.pop_front().is_some());
    assert!(queue.pop_front().is_none());
    assert_eq!(queue.dropped_newest(), 0);
}

/// Closing releases the queue: pending media is dropped, no pacing drop is
/// counted, so a Closed session retains no media memory (§5.4 invariant 6).
#[test]
fn closing_clears_pending_media() {
    let mut queue = MediaSendQueue::new();
    queue.enqueue((0..5u8).map(|i| vec![i; 4]));
    queue.clear();
    assert_eq!(queue.len(), 0);
    assert!(queue.pop_front().is_none());
    assert_eq!(queue.dropped_newest(), 0, "clear is not a queue overflow");
}

/// §7.4/D12: the send-ahead window is 100 ms of media at the published rate
/// (48 kHz → 4800 frames). A single burst inside the window is allowed.
#[test]
fn one_window_of_media_may_burst_immediately() {
    let mut pacer = MediaPacer::new(48_000);
    let t0 = Instant::now();
    assert_eq!(pacer.window_frames(), 4800);
    assert_eq!(pacer.try_consume(t0, 4800), Ok(()));
}

/// A burst larger than the window is refused — never a whole-callback burst
/// without limit (baseline §Live Playback).
#[test]
fn a_burst_beyond_the_window_is_refused() {
    let mut pacer = MediaPacer::new(48_000);
    let t0 = Instant::now();
    assert_eq!(
        pacer.try_consume(t0, 4801),
        Err(PacingViolation::WindowExceeded)
    );
}

/// The window refills at the media rate: after 10 ms at 48 kHz, 480 frames
/// become available again; no more than that.
#[test]
fn the_window_refills_at_the_media_rate() {
    let mut pacer = MediaPacer::new(48_000);
    let t0 = Instant::now();
    assert_eq!(pacer.try_consume(t0, 4800), Ok(()));
    assert_eq!(
        pacer.try_consume(t0 + Duration::from_millis(10), 481),
        Err(PacingViolation::WindowExceeded),
        "only 480 frames have accrued"
    );
    assert_eq!(
        pacer.try_consume(t0 + Duration::from_millis(10), 480),
        Ok(())
    );
}

/// Unused budget never exceeds one window, so a long idle gap cannot later be
/// spent as a giant burst.
#[test]
fn idle_time_does_not_accumulate_beyond_one_window() {
    let mut pacer = MediaPacer::new(48_000);
    let t0 = Instant::now();
    assert_eq!(
        pacer.try_consume(t0 + Duration::from_secs(10), 4800),
        Ok(())
    );
    assert_eq!(
        pacer.try_consume(t0 + Duration::from_secs(10), 1),
        Err(PacingViolation::WindowExceeded)
    );
}

/// Regression (M5-08): each `try_consume` used to truncate the fractional
/// frame accrual and advance `last_refill`, so a jittered call cadence
/// permanently lost up to half a frame per call (~2 000 ppm at ~200 calls/s).
/// Over 10 s the bucket must deliver exactly `sample_rate` frames regardless
/// of call cadence.
///
/// Exact 1/3/7 ms cadences accrue whole frames (48/144/336) and would not
/// expose the truncation, so the cadence carries sub-millisecond jitter. The
/// expected 480 000 frames comes from §8's rate-match budget (10 s × 48 kHz),
/// not from replaying the pacer's own arithmetic.
#[test]
fn fractional_refill_carries_across_a_jittered_call_cadence() {
    let mut pacer = MediaPacer::new(48_000);
    let t0 = Instant::now();
    // Drain the initial 100 ms window so the measurement covers only refills.
    while pacer.try_consume(t0, 1).is_ok() {}
    assert_eq!(pacer.consumed_frames(), 4_800);

    let cadence = [
        Duration::from_nanos(1_100_000),
        Duration::from_nanos(3_100_000),
        Duration::from_nanos(7_100_000),
    ];
    let mut elapsed = Duration::ZERO;
    let mut index = 0usize;
    while elapsed < Duration::from_secs(10) {
        let now = t0 + elapsed;
        while pacer.try_consume(now, 1).is_ok() {}
        elapsed += cadence[index % cadence.len()];
        index += 1;
    }
    // Settle exactly at the 10 s measurement boundary and consume the tail.
    let end = t0 + Duration::from_secs(10);
    while pacer.try_consume(end, 1).is_ok() {}

    let delivered = pacer.consumed_frames() - 4_800;
    assert!(
        delivered.abs_diff(480_000) <= 1,
        "10 s at 48 kHz must deliver 480000 ± 1 frames under a jittered cadence, got {delivered}"
    );
}

/// §7.2: the timestamp carried by the first payload frame is the capture
/// clock position; it advances by the frames actually chunked, so the media
/// timeline the pacer enforces is the same one the receiver sees.
#[test]
fn consumed_frames_match_the_packetizer_clock() {
    let mut pacer = MediaPacer::new(48_000);
    let t0 = Instant::now();
    // Two 480-frame datagrams inside the initial window.
    assert_eq!(pacer.try_consume(t0, 480), Ok(()));
    assert_eq!(pacer.try_consume(t0, 480), Ok(()));
    assert_eq!(pacer.consumed_frames(), 960);
}
