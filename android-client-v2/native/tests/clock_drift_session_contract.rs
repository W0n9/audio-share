//! M5-04 drift/correction wiring through the public `MediaSession` seam.
//!
//! These tests pin the §8 accounting rules at the seam the session layer, JNI
//! and the Kotlin player actually use: the clock estimate, the ±100 ppm rate
//! decision, sustained frame insert/delete (counted only when applied), and the
//! hard-limit re-anchor.

use audio_share_client_v2_native::media::{MediaHeader, FLAG_STREAM_START};
use audio_share_client_v2_native::media_session::{MediaSession, PlaybackReport};
use audio_share_client_v2_native::proto::{Configuration, PcmParameters};

const RATE: u32 = 48_000;
const FRAME_BYTES: u64 = 8; // stereo F32LE
const TARGET_MS: u32 = 60;

fn config() -> Configuration {
    Configuration {
        config_generation: 1,
        codec: 1,
        pcm: Some(PcmParameters {
            sample_rate: RATE,
            channels: 2,
            sample_format: 1,
        }),
    }
}

fn datagram(sequence: u32, timestamp: u64, frames: u64) -> Vec<u8> {
    let header = MediaHeader {
        stream_instance_id: 1,
        config_generation: 1,
        sequence,
        timestamp,
        flags: if sequence == 0 { FLAG_STREAM_START } else { 0 },
    };
    let mut out = Vec::new();
    header.write_be(&mut out);
    out.extend(std::iter::repeat_n(
        0u8,
        frames as usize * FRAME_BYTES as usize,
    ));
    out
}

fn session() -> MediaSession {
    MediaSession::start(1, 1, 0, &config(), None).expect("F32LE stereo")
}

/// Capture-clock frame position at `now_ms` when the capture device runs at
/// `RATE * (1 + ppm/1e6)` frames/s.
fn capture_at(now_ms: u64, ppm: i32) -> u64 {
    ((now_ms as i128 * RATE as i128 * (1_000_000 + ppm as i128)) / 1_000_000_000) as u64
}

/// Playback-clock position at `now_ms` when the platform consumes at exactly
/// `RATE` frames/s.
fn playback_at(now_ms: u64) -> u64 {
    now_ms * RATE as u64 / 1_000
}

/// Feed a synthetic session where the capture device runs `ppm` fast and the
/// platform consumes at the nominal rate. Datagram timestamps stay contiguous
/// with the frames actually sent, so this is clock drift — not the M5-05
/// server-drop offset.
fn drive(session: &mut MediaSession, from_ms: u64, to_ms: u64, ppm: i32, underruns: u64) {
    let mut previous_capture = if from_ms == 0 {
        0
    } else {
        capture_at(from_ms - 100, ppm)
    };
    let mut sequence = (from_ms / 100) as u32;
    let mut now = from_ms;
    while now <= to_ms {
        let capture = capture_at(now, ppm);
        let frames = capture.saturating_sub(previous_capture);
        session.on_datagram(now, &datagram(sequence, previous_capture, frames));
        previous_capture = capture;
        session.record_playback_at(
            now,
            PlaybackReport {
                submitted_bytes: playback_at(now) * FRAME_BYTES,
                underruns,
                buffer_current_ms: TARGET_MS,
            },
        );
        sequence += 1;
        now += 100;
    }
}

#[test]
fn a_synthetic_one_hundred_ppm_capture_clock_is_measured_and_clamped() {
    let mut media = session();
    drive(&mut media, 0, 12_000, 100, 0);

    let drift = media.drift_ppm().expect("a full 10 s window");
    assert!(
        (99..=101).contains(&drift),
        "synthetic +100 ppm must measure ~100, got {drift}"
    );
    assert_eq!(
        media.drift_correction_ppm(),
        100,
        "the smooth controller converges on the ±100 ppm ceiling"
    );
    assert_eq!(
        media.take_drift_intervention(),
        0,
        "rate matching alone keeps the buffer at target"
    );
}

#[test]
fn the_correction_never_exceeds_plus_or_minus_one_hundred_ppm() {
    let mut media = session();
    // A 1% capture clock is far outside §8's capability: the clamp is the
    // whole contract, and a wild estimate must not command a wild rate.
    drive(&mut media, 0, 12_000, 10_000, 0);
    assert_eq!(media.drift_correction_ppm(), 100);

    let mut media = session();
    drive(&mut media, 0, 12_000, -10_000, 0);
    assert_eq!(media.drift_correction_ppm(), -100);
}

#[test]
fn a_timestamp_rebase_does_not_produce_a_bogus_ppm() {
    let mut media = session();
    drive(&mut media, 0, 11_000, 100, 0);
    let before = media.drift_ppm().expect("window filled before the jump");
    assert!((99..=101).contains(&before), "got {before}");

    // Server-side drop (M5-05): contiguous sequence, timestamp permanently
    // jumped by 4800 frames. M5-08 folds the known position step out instead
    // of clearing the 10 s window, so the estimate stays a real rate.
    let jumped = capture_at(11_000, 100) + 4_800;
    media.on_datagram(11_000, &datagram(111, jumped, 64));
    media.record_playback_at(
        11_000,
        PlaybackReport {
            // Playback also advanced by the 64 frames the datagram carried.
            submitted_bytes: (playback_at(11_000) + 64) * FRAME_BYTES,
            underruns: 0,
            buffer_current_ms: TARGET_MS,
        },
    );
    let after = media
        .drift_ppm()
        .expect("a re-base no longer starves the window");
    assert!(
        (95..=105).contains(&after),
        "a re-base is a position step, not drift; got {after} ppm"
    );
}

#[test]
fn an_underrun_does_not_restart_the_drift_window() {
    let mut media = session();
    drive(&mut media, 0, 10_000, 100, 0);
    assert!(media.drift_ppm().is_some());

    // The platform underrun counter rises (a playback stall). It is a §9
    // diagnostic, not a reason to discard the measurement window.
    drive(&mut media, 10_100, 13_000, 100, 1);
    let ppm = media
        .drift_ppm()
        .expect("an underrun counter bump must not starve the window");
    assert!((95..=105).contains(&ppm), "got {ppm} ppm");
}

// A soft target trim advances the playout position over skipped media. That is
// a position discontinuity: M5-08 subtracts it from the estimate's playback
// side instead of restarting the 10 s window, so it never reads as a rate step.
#[test]
fn a_soft_target_trim_does_not_produce_a_bogus_ppm() {
    let mut media = session();
    drive(&mut media, 0, 11_000, 100, 0);
    assert!(media.drift_ppm().is_some(), "the window filled first");

    media.record_soft_trim(240);
    drive(&mut media, 11_100, 14_000, 100, 0);
    let ppm = media
        .drift_ppm()
        .expect("a trim must not starve the window");
    assert!(
        (95..=105).contains(&ppm),
        "a 240-frame trim must not read as a large ppm, got {ppm}"
    );
}

/// A hard-limit re-anchor is the same kind of position step as a soft trim.
#[test]
fn a_hard_re_anchor_does_not_produce_a_bogus_ppm() {
    let mut media = session();
    drive(&mut media, 0, 11_000, 100, 0);
    media.record_trim(4_800);
    drive(&mut media, 11_100, 14_000, 100, 0);
    let ppm = media
        .drift_ppm()
        .expect("a re-anchor must not starve the window");
    assert!((95..=105).contains(&ppm), "got {ppm} ppm");
}

/// M5-08: an accepted underrun fill hands the platform one period of silence
/// (accepted bytes rise; `position_adjust_frames` falls). The platform clock
/// still advanced over that silence, so the fill must not leave a position
/// step that reads as a large ppm in the rate estimate.
#[test]
fn an_accepted_underrun_fill_does_not_produce_a_bogus_ppm() {
    let mut media = session();
    drive(&mut media, 0, 11_000, 100, 0);
    media.record_underrun(480);
    drive(&mut media, 11_100, 14_000, 100, 0);
    let ppm = media
        .drift_ppm()
        .expect("an accepted fill must not starve the window");
    assert!(
        (95..=105).contains(&ppm),
        "a 480-frame underrun fill must not read as a large ppm, got {ppm}"
    );
}

#[test]
fn a_hard_limit_re_anchor_is_concealment_and_jumps_the_position() {
    let mut media = session();
    for sequence in 0..5u32 {
        media.on_datagram(0, &datagram(sequence, u64::from(sequence) * 64, 64));
    }
    assert_eq!(media.submitted_position(), 0, "nothing was submitted yet");

    // Kotlin's hard-limit re-anchor dropped the oldest 64 frames down to the
    // §8 target and advanced its read cursor.
    media.record_trim(64);

    assert_eq!(
        media.submitted_position(),
        64,
        "the submitted position must jump forward over deleted media"
    );
    assert_eq!(media.trimmed_frames(), 64, "the trim diagnostic is kept");
    let diagnostics = media.diagnostics(TARGET_MS);
    assert_eq!(diagnostics.cumulative_concealments, 1);
    assert_eq!(diagnostics.concealed_frames, -64);
}

#[test]
fn a_sustained_buffer_deviation_asks_the_player_for_frames_and_counts_the_ack() {
    let mut media = session();
    // No datagrams: playback position cannot advance, so the estimator stays
    // silent. The sustained-deviation monitor is buffer-error driven.
    let mut now = 0u64;
    while now <= 5_300 {
        media.record_playback_at(
            now,
            PlaybackReport {
                submitted_bytes: 0,
                underruns: 0,
                buffer_current_ms: 85, // 25 ms above target
            },
        );
        now += 100;
    }
    assert_eq!(
        media.take_drift_intervention(),
        -240,
        "5 ms over the deadband is 240 frames to delete"
    );
    assert_eq!(
        media.take_drift_intervention(),
        0,
        "taking the pending adjustment clears it"
    );
    assert_eq!(
        media.diagnostics(TARGET_MS).cumulative_concealments,
        0,
        "the intervention is not counted until the player applies it"
    );

    media.record_drift_intervention(-240);
    let diagnostics = media.diagnostics(TARGET_MS);
    assert_eq!(
        diagnostics.cumulative_concealments, 1,
        "every applied drift intervention is a §8 concealment"
    );
    assert_eq!(diagnostics.concealed_frames, -240);
    assert_eq!(
        media.submitted_position(),
        240,
        "deleted frames advance the playback position on the capture clock"
    );
}

#[test]
fn an_applied_drift_insertion_offsets_the_position_back() {
    let mut media = session();
    media.record_drift_intervention(96);
    let diagnostics = media.diagnostics(TARGET_MS);
    assert_eq!(diagnostics.cumulative_concealments, 1);
    assert_eq!(diagnostics.concealed_frames, 96);
    // Inserted silence is not a capture position, so the platform-accepted
    // frames it contributes are offset out of the capture-clock position.
    media.record_playback(PlaybackReport {
        submitted_bytes: 96 * FRAME_BYTES,
        underruns: 0,
        buffer_current_ms: TARGET_MS,
    });
    assert_eq!(media.submitted_position(), 0);
}
