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
/// progress between reports. A deficit on either side or a step in their
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
    sample_rate: u32,
    samples: VecDeque<DriftSample>,
    /// Number of rejected clock intervals, not number of stall episodes.
    discontinuous_intervals: u64,
}

impl DriftEstimator {
    pub fn new(window_ms: u64, sample_rate: u32) -> Self {
        Self {
            window_ms: window_ms.max(1),
            sample_rate: sample_rate.max(1),
            samples: VecDeque::new(),
            discontinuous_intervals: 0,
        }
    }

    /// Discard the observation window.
    pub fn reset(&mut self) {
        self.samples.clear();
    }

    /// Fold one (session time, capture position, playback position) sample.
    /// A stalled observation or backlog discontinuity discards the old window.
    pub fn observe(&mut self, now_ms: u64, capture_frames: u64, playback_frames: u64) {
        if self
            .samples
            .back()
            .is_some_and(|last| self.discontinuous(last, now_ms, capture_frames, playback_frames))
        {
            self.reset();
            self.discontinuous_intervals = self.discontinuous_intervals.saturating_add(1);
        }
        let sample = DriftSample {
            now_ms,
            capture_frames,
            playback_frames,
        };
        match self.samples.back() {
            // Ignore a non-advancing duplicate instant; keep the newest value.
            Some(last) if now_ms <= last.now_ms => {
                *self.samples.back_mut().unwrap() = sample;
                return;
            }
            _ => {}
        }
        self.samples.push_back(sample);
        // Keep ONE sample at or before the cutoff. Evicting every sample older
        // than the cutoff leaves the retained span strictly shorter than the
        // window whenever arrivals are jittered (`front > cutoff`), and `ppm()`
        // then never returns `Some`. Retaining the latest sample at/before the
        // cutoff guarantees `last - front >= window_ms` while still bounding
        // the window to one extra cadence interval.
        let cutoff = now_ms.saturating_sub(self.window_ms);
        while self.samples.len() > 1 && self.samples[1].now_ms <= cutoff {
            self.samples.pop_front();
        }
    }

    fn discontinuous(
        &self,
        last: &DriftSample,
        now_ms: u64,
        capture_frames: u64,
        playback_frames: u64,
    ) -> bool {
        let capture = (i128::from(capture_frames) - i128::from(last.capture_frames)) * 1_000;
        let playback = (i128::from(playback_frames) - i128::from(last.playback_frames)) * 1_000;
        let rate = i128::from(self.sample_rate);
        let nominal = i128::from(now_ms.saturating_sub(last.now_ms)) * rate;
        let tolerance = rate * i128::from(OBSERVATION_TOLERANCE_MS);
        // Two nearly equal advances are not enough: both sides can starve.
        nominal - capture > tolerance
            || nominal - playback > tolerance
            || (capture - playback).abs() > tolerance
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
    pub fn ppm(&self) -> Option<i32> {
        let first = self.samples.front()?;
        let last = self.samples.back()?;
        if last.now_ms.saturating_sub(first.now_ms) < self.window_ms {
            return None;
        }
        if self.samples.len() < MIN_WINDOW_SAMPLES {
            return None;
        }
        // Without playback progress the "slope" is just the capture rate, not
        // a rate mismatch.
        if last.playback_frames <= first.playback_frames
            || last.capture_frames <= first.capture_frames
        {
            return None;
        }
        let n = self.samples.len() as f64;
        let mean_t = self.samples.iter().map(|s| s.now_ms as f64).sum::<f64>() / n;
        let mean_e = self
            .samples
            .iter()
            .map(|s| s.capture_frames as f64 - s.playback_frames as f64)
            .sum::<f64>()
            / n;
        let mut covariance = 0.0f64;
        let mut variance = 0.0f64;
        for sample in &self.samples {
            let dt = sample.now_ms as f64 - mean_t;
            let de = (sample.capture_frames as f64 - sample.playback_frames as f64) - mean_e;
            covariance += dt * de;
            variance += dt * dt;
        }
        if variance <= 0.0 || !covariance.is_finite() {
            return None;
        }
        // frames per ms → ppm: slope * 1000 frames/s / sample_rate * 1e6.
        let ppm = covariance / variance * 1_000_000_000.0 / f64::from(self.sample_rate);
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
}

impl DriftController {
    pub fn new(config: DriftConfig) -> Self {
        Self {
            config,
            applied_ppm: 0,
            deviation_since_ms: None,
            seen_deadband: false,
        }
    }

    pub const fn config(&self) -> DriftConfig {
        self.config
    }

    /// The rate currently applied, in ppm. Read it through JNI for
    /// `AudioTrack.setPlaybackRate`.
    pub const fn applied_ppm(&self) -> i32 {
        self.applied_ppm
    }

    /// Forget the smoothing state and the sustained-deviation run. Used on a
    /// stream/configuration change.
    pub fn reset(&mut self) {
        self.applied_ppm = 0;
        self.deviation_since_ms = None;
        self.seen_deadband = false;
    }

    /// Advance the controller. `estimate` is the estimator output (None before
    /// the window fills). `buffer_current_ms` is the platform-reported fill.
    pub fn update(
        &mut self,
        now_ms: u64,
        estimate: Option<i32>,
        buffer_current_ms: u32,
        sample_rate: u32,
    ) -> DriftDecision {
        // (1) Smooth rate correction toward the clamped estimate.
        let target = estimate.unwrap_or(0).clamp(
            -self.config.max_correction_ppm,
            self.config.max_correction_ppm,
        );
        let delta = (target - self.applied_ppm).clamp(
            -self.config.slew_ppm_per_update,
            self.config.slew_ppm_per_update,
        );
        self.applied_ppm += delta;

        // (2) Sustained buffer deviation → whole-frame insert/delete.
        //
        // Insertion adds silence at the playout point and can be audible, so it
        // is only allowed once the buffer has actually settled inside the
        // deadband (a startup fill or a permanently shallow application ring is
        // not drift) AND a real drift measurement exists. Deletion only reduces
        // latency and is safe whenever the buffer is genuinely too deep.
        let error_ms = buffer_current_ms as i32 - self.config.target_ms as i32;
        let mut frame_adjust = 0i32;
        if error_ms.abs() <= self.config.deviation_threshold_ms {
            self.seen_deadband = true;
            self.deviation_since_ms = None;
        } else if self.deviation_since_ms.is_none() {
            self.deviation_since_ms = Some(now_ms);
        } else if now_ms.saturating_sub(self.deviation_since_ms.unwrap()) > self.config.sustained_ms
        {
            let insert_allowed = self.seen_deadband && estimate.is_some();
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
}
