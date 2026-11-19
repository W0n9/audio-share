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
//! M5-05 TIMESTAMP TRAP: the server packetizer advances `timestamp` only by the
//! frames it actually sent, so a server-side drop leaves a one-time permanent
//! offset. A step is a position offset, not a rate; `MediaSession` resets this
//! estimator whenever the jitter buffer reports such a re-base, and whenever
//! playback reports an underrun or the reader re-anchors — so our own
//! corrections never feed back into the estimate.

use std::collections::VecDeque;

/// §8 measurement window.
pub const DRIFT_WINDOW_MS: u64 = 10_000;
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
}

impl DriftEstimator {
    pub fn new(window_ms: u64, sample_rate: u32) -> Self {
        Self {
            window_ms: window_ms.max(1),
            sample_rate: sample_rate.max(1),
            samples: VecDeque::new(),
        }
    }

    /// Discard the window. Called on every discontinuity (re-anchor, drift
    /// frame intervention, underrun, timestamp re-base) so the event is not
    /// mistaken for clock drift.
    pub fn reset(&mut self) {
        self.samples.clear();
    }

    /// Fold one (session time, capture position, playback position) sample.
    pub fn observe(&mut self, now_ms: u64, capture_frames: u64, playback_frames: u64) {
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
        let cutoff = now_ms.saturating_sub(self.window_ms);
        while self.samples.len() > 1 && self.samples.front().unwrap().now_ms < cutoff {
            self.samples.pop_front();
        }
    }

    /// The measured drift in ppm, or `None` until a full `window_ms` of paired
    /// observations exists. Positive means the capture clock is ahead of the
    /// playback clock (the buffer is growing; playback must speed up).
    pub fn ppm(&self) -> Option<i32> {
        let first = self.samples.front()?;
        let last = self.samples.back()?;
        if last.now_ms.saturating_sub(first.now_ms) < self.window_ms {
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
}
