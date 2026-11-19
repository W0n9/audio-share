//! M5-04 clock drift contract (spec §8).
//!
//! Expected values come from the normative §8 rows: a 10 s rolling measurement
//! window, a ±100 ppm playback-rate correction, and a >20 ms / >5 s sustained
//! deviation that is corrected by whole-frame insertion/deletion. Every clock
//! is injected — no test sleeps.

use audio_share_client_v2_native::drift::{
    DriftConfig, DriftController, DriftEstimator, MAX_CORRECTION_PPM, RATE_SLEW_PPM_PER_UPDATE,
};

const RATE: u32 = 48_000;
const WINDOW_MS: u64 = 10_000;

fn estimator() -> DriftEstimator {
    DriftEstimator::new(WINDOW_MS, RATE)
}

/// `t` ms after start at a capture clock `rate * (1 + ppm/1e6)` frames/s,
/// while the playback clock advances at exactly `rate` frames/s. The excess
/// (`capture - playback`) therefore grows at `ppm` frames/s.
fn feed(estimator: &mut DriftEstimator, ppm: i32) {
    let step_ms = 100i128;
    let steps = (WINDOW_MS / 100) as i128;
    for step in 0..=steps {
        let now = step * step_ms;
        let capture = now * RATE as i128 * (1_000_000 + ppm as i128) / 1_000_000 / 1_000;
        let playback = now * RATE as i128 / 1_000;
        estimator.observe(now as u64, capture as u64, playback as u64);
    }
}

#[test]
fn no_estimate_until_the_ten_second_window_is_full() {
    let mut e = estimator();
    for step in 0..=90u64 {
        let now = step * 100;
        e.observe(now, now * 48, now * 48);
    }
    assert_eq!(e.ppm(), None, "9 s of history is not the §8 window");
    e.observe(WINDOW_MS, WINDOW_MS * 48, WINDOW_MS * 48);
    assert_eq!(e.ppm(), Some(0), "a matched pair of clocks is 0 ppm");
}

#[test]
fn a_capture_clock_running_fast_is_a_positive_ppm() {
    let mut e = estimator();
    feed(&mut e, 100);
    let ppm = e.ppm().expect("full window");
    assert!(
        (99..=101).contains(&ppm),
        "synthetic +100 ppm clocks must measure ~100, got {ppm}"
    );
}

#[test]
fn a_capture_clock_running_slow_is_a_negative_ppm() {
    let mut e = estimator();
    feed(&mut e, -100);
    let ppm = e.ppm().expect("full window");
    assert!(
        (-101..=-99).contains(&ppm),
        "synthetic -100 ppm clocks must measure ~-100, got {ppm}"
    );
}

#[test]
fn a_fifty_ppm_offset_is_not_rounded_to_the_hundred_ppm_clamp() {
    let mut e = estimator();
    feed(&mut e, 50);
    let ppm = e.ppm().expect("full window");
    assert!((49..=51).contains(&ppm), "got {ppm}");
}

#[test]
fn a_constant_position_offset_does_not_look_like_drift() {
    // A stream whose capture clock is permanently offset (e.g. the instance
    // anchor, or the M5-05 drop offset present from the start) is not drifting:
    // only the *rate* matters. Mid-window step resets are owned by
    // MediaSession (see `a_timestamp_rebase_restarts_the_drift_window`).
    let mut e = estimator();
    for step in 0..=100u64 {
        let now = step * 100;
        e.observe(now, now * 48 + 4_800, now * 48);
    }
    let ppm = e.ppm().expect("full window");
    assert_eq!(ppm, 0, "a constant offset must not read as clock drift");
}

#[test]
fn reset_discards_the_window() {
    let mut e = estimator();
    feed(&mut e, 100);
    assert!(e.ppm().is_some());
    e.reset();
    assert_eq!(e.ppm(), None);
}

#[test]
fn the_controller_clamps_correction_at_one_hundred_ppm() {
    let mut c = DriftController::new(DriftConfig::default());
    let mut now = 0u64;
    for _ in 0..200 {
        now += 100;
        c.update(now, Some(5_000), 60, RATE);
    }
    assert_eq!(c.applied_ppm(), MAX_CORRECTION_PPM);
    assert_eq!(c.applied_ppm(), 100);

    let mut c = DriftController::new(DriftConfig::default());
    let mut now = 0u64;
    for _ in 0..200 {
        now += 100;
        c.update(now, Some(-5_000), 60, RATE);
    }
    assert_eq!(c.applied_ppm(), -MAX_CORRECTION_PPM);
}

#[test]
fn rate_correction_is_slewed_not_stepped() {
    let mut c = DriftController::new(DriftConfig::default());
    let decision = c.update(0, Some(1_000), 60, RATE);
    assert!(
        decision.rate_ppm > 0 && decision.rate_ppm <= RATE_SLEW_PPM_PER_UPDATE,
        "one update may move by at most the slew limit, got {}",
        decision.rate_ppm
    );
    assert_eq!(decision.rate_ppm, RATE_SLEW_PPM_PER_UPDATE);
}

#[test]
fn the_controller_converges_on_an_honest_estimate() {
    let mut c = DriftController::new(DriftConfig::default());
    let mut now = 0u64;
    for _ in 0..100 {
        now += 100;
        c.update(now, Some(73), 60, RATE);
    }
    assert_eq!(c.applied_ppm(), 73, "smooth steps still converge");
}

#[test]
fn a_zero_estimate_slews_back_to_zero() {
    let mut c = DriftController::new(DriftConfig::default());
    let mut now = 0u64;
    for _ in 0..50 {
        now += 100;
        c.update(now, Some(100), 60, RATE);
    }
    assert_eq!(c.applied_ppm(), 100);
    for _ in 0..50 {
        now += 100;
        c.update(now, Some(0), 60, RATE);
    }
    assert_eq!(c.applied_ppm(), 0);
}

#[test]
fn no_frame_intervention_before_five_seconds_of_sustained_deviation() {
    let mut c = DriftController::new(DriftConfig::default());
    let mut now = 0u64;
    // 85 ms against a 60 ms target is a 25 ms deviation.
    for _ in 0..=50 {
        let decision = c.update(now, Some(0), 85, RATE);
        assert_eq!(decision.frame_adjust, 0, "at {now} ms");
        now += 100;
    }
}

#[test]
fn a_sustained_high_buffer_deletes_whole_frames_after_five_seconds() {
    let mut c = DriftController::new(DriftConfig::default());
    assert_eq!(c.update(0, Some(0), 85, RATE).frame_adjust, 0);
    assert_eq!(c.update(5_000, Some(0), 85, RATE).frame_adjust, 0);
    // "> 5 s", strictly.
    let decision = c.update(5_100, Some(0), 85, RATE);
    assert_eq!(
        decision.frame_adjust, -240,
        "25 ms - 20 ms threshold = 5 ms = 240 frames deleted"
    );
}

#[test]
fn a_sustained_low_buffer_inserts_whole_frames_after_five_seconds() {
    let mut c = DriftController::new(DriftConfig::default());
    // The buffer must first settle inside the deadband: a never-settling
    // startup fill is not drift (see the guard tests below).
    assert_eq!(c.update(0, Some(0), 60, RATE).frame_adjust, 0);
    assert_eq!(c.update(100, Some(0), 38, RATE).frame_adjust, 0);
    let decision = c.update(5_201, Some(0), 38, RATE);
    assert_eq!(
        decision.frame_adjust, 96,
        "22 ms - 20 ms threshold = 2 ms = 96 frames inserted"
    );
}

#[test]
fn a_permanently_shallow_buffer_never_inserts_silence() {
    // The application ring can legitimately sit well below the 60 ms target
    // while the AudioTrack's own buffer holds the rest. Inserting silence every
    // 5 s would be an audible dropout cadence, so insertion requires a prior
    // deadband pass; this buffer never has one.
    let mut c = DriftController::new(DriftConfig::default());
    let mut now = 0u64;
    while now <= 20_000 {
        assert_eq!(
            c.update(now, Some(0), 10, RATE).frame_adjust,
            0,
            "no insert at {now} ms"
        );
        now += 100;
    }
}

#[test]
fn insertions_require_a_drift_estimate() {
    let mut c = DriftController::new(DriftConfig::default());
    // A deadband pass alone is not enough: the §8 measurement must exist.
    assert_eq!(c.update(0, Some(0), 60, RATE).frame_adjust, 0);
    assert_eq!(c.update(100, None, 38, RATE).frame_adjust, 0);
    assert_eq!(
        c.update(5_201, None, 38, RATE).frame_adjust,
        0,
        "no silence insertion before the 10 s window fills"
    );
}

#[test]
fn a_deviation_that_recovers_resets_the_five_second_clock() {
    let mut c = DriftController::new(DriftConfig::default());
    assert_eq!(c.update(0, Some(0), 85, RATE).frame_adjust, 0);
    assert_eq!(
        c.update(4_000, Some(0), 60, RATE).frame_adjust,
        0,
        "back inside the deadband"
    );
    assert_eq!(
        c.update(7_000, Some(0), 85, RATE).frame_adjust,
        0,
        "the clock restarted at 4 s"
    );
    assert_ne!(
        c.update(12_001, Some(0), 85, RATE).frame_adjust,
        0,
        "5 s of the NEW run"
    );
}

#[test]
fn frame_interventions_are_bounded_and_rearm_the_timer() {
    let mut c = DriftController::new(DriftConfig::default());
    assert_eq!(c.update(0, Some(0), 400, RATE).frame_adjust, 0);
    let first = c.update(5_100, Some(0), 400, RATE).frame_adjust;
    assert_eq!(first, -480, "capped at 10 ms of frames");
    // Immediately after an intervention the 5 s clock must restart, or a
    // single burst would fire on every report.
    assert_eq!(c.update(5_200, Some(0), 400, RATE).frame_adjust, 0);
    assert_eq!(c.update(10_100, Some(0), 400, RATE).frame_adjust, 0);
    assert_eq!(c.update(10_201, Some(0), 400, RATE).frame_adjust, -480);
}
