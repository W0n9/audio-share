//! Public `audio_share_core::capture` seam: the synthetic backend.
//!
//! These tests use the real backend API (`CaptureBackend::start`,
//! `CaptureSession::stop`) and a real channel. They do not need any audio
//! device, thread naming, or private capture internals.

use std::time::Duration;

use audio_share_core::capture::{CaptureBackend, CaptureConfig, Endpoint, SyntheticCapture};
use audio_share_core::protocol::{AudioFormat, Encoding};
use tokio::sync::mpsc;
use tokio::time::timeout;

fn f32_stereo_48k() -> AudioFormat {
    AudioFormat::new(Encoding::PcmFloat, 2, 48_000)
}

#[tokio::test]
async fn synthetic_capture_emits_exact_frames_in_order() {
    let format = AudioFormat::new(Encoding::Pcm16Bit, 2, 48_000); // block_align 4
    let first = vec![0x11u8; 16];
    let second = vec![0x22u8; 8];
    let capture = SyntheticCapture::with_frames(
        format.clone(),
        vec![first.clone(), second.clone()],
        Duration::from_millis(1),
    )
    .unwrap();

    let (sink, mut frames) = mpsc::channel(8);
    let session = capture.start(&CaptureConfig::default(), sink).unwrap();
    assert_eq!(session.format, format);

    let one = timeout(Duration::from_secs(2), frames.recv())
        .await
        .expect("first frame timed out")
        .expect("capture channel closed");
    let two = timeout(Duration::from_secs(2), frames.recv())
        .await
        .expect("second frame timed out")
        .expect("capture channel closed");
    assert_eq!(one, first);
    assert_eq!(two, second, "frames must be emitted in constructor order");

    session.stop().unwrap();
}

#[tokio::test]
async fn synthetic_capture_generator_is_deterministic_and_block_aligned() {
    let format = f32_stereo_48k(); // block_align 8
    let capture_a = SyntheticCapture::new(format.clone(), 64, Duration::from_millis(1)).unwrap();
    let capture_b = SyntheticCapture::new(format.clone(), 64, Duration::from_millis(1)).unwrap();

    let (sink_a, mut frames_a) = mpsc::channel(4);
    let (sink_b, mut frames_b) = mpsc::channel(4);
    let session_a = capture_a.start(&CaptureConfig::default(), sink_a).unwrap();
    let session_b = capture_b.start(&CaptureConfig::default(), sink_b).unwrap();

    let first_a = timeout(Duration::from_secs(2), frames_a.recv())
        .await
        .unwrap()
        .unwrap();
    let first_b = timeout(Duration::from_secs(2), frames_b.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first_a.len(), 64);
    assert_eq!(first_a.len() % format.block_align(), 0);
    assert_eq!(first_a, first_b, "the generator must be deterministic");

    session_a.stop().unwrap();
    session_b.stop().unwrap();
}

#[tokio::test]
async fn synthetic_capture_rejects_unaligned_or_empty_frames() {
    let format = f32_stereo_48k();

    assert!(
        SyntheticCapture::new(format.clone(), 12, Duration::from_millis(1)).is_err(),
        "frame length must be a multiple of block_align"
    );
    assert!(
        SyntheticCapture::new(format.clone(), 0, Duration::from_millis(1)).is_err(),
        "frame length must be non-zero"
    );
    assert!(
        SyntheticCapture::with_frames(format.clone(), vec![], Duration::from_millis(1)).is_err(),
        "at least one frame is required"
    );
    assert!(
        SyntheticCapture::with_frames(
            format.clone(),
            vec![vec![0u8; 12]],
            Duration::from_millis(1)
        )
        .is_err(),
        "explicit frames must be block aligned"
    );
    assert!(
        SyntheticCapture::new(format, 16, Duration::ZERO).is_err(),
        "a zero frame interval would spin"
    );

    let invalid_format = AudioFormat::new(Encoding::Invalid, 2, 48_000);
    assert!(
        SyntheticCapture::new(invalid_format, 16, Duration::from_millis(1)).is_err(),
        "an invalid encoding has no block_align"
    );
}

/// Drain frames until the sender is dropped, failing if the channel stays open.
///
/// Frames already queued when the session stops are legitimate; the contract is
/// that the stream ends, not that the channel is empty at that instant. The
/// bound is deliberately larger than the channel capacity so a slow Windows
/// scheduler cannot turn a correct stop into a flaky timeout.
async fn expect_frame_stream_to_end(frames: &mut mpsc::Receiver<Vec<u8>>, context: &str) {
    let ended = timeout(Duration::from_secs(2), async {
        while frames.recv().await.is_some() {}
    })
    .await;
    assert!(ended.is_ok(), "{context}");
}

#[tokio::test]
async fn stopping_or_dropping_a_session_ends_the_frame_stream() {
    let capture = SyntheticCapture::new(f32_stereo_48k(), 16, Duration::from_millis(1)).unwrap();

    let (sink, mut frames) = mpsc::channel(4);
    let session = capture.start(&CaptureConfig::default(), sink).unwrap();
    timeout(Duration::from_secs(2), frames.recv())
        .await
        .expect("frame timed out")
        .expect("channel closed early");
    session.stop().unwrap();
    expect_frame_stream_to_end(&mut frames, "stop must drop the frame sender").await;

    let (sink, mut frames) = mpsc::channel(4);
    let session = capture.start(&CaptureConfig::default(), sink).unwrap();
    timeout(Duration::from_secs(2), frames.recv())
        .await
        .expect("frame timed out")
        .expect("channel closed early");
    drop(session);
    expect_frame_stream_to_end(
        &mut frames,
        "dropping the session must also stop the capture",
    )
    .await;
}

#[tokio::test]
async fn synthetic_backend_reports_a_default_endpoint() {
    let capture = SyntheticCapture::new(f32_stereo_48k(), 16, Duration::from_millis(1)).unwrap();
    let endpoints = capture.endpoints().unwrap();
    assert_eq!(endpoints.len(), 1);
    assert_eq!(
        capture.default_endpoint().unwrap(),
        Some(endpoints[0].clone())
    );
    assert_eq!(
        endpoints[0],
        Endpoint {
            id: "synthetic".to_owned(),
            name: "Synthetic Capture".to_owned()
        }
    );
}
