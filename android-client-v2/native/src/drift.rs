//! Capture/playback clock drift measurement and correction (M5-04; spec §8).
//!
//! Two independent sample clocks (the capture device and the Android audio
//! device) never run at exactly the same rate. Over a long playback session the
//! difference shows up as the jitter buffer slowly growing (playback clock slow)
//! or draining into underruns (playback clock fast). §8 breaks the fix into:
//!
//! 1. **Measurement** — capture clock vs playback clock over a **10 s rolling
//!    window**.
//! 2. **Rate correction** — `AudioTrack` playback-rate adjustment of at most
//!    **±100 ppm**, smooth (slewed), never frame-counted.
//! 3. **Sustained deviation** — when the buffer stays more than **20 ms** away
//!    from target for more than **5 s** despite rate matching, insert or delete
//!    whole sample frames and count the intervention as concealment.
//!
//! Everything here is pure and clock-injected: the caller passes `now_ms` on
//! every update, so the behaviour is deterministic in host tests with no
//! sleeping.
//!
//! ## Clocks
//!
//! The *capture clock* is the wire `timestamp` (a frame position from the
//! Sender's capture device). The *playback clock* is `submitted_position`: the
//! frame position the platform playback API has actually accepted (§9). Both
//! are capture-clock positions, so their difference is the buffered production
//! backlog; its slope over the window is the rate mismatch in ppm.
//!
//! `MediaSession` removes known frame-position steps before observing the
//! clocks, so a trim or timestamp re-base alone does not break the window.
//! Even after that accounting, a stall/catch-up can jump the measured backlog:
//! arrival and platform acceptance measure progress, not the hardware clocks
//! directly. Do not fit a single rate across such a discontinuity.

use std::collections::VecDeque;

/// §8 measurement window.
pub const DRIFT_WINDOW_MS: u64 = 10_000;
/// A full window with only a couple of samples is still a short/quantised
/// baseline: a single ~10 ms playout step at a window edge can swing the slope
/// by hundreds of ppm. Publish only when at least this many paired reports
/// span the window (1 Hz reporting would take 10 s; the normal 10 Hz cadence
/// needs ~1 s of the window's tail, so this never delays a genuine estimate).
pub const MIN_WINDOW_SAMPLES: usize = 10;
/// Allow a playout target's worth of short-term batching when comparing clock
/// progress between reports. A large unmatched deficit or a step in the
/// difference beyond 60 ms is not reliable clock progress. This is a window
/// continuity budget after known position jumps are removed, not a ppm limit
/// or the reader's trim policy. Even 10,000 ppm moves the clocks apart by
/// just 1 ms per typical 100 ms report.
const OBSERVATION_TOLERANCE_MS: u64 = 60;
/// §8 rate-correction bound.
pub const MAX_CORRECTION_PPM: i32 = 100;
/// §8 sustained-deviation threshold.
pub const DEVIATION_THRESHOLD_MS: i32 = 20;
/// §8 sustained-deviation duration ("for > 5 s").
pub const SUSTAINED_DEVIATION_MS: u64 = 5_000;

/// Smoothing: at most this many ppm of correction change per controller
/// update. Playback reports arrive ~10×/s, so the full ±100 ppm range takes
/// about a second to ramp — smooth, not an audible frame-counted jump.
pub const RATE_SLEW_PPM_PER_UPDATE: i32 = 10;

/// A frame intervention is one bounded chunk (10 ms at the active rate), so a
/// single trigger can never produce a large audible hole.
pub const MAX_FRAME_ADJUST_MS: i32 = 10;

/// M5-12: long baseline for the residual slope that drives the rate
/// controller (the §8 10 s window remains the diagnostic measurement). A 10 s
/// least-squares slope over a playout position quantised to 10–20 ms swings by
/// more than 1,000 ppm when one step lands near a window edge, so the
/// controller must not treat it as a rate difference. A three-minute baseline
/// bounds the worst-case un-corrected backlog to ~18 ms at the §8 ±100 ppm
/// limit while averaging the quantisation staircase down to a few ppm.
///
/// This is the ticket-authorized "longer integration"; §8 itself specifies
/// only the 10 s *measurement* window, which is unchanged.
pub const RESIDUAL_BASELINE_MS: u64 = 180_000;

/// The single derivation of the residual baseline from a configured
/// diagnostic window. Both the estimator and the controller use this, so the
/// window whose slope is reconstructed and the window the controller averages
/// its own applied rate over cannot diverge.
pub const fn residual_baseline_ms(window_ms: u64) -> u64 {
    if window_ms > RESIDUAL_BASELINE_MS {
        window_ms
    } else {
        RESIDUAL_BASELINE_MS
    }
}

/// M5-12: damping on the reconstructed open-loop drift. Reconstructing
/// `d = residual + mean(applied)` creates a delayed feedback path; a 20 s
/// first-order filter removes the residual low-frequency wander without
/// adding material lag to a quasi-static clock rate.
pub const RESIDUAL_SMOOTHING_MS: u64 = 20_000;

/// One paired observation of the two clocks at a session-clock instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DriftSample {
    now_ms: u64,
    /// Latest capture-clock position (wire `timestamp` + frames).
    capture_frames: u64,
    /// Platform-accepted playback position on the same capture clock.
    playback_frames: u64,
}

/// 10 s rolling-window ppm estimator.
///
/// The estimate is the least-squares slope of `capture − playback` (frames)
/// against session time, converted to ppm at the published sample rate. A
/// position offset (anchor or a drop) shifts the intercept only; it does not
/// change the slope.
pub struct DriftEstimator {
    window_ms: u64,
    /// M5-12 long residual baseline. Always at least `window_ms` and
    /// [`RESIDUAL_BASELINE_MS`]; derived by [`residual_baseline_ms`].
    residual_baseline_ms: u64,
    sample_rate: u32,
    samples: VecDeque<DriftSample>,
    /// The same observations over the longer residual baseline. Kept separate
    /// so [`Self::ppm`] (the §8 diagnostic) keeps its exact 10 s semantics.
    residual_samples: VecDeque<DriftSample>,
    /// Number of rejected clock intervals, not number of stall episodes.
    discontinuous_intervals: u64,
}

impl DriftEstimator {
    pub fn new(window_ms: u64, sample_rate: u32) -> Self {
        let window_ms = window_ms.max(1);
        Self {
            window_ms,
            residual_baseline_ms: residual_baseline_ms(window_ms),
            sample_rate: sample_rate.max(1),
            samples: VecDeque::new(),
            residual_samples: VecDeque::new(),
            discontinuous_intervals: 0,
        }
    }

    /// The long baseline used by [`Self::residual_ppm`]. Exposed so the
    /// controller can assert it averages over the same span.
    pub const fn residual_baseline_ms(&self) -> u64 {
        self.residual_baseline_ms
    }

    /// Discard both observation windows.
    pub fn reset(&mut self) {
        self.samples.clear();
        self.residual_samples.clear();
    }

    /// Fold one (session time, capture position, playback position) sample.
    /// A stalled observation or backlog discontinuity discards the old window.
    pub fn observe(&mut self, now_ms: u64, capture_frames: u64, playback_frames: u64) {
        let sample = DriftSample {
            now_ms,
            capture_frames,
            playback_frames,
        };
        if self
            .samples
            .back()
            .is_some_and(|last| self.discontinuous(*last, sample))
        {
            self.reset();
            self.discontinuous_intervals = self.discontinuous_intervals.saturating_add(1);
        }
        match self.samples.back() {
            // Ignore a non-advancing duplicate instant; keep the newest value.
            Some(last) if now_ms <= last.now_ms => {
                if let Some(back) = self.samples.back_mut() {
                    *back = sample;
                }
                if let Some(back) = self.residual_samples.back_mut() {
                    *back = sample;
                }
                return;
            }
            _ => {}
        }
        // Keep ONE sample at or before each cutoff. Evicting every sample older
        // than the cutoff leaves the retained span strictly shorter than the
        // window whenever arrivals are jittered (`front > cutoff`), and `ppm()`
        // then never returns `Some`. Retaining the latest sample at/before the
        // cutoff guarantees `last - front >= window_ms` while still bounding
        // the window to one extra cadence interval. The two windows are pushed
        // and cleared together, so they always share the same newest sample.
        Self::push_and_trim(&mut self.samples, self.window_ms, sample);
        Self::push_and_trim(
            &mut self.residual_samples,
            self.residual_baseline_ms,
            sample,
        );
    }

    fn push_and_trim(samples: &mut VecDeque<DriftSample>, window_ms: u64, sample: DriftSample) {
        samples.push_back(sample);
        let cutoff = sample.now_ms.saturating_sub(window_ms);
        while samples.len() > 1 && samples[1].now_ms <= cutoff {
            samples.pop_front();
        }
    }

    fn discontinuous(&self, last: DriftSample, sample: DriftSample) -> bool {
        let capture = (i128::from(sample.capture_frames) - i128::from(last.capture_frames)) * 1_000;
        let playback =
            (i128::from(sample.playback_frames) - i128::from(last.playback_frames)) * 1_000;
        let rate = i128::from(self.sample_rate);
        let nominal = i128::from(sample.now_ms.saturating_sub(last.now_ms)) * rate;
        let tolerance = rate * i128::from(OBSERVATION_TOLERANCE_MS);
        let relative_step = (capture - playback).abs();
        // A shared pause need not invalidate the relative clock observation.
        // When either side is stalled, tolerate only one nominal millisecond
        // of skew (the observation clock is sampled in whole milliseconds).
        relative_step > tolerance
            || (nominal - capture.min(playback) > tolerance && relative_step > rate)
    }

    /// Rejected adjacent observation intervals for this measurement series.
    /// A sustained stall can reject multiple consecutive intervals.
    pub(crate) const fn discontinuous_intervals(&self) -> u64 {
        self.discontinuous_intervals
    }

    /// The measured drift in ppm, or `None` until a full `window_ms` of paired
    /// observations with at least [`MIN_WINDOW_SAMPLES`] samples exists.
    /// Positive means the capture clock is ahead of the playback clock (the
    /// buffer is growing; playback must speed up).
    ///
    /// This is the §8 10 s diagnostic and is deliberately **not** clamped or
    /// smoothed (M5-12). The rate controller uses [`Self::residual_ppm`].
    pub fn ppm(&self) -> Option<i32> {
        Self::window_ppm(&self.samples, self.window_ms, self.sample_rate)
    }

    /// M5-12: the drift measured over the longer [`RESIDUAL_BASELINE_MS`]
    /// baseline, used to drive the rate controller.
    ///
    /// A 10 s slope over a 10–20 ms-quantised playout position has a slope
    /// error of order `q / T` (≈1,000–1,800 ppm), which is larger than the
    /// whole ±100 ppm correction range. Over three minutes the same
    /// quantisation contributes only a few ppm. This is still the *residual*
    /// drift `d - applied`; [`DriftController::update_from_residual`]
    /// reconstructs the open-loop drift before smoothing.
    ///
    /// `None` until the residual baseline is full (or after a reset).
    pub fn residual_ppm(&self) -> Option<i32> {
        Self::window_ppm(
            &self.residual_samples,
            self.residual_baseline_ms,
            self.sample_rate,
        )
    }

    /// Least-squares slope of `capture − playback` over one retained window.
    fn window_ppm(
        samples: &VecDeque<DriftSample>,
        window_ms: u64,
        sample_rate: u32,
    ) -> Option<i32> {
        let first = samples.front()?;
        let last = samples.back()?;
        if last.now_ms.saturating_sub(first.now_ms) < window_ms {
            return None;
        }
        if samples.len() < MIN_WINDOW_SAMPLES {
            return None;
        }
        // Without playback progress the "slope" is just the capture rate, not
        // a rate mismatch.
        if last.playback_frames <= first.playback_frames
            || last.capture_frames <= first.capture_frames
        {
            return None;
        }
        let n = samples.len() as f64;
        let mean_t = samples.iter().map(|s| s.now_ms as f64).sum::<f64>() / n;
        let mean_e = samples
            .iter()
            .map(|s| s.capture_frames as f64 - s.playback_frames as f64)
            .sum::<f64>()
            / n;
        let mut covariance = 0.0f64;
        let mut variance = 0.0f64;
        for sample in samples {
            let dt = sample.now_ms as f64 - mean_t;
            let de = (sample.capture_frames as f64 - sample.playback_frames as f64) - mean_e;
            covariance += dt * de;
            variance += dt * dt;
        }
        if variance <= 0.0 || !covariance.is_finite() {
            return None;
        }
        // frames per ms → ppm: slope * 1000 frames/s / sample_rate * 1e6.
        let ppm = covariance / variance * 1_000_000_000.0 / f64::from(sample_rate);
        if !ppm.is_finite() {
            return None;
        }
        Some(ppm.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32)
    }

    /// The retained window's span in ms and its sample count. Logged with every
    /// `playback.drift` line so a short/quantised estimate is recognisable
    /// (M5-08).
    pub fn window_stats(&self) -> (u64, usize) {
        let span_ms = match (self.samples.front(), self.samples.back()) {
            (Some(first), Some(last)) => last.now_ms.saturating_sub(first.now_ms),
            _ => 0,
        };
        (span_ms, self.samples.len())
    }
}

/// Tunables for measurement + correction. Defaults are the §8 values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriftConfig {
    pub window_ms: u64,
    pub max_correction_ppm: i32,
    pub deviation_threshold_ms: i32,
    pub sustained_ms: u64,
    pub slew_ppm_per_update: i32,
    /// The §8 jitter-buffer target, used to derive the buffer error.
    pub target_ms: u32,
}

impl Default for DriftConfig {
    fn default() -> Self {
        Self {
            window_ms: DRIFT_WINDOW_MS,
            max_correction_ppm: MAX_CORRECTION_PPM,
            deviation_threshold_ms: DEVIATION_THRESHOLD_MS,
            sustained_ms: SUSTAINED_DEVIATION_MS,
            slew_ppm_per_update: RATE_SLEW_PPM_PER_UPDATE,
            target_ms: 60,
        }
    }
}

/// What the controller wants the playback path to do this update.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DriftDecision {
    /// Playback-rate correction in ppm, already clamped and slewed. Always the
    /// full current value (not a delta), so the caller applies it idempotently.
    pub rate_ppm: i32,
    /// Whole-frame intervention: positive = insert silence, negative = delete.
    /// Zero means none.
    pub frame_adjust: i32,
}

/// Smooth ±100 ppm rate controller plus the §8 sustained-deviation monitor.
pub struct DriftController {
    config: DriftConfig,
    applied_ppm: i32,
    /// `now_ms` at which the current out-of-deadband run began.
    deviation_since_ms: Option<u64>,
    /// Whether the buffer has been inside the deadband since the last reset.
    /// Gates silence insertion (see `update`).
    seen_deadband: bool,
    /// M5-12: the rate the executor *actually* applied, over the control
    /// window. Used to reconstruct the open-loop drift from the residual
    /// measurement (`residual = d - applied`).
    applied_history: VecDeque<(u64, i32)>,
    /// M5-12: damped reconstruction of the open-loop drift.
    smoothed_open_loop: Option<f64>,
    last_control_update_ms: Option<u64>,
}

impl DriftController {
    pub fn new(config: DriftConfig) -> Self {
        Self {
            config,
            applied_ppm: 0,
            deviation_since_ms: None,
            seen_deadband: false,
            applied_history: VecDeque::new(),
            smoothed_open_loop: None,
            last_control_update_ms: None,
        }
    }

    pub const fn config(&self) -> DriftConfig {
        self.config
    }

    /// The rate currently commanded, in ppm. Read it through JNI for
    /// `AudioTrack.setPlaybackRate`.
    pub const fn applied_ppm(&self) -> i32 {
        self.applied_ppm
    }

    /// The span whose applied-rate mean is reconstructed. Derived by the same
    /// function as [`DriftEstimator::residual_baseline_ms`], so the two cannot
    /// diverge for a non-default diagnostic window.
    pub const fn residual_baseline_ms(&self) -> u64 {
        residual_baseline_ms(self.config.window_ms)
    }

    /// The largest ppm correction the integer-Hz executor can actually reach
    /// for this configuration. This is a *reachability* limit, not a change to
    /// the §8 ±`max_correction_ppm` upper bound.
    fn reachable_max_ppm(&self, sample_rate: u32) -> i32 {
        actuator_ppm(
            self.config.max_correction_ppm,
            self.config.max_correction_ppm,
            sample_rate,
        )
    }

    /// Move the command toward `target_ppm` by at most one §8 slew step.
    fn slew_toward(&mut self, target_ppm: i32) {
        let delta = (target_ppm - self.applied_ppm).clamp(
            -self.config.slew_ppm_per_update,
            self.config.slew_ppm_per_update,
        );
        self.applied_ppm += delta;
    }

    /// Forget the smoothing state and the sustained-deviation run. Used on a
    /// stream/configuration change.
    pub fn reset(&mut self) {
        self.applied_ppm = 0;
        self.deviation_since_ms = None;
        self.seen_deadband = false;
        self.applied_history.clear();
        self.smoothed_open_loop = None;
        self.last_control_update_ms = None;
    }

    /// Advance the controller treating `estimate` as the **absolute** rate
    /// correction (open-loop semantics).
    ///
    /// Kept for callers/tests that supply an already-open-loop correction.
    /// Production (`MediaSession`) uses [`Self::update_from_residual`], because
    /// the hardware playout clock reflects the correction already and its
    /// measurement is the *residual* drift.
    pub fn update(
        &mut self,
        now_ms: u64,
        estimate: Option<i32>,
        buffer_current_ms: u32,
        sample_rate: u32,
    ) -> DriftDecision {
        let reachable = self.reachable_max_ppm(sample_rate);
        let target = estimate.unwrap_or(0).clamp(-reachable, reachable);
        self.slew_toward(target);
        self.sustained_decision(now_ms, estimate, buffer_current_ms, sample_rate)
    }

    /// M5-12: advance the controller from the *residual* drift measurement.
    ///
    /// `measurement` is the raw 10 s diagnostic (used only for the §8
    /// sustained-deviation gate, exactly as before). `residual` is the
    /// long-baseline residual `d - applied` from
    /// [`DriftEstimator::residual_ppm`]:
    ///
    /// 1. reconstruct the open-loop drift as `residual + mean(applied)`, where
    ///    the mean is over the same residual baseline and uses what the
    ///    integer-Hz executor actually applied;
    /// 2. damp that reconstruction with a [`RESIDUAL_SMOOTHING_MS`] low-pass;
    /// 3. slew the command toward it. The §8 ±`max_correction_ppm` *upper
    ///    bound* is unchanged; the target is additionally capped to what the
    ///    integer-Hz executor can reach.
    ///
    /// While the baseline is filling (or rebuilding after a discontinuity) the
    /// controller uses the **same residual semantics** on the 10 s
    /// `measurement` instead of the old absolute-target fallback. That avoids
    /// the systematic `d/2` under-correction during warm-up; the 10 s noise can
    /// still drive the rail there. The applied history is recorded on both
    /// sides, so in the closed loop `residual + mean(applied) = d` holds across
    /// the hand-over.
    pub fn update_from_residual(
        &mut self,
        now_ms: u64,
        measurement: Option<i32>,
        residual: Option<i32>,
        buffer_current_ms: u32,
        sample_rate: u32,
    ) -> DriftDecision {
        if residual.is_none() {
            // The long baseline was reset or is rebuilding. Never blend the
            // rebuilt estimate with the previous generation's damped value.
            self.smoothed_open_loop = None;
            self.last_control_update_ms = None;
        }
        let reachable = self.reachable_max_ppm(sample_rate);
        let target = match residual {
            Some(residual) => {
                // The residual and the applied mean must cover the *same*
                // span; the long baseline is both.
                let mean_applied = {
                    self.record_applied(now_ms, sample_rate);
                    self.applied_mean(now_ms, self.residual_baseline_ms())
                };
                let open_loop = residual.saturating_add(mean_applied);
                let smoothed = self.smooth_open_loop(now_ms, open_loop);
                smoothed
                    .round()
                    .clamp(-f64::from(reachable), f64::from(reachable)) as i32
            }
            // Warm-up/rebuild: the same residual reconstruction, but on the
            // short measurement. The applied mean must cover that same short
            // span; using the 180 s mean here would under-count the recent
            // correction and leave a systematic warm-up error.
            //
            // Never fall back to treating the residual as an absolute target
            // (`applied = d - applied` would settle at d/2). With no
            // measurement at all (a rebuild) the old behaviour is kept: the
            // rate slews back to zero until fresh evidence exists.
            None => {
                self.record_applied(now_ms, sample_rate);
                measurement.map_or(0, |ppm| {
                    let mean_applied = self.applied_mean(now_ms, self.config.window_ms);
                    ppm.saturating_add(mean_applied)
                        .clamp(-reachable, reachable)
                })
            }
        };
        self.slew_toward(target);
        self.sustained_decision(now_ms, measurement, buffer_current_ms, sample_rate)
    }

    /// Record the rate in effect since the previous update.
    ///
    /// The estimator measures `capture - playback`, so after correction it
    /// reports `d - applied`, not `d`. Adding the mean applied correction over
    /// the same window recovers the open-loop drift. The integer-Hz executor is
    /// included so the reconstruction uses the rate the platform actually
    /// reached, not the pre-quantisation command.
    fn record_applied(&mut self, now_ms: u64, sample_rate: u32) {
        let actual = actuator_ppm(
            self.applied_ppm,
            self.config.max_correction_ppm,
            sample_rate,
        );
        self.applied_history.push_back((now_ms, actual));
        let cutoff = now_ms.saturating_sub(self.residual_baseline_ms());
        while self.applied_history.len() > 1 {
            let (at_ms, _) = *self.applied_history.front().expect("non-empty");
            if at_ms > cutoff {
                break;
            }
            self.applied_history.pop_front();
        }
    }

    /// Mean of the actually-applied rate over the trailing `window_ms`.
    /// `record_applied` keeps enough history for both the diagnostic window
    /// (warm-up) and the residual baseline.
    fn applied_mean(&self, now_ms: u64, window_ms: u64) -> i32 {
        let cutoff = now_ms.saturating_sub(window_ms);
        let mut sum = 0i64;
        let mut count = 0i64;
        for &(at_ms, ppm) in &self.applied_history {
            if at_ms >= cutoff {
                sum += i64::from(ppm);
                count += 1;
            }
        }
        if count == 0 {
            0
        } else {
            (sum / count) as i32
        }
    }

    /// First-order damping of the reconstructed drift. Seeded from the first
    /// sample; [`Self::update_from_residual`] clears it whenever the residual
    /// input is absent, so a rebuilt window is never filtered against stale
    /// history.
    fn smooth_open_loop(&mut self, now_ms: u64, open_loop_ppm: i32) -> f64 {
        let value = f64::from(open_loop_ppm);
        let smoothed = match (self.smoothed_open_loop, self.last_control_update_ms) {
            (Some(previous), Some(last_ms)) => {
                let dt_ms = now_ms.saturating_sub(last_ms) as f64;
                let alpha = 1.0 - (-dt_ms / RESIDUAL_SMOOTHING_MS as f64).exp();
                previous + alpha * (value - previous)
            }
            _ => value,
        };
        self.smoothed_open_loop = Some(smoothed);
        self.last_control_update_ms = Some(now_ms);
        smoothed
    }

    /// §8 sustained buffer deviation → whole-frame insert/delete.
    ///
    /// Insertion adds silence at the playout point and can be audible, so it
    /// is only allowed once the buffer has actually settled inside the
    /// deadband (a startup fill or a permanently shallow application ring is
    /// not drift) AND a real drift measurement exists. Deletion only reduces
    /// latency and is safe whenever the buffer is genuinely too deep.
    fn sustained_decision(
        &mut self,
        now_ms: u64,
        measurement: Option<i32>,
        buffer_current_ms: u32,
        sample_rate: u32,
    ) -> DriftDecision {
        let error_ms = buffer_current_ms as i32 - self.config.target_ms as i32;
        let mut frame_adjust = 0i32;
        if error_ms.abs() <= self.config.deviation_threshold_ms {
            self.seen_deadband = true;
            self.deviation_since_ms = None;
        } else if self.deviation_since_ms.is_none() {
            self.deviation_since_ms = Some(now_ms);
        } else if now_ms.saturating_sub(self.deviation_since_ms.unwrap()) > self.config.sustained_ms
        {
            let insert_allowed = self.seen_deadband && measurement.is_some();
            if error_ms > 0 || insert_allowed {
                frame_adjust = Self::frames_for(error_ms, sample_rate, self.config);
            }
            // Re-arm: the correction needs another full 5 s run.
            self.deviation_since_ms = None;
        }

        DriftDecision {
            rate_ppm: self.applied_ppm,
            frame_adjust,
        }
    }

    /// Correct the excess over the deadband, bounded to one 10 ms chunk.
    fn frames_for(error_ms: i32, sample_rate: u32, config: DriftConfig) -> i32 {
        let excess_ms = error_ms.abs().saturating_sub(config.deviation_threshold_ms);
        let max_frames = sample_rate as i32 * MAX_FRAME_ADJUST_MS / 1_000;
        let frames = (sample_rate as i32 * excess_ms / 1_000).clamp(1, max_frames.max(1));
        // Too much buffered media → delete; too little → insert.
        if error_ms > 0 {
            -frames
        } else {
            frames
        }
    }
}

/// The ppm rate `AudioTrack.setPlaybackRate` can actually reach for a
/// configured correction ceiling.
///
/// The platform API accepts whole Hz, so a 48 kHz stream moves in ~20.83 ppm
/// steps and rounding a ±100 ppm command to 5 Hz would apply **104.2 ppm**,
/// outside the §8 bound. Clamp the *Hz* delta to the largest whole-Hz value
/// that does not exceed `max_correction_ppm`, then report the actual ppm.
/// This mirrors `playbackRateHz` in `MediaPlayer.kt`.
fn actuator_ppm(commanded_ppm: i32, max_correction_ppm: i32, sample_rate: u32) -> i32 {
    if sample_rate == 0 {
        return 0;
    }
    let rate = i64::from(sample_rate);
    let max_delta_hz = (i64::from(max_correction_ppm.max(0)) * rate / 1_000_000).max(0);
    let requested =
        (f64::from(commanded_ppm) * f64::from(sample_rate) / 1_000_000.0).round() as i64;
    let delta_hz = requested.clamp(-max_delta_hz, max_delta_hz);
    (delta_hz * 1_000_000 / rate) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_normative() {
        let config = DriftConfig::default();
        assert_eq!(config.window_ms, 10_000);
        assert_eq!(config.max_correction_ppm, 100);
        assert_eq!(config.deviation_threshold_ms, 20);
        assert_eq!(config.sustained_ms, 5_000);
        assert_eq!(config.target_ms, 60);
    }

    #[test]
    fn estimator_needs_two_progressing_clock_samples() {
        let mut e = DriftEstimator::new(10_000, 48_000);
        assert_eq!(e.ppm(), None);
        e.observe(0, 0, 0);
        assert_eq!(e.ppm(), None);
    }

    /// RED before the eviction fix: retaining the oldest sample strictly newer
    /// than the cutoff left a span just under 10 s, so a realistic (jittered)
    /// arrival cadence never produced an estimate.
    #[test]
    fn estimator_reports_within_a_bounded_number_of_jittered_updates() {
        let mut estimator = DriftEstimator::new(DRIFT_WINDOW_MS, 48_000);
        // Real playback reports are ~10/s with scheduling jitter.
        let cadence_ms = [100u64, 101, 99, 100, 102, 98, 100, 100, 103, 97];
        let mut now_ms = 0u64;
        let mut updates = 0u64;
        let mut measurement = None;

        while measurement.is_none() && now_ms <= 13_000 {
            // Capture runs +100 ppm: 48.0048 frames/ms; playback is nominal.
            let capture = (u128::from(now_ms) * 48_000 * 1_000_100 / 1_000_000_000) as u64;
            let playback = now_ms * 48;
            estimator.observe(now_ms, capture, playback);
            updates += 1;
            measurement = estimator.ppm().map(|ppm| (updates, ppm));
            now_ms += cadence_ms[(updates as usize) % cadence_ms.len()];
        }

        let (updates, ppm) = measurement.expect("a jittered cadence must produce an estimate");
        assert!(
            updates <= 112,
            "the 10 s window plus one cadence must fill in bounded updates, took {updates}"
        );
        assert!(
            (95..=105).contains(&ppm),
            "a +100 ppm synthetic capture clock must measure ~100 ppm, got {ppm}"
        );
    }

    /// A ten-second span with only a handful of samples is still a
    /// short-window estimate: one quantised playout step near an edge swings
    /// the slope by hundreds of ppm. It must not publish.
    #[test]
    fn estimator_needs_a_minimum_sample_count() {
        let mut estimator = DriftEstimator::new(DRIFT_WINDOW_MS, 48_000);
        // Two paired clocks 10 s apart, both otherwise matched.
        estimator.observe(0, 0, 0);
        estimator.observe(DRIFT_WINDOW_MS, DRIFT_WINDOW_MS * 48, DRIFT_WINDOW_MS * 48);
        assert_eq!(estimator.ppm(), None, "two samples are not a window");
        // The normal 10 Hz report cadence reaches the minimum quickly.
        let mut now = 10_100u64;
        while now <= DRIFT_WINDOW_MS + 1_000 {
            estimator.observe(now, now * 48, now * 48);
            now += 100;
        }
        assert_eq!(estimator.ppm(), Some(0), "enough samples spans the window");
        let (span_ms, samples) = estimator.window_stats();
        assert!(span_ms >= DRIFT_WINDOW_MS, "span {span_ms}");
        assert!(samples >= MIN_WINDOW_SAMPLES, "samples {samples}");
    }

    /// M5-12: the diagnostic window and the residual baseline have independent
    /// spans. The §8 10 s value must publish first; the controller must wait
    /// for the longer baseline.
    #[test]
    fn residual_ppm_requires_the_long_baseline() {
        let mut estimator = DriftEstimator::new(DRIFT_WINDOW_MS, 48_000);
        let mut now = 0u64;
        while now <= DRIFT_WINDOW_MS + 1_000 {
            estimator.observe(now, now * 48, now * 48);
            now += 100;
        }
        assert_eq!(estimator.ppm(), Some(0));
        assert_eq!(
            estimator.residual_ppm(),
            None,
            "10 s is not the residual baseline"
        );
        while now <= RESIDUAL_BASELINE_MS + 1_000 {
            estimator.observe(now, now * 48, now * 48);
            now += 100;
        }
        assert_eq!(estimator.residual_ppm(), Some(0));
        // A reset clears both.
        estimator.reset();
        assert_eq!(estimator.ppm(), None);
        assert_eq!(estimator.residual_ppm(), None);
    }

    /// M5-12: the integer-Hz executor never commands more than the configured
    /// §8 ceiling allows. 48 kHz: 5 Hz would be 104.2 ppm, so the reachable
    /// ceiling is 4 Hz. 44.1 kHz truncates to 90 ppm; 8 kHz has no whole-Hz
    /// step inside ±100 ppm.
    #[test]
    fn integer_hz_actuator_stays_inside_the_ppm_bound() {
        assert_eq!(actuator_ppm(0, MAX_CORRECTION_PPM, 48_000), 0);
        assert_eq!(
            actuator_ppm(100, MAX_CORRECTION_PPM, 48_000),
            83,
            "4 Hz, not 104.2 ppm"
        );
        assert_eq!(actuator_ppm(-100, MAX_CORRECTION_PPM, 48_000), -83);
        assert_eq!(
            actuator_ppm(75, MAX_CORRECTION_PPM, 48_000),
            83,
            "3.6 Hz rounds to 4 Hz"
        );
        assert_eq!(actuator_ppm(10_000, MAX_CORRECTION_PPM, 48_000), 83);
        assert_eq!(actuator_ppm(100, MAX_CORRECTION_PPM, 44_100), 90);
        assert_eq!(
            actuator_ppm(100, MAX_CORRECTION_PPM, 8_000),
            0,
            "no whole-Hz step fits inside ±100 ppm at 8 kHz"
        );
        // A smaller configured ceiling is honoured: 40 ppm at 48 kHz can
        // reach 1 Hz = 20 ppm, not the default 83.
        assert_eq!(actuator_ppm(40, 40, 48_000), 20);
    }

    /// M5-12: the estimator's residual baseline and the controller's
    /// applied-rate mean derive from the same function, including for a
    /// non-default diagnostic window.
    #[test]
    fn estimator_and_controller_agree_on_the_residual_baseline() {
        for window_ms in [DRIFT_WINDOW_MS, 30_000, RESIDUAL_BASELINE_MS + 60_000] {
            let estimator = DriftEstimator::new(window_ms, 48_000);
            let controller = DriftController::new(DriftConfig {
                window_ms,
                ..DriftConfig::default()
            });
            assert_eq!(
                estimator.residual_baseline_ms(),
                controller.residual_baseline_ms(),
                "window_ms={window_ms}"
            );
            assert_eq!(
                estimator.residual_baseline_ms(),
                residual_baseline_ms(window_ms)
            );
        }
    }

    /// M5-12: the command never exceeds what the integer-Hz executor can reach,
    /// even when the §8 ceiling is 100 ppm.
    #[test]
    fn the_controller_command_never_exceeds_the_actuator_limit() {
        for (rate, expected) in [(48_000u32, 83i32), (44_100, 90), (8_000, 0)] {
            let mut controller = DriftController::new(DriftConfig::default());
            let mut now = 0u64;
            for _ in 0..50 {
                now += 100;
                controller.update(now, Some(10_000), 60, rate);
            }
            assert_eq!(controller.applied_ppm(), expected, "sample_rate={rate}");
        }
    }
}
