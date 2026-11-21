//! M5-04 drift/correction wiring through the public `MediaSession` seam.
//!
//! These tests pin the §8 accounting rules at the seam the session layer, JNI
//! and the Kotlin player actually use: the clock estimate, the ±100 ppm rate
//! decision, sustained frame insert/delete (counted only when applied), and the
//! hard-limit re-anchor.

mod common;

use audio_share_client_v2_native::drift::MAX_CORRECTION_PPM;
use audio_share_client_v2_native::media::{MediaHeader, FLAG_STREAM_START};
use audio_share_client_v2_native::media_session::{DatagramOutcome, MediaSession, PlaybackReport};
use audio_share_client_v2_native::proto::{Configuration, PcmParameters};

const RATE: u32 = 48_000;
const FRAME_BYTES: u64 = 8; // stereo F32LE
const TARGET_MS: u32 = 60;
/// 48 kHz integer-Hz executor at the default §8 100 ppm ceiling: 4 Hz = 83 ppm.
const RATE_48K_ACTUATOR_LIMIT_PPM: i32 = 83;

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

/// Host-only reader/sink model, not the Kotlin loop or AudioTrack. It submits
/// delivered frames (or accepted silence in the control) and acknowledges only
/// the hard trims needed for the burst scenario through the real native seam.
fn model_delivery_to_media_session(
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
        if available > playback_at(media.jitter_config().hard_limit_ms.into()) {
            let trimmed = available - playback_at(TARGET_MS.into());
            media.record_trim(trimmed);
            read += trimmed;
            available -= trimmed;
        }
        if now > u64::from(TARGET_MS) {
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
    let trace = model_delivery_to_media_session(&[], 17_000, 0, false);
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
    let trace = model_delivery_to_media_session(&holds, 27_000, 0, false);
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
    let trace = model_delivery_to_media_session(&[(11_500, 11_751)], 24_000, 75, false);
    assert_eq!(trace.trim_events, 1);
    let before = trace
        .samples
        .iter()
        .rfind(|sample| sample.now_ms < 11_500)
        .unwrap();
    assert!((74..=76).contains(&before.raw_ppm.unwrap()));
    // This fixture's accepted-bytes clock is open-loop (the model's read loop
    // ignores `drift_correction_ppm`). Under M5-12's residual reconstruction
    // the open-loop drift is `residual + applied`, so the command correctly
    // rises toward the 48 kHz actuator cap (83 ppm) instead of stopping at 75.
    // On the real closed-loop hardware path this reconstruction yields `d`,
    // not `2d`. The stall-invalidation behaviour below is what this test pins.
    assert!(
        (74..=RATE_48K_ACTUATOR_LIMIT_PPM).contains(&before.correction_ppm),
        "before the stall: correction {}",
        before.correction_ppm
    );
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
    assert!(
        (74..=RATE_48K_ACTUATOR_LIMIT_PPM).contains(&recovered.correction_ppm),
        "after recovery: correction {}",
        recovered.correction_ppm
    );
}

#[test]
fn accepted_silence_does_not_turn_a_capture_stall_into_clock_drift() {
    let trace = model_delivery_to_media_session(&[(11_000, 11_251)], 23_000, 0, true);
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
    let trace = model_delivery_to_media_session(&holds, 12_000, 0, false);
    assert_eq!(trace.trim_events, 0);
    assert!(trace
        .samples
        .iter()
        .filter(|sample| sample.now_ms >= 10_200)
        .all(|sample| sample.raw_ppm.is_some()));
}

#[test]
fn diagnostic_interval_preserves_a_recovered_outlier_and_reports_missing_coverage() {
    let mut media = session();
    drive(&mut media, 0, 9_900, 0, 0);
    let warmup = media.take_drift_interval_at(9_900);
    assert_eq!(
        (warmup.reports, warmup.valid_estimates, warmup.extrema_ppm),
        (100, 0, None)
    );
    assert_eq!(warmup.max_report_gap_ms, 100);

    let mut captured = playback_at(9_900);
    for now in (10_000..=15_000).step_by(100) {
        let end = playback_at(now) - if now == 10_000 { playback_at(30) } else { 0 };
        media.on_datagram(now, &datagram((now / 100) as u32, captured, end - captured));
        captured = end;
        media.record_playback_at(
            now,
            PlaybackReport {
                submitted_bytes: playback_at(now) * FRAME_BYTES,
                underruns: 0,
                buffer_current_ms: TARGET_MS,
            },
        );
    }
    assert_eq!(
        media.drift_ppm(),
        Some(0),
        "last-report-only logging would miss the outlier"
    );
    let interval = media.take_drift_interval_at(15_000);
    assert_eq!((interval.start_ms, interval.end_ms), (9_900, 15_000));
    assert_eq!((interval.reports, interval.valid_estimates), (51, 51));
    assert_eq!(interval.extrema_ppm, Some((-175, 0)));
    assert_eq!(interval.max_report_gap_ms, 100);

    let empty = media.take_drift_interval_at(20_000);
    assert_eq!((empty.start_ms, empty.end_ms), (15_000, 20_000));
    assert_eq!(
        (empty.reports, empty.valid_estimates, empty.extrema_ppm),
        (0, 0, None)
    );
    assert_eq!(empty.max_report_gap_ms, 5_000);

    drive(&mut media, 20_100, 20_100, 0, 0);
    let resumed = media.take_drift_interval_at(20_100);
    assert_eq!(resumed.reports, 1);
    assert_eq!(
        resumed.max_report_gap_ms, 5_100,
        "a gap must carry across the export boundary"
    );
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
    let mut quantization_errors = Vec::new();
    let mut predicted_bias_sum = 0.0;
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
        quantization_errors.push((now, ideal - accepted));
        if now >= 11_000 {
            let (span, count) = media.drift_window();
            assert!(span >= 10_000, "window starved at {now} ms");
            estimates.push(media.drift_ppm().expect("full window after startup"));
            ideal_estimates.push(
                calibration
                    .ppm()
                    .expect("full unquantized reference window"),
            );

            // Linear regression is linear in its measured positions. Compute
            // only the extra slope contributed by the known rounding errors,
            // independently of the actual capture/playback observations.
            let window = &quantization_errors[quantization_errors.len() - count..];
            let mean_ms = window.iter().map(|&(ms, _)| ms as f64).sum::<f64>() / count as f64;
            let (covariance, variance) =
                window.iter().fold((0.0, 0.0), |(cov, var), &(ms, lost)| {
                    let dt = ms as f64 - mean_ms;
                    (cov + dt * lost as f64, var + dt * dt)
                });
            predicted_bias_sum += covariance / variance * (1_000_000_000.0 / f64::from(RATE));
        }
    }

    // The aligned unquantized control keeps the same packet and report times.
    // Its mean differs from the quantized mean by ~2 ppm for this pattern.
    // Don't turn that fixture-specific result into a per-window tolerance.
    let mean = |values: &[i32]| f64::from(values.iter().sum::<i32>()) / values.len() as f64;
    let average = mean(&estimates);
    let reference = mean(&ideal_estimates);
    let bias = average - reference;
    let predicted_bias = predicted_bias_sum / estimates.len() as f64;
    estimates.sort_unstable();
    let min = estimates.first().unwrap();
    let median = estimates[estimates.len() / 2];
    let max = estimates.last().unwrap();
    let exceeds_b = estimates
        .iter()
        .filter(|ppm| ppm.unsigned_abs() > 100)
        .count();
    eprintln!("samples={} mean={average:.2} ppm min={min} median={median} max={max} exceeds_b={exceeds_b} calibration_bias={bias:.2} max_deficit_frames={max_deficit} max_backlog_step_frames={max_backlog_step}", estimates.len());
    assert_eq!(reference.round() as i32, CAPTURE_PPM);
    // The two independently rounded integer ppm outputs can differ from the
    // analytical bias by at most 0.5 + 0.5 ppm per window, also after averaging.
    assert!(
        (bias - predicted_bias).abs() <= 1.0,
        "measured long-run bias {bias}, derived quantization bias {predicted_bias}"
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
        83,
        "the smooth controller converges on the reachable 48 kHz ceiling \
         (4 Hz = 83 ppm); §8's 100 ppm remains the upper bound"
    );
    assert!(media.drift_correction_ppm().abs() <= 100);
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
    assert_eq!(media.drift_correction_ppm(), 83);
    assert!(media.drift_correction_ppm().abs() <= 100);

    let mut media = session();
    drive(&mut media, 0, 12_000, -10_000, 0);
    assert_eq!(media.drift_ppm(), Some(-10_000));
    assert_eq!(media.drift_correction_ppm(), -83);
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

// ---------------------------------------------------------------------------
// M5-11: the playout clock source.
//
// B3 measured per-window residuals of −645…+532 ppm with a correct long-run
// mean while `submitted_position()` was built from bytes `AudioTrack.write`
// had *accepted*. The reader hands the platform a 2052-frame block at a time,
// so the accepted clock is a staircase against continuous hardware playout.
// These tests model that staircase and the hardware head at the real
// `MediaSession` report seam.
// ---------------------------------------------------------------------------

/// One synthetic media packet period. Keep the capture clock fine so the
/// accepted-bytes staircase is the only coarse input under test.
const PACKET_MS: u64 = 1;
/// The platform buffer AudioTrack adds in front of the hardware clock. It is
/// a constant delay, so it must not change a 10 s slope.
const PLATFORM_DELAY_MS: u64 = 42;

struct BatchTrace {
    estimates: Vec<i32>,
    corrections: Vec<i32>,
}

fn mean(values: &[i32]) -> f64 {
    values.iter().map(|v| f64::from(*v)).sum::<f64>() / values.len() as f64
}

/// Deterministic 100 ms ± 20 ms report cadence, independent of the estimator.
fn report_cadence(seed: &mut u32) -> u64 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    80 + u64::from((*seed >> 16) % 41)
}

/// Drive a dual-48 kHz session where the capture device runs `ppm` fast.
///
/// Media is paced in 1 ms packets, and the reader reports every 100 ± 20 ms.
/// `submitted_bytes` is the accepted media quantised down to whole
/// `batch_frames` blocks (the platform acceptance staircase). When `hardware`
/// is `Some(n)` the same report also carries the true played frame position at
/// the report instant, rounded down to `n` frames and delayed by the platform
/// buffer.
fn drive_batched_playback(
    ppm: i32,
    batch_frames: u64,
    hardware: Option<u64>,
    until_ms: u64,
) -> BatchTrace {
    let mut media = session();
    let mut seed = 0x6d5a_56da_u32;
    let mut now = 0u64;
    let mut packet_end = PACKET_MS;
    let mut next_report = 0u64;
    let mut estimates = Vec::new();
    let mut corrections = Vec::new();
    while now < until_ms {
        while packet_end <= now {
            let start = capture_at(packet_end - PACKET_MS, ppm);
            let end = capture_at(packet_end, ppm);
            assert_eq!(
                media.on_datagram(
                    packet_end,
                    &datagram((packet_end / PACKET_MS - 1) as u32, start, end - start,),
                ),
                DatagramOutcome::Delivered,
            );
            packet_end += PACKET_MS;
        }
        if now >= next_report {
            let ideal = playback_at(now.saturating_sub(u64::from(TARGET_MS)));
            let accepted = ideal / batch_frames * batch_frames;
            let head = hardware.map(|mixer_frames| {
                let played = playback_at(now.saturating_sub(PLATFORM_DELAY_MS));
                played / mixer_frames * mixer_frames
            });
            media.record_playback_with_head_at(
                now,
                PlaybackReport {
                    submitted_bytes: accepted * FRAME_BYTES,
                    underruns: 0,
                    buffer_current_ms: TARGET_MS,
                },
                head,
            );
            if let Some(estimate) = media.drift_ppm() {
                estimates.push(estimate);
                corrections.push(media.drift_correction_ppm());
            }
            next_report = now + report_cadence(&mut seed);
        }
        now += 1;
    }
    BatchTrace {
        estimates,
        corrections,
    }
}

/// RED evidence for M5-11: the accepted-bytes clock produces hundreds-of-ppm
/// per-window residuals while its long-run mean is still correct. Finer and
/// coarser acceptance blocks are included; a block coarser than the 60 ms
/// continuity budget additionally starves the window, which is recorded
/// rather than asserted away.
#[test]
fn accepted_bytes_quantised_acceptance_exceeds_the_b_drift_bound() {
    // Same fixture with exact accepted progress isolates the quantisation.
    let control = mean(&drive_batched_playback(75, 1, None, 120_000).estimates);
    for &batch in &[1_026u64, 2_052] {
        let trace = drive_batched_playback(75, batch, None, 120_000);
        assert!(
            !trace.estimates.is_empty(),
            "batch {batch}: a full 10 s window must still publish"
        );
        let exceeds = trace
            .estimates
            .iter()
            .filter(|ppm| ppm.unsigned_abs() > 100)
            .count();
        let average = mean(&trace.estimates);
        eprintln!(
            "accepted batch={batch} estimates={} exceeds_b={exceeds} mean={average:.2} control={control:.2} min={} max={}",
            trace.estimates.len(),
            trace.estimates.iter().min().unwrap(),
            trace.estimates.iter().max().unwrap(),
        );
        assert!(
            exceeds * 2 > trace.estimates.len(),
            "batch {batch}: most windows must exceed ±100 ppm, got {exceeds}/{}",
            trace.estimates.len()
        );
        assert!(
            (average - control).abs() < 10.0,
            "batch {batch}: the long-run mean must stay correct ({average:.2} vs exact-acceptance {control:.2})"
        );
    }

    // A block coarser than the 60 ms observation tolerance is a different
    // symptom of the same accepted-bytes clock: the window never survives.
    // Record it instead of pretending it produced a residual.
    let coarse = drive_batched_playback(75, 4_104, None, 120_000);
    let coarse_exceeds = coarse
        .estimates
        .iter()
        .filter(|ppm| ppm.unsigned_abs() > 100)
        .count();
    eprintln!(
        "accepted batch=4104 estimates={} exceeds_b={coarse_exceeds}",
        coarse.estimates.len()
    );
    assert!(
        coarse.estimates.is_empty() || coarse_exceeds * 2 > coarse.estimates.len(),
        "a 4104-frame accepted clock must fail B (starve the window or exceed ±100 ppm), \
         got {coarse_exceeds}/{} out of bound",
        coarse.estimates.len()
    );
}

/// The same staircase, now with the hardware playout position as the clock
/// source. Every full window must satisfy B's ±100 ppm bound and the smooth
/// correction must not sit on the ±100 rail.
///
/// The head is the played frame position at the report instant. That is what
/// `AudioTrack.getTimestamp()`'s `(framePosition, nanoTime)` pair provides
/// after interpolation, and it is integer-frame accurate. A raw
/// `getPlaybackHeadPosition()` value sampled at a *jittered* 100 ms cadence
/// carries the mixer-period quantisation; with a 240-frame (5 ms) block the
/// same fixture leaves ~43/491 windows just outside ±100 ppm. That is why the
/// production reader interpolates the frame+time pair (see MediaPlayer.kt)
/// rather than reporting a stale mixer block. `48` frames below is a 1 ms
/// quantisation robustness check.
#[test]
fn hardware_playback_head_keeps_every_batched_window_within_the_b_bound() {
    for &(batch, head_frames_at_report) in &[(480u64, 1u64), (2_052, 1), (4_104, 1), (2_052, 48)] {
        let mixer = head_frames_at_report;
        for &ppm in &[0, 75, -75] {
            let trace = drive_batched_playback(ppm, batch, Some(mixer), 60_000);
            assert!(
                trace.estimates.len() > 400,
                "batch {batch} mixer {mixer} drift {ppm}: only {} windows",
                trace.estimates.len()
            );
            let bad: Vec<i32> = trace
                .estimates
                .iter()
                .copied()
                .filter(|estimate| estimate.unsigned_abs() > 100)
                .collect();
            assert!(
                bad.is_empty(),
                "batch {batch} mixer {mixer} drift {ppm}: {} windows outside ±100 ppm: {:?}",
                bad.len(),
                bad.iter().take(8).collect::<Vec<_>>()
            );
            let average = mean(&trace.estimates);
            assert!(
                (average - f64::from(ppm)).abs() < 3.0,
                "batch {batch} mixer {mixer}: long-run mean {average:.2} != {ppm}"
            );
            assert!(
                trace
                    .corrections
                    .iter()
                    .all(|correction| correction.abs() < 100),
                "batch {batch} mixer {mixer} drift {ppm}: correction pinned at ±100"
            );
            let settled = *trace.corrections.last().expect("a published estimate");
            assert!(
                (settled - ppm).abs() <= 10,
                "batch {batch} mixer {mixer}: correction settled at {settled} ppm, not {ppm}"
            );
        }
    }
}

/// Feed the session for `from_ms..=to_ms`: 1 ms capture packets and 100 ± 20 ms
/// reports. `head_at` supplies the hardware head for each report (or `None` for
/// the accepted-bytes fallback). Returns every published estimate.
fn drive_with_head<F>(
    media: &mut MediaSession,
    from_ms: u64,
    to_ms: u64,
    ppm: i32,
    mut head_at: F,
) -> Vec<i32>
where
    F: FnMut(u64) -> Option<u64>,
{
    let mut seed = 0x1234_5678u32;
    let mut now = from_ms;
    let mut packet_end = from_ms + PACKET_MS;
    let mut next_report = from_ms;
    let mut estimates = Vec::new();
    while now <= to_ms {
        while packet_end <= now {
            let start = capture_at(packet_end - PACKET_MS, ppm);
            let end = capture_at(packet_end, ppm);
            media.on_datagram(
                packet_end,
                &datagram((packet_end / PACKET_MS - 1) as u32, start, end - start),
            );
            packet_end += PACKET_MS;
        }
        if now >= next_report {
            let accepted = playback_at(now.saturating_sub(u64::from(TARGET_MS)));
            media.record_playback_with_head_at(
                now,
                PlaybackReport {
                    submitted_bytes: accepted * FRAME_BYTES,
                    underruns: 0,
                    buffer_current_ms: TARGET_MS,
                },
                head_at(now),
            );
            if let Some(estimate) = media.drift_ppm() {
                estimates.push(estimate);
            }
            next_report = now + report_cadence(&mut seed);
        }
        now += 1;
    }
    estimates
}

/// The M5-11 accounting rule: `position_adjust_frames` and
/// `drift_playback_step` exist for the accepted-bytes clock and the §9
/// `submitted_position`. The hardware head already reflects trims and inserted
/// silence, so applying those offsets to it too would double-compensate.
#[test]
fn hardware_clock_is_not_double_compensated_by_position_offsets() {
    let ideal_head = |now: u64| Some(playback_at(now.saturating_sub(PLATFORM_DELAY_MS)));
    let mut media = session();
    let warmup = drive_with_head(&mut media, 0, 11_000, 0, ideal_head);
    assert_eq!(media.drift_clock_source(), "hardware");
    assert_eq!(media.drift_ppm(), Some(0));
    assert!(warmup.iter().all(|estimate| estimate.abs() <= 1));

    // A hard re-anchor moves the §9 accepted position forward over skipped
    // media. The hardware clock must not see that as a rate.
    let submitted_before = media.submitted_position();
    media.record_trim(4_800);
    assert_eq!(
        media.submitted_position(),
        submitted_before + 4_800,
        "§9 submitted_position keeps its accepted-bytes semantics"
    );
    let after_trim = drive_with_head(&mut media, 11_000, 14_000, 0, ideal_head);
    assert_eq!(
        media.drift_ppm(),
        Some(0),
        "a trim reset or biased the hardware window"
    );
    assert!(after_trim.iter().all(|estimate| estimate.abs() <= 1));

    // Soft trim, an accepted underrun fill and a drift insertion/deletion each
    // move the same offsets. None may appear on the hardware clock.
    media.record_soft_trim(240);
    media.record_underrun(480);
    media.record_drift_intervention(96);
    let after_offsets = drive_with_head(&mut media, 14_000, 17_000, 0, ideal_head);
    assert_eq!(media.drift_ppm(), Some(0));
    assert!(after_offsets.iter().all(|estimate| estimate.abs() <= 1));
}

/// One missing head is a transient report failure, not a reason to abandon
/// the hardware clock. Every 5 s (every ~50th 100 ms report) the window must
/// keep advancing; only a sustained outage may switch sources.
#[test]
fn periodic_single_missing_heads_keep_the_hardware_window() {
    let mut media = session();
    let mut reports = 0u64;
    let estimates = drive_with_head(&mut media, 0, 36_000, 0, |now| {
        reports += 1;
        if reports.is_multiple_of(50) {
            None
        } else {
            Some(playback_at(now.saturating_sub(PLATFORM_DELAY_MS)))
        }
    });
    // Finish on a report that has a head, so the final estimate is current.
    drive_with_head(&mut media, 36_000, 36_100, 0, |now| {
        Some(playback_at(now.saturating_sub(PLATFORM_DELAY_MS)))
    });
    assert_eq!(
        media.drift_clock_source(),
        "hardware",
        "a single miss must not switch sources"
    );
    assert!(
        media.playback_clock_stats().hardware_missing_reports >= 6,
        "every miss is counted: {:?}",
        media.playback_clock_stats()
    );
    assert_eq!(media.playback_clock_stats().accepted_fallback_reports, 0);
    assert_eq!(media.drift_ppm(), Some(0));
    assert!(
        media.drift_window().0 >= 10_000,
        "the window must keep advancing across misses: {:?}",
        media.drift_window()
    );
    assert!(estimates.iter().all(|estimate| estimate.abs() <= 1));
}

/// A sustained hardware-head outage must eventually fall back to accepted
/// bytes and start a fresh window; a short miss must not.
#[test]
fn a_sustained_hardware_head_outage_switches_to_accepted() {
    let mut media = session();
    drive_with_head(&mut media, 0, 11_000, 0, |now| {
        Some(playback_at(now.saturating_sub(PLATFORM_DELAY_MS)))
    });
    assert_eq!(media.drift_ppm(), Some(0));

    // Seven consecutive misses at the 100 ms cadence: enough for the last one
    // to be >= 500 ms after the final head report.
    for now in (11_000..=11_600).step_by(100) {
        media.record_playback_with_head_at(
            now,
            PlaybackReport {
                submitted_bytes: playback_at(now.saturating_sub(u64::from(TARGET_MS)))
                    * FRAME_BYTES,
                underruns: 0,
                buffer_current_ms: TARGET_MS,
            },
            None,
        );
    }
    assert_eq!(media.drift_clock_source(), "accepted");
    assert_eq!(
        media.playback_clock_stats().head_give_up_events,
        1,
        "exactly one sustained-outage switch"
    );
    assert_eq!(
        media.drift_ppm(),
        None,
        "the source switch starts a fresh window"
    );

    // The accepted path measures the same (zero) drift after a full window.
    let estimates = drive_with_head(&mut media, 11_700, 23_000, 0, |_| None);
    assert_eq!(media.drift_ppm(), Some(0));
    assert_eq!(media.drift_clock_source(), "accepted");
    assert_eq!(media.playback_clock_stats().head_give_up_events, 1);
    assert!(estimates.iter().all(|estimate| estimate.abs() <= 1));
}

/// An AudioTrack rebuild restarts `getPlaybackHeadPosition()` at 0. Native must
/// re-anchor the base (never move backwards or read the 0 as a real position)
/// and start a fresh window, then recover the true rate.
#[test]
fn a_rebuilt_audio_track_reanchors_the_head_and_recovers() {
    let mut media = session();
    drive_with_head(&mut media, 0, 11_000, 0, |now| {
        Some(playback_at(now.saturating_sub(PLATFORM_DELAY_MS)))
    });
    let before = media.playback_head_frames().expect("a hardware head");
    assert!(before > 500_000);

    const REBUILD_MS: u64 = 11_100;
    media.record_playback_with_head_at(
        REBUILD_MS,
        PlaybackReport {
            submitted_bytes: playback_at(REBUILD_MS - 60) * FRAME_BYTES,
            underruns: 0,
            buffer_current_ms: TARGET_MS,
        },
        Some(0),
    );
    assert_eq!(
        media.playback_clock_stats().head_reanchor_events,
        1,
        "a head restart is a re-anchor event"
    );
    assert_eq!(media.drift_clock_source(), "hardware");
    assert_eq!(
        media.drift_ppm(),
        None,
        "a rebuild starts a fresh window instead of publishing a step"
    );
    let after = media.playback_head_frames().expect("a hardware head");
    assert!(after >= before, "the effective head must stay monotonic");
    assert!(
        after - before <= playback_at(200),
        "a rebuild must not invent played frames"
    );

    // The new track's head grows from 0; the re-anchored base keeps it
    // monotonic. A fresh window must recover the real (zero) drift.
    let estimates = drive_with_head(&mut media, REBUILD_MS + 100, 23_000, 0, |now| {
        Some(playback_at(now - REBUILD_MS))
    });
    assert_eq!(media.drift_ppm(), Some(0));
    assert!(estimates.iter().all(|estimate| estimate.abs() <= 1));
    assert_eq!(media.playback_clock_stats().head_reanchor_events, 1);
}

/// The JNI contract says the head is **already extended** to a monotonic
/// frame count (Kotlin owns the 32-bit wrap). Native must therefore read *any*
/// regression as a sink rebuild — including one after the extended value has
/// passed the old 32-bit heuristics. A regression at a long-run position must
/// still re-anchor and start a fresh window, not add 2^32.
#[test]
fn an_extended_long_run_head_then_rebuild_reanchors() {
    // ~4.16e9 frames, above the old `0xF000_0000` heuristic: a rebuild here
    // used to be mistaken for a 32-bit wrap.
    const LONG_RUN_START: u64 = 0xF800_0000;
    let mut media = session();
    let first = drive_with_head(&mut media, 0, 11_000, 0, |now| {
        Some(LONG_RUN_START + playback_at(now.saturating_sub(PLATFORM_DELAY_MS)))
    });
    assert_eq!(media.drift_clock_source(), "hardware");
    assert_eq!(media.drift_ppm(), Some(0));
    assert!(first.iter().all(|estimate| estimate.abs() <= 1));
    let before = media.playback_head_frames().expect("a hardware head");
    assert!(before > LONG_RUN_START);

    const REBUILD_MS: u64 = 11_100;
    media.record_playback_with_head_at(
        REBUILD_MS,
        PlaybackReport {
            submitted_bytes: playback_at(REBUILD_MS - 60) * FRAME_BYTES,
            underruns: 0,
            buffer_current_ms: TARGET_MS,
        },
        Some(0),
    );
    assert_eq!(
        media.playback_clock_stats().head_reanchor_events,
        1,
        "an already-extended regression is a rebuild, not a 2^32 wrap"
    );
    assert_eq!(
        media.drift_ppm(),
        None,
        "a rebuild starts a fresh window instead of publishing a step"
    );
    let after = media.playback_head_frames().expect("a hardware head");
    assert!(after >= before, "the effective head must stay monotonic");
    assert!(
        after - before <= playback_at(200),
        "a rebuild must not add 2^32 frames (jumped by {})",
        after - before
    );

    // The new track's head grows from 0; the re-anchored base keeps it
    // monotonic and a fresh window recovers the real (zero) drift.
    let recovered = drive_with_head(&mut media, REBUILD_MS + 100, 23_000, 0, |now| {
        Some(playback_at(now - REBUILD_MS))
    });
    assert_eq!(media.drift_ppm(), Some(0));
    assert!(recovered.iter().all(|estimate| estimate.abs() <= 1));
    assert_eq!(media.playback_clock_stats().head_reanchor_events, 1);
}

// ---------------------------------------------------------------------------
// M5-12 through the production session wiring.
//
// The controller/estimator contract lives in `clock_drift_contract`. This test
// closes the loop at `MediaSession::record_playback_with_head_at`: the hardware
// head advances at the rate the integer-Hz executor actually reached, the
// reported position carries the 10-20 ms staircase, and the estimator therefore
// sees the residual `d - applied` exactly as on device. It exercises
// `update_from_residual`'s long-baseline branch, which every other session test
// (all <= 120 s, mostly open-loop fixtures) never reaches.
// ---------------------------------------------------------------------------

const SESSION_NOISE_PACKET_MS: u64 = 2;

fn drive_closed_loop_session(
    drift_ppm: i32,
    quantum_ms: i64,
    hold_ms: u64,
    until_ms: u64,
) -> Vec<(u64, i32)> {
    let mut media = session();
    let mut seed = 0x6d5a_56da_u32;
    let mut rng = common::NoiseRng::new(0x1234_5678);
    let quantum_frames = quantum_ms * i64::from(RATE) / 1_000;
    let mut now = 0u64;
    let mut packet_end = SESSION_NOISE_PACKET_MS;
    let mut next_report = 0u64;
    let mut next_offset_ms = 0u64;
    let mut offset_frames = 0i64;
    let mut played = 0.0f64;
    let mut last_raw = 0u64;
    let mut actual_ppm = 0i32;
    let mut samples = Vec::new();

    while now <= until_ms {
        while packet_end <= now {
            let start = capture_at(packet_end - SESSION_NOISE_PACKET_MS, drift_ppm);
            let end = capture_at(packet_end, drift_ppm);
            media.on_datagram(
                packet_end,
                &datagram(
                    (packet_end / SESSION_NOISE_PACKET_MS - 1) as u32,
                    start,
                    end - start,
                ),
            );
            packet_end += SESSION_NOISE_PACKET_MS;
        }

        // The real track plays at the last rate the platform accepted.
        played += f64::from(RATE) * (1.0 + f64::from(actual_ppm) / 1_000_000.0) / 1_000.0;

        if now >= next_report {
            if now >= next_offset_ms {
                offset_frames = rng.centred(quantum_frames / 2);
                next_offset_ms = now + hold_ms;
            }
            let raw = (played.round() as i64 + offset_frames).max(last_raw as i64) as u64;
            last_raw = raw;
            media.record_playback_with_head_at(
                now,
                PlaybackReport {
                    submitted_bytes: played.round() as u64 * FRAME_BYTES,
                    underruns: 0,
                    buffer_current_ms: TARGET_MS,
                },
                Some(raw),
            );
            let command = media.drift_correction_ppm();
            actual_ppm = common::actuator_ppm_oracle(command, MAX_CORRECTION_PPM, RATE);
            samples.push((now, command));
            next_report = now + report_cadence(&mut seed);
        }
        now += 1;
    }
    assert_eq!(
        media.drift_clock_source(),
        "hardware",
        "the M5-12 reconstruction assumes the hardware playout clock"
    );
    assert!(
        media.drift_residual_ppm().is_some(),
        "the 180 s residual baseline must be full"
    );
    samples
}

#[test]
fn closed_loop_session_does_not_saturate_and_tracks_real_drift() {
    for (drift_ppm, max_error_ppm) in [(0i32, 25.0f64), (75, 25.0), (-75, 25.0)] {
        let samples = drive_closed_loop_session(drift_ppm, 15, 2_000, 300_000);
        let measured: Vec<i32> = samples
            .iter()
            .filter(|&&(now, _)| now >= 240_000)
            .map(|&(_, command)| command)
            .collect();
        assert!(!measured.is_empty(), "a 60 s measurement window");
        let rails = measured.iter().filter(|ppm| ppm.abs() >= 100).count();
        let rail_fraction = rails as f64 / measured.len() as f64;
        let mean = measured.iter().map(|&ppm| f64::from(ppm)).sum::<f64>() / measured.len() as f64;
        eprintln!(
            "M5-12 session drift={drift_ppm:+}: rail={:.1}% mean={mean:+.1} samples={}",
            rail_fraction * 100.0,
            measured.len()
        );
        assert!(
            rail_fraction < 0.10,
            "drift {drift_ppm:+}: correction pinned at ±100 ppm for {:.1}% of reports",
            rail_fraction * 100.0
        );
        assert!(
            (mean - f64::from(drift_ppm)).abs() <= max_error_ppm,
            "drift {drift_ppm:+}: settled at {mean:+.1} ppm"
        );
        assert!(measured.iter().all(|ppm| ppm.abs() <= 100));
    }
}
