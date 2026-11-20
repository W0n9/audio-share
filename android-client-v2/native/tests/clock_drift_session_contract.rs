//! M5-04 drift/correction wiring through the public `MediaSession` seam.
//!
//! These tests pin the §8 accounting rules at the seam the session layer, JNI
//! and the Kotlin player actually use: the clock estimate, the ±100 ppm rate
//! decision, sustained frame insert/delete (counted only when applied), and the
//! hard-limit re-anchor.

use audio_share_client_v2_native::media::{MediaHeader, FLAG_STREAM_START};
use audio_share_client_v2_native::media_session::{DatagramOutcome, MediaSession, PlaybackReport};
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

/// Fixed, identical 48 kHz source/sink clocks. The receiver normally sees one
/// PCM packet per ms. Only its delivery is held in `holds`; at each release,
/// the queued packets arrive before the next reader iteration. The reader
/// advances over the oldest frames using the Kotlin player's 60/80/200 ms
/// thresholds and submits only available frames (or accepted underrun silence
/// when enabled as the control). It does not inject estimator positions.
struct DeliverySample {
    now_ms: u64,
    raw_ppm: Option<i32>,
    correction_ppm: i32,
    window: (u64, usize),
}

struct DeliveryTrace {
    samples: Vec<DeliverySample>,
    trim_events: u64,
    trimmed_frames: u64,
}

fn delivery_trace(
    holds: &[(u64, u64)],
    until_ms: u64,
    capture_ppm: i32,
    keep_sink_fed: bool,
) -> DeliveryTrace {
    let mut media = session();
    let mut next_packet = 1;
    let mut read = 0;
    let mut accepted = 0;
    let mut next_report = 1;
    let mut samples = Vec::new();
    for now in 1..=until_ms {
        let holding = holds
            .iter()
            .any(|&(start, end)| (start..end).contains(&now));
        if !holding {
            while next_packet <= now {
                let start = capture_at(next_packet - 1, capture_ppm);
                let frames = capture_at(next_packet, capture_ppm) - start;
                let outcome =
                    media.on_datagram(now, &datagram((next_packet - 1) as u32, start, frames));
                assert!(matches!(
                    outcome,
                    DatagramOutcome::Delivered | DatagramOutcome::LateDropped
                ));
                next_packet += 1;
            }
        }
        let mut available = media.delivered_frames() - read;
        if available > playback_at(80) {
            let trimmed = available - playback_at(60);
            if available > playback_at(200) {
                media.record_trim(trimmed);
            } else {
                media.record_soft_trim(trimmed);
            }
            read += trimmed;
            available -= trimmed;
        }
        if now > 60 {
            let frames = available.min(48);
            accepted += frames;
            read += frames;
            available -= frames;
            if keep_sink_fed && frames < 48 {
                accepted += 48 - frames;
                media.record_underrun(48 - frames);
            }
        }
        if now == next_report * 101 + next_report * 79 / 99 {
            media.record_playback_at(
                now,
                PlaybackReport {
                    submitted_bytes: accepted * FRAME_BYTES,
                    underruns: 0,
                    buffer_current_ms: (available / 48) as u32,
                },
            );
            next_report += 1;
            samples.push(DeliverySample {
                now_ms: now,
                raw_ppm: media.drift_ppm(),
                correction_ppm: media.drift_correction_ppm(),
                window: media.drift_window(),
            });
        }
    }
    DeliveryTrace {
        samples,
        trim_events: media.trim_events(),
        trimmed_frames: media.trimmed_frames(),
    }
}

#[test]
fn paced_delivery_measures_equal_clocks_without_trimming() {
    let trace = delivery_trace(&[], 17_000, 0, false);
    assert_eq!(trace.samples.last().unwrap().raw_ppm, Some(0));
    assert!(trace
        .samples
        .iter()
        .filter_map(|sample| sample.raw_ppm)
        .all(|ppm| ppm == 0));
    assert_eq!(trace.trim_events, 0);
}

#[test]
fn catch_up_delivery_does_not_publish_a_playback_clock_outlier() {
    // Synthetic B2-scale input, NOT a device replay. The observed 5 s bucket
    // had seven hard trims totalling 64,672 frames. These timings use ~250 ms
    // holds and batch-release whole 48-frame packets (64,656 trimmed frames).
    let holds = [
        (11_000, 11_251),
        (11_650, 11_901),
        (12_300, 12_551),
        (12_950, 13_201),
        (13_600, 13_851),
        (14_250, 14_501),
        (14_900, 15_154),
    ];
    let trace = delivery_trace(&holds, 27_000, 0, false);
    assert_eq!(trace.trim_events, 7);
    assert_eq!(trace.trimmed_frames, 64_656);
    let worst = trace
        .samples
        .iter()
        .max_by_key(|sample| sample.raw_ppm.unwrap_or(0).unsigned_abs())
        .unwrap();
    assert!(
        worst.raw_ppm.is_none_or(|ppm| ppm.abs() <= 100),
        "equal clocks produced {:?} ppm at {} ms (window: {:?})",
        worst.raw_ppm,
        worst.now_ms,
        worst.window
    );
    assert_eq!(
        trace.samples.last().unwrap().raw_ppm,
        Some(0),
        "must recover when a clean window fills"
    );
}

#[test]
fn one_stalled_delivery_invalidates_old_drift_and_recovers_real_nonzero_drift() {
    let trace = delivery_trace(&[(11_500, 11_751)], 24_000, 75, false);
    assert_eq!(trace.trim_events, 1);
    let before = trace
        .samples
        .iter()
        .rfind(|sample| sample.now_ms < 11_500)
        .unwrap();
    assert!((74..=76).contains(&before.raw_ppm.unwrap()));
    assert!((74..=76).contains(&before.correction_ppm));
    let rebuilding: Vec<_> = trace
        .samples
        .iter()
        .filter(|sample| (11_600..21_750).contains(&sample.now_ms))
        .collect();
    for sample in &rebuilding {
        assert_eq!(
            sample.raw_ppm, None,
            "stale estimate at {} ms",
            sample.now_ms
        );
        assert!(sample.window.0 < 10_000);
    }
    assert_eq!(rebuilding.last().unwrap().correction_ppm, 0);
    assert!(trace
        .samples
        .windows(2)
        .all(|pair| (pair[1].correction_ppm - pair[0].correction_ppm).abs() <= 10));
    let recovered = trace.samples.last().unwrap();
    assert!((74..=76).contains(&recovered.raw_ppm.unwrap()));
    assert!((74..=76).contains(&recovered.correction_ppm));
}

#[test]
fn accepted_silence_does_not_turn_a_capture_stall_into_clock_drift() {
    let trace = delivery_trace(&[(11_000, 11_251)], 23_000, 0, true);
    let during_stall = trace
        .samples
        .iter()
        .find(|sample| sample.now_ms == 11_095)
        .unwrap();
    assert_eq!(during_stall.raw_ppm, None);
    assert!(trace
        .samples
        .iter()
        .filter_map(|sample| sample.raw_ppm)
        .all(|ppm| ppm == 0));
    assert_eq!(trace.samples.last().unwrap().raw_ppm, Some(0));
}

#[test]
fn short_delivery_batches_do_not_starve_the_drift_window() {
    let holds: Vec<_> = (5..12_000)
        .step_by(25)
        .map(|start| (start, start + 20))
        .collect();
    let trace = delivery_trace(&holds, 12_000, 0, false);
    assert_eq!(trace.trim_events, 0);
    assert!(trace
        .samples
        .iter()
        .filter(|sample| sample.now_ms >= 10_200)
        .all(|sample| sample.raw_ppm.is_some()));
}

#[test]
fn batched_platform_writes_keep_the_window_and_reveal_long_run_drift() {
    // 2052 frames is a platform buffer capacity, NOT a measured write size.
    // Stress the estimator by quantizing all accepted progress to that batch.
    const BATCH_FRAMES: u64 = 2052;
    const CAPTURE_PPM: i32 = 75;
    const PACKET_MS: u64 = 2;
    let mut media = session();
    let mut seed = 0x6d5a56dau32;
    let mut now = 0;
    let mut packet_end = PACKET_MS;
    let mut previous = None;
    let mut max_deficit = 0;
    let mut max_backlog_step = 0;
    let mut estimates = Vec::new();
    let mut calibration = audio_share_client_v2_native::drift::DriftEstimator::new(10_000, RATE);
    let mut ideal_estimates = Vec::new();
    while now < 120_000 {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        now += 80 + u64::from((seed >> 16) % 41); // 100 ms ±20 ms cadence
        while packet_end <= now {
            let start = capture_at(packet_end - PACKET_MS, CAPTURE_PPM);
            let end = capture_at(packet_end, CAPTURE_PPM);
            assert_eq!(
                media.on_datagram(
                    packet_end,
                    &datagram((packet_end / PACKET_MS - 1) as u32, start, end - start),
                ),
                DatagramOutcome::Delivered,
            );
            packet_end += PACKET_MS;
        }
        let ideal = playback_at(now.saturating_sub(u64::from(TARGET_MS)));
        let accepted = ideal / BATCH_FRAMES * BATCH_FRAMES;
        let delivered = media.delivered_frames();
        if let Some((previous_ms, previous_accepted, previous_delivered)) = previous {
            let nominal = playback_at(now - previous_ms);
            let playback_delta = accepted - previous_accepted;
            let capture_delta = delivered - previous_delivered;
            max_deficit = max_deficit.max(nominal.saturating_sub(playback_delta));
            max_backlog_step = max_backlog_step.max(u64::abs_diff(capture_delta, playback_delta));
        }
        previous = Some((now, accepted, delivered));
        media.record_playback_at(
            now,
            PlaybackReport {
                submitted_bytes: accepted * FRAME_BYTES,
                underruns: 0,
                buffer_current_ms: ((delivered - accepted) * 1_000 / u64::from(RATE)) as u32,
            },
        );
        calibration.observe(now, delivered, ideal);
        if now >= 11_000 {
            assert!(
                media.drift_window().0 >= 10_000,
                "window starved at {now} ms"
            );
            estimates.push(media.drift_ppm().expect("full window after startup"));
            ideal_estimates.push(
                calibration
                    .ppm()
                    .expect("full unquantized reference window"),
            );
        }
    }

    // The aligned unquantized control keeps the same packet and report times.
    // Its mean differs from the quantized mean by ~2 ppm for this pattern.
    // Don't turn that fixture-specific result into a per-window tolerance.
    let mean = |values: &[i32]| f64::from(values.iter().sum::<i32>()) / values.len() as f64;
    let average = mean(&estimates);
    let bias = average - mean(&ideal_estimates);
    estimates.sort_unstable();
    let min = estimates.first().unwrap();
    let median = estimates[estimates.len() / 2];
    let max = estimates.last().unwrap();
    let exceeds_b = estimates
        .iter()
        .filter(|ppm| ppm.unsigned_abs() > 100)
        .count();
    eprintln!("samples={} mean={average:.2} ppm min={min} median={median} max={max} exceeds_b={exceeds_b} calibration_bias={bias:.2} max_deficit_frames={max_deficit} max_backlog_step_frames={max_backlog_step}", estimates.len());
    assert_eq!(
        average.round() as i32,
        77,
        "expected long-run drift for this fixed sampling pattern"
    );
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
    assert_eq!(media.drift_ppm(), Some(10_000));
    assert_eq!(media.drift_correction_ppm(), 100);

    let mut media = session();
    drive(&mut media, 0, 12_000, -10_000, 0);
    assert_eq!(media.drift_ppm(), Some(-10_000));
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
