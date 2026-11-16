//! The protocol-independent media tap over the shared capture (ticket 13).
//!
//! These are the Server-side media seam: what a v2 session can observe and
//! what the capture path is allowed to spend on it. Expected positions come
//! from frame arithmetic (48 kHz stereo F32LE = 8 bytes/frame), not from the
//! tap's own counters.

use std::time::Duration;

use audio_share_core::capture::media::{MediaFormat, MediaTap};

fn f32_stereo() -> MediaFormat {
    MediaFormat {
        sample_rate: 48_000,
        channels: 2,
        bytes_per_sample: 4,
    }
}

/// §7.2: a chunk's reviewed position is the capture-clock sample-frame
/// position; `end - start` is the frame count, and positions are contiguous.
#[test]
fn published_chunks_carry_contiguous_frame_positions() {
    let tap = MediaTap::with_instance(f32_stereo(), 3);
    let mut receiver = tap.subscribe();
    // 8192 bytes = 1024 frames; a second 4000-byte chunk is 500 frames.
    tap.publish(&vec![0x11; 8192]);
    tap.publish(&vec![0x22; 4000]);

    let first = receiver.try_recv().expect("first chunk");
    assert_eq!(first.stream_instance_id, 3);
    assert_eq!(first.config_generation, 1);
    assert_eq!(first.start_frame, 0);
    assert_eq!(first.end_frame, 1024);
    assert_eq!(first.pcm.len(), 8192);

    let second = receiver.try_recv().expect("second chunk");
    assert_eq!(second.start_frame, 1024);
    assert_eq!(second.end_frame, 1524);
    assert_eq!(tap.stream().expect("active").next_frame, 1524);
}

/// A capture buffer that is not whole frames is rejected, not truncated.
#[test]
fn misaligned_capture_buffers_are_rejected_and_counted() {
    let tap = MediaTap::with_instance(f32_stereo(), 1);
    let mut receiver = tap.subscribe();
    tap.publish(&[0u8; 5]);
    assert!(receiver.try_recv().is_err(), "no chunk published");
    assert_eq!(tap.stream().expect("active").next_frame, 0);
    assert_eq!(tap.dropped_chunks(), 1);
}

/// The capture path never blocks: a subscriber that never drains loses
/// chunks, gets counted, and the publish calls still return immediately.
#[test]
fn a_stalled_subscriber_never_blocks_publishing() {
    let tap = MediaTap::with_instance(f32_stereo(), 1);
    let _stalled = tap.subscribe();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for _ in 0..64 {
                tap.publish(&vec![0u8; 8192]);
            }
        });
    });
    assert!(
        tap.dropped_chunks() > 0,
        "a never-draining subscriber must lose chunks instead of stalling capture"
    );
    assert_eq!(tap.stream().expect("active").next_frame, 64 * 1024);
}

/// Independent subscribers each see the same stream in order; a slow one does
/// not affect a fast one.
#[test]
fn subscribers_are_independent() {
    let tap = MediaTap::with_instance(f32_stereo(), 2);
    let mut first = tap.subscribe();
    let mut second = tap.subscribe();
    for _ in 0..6 {
        tap.publish(&vec![0u8; 800]);
    }
    // First drains as fast as it is published; second drains afterwards.
    let mut first_frames = Vec::new();
    while let Ok(chunk) = first.try_recv() {
        first_frames.push((chunk.start_frame, chunk.end_frame));
    }
    assert_eq!(
        first_frames,
        vec![
            (0, 100),
            (100, 200),
            (200, 300),
            (300, 400),
            (400, 500),
            (500, 600)
        ]
    );

    let mut second_count = 0;
    while let Ok(chunk) = second.try_recv() {
        assert_eq!(chunk.start_frame, second_count * 100);
        second_count += 1;
    }
    assert_eq!(second_count, 6, "both subscribers see every chunk");
}

/// §5.2: ending the stream closes subscribers and clears the snapshot, so the
/// session layer can push StreamEnded and refuse further media.
#[test]
fn ending_the_stream_closes_subscribers_and_clears_the_snapshot() {
    let tap = MediaTap::with_instance(f32_stereo(), 4);
    let mut receiver = tap.subscribe();
    tap.publish(&vec![0u8; 800]);
    tap.end();
    assert!(tap.stream().is_none());
    assert!(!tap.is_active());
    assert!(
        receiver.try_recv().is_ok(),
        "already-published chunk stays readable"
    );
    assert!(
        matches!(
            receiver.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
        ),
        "the channel is closed after end()"
    );
}

/// A subscriber that joins a running stream starts at the live anchor (§4.4:
/// a late Subscriber's `StreamStarted.initial_timestamp` is mid-stream).
#[test]
fn a_late_subscriber_joins_at_the_live_position() {
    let tap = MediaTap::with_instance(f32_stereo(), 5);
    tap.publish(&vec![0u8; 8192]);
    let mut late = tap.subscribe();
    assert_eq!(tap.stream().expect("active").next_frame, 1024);
    tap.publish(&vec![0u8; 800]);
    let chunk = late.try_recv().expect("late subscriber's first chunk");
    assert_eq!(chunk.start_frame, 1024);
    assert_eq!(chunk.end_frame, 1124);
}

/// Instance ids are process-unique and never zero (§7.1/§7.2).
#[test]
fn instance_ids_are_nonzero_and_distinct() {
    let first = MediaTap::new(f32_stereo());
    let second = MediaTap::new(f32_stereo());
    let a = first.stream().expect("active").stream_instance_id;
    let b = second.stream().expect("active").stream_instance_id;
    assert_ne!(a, 0);
    assert_ne!(b, 0);
    assert_ne!(a, b);
}

/// Publishing after `end` is a no-op (the tap holds no stream to feed).
#[test]
fn publishing_after_end_is_ignored() {
    let tap = MediaTap::with_instance(f32_stereo(), 6);
    tap.end();
    tap.publish(&vec![0u8; 800]);
    assert!(tap.stream().is_none());
    assert_eq!(tap.dropped_chunks(), 0);
}

/// Publishing is bounded work per subscriber; a plain publish of one buffer
/// must complete far below any real-time budget even with several subscribers.
#[test]
fn publishing_stays_cheap_with_several_subscribers() {
    let tap = MediaTap::with_instance(f32_stereo(), 7);
    let _subs: Vec<_> = (0..8).map(|_| tap.subscribe()).collect();
    let started = std::time::Instant::now();
    for _ in 0..1_000 {
        tap.publish(&vec![0u8; 8192]);
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "1000 frame-aligned publishes with 8 subscribers must not stall: {:?}",
        started.elapsed()
    );
}
