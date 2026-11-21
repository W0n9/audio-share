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
fn repeated_shared_stalls_do_not_starve_a_real_drift_estimate() {
    let mut e = estimator();
    for now in (0..=35_000u64).step_by(100) {
        // A common 61 ms pause every 5 s; capture remains +75 ppm faster
        // than playback over the observed media progress.
        let active_ms = now - now / 5_000 * 61;
        let playback = active_ms * 48;
        let capture = playback * 1_000_075 / 1_000_000;
        e.observe(now, capture, playback);
        if now >= WINDOW_MS {
            let ppm = e
                .ppm()
                .unwrap_or_else(|| panic!("periodic shared pauses starved the window at {now} ms"));
            assert!(
                (73..=76).contains(&ppm),
                "expected roughly +75 ppm at {now} ms, got {ppm}"
            );
        }
    }
}

#[test]
fn unequal_progress_during_a_shared_stall_invalidates_the_drift_window() {
    let mut e = estimator();
    feed(&mut e, 75);
    assert_eq!(e.ppm(), Some(75));
    let playback = (WINDOW_MS + 19) * 48; // 81 ms short
    let capture = (WINDOW_MS + 39) * 48 * 1_000_075 / 1_000_000; // 61 ms short
    e.observe(WINDOW_MS + 100, capture, playback);
    assert_eq!(
        e.ppm(),
        None,
        "a 20 ms relative jump during a stall is not drift"
    );
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

// ---------------------------------------------------------------------------
// M5-12: noise-driven rate-correction saturation (host noise model).
//
// Device evidence (M5-11 / B3 / 48 kHz) shows per-window estimates swinging by
// roughly ±1,000-1,800 ppm while the long-term drift is ~0, and the rate
// correction pinned at ±100 ppm for 63-79 % of snapshots. The noise is the
// 10-20 ms playout-position staircase, not white noise: the reported position
// holds an offset for ~1-2 s and then steps by one quantum. A step near a 10 s
// window edge swings the least-squares slope by hundreds to ~1,800 ppm.
//
// The fixtures are CLOSED LOOP: the playback clock advances at the rate the
// controller applied (`AudioTrack.setPlaybackRate`), so the estimator observes
// the residual drift `d - c`, exactly like the hardware head path. An
// open-loop fixture cannot see either the saturation or the half-correction
// error.
// ---------------------------------------------------------------------------

const TARGET_MS: u32 = 60;
/// Measure only after the 180 s control window has filled and settled. The
/// same window is used by the RED (10 s only) and GREEN variants so the
/// thresholds are independent literal expectations, not implementation
/// formulas.
const MEASURE_FROM_MS: u64 = 240_000;
const MEASURE_UNTIL_MS: u64 = 300_000;
const RAIL_FRACTION_MAX: f64 = 0.10;
const STEADY_ERROR_MAX_PPM: f64 = 25.0;
const NOISELESS_MAX_PPM: i32 = 5;

/// Integer-Hz playback-rate executor, mirroring `playbackRateHz` in
/// `MediaPlayer.kt`: round the ppm correction to the nearest whole Hz, then
/// clamp the Hz delta so the applied rate can never exceed the §8 ±100 ppm
/// bound (`AudioTrack.setPlaybackRate` accepts whole Hz only). At 48 kHz the
/// reachable range is ±4 Hz = ±83.3 ppm — a 20.8 ppm actuator step. The
/// device diagnostic `correction_ppm` is the pre-quantisation command; the
/// clock advances at the actual rate this returns.
fn actuator_ppm(commanded_ppm: i32) -> i32 {
    let max_delta_hz = (MAX_CORRECTION_PPM as i64 * i64::from(RATE) / 1_000_000) as i32;
    let delta_hz = (f64::from(commanded_ppm) * f64::from(RATE) / 1_000_000.0).round() as i32;
    let delta_hz = delta_hz.clamp(-max_delta_hz, max_delta_hz);
    (f64::from(delta_hz) * 1_000_000.0 / f64::from(RATE)).round() as i32
}

/// Deterministic 32-bit LCG, the same generator the existing host fixtures use.
struct NoiseRng(u32);

impl NoiseRng {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }

    /// Uniform integer in `[-bound, bound]`.
    fn centred(&mut self, bound: i64) -> i64 {
        if bound == 0 {
            return 0;
        }
        let span = 2 * bound + 1;
        i64::from(self.next() >> 16) % span - bound
    }
}

#[derive(Clone, Copy)]
struct NoiseCase {
    drift_ppm: i32,
    /// Position-measurement quantum in ms (`0` = no noise).
    quantum_ms: i32,
    /// How long one quantisation offset is held before it steps.
    hold_ms: u64,
    seed: u32,
}

#[derive(Clone, Copy)]
struct NoiseMetrics {
    rail_fraction: f64,
    mean_applied: f64,
    max_abs_applied: i32,
    raw_min: i32,
    raw_max: i32,
    samples: usize,
}

/// One control cycle is one playback report: observe the reported head, run the
/// controller, then advance both clocks by the report interval with the new
/// playback rate in effect.
struct ClosedLoop {
    estimator: DriftEstimator,
    controller: DriftController,
    drift_ppm: i32,
    quantum_frames: i64,
    hold_ms: u64,
    next_offset_ms: u64,
    offset_frames: i64,
    rng: NoiseRng,
    capture_frames: f64,
    playback_frames: f64,
    now_ms: u64,
    applied_ppm: i32,
    ticks: usize,
}

impl ClosedLoop {
    fn new(case: NoiseCase) -> Self {
        Self {
            estimator: DriftEstimator::new(WINDOW_MS, RATE),
            controller: DriftController::new(DriftConfig::default()),
            drift_ppm: case.drift_ppm,
            quantum_frames: i64::from(case.quantum_ms) * i64::from(RATE) / 1_000,
            hold_ms: case.hold_ms,
            next_offset_ms: 0,
            offset_frames: 0,
            rng: NoiseRng(case.seed),
            capture_frames: 0.0,
            playback_frames: 0.0,
            now_ms: 0,
            applied_ppm: 0,
            ticks: 0,
        }
    }

    fn step(&mut self) {
        if self.now_ms >= self.next_offset_ms {
            self.offset_frames = self.rng.centred(self.quantum_frames / 2);
            self.next_offset_ms = self.now_ms + self.hold_ms;
        }
        let capture = self.capture_frames.round() as u64;
        let playback = (self.playback_frames + self.offset_frames as f64).round() as u64;
        self.estimator.observe(self.now_ms, capture, playback);
        let estimate = self.estimator.ppm();
        let decision = self
            .controller
            .update(self.now_ms, estimate, TARGET_MS, RATE);
        self.applied_ppm = decision.rate_ppm;

        // 100 ms ± 3 ms report cadence. The jitter is what turns the
        // quantisation phase into a staircase rather than a fixed sawtooth.
        const CADENCE_MS: [u64; 8] = [100, 103, 99, 101, 100, 102, 98, 100];
        let dt = CADENCE_MS[self.ticks % CADENCE_MS.len()];
        let actual_ppm = actuator_ppm(self.applied_ppm);
        self.capture_frames +=
            dt as f64 * f64::from(RATE) * (1.0 + f64::from(self.drift_ppm) / 1_000_000.0) / 1_000.0;
        self.playback_frames +=
            dt as f64 * f64::from(RATE) * (1.0 + f64::from(actual_ppm) / 1_000_000.0) / 1_000.0;
        self.now_ms += dt;
        self.ticks += 1;
    }
}

fn run_noise_case(case: NoiseCase) -> NoiseMetrics {
    let mut plant = ClosedLoop::new(case);
    let mut applied = Vec::new();
    let mut raw = Vec::new();
    while plant.now_ms <= MEASURE_UNTIL_MS {
        plant.step();
        if plant.now_ms >= MEASURE_FROM_MS {
            applied.push(plant.applied_ppm);
            if let Some(ppm) = plant.estimator.ppm() {
                raw.push(ppm);
            }
        }
    }
    let samples = applied.len();
    let rails = applied
        .iter()
        .filter(|ppm| ppm.unsigned_abs() >= MAX_CORRECTION_PPM.unsigned_abs())
        .count();
    NoiseMetrics {
        rail_fraction: rails as f64 / samples.max(1) as f64,
        mean_applied: applied.iter().map(|&ppm| f64::from(ppm)).sum::<f64>()
            / samples.max(1) as f64,
        max_abs_applied: applied.iter().map(|ppm| ppm.abs()).max().unwrap_or(0),
        raw_min: raw.iter().copied().min().unwrap_or(0),
        raw_max: raw.iter().copied().max().unwrap_or(0),
        samples,
    }
}

/// Quantum amplitudes and dwell times chosen to put the raw 10 s estimate in
/// the device's ±1,000-1,800 ppm band (a 10-20 ms step near a window edge).
const NOISE_MATRIX: [(i32, u64); 6] = [
    (10, 1_000),
    (15, 1_000),
    (20, 1_000),
    (10, 2_000),
    (15, 2_000),
    (20, 2_000),
];
const NOISE_SEEDS: [u32; 2] = [0x6d5a_56da, 0x1234_5678];

#[test]
fn quantised_position_noise_must_not_saturate_the_rate_correction() {
    let mut failures = Vec::new();
    for &seed in &NOISE_SEEDS {
        for &(quantum_ms, hold_ms) in &NOISE_MATRIX {
            let case = NoiseCase {
                drift_ppm: 0,
                quantum_ms,
                hold_ms,
                seed,
            };
            let m = run_noise_case(case);
            eprintln!(
                "M5-12 drift=0 q={quantum_ms}ms hold={hold_ms}ms seed={seed:#x}: \
                 rail={:.1}% mean_applied={:+.1} max|applied|={} raw=[{},{}] samples={}",
                m.rail_fraction * 100.0,
                m.mean_applied,
                m.max_abs_applied,
                m.raw_min,
                m.raw_max,
                m.samples,
            );
            if m.rail_fraction >= RAIL_FRACTION_MAX {
                failures.push(format!(
                    "noise alone pinned the correction at the ±100 ppm rail \
                     (q={quantum_ms}ms hold={hold_ms}ms seed={seed:#x}: {:.1}%)",
                    m.rail_fraction * 100.0
                ));
            }
            if m.mean_applied.abs() > STEADY_ERROR_MAX_PPM {
                failures.push(format!(
                    "zero true drift commanded a sustained correction \
                     (q={quantum_ms}ms hold={hold_ms}ms seed={seed:#x}: {:.1} ppm)",
                    m.mean_applied
                ));
            }
            if m.max_abs_applied > MAX_CORRECTION_PPM {
                failures.push(format!(
                    "the §8 ±100 ppm bound was exceeded (q={quantum_ms}ms hold={hold_ms}ms \
                     seed={seed:#x}: {})",
                    m.max_abs_applied
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "M5-12 noise-only control failures:\n{}",
        failures.join("\n")
    );
}

#[test]
fn a_real_seventy_five_ppm_drift_is_tracked_without_long_term_saturation() {
    let mut failures = Vec::new();
    for &seed in &NOISE_SEEDS {
        for &drift_ppm in &[75, -75] {
            for &(quantum_ms, hold_ms) in &NOISE_MATRIX {
                let case = NoiseCase {
                    drift_ppm,
                    quantum_ms,
                    hold_ms,
                    seed,
                };
                let m = run_noise_case(case);
                eprintln!(
                    "M5-12 drift={drift_ppm:+} q={quantum_ms}ms hold={hold_ms}ms seed={seed:#x}: \
                     rail={:.1}% mean_applied={:+.1} err={:+.1} raw=[{},{}]",
                    m.rail_fraction * 100.0,
                    m.mean_applied,
                    m.mean_applied - f64::from(drift_ppm),
                    m.raw_min,
                    m.raw_max,
                );
                if m.rail_fraction >= RAIL_FRACTION_MAX {
                    failures.push(format!(
                        "a real {drift_ppm:+} ppm drift sat on the rail \
                         (q={quantum_ms}ms hold={hold_ms}ms seed={seed:#x}: {:.1}%)",
                        m.rail_fraction * 100.0
                    ));
                }
                if (m.mean_applied - f64::from(drift_ppm)).abs() > STEADY_ERROR_MAX_PPM {
                    failures.push(format!(
                        "the correction did not track a real {drift_ppm:+} ppm drift \
                         (q={quantum_ms}ms hold={hold_ms}ms seed={seed:#x}: settled {:.1} ppm)",
                        m.mean_applied
                    ));
                }
                if m.max_abs_applied > MAX_CORRECTION_PPM {
                    failures.push(format!(
                        "the §8 ±100 ppm bound was exceeded (q={quantum_ms}ms \
                         hold={hold_ms}ms seed={seed:#x}: {})",
                        m.max_abs_applied
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "M5-12 real-drift control failures:\n{}",
        failures.join("\n")
    );
}

#[test]
fn a_noiseless_zero_drift_loop_settles_near_zero() {
    let m = run_noise_case(NoiseCase {
        drift_ppm: 0,
        quantum_ms: 0,
        hold_ms: 1_000,
        seed: 0x6d5a_56da,
    });
    eprintln!(
        "M5-12 noiseless drift=0: mean_applied={:+.1} max|applied|={}",
        m.mean_applied, m.max_abs_applied
    );
    assert!(
        m.mean_applied.abs() <= f64::from(NOISELESS_MAX_PPM),
        "without measurement noise the correction must stay near zero, got {:.1} ppm",
        m.mean_applied
    );
}

#[test]
fn the_integer_hz_executor_cannot_command_more_than_the_ppm_bound() {
    // 48 kHz: a whole-Hz actuator reaches 4 Hz (83.3 ppm), not 5 Hz
    // (104.2 ppm). Rounding must never push || over the §8 bound.
    assert_eq!(actuator_ppm(0), 0);
    assert_eq!(actuator_ppm(100), 83, "4 Hz, not 104.2 ppm");
    assert_eq!(actuator_ppm(-100), -83);
    assert_eq!(actuator_ppm(75), 83, "3.6 Hz rounds to 4 Hz");
    for commanded in [-10_000, -100, -75, -1, 0, 1, 75, 100, 10_000] {
        let actual = actuator_ppm(commanded);
        assert!(
            actual.abs() <= MAX_CORRECTION_PPM,
            "commanded {commanded} ppm landed at {actual} ppm, beyond §8"
        );
    }
}
