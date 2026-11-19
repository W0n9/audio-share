//! M5-06 §11.2 impairment matrix — Client/host half.
//!
//! Each test is one independently reported matrix row. Everything here runs
//! on the host against the **application layer**: `MediaSession::on_datagram`
//! plus the pure `JitterBuffer`/`DriftEstimator`/`DriftController`. No real
//! network, no audio device, no sleeps. The injected session clock is the only
//! time source, and every impairment is driven by an inline splitmix64 PRNG
//! whose seed is printed (override with `M5_FUZZ_SEED`).
//!
//! Spec references: §7.5 (receiver accounting), §8 (playout, late drops,
//! concealment, drift), §11.2 (matrix).

use audio_share_client_v2_native::drift::{DriftConfig, DriftController, DriftEstimator};
use audio_share_client_v2_native::media::{MediaHeader, FLAG_STREAM_START};
use audio_share_client_v2_native::media_session::{DatagramOutcome, MediaSession, PlaybackReport};
use audio_share_client_v2_native::proto::{Configuration, PcmParameters};

mod common;
use common::Rng;

const DEFAULT_SEED: u64 = 0x4D35_0006_5EED_0001;

fn seed() -> u64 {
    common::seed(DEFAULT_SEED, "m5-06 client matrix")
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const SAMPLE_RATE: u32 = 48_000;
const FRAME_BYTES: usize = 8; // F32LE stereo

fn config(generation: u32) -> Configuration {
    Configuration {
        config_generation: generation,
        codec: 1,
        pcm: Some(PcmParameters {
            sample_rate: SAMPLE_RATE,
            channels: 2,
            sample_format: 1, // F32LE
        }),
    }
}

fn session() -> MediaSession {
    MediaSession::start(1, 1, 0, &config(1), None).expect("decodable geometry")
}

fn datagram(sequence: u32, timestamp: u64, frames: usize) -> Vec<u8> {
    let header = MediaHeader {
        stream_instance_id: 1,
        config_generation: 1,
        sequence,
        timestamp,
        flags: if sequence == 0 { FLAG_STREAM_START } else { 0 },
    };
    let mut out = Vec::with_capacity(24 + frames * FRAME_BYTES);
    header.write_be(&mut out);
    out.extend(std::iter::repeat_n(0x5au8, frames * FRAME_BYTES));
    out
}

/// Arrival order for the two reorder profiles. `seq 0` always leads (it
/// anchors the epoch); displacement never exceeds one 8-datagram block, far
/// below the 60 ms target, so every displaced packet is still timely.
fn reorder_order(count: u32, permille: u64, rng: &mut Rng) -> Vec<u32> {
    let mut order: Vec<u32> = (0..count).collect();
    if permille <= 250 {
        // 25 %: swap a fraction of adjacent pairs (displacement 1).
        let mut index = 1usize;
        while index + 1 < order.len() {
            if rng.chance(permille) {
                order.swap(index, index + 1);
                index += 2;
            } else {
                index += 1;
            }
        }
    } else {
        // 50 %: reverse blocks of eight (displacement up to 7).
        order.clear();
        order.push(0);
        let mut start = 1u32;
        while start < count {
            let end = (start + 8).min(count);
            order.extend((start..end).rev());
            start = end;
        }
    }
    order
}

/// Expected (reordered count, max displacement) for a contiguous-sequence
/// stream with no wraps: an arrival below the running high-water mark.
fn expected_reorder_stats(order: &[u32]) -> (u64, u32) {
    let mut high = order[0];
    let mut reordered = 0u64;
    let mut max_distance = 0u32;
    for &sequence in &order[1..] {
        if sequence <= high {
            reordered += 1;
            max_distance = max_distance.max(high - sequence);
        } else {
            high = sequence;
        }
    }
    (reordered, max_distance)
}

// ---------------------------------------------------------------------------
// Row 1 — 2 % independent loss
// ---------------------------------------------------------------------------

/// Matrix row 1, media half: deterministic 2 % independent loss. Gaps are
/// concealed with silence; `cumulative_lost` equals the injection exactly and
/// `late_dropped` stays zero (it is a *different* counter, §8).
///
/// Control-plane cleanliness is NOT exercised here: this test never touches
/// the control stream, and a real lossy QUIC path is out of scope on the host
/// (see the evidence `NOT RUN` entry).
#[test]
fn row1_two_percent_independent_loss_is_concealed_and_accounted() {
    let mut rng = Rng::new(seed());
    let count: u32 = 600;
    let frames = 64usize;
    let mut session = session();
    let mut dropped = 0u64;

    for sequence in 0..count {
        // Never lose the epoch anchor; keep the last 80 datagrams clean so the
        // final hole can be flushed (a trailing hole is never concealed).
        let lose = sequence > 0 && sequence < count - 80 && rng.chance(20);
        if lose {
            dropped += 1;
            continue;
        }
        let outcome = session.on_datagram(
            u64::from(sequence) * 2,
            &datagram(sequence, u64::from(sequence) * frames as u64, frames),
        );
        assert_eq!(
            outcome,
            DatagramOutcome::Delivered,
            "seq {sequence} (unique, timely)"
        );
    }

    let diagnostics = session.diagnostics(60);
    assert_eq!(
        diagnostics.cumulative_lost, dropped as i64,
        "network loss accounting must match the injected drops exactly"
    );
    assert_eq!(
        diagnostics.cumulative_late_drops, 0,
        "in-order loss never produces late drops (§8 separate counter)"
    );
    assert_eq!(
        diagnostics.cumulative_received,
        u64::from(count) - dropped,
        "only the non-injected datagrams arrived"
    );
    assert!(
        diagnostics.cumulative_concealments > 0,
        "gaps must be concealed"
    );
    assert_eq!(
        diagnostics.concealed_frames,
        (dropped * frames as u64) as i64,
        "silence covers exactly the missing frames"
    );
    assert_eq!(
        session.delivered_frames(),
        u64::from(count) * frames as u64,
        "every playout slot is written (real payload or concealment)"
    );
}

// ---------------------------------------------------------------------------
// Rows 2 and 3 — 25 % / 50 % reorder
// ---------------------------------------------------------------------------

fn reorder_row(permille: u64, label: &str) {
    let mut rng = Rng::new(seed());
    let count: u32 = 480;
    let frames = 64usize;
    let order = reorder_order(count, permille, &mut rng);
    let (expected_reordered, expected_max_distance) = expected_reorder_stats(&order);

    let mut session = session();
    for (arrival, &sequence) in order.iter().enumerate() {
        let outcome = session.on_datagram(
            arrival as u64 * 2,
            &datagram(sequence, u64::from(sequence) * frames as u64, frames),
        );
        assert_eq!(
            outcome,
            DatagramOutcome::Delivered,
            "{label}: seq {sequence} arrival {arrival}"
        );
    }

    let diagnostics = session.diagnostics(60);
    assert_eq!(
        diagnostics.cumulative_lost, 0,
        "{label}: a timely reorder is not network loss"
    );
    assert_eq!(
        diagnostics.cumulative_late_drops, 0,
        "{label}: displaced packets stay inside the target, so none are late"
    );
    assert_eq!(
        diagnostics.cumulative_reordered, expected_reordered,
        "{label}: every below-high-water arrival is counted as reordered"
    );
    assert_eq!(
        diagnostics.max_reorder_distance, expected_max_distance,
        "{label}: reorder distance is tracked"
    );
    assert_eq!(
        diagnostics.cumulative_concealments, 0,
        "{label}: timely reorder needs no concealment"
    );
    assert_eq!(
        session.delivered_frames(),
        u64::from(count) * frames as u64,
        "{label}: every frame is played"
    );
}

/// Matrix row 2: 25 % reorder.
#[test]
fn row2_twenty_five_percent_reorder_loses_nothing() {
    reorder_row(250, "25% reorder");
}

/// Matrix row 3: 50 % reorder.
#[test]
fn row3_fifty_percent_reorder_loses_nothing() {
    reorder_row(500, "50% reorder");
}

/// §8 separation proof: an arrival after its playout deadline is counted as
/// `late_dropped`, not as network loss. The missing sequence is first a
/// network gap; when it finally arrives too late it is a late drop and the
/// RFC 3550 A.3 loss term returns to zero — the two counters never merge.
#[test]
fn a_far_late_arrival_is_late_dropped_and_not_network_loss() {
    let count: u32 = 300;
    let frames = 64usize;
    let mut session = session();

    // seq 0 anchors; then omit seq 1 and send the rest in order.
    assert_eq!(
        session.on_datagram(0, &datagram(0, 0, frames)),
        DatagramOutcome::Delivered
    );
    for sequence in 2..count {
        assert_eq!(
            session.on_datagram(
                u64::from(sequence) * 2,
                &datagram(sequence, u64::from(sequence) * frames as u64, frames),
            ),
            DatagramOutcome::Delivered
        );
    }

    let before = session.diagnostics(60);
    assert_eq!(before.cumulative_lost, 1, "the hole is network loss so far");
    assert_eq!(before.cumulative_late_drops, 0, "nothing has arrived late");
    let delivered_before = session.delivered_frames();
    assert_eq!(
        delivered_before,
        u64::from(count) * frames as u64,
        "the hole was concealed, so every slot was written"
    );

    // The missing datagram arrives far past its deadline.
    assert_eq!(
        session.on_datagram(u64::from(count) * 2, &datagram(1, frames as u64, frames),),
        DatagramOutcome::LateDropped
    );

    let after = session.diagnostics(60);
    assert_eq!(
        after.cumulative_late_drops, 1,
        "late_dropped is counted independently"
    );
    assert_eq!(
        after.cumulative_lost, 0,
        "RFC 3550 A.3 reorder accounting cancels the earlier loss term"
    );
    assert_eq!(
        after.cumulative_reordered, 1,
        "the late arrival is still a reordered arrival"
    );
    assert_eq!(
        session.delivered_frames(),
        delivered_before,
        "a late datagram is never written to the ring"
    );
}

// ---------------------------------------------------------------------------
// Row 4 — 5 % duplicate
// ---------------------------------------------------------------------------

/// Matrix row 4, application half: 5 % duplicates are dropped by the
/// sequence-level dedup window and never reach the ring. QUIC packet-number
/// dedup is transport-level and NOT exercised here (recorded NOT RUN).
#[test]
fn row4_five_percent_duplicates_are_invisible_at_the_app_layer() {
    let mut rng = Rng::new(seed());
    let count: u32 = 500;
    let frames = 64usize;
    let mut session = session();
    let mut duplicates = 0u64;

    for sequence in 0..count {
        let bytes = datagram(sequence, u64::from(sequence) * frames as u64, frames);
        assert_eq!(
            session.on_datagram(u64::from(sequence) * 2, &bytes),
            DatagramOutcome::Delivered
        );
        if rng.chance(50) {
            assert_eq!(
                session.on_datagram(u64::from(sequence) * 2 + 1, &bytes),
                DatagramOutcome::Duplicate,
                "seq {sequence} duplicate"
            );
            duplicates += 1;
        }
    }

    let diagnostics = session.diagnostics(60);
    assert_eq!(
        diagnostics.cumulative_duplicates, duplicates,
        "duplicates are counted, one per injected copy"
    );
    assert_eq!(diagnostics.cumulative_lost, 0, "duplicates are not loss");
    assert_eq!(
        diagnostics.cumulative_late_drops, 0,
        "duplicates are not late drops"
    );
    assert_eq!(
        diagnostics.cumulative_concealments, 0,
        "no concealment for a duplicate"
    );
    assert_eq!(
        session.delivered_frames(),
        u64::from(count) * frames as u64,
        "the ring sees each frame exactly once"
    );
}

// ---------------------------------------------------------------------------
// Row 5 — 10 ms ± 5 ms jitter
// ---------------------------------------------------------------------------

/// Matrix row 5: arrivals are delayed by 5–15 ms (10 ms ± 5 ms) around their
/// nominal 1 ms spacing. The loop never deadlocks (all frames commit), and the
/// RFC 3550 A.8 estimate tracks the injected variation.
#[test]
fn row5_ten_plus_minus_five_ms_jitter_tracks_and_never_deadlocks() {
    let mut rng = Rng::new(seed());
    let count: u32 = 600;
    // 48 frames at 48 kHz = exactly 1 ms, so the nominal arrival clock is
    // `seq` milliseconds with no rounding noise in the control run.
    let frames = 48usize;
    let mut subject = session();

    for sequence in 0..count {
        let jitter: u64 = 5 + rng.below(11); // 5..=15 ms
        let outcome = subject.on_datagram(
            u64::from(sequence) + jitter,
            &datagram(sequence, u64::from(sequence) * frames as u64, frames),
        );
        assert_eq!(outcome, DatagramOutcome::Delivered);
    }

    let diagnostics = subject.diagnostics(60);
    assert_eq!(diagnostics.cumulative_lost, 0);
    assert_eq!(diagnostics.cumulative_late_drops, 0);
    assert_eq!(
        subject.delivered_frames(),
        u64::from(count) * frames as u64,
        "no deadlock: every frame is committed"
    );
    assert!(
        (500..=15_000).contains(&diagnostics.jitter_microseconds),
        "A.8 estimate should track a 5–15 ms arrival spread, got {} µs",
        diagnostics.jitter_microseconds
    );

    // Control: perfectly spaced arrivals report ~0 jitter.
    let mut control = session();
    for sequence in 0..count {
        let _ = control.on_datagram(
            u64::from(sequence),
            &datagram(sequence, u64::from(sequence) * frames as u64, frames),
        );
    }
    assert!(
        control.diagnostics(60).jitter_microseconds <= 1_500,
        "zero jitter must estimate near zero"
    );
}

// ---------------------------------------------------------------------------
// Row 8 — ±100 ppm clock drift
// ---------------------------------------------------------------------------

/// Estimate the ppm from simulated clocks: capture advances at the nominal
/// rate, playback at `(1 - ppm/1e6)` of it (positive ppm = capture ahead).
fn estimate_ppm(ppm: i32) -> i32 {
    let mut estimator = DriftEstimator::new(10_000, SAMPLE_RATE);
    let buffer_offset_frames = 2_880; // a 60 ms startup buffer
    let duration_ms = 30_000u64;
    for time_ms in (0..=duration_ms).step_by(100) {
        let capture = (time_ms as f64 * f64::from(SAMPLE_RATE) / 1_000.0).round() as u64;
        let playback = ((time_ms as f64 * f64::from(SAMPLE_RATE) / 1_000.0)
            * (1.0 - f64::from(ppm) / 1_000_000.0))
            .round() as u64
            + buffer_offset_frames;
        estimator.observe(time_ms, capture, playback);
    }
    estimator.ppm().expect("a full 10 s window")
}

/// Matrix row 8, host-simulated: ±100 ppm is measured, the applied playback
/// rate is clamped to ±100 ppm and slewed, the simulated buffer stays inside
/// the §8 deadband, and a sustained deviation produces a counted intervention.
#[test]
fn row8_hundred_ppm_drift_engages_rate_match_and_stays_bounded() {
    for ppm in [100i32, -100] {
        let measurement = estimate_ppm(ppm);
        assert!(
            (measurement - ppm).abs() <= 5,
            "estimated {measurement} ppm for injected {ppm} ppm"
        );

        // The controller slews to the clamped estimate and never exceeds the
        // §8 ±100 ppm bound.
        let mut controller = DriftController::new(DriftConfig::default());
        let mut decision = controller.update(0, Some(measurement), 60, SAMPLE_RATE);
        let mut buffer_ms = 60.0f64;
        let mut peak = 0i32;
        for tick in 1..=300u64 {
            let now = tick * 100;
            decision = controller.update(
                now,
                Some(measurement),
                buffer_ms.round() as u32,
                SAMPLE_RATE,
            );
            peak = peak.max(decision.rate_ppm.abs());
            // Simulated buffer: 100 ms captured, playback consumes the rate-
            // corrected amount. Bounded because the correction matches drift.
            let consumed = 100.0 * (1.0 + f64::from(decision.rate_ppm) / 1_000_000.0);
            buffer_ms += 100.0 - consumed;
            assert!(
                (buffer_ms - 60.0).abs() < 20.0,
                "buffer must stay inside the §8 deadband, got {buffer_ms} ms"
            );
            assert_eq!(
                decision.frame_adjust, 0,
                "a ±100 ppm rate error alone never needs frame surgery"
            );
        }
        assert!(
            peak <= 100,
            "rate correction must respect the ±100 ppm bound"
        );
        assert_eq!(
            decision.rate_ppm,
            measurement.clamp(-100, 100),
            "the controller converges to the clamped estimate"
        );
    }

    // A sustained deviation (buffer 200 ms, > 20 ms from target for > 5 s)
    // does produce a whole-frame correction, and acknowledging it is counted
    // as §8 concealment.
    let mut media = MediaSession::start(1, 1, 0, &config(1), None).expect("geometry");
    assert_eq!(
        media.on_datagram(0, &datagram(0, 0, 4_800)),
        DatagramOutcome::Delivered,
        "establish the capture position"
    );
    let mut signalled = 0i32;
    for tick in 0..=70u64 {
        media.record_playback_at(
            tick * 100,
            PlaybackReport {
                submitted_bytes: 0,
                underruns: 0,
                buffer_current_ms: 200,
            },
        );
        let adjustment = media.take_drift_intervention();
        if adjustment != 0 {
            signalled = adjustment;
            break;
        }
    }
    assert!(
        signalled < 0,
        "a sustained 200 ms buffer must delete frames, got {signalled}"
    );
    let before = media.diagnostics(60);
    media.record_drift_intervention(signalled);
    let after = media.diagnostics(60);
    assert_eq!(
        after.cumulative_concealments,
        before.cumulative_concealments + 1,
        "the correction is counted as a concealment intervention"
    );
    assert_eq!(
        after.concealed_frames,
        before.concealed_frames + i64::from(signalled),
        "deleted frames are accounted with the sign of the intervention"
    );
    assert!(media.drift_correction_ppm().abs() <= 100);
}

// ---------------------------------------------------------------------------
// Row 7 (client parser half) — path MTU 1280
// ---------------------------------------------------------------------------

/// Matrix row 7, client half: datagrams sized for a 1280-byte path
/// (24-byte header + frame-aligned payload) parse and deliver at the app
/// layer. DPLPMTUD re-probing and the absence of QUIC fragmentation are
/// transport concerns, recorded NOT RUN.
#[test]
fn row7_mtu_1280_datagrams_parse_and_deliver() {
    let mut session = session();
    // 1256 = 157 × 8-byte frames; exactly fills a 1280-byte datagram.
    let header = MediaHeader {
        stream_instance_id: 1,
        config_generation: 1,
        sequence: 0,
        timestamp: 0,
        flags: FLAG_STREAM_START,
    };
    let mut datagram = Vec::with_capacity(1_280);
    header.write_be(&mut datagram);
    datagram.extend(std::iter::repeat_n(0x11u8, 1_256));
    assert_eq!(datagram.len(), 1_280);
    let (parsed, payload) = MediaHeader::parse(&datagram).expect("24-byte header");
    assert_eq!(parsed.sequence, 0);
    assert_eq!(payload.len(), 1_256);
    assert_eq!(
        session.on_datagram(0, &datagram),
        DatagramOutcome::Delivered
    );
    assert_eq!(session.delivered_frames(), 157);
}
