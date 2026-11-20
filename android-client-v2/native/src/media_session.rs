//! The Android media delivery interface (ticket 14, spec §7.5–§7.9).
//!
//! One module owns every per-session media decision so the session layer, the
//! JNI shim and the tests all cross the same seam:
//!
//! - which `(instance, generation)` is current (control pushes are the only
//!   authority, datagrams carry their own generation),
//! - §7.5 receiver rules — dedup, reorder/late accounting, alignment,
//!   impossible fields, reserved flag bits,
//! - §7.6 unknown-generation hold (bounded, never decoded with the old
//!   configuration),
//! - the PCM bytes that reach the native ring,
//! - §9 diagnostics, built from the receiver counters plus the *actually
//!   submitted* playback position the Kotlin side reports.
//!
//! M4 deliberately has no playout scheduling: datagrams are delivered in
//! arrival order after dedup. Jitter buffering, late drops and concealment are
//! M5 and are reported as zero here rather than invented.

use std::collections::VecDeque;

use crate::drift::{DriftConfig, DriftController, DriftEstimator};
use crate::jitter::JitterBuffer;
use crate::media::{MediaHeader, MediaReceiver, ObserveVerdict, HEADER_SIZE};
use crate::proto::{Configuration, PlaybackDiagnostics};
use crate::ring::MediaRing;

/// Re-export the jitter types so host tests and the session layer share one
/// definition of the §8 target/hard-limit defaults and concealment kinds.
pub use crate::jitter::{ConcealmentKind, JitterConfig};

/// §7.6 bounds: hold at most 100 ms of media or 32 datagrams, whichever fills
/// first; overflow drops the oldest held datagram.
pub const UNKNOWN_CONFIG_MAX_DATAGRAMS: usize = 32;
pub const UNKNOWN_CONFIG_MAX_MS: u64 = 100;

/// How long a held generation may stay unknown before the session gives up
/// (§7.6 step 3: 2 s → Error(403)+Close(403)).
pub const UNKNOWN_CONFIG_GIVE_UP_MS: u64 = 2_000;

/// M5-11: a hardware playout head missing for this long is a sustained
/// outage, not a transient report failure. 500 ms is five report intervals at
/// the normal 100 ms cadence and more than eight times the 60 ms
/// window-continuity budget: shorter gaps keep the hardware window and skip
/// only that observation, while this bound is long enough that a continuous
/// slope can no longer be trusted. At/after it the clock source switches to
/// accepted bytes and the window restarts.
pub const HARDWARE_HEAD_GIVE_UP_MS: u64 = 500;

/// §4.4 `SampleFormat` values (numbers mirror v1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    F32Le,
    U8,
    S16Le,
    S24Le,
    S32Le,
}

impl SampleFormat {
    fn from_wire(value: i32) -> Option<Self> {
        match value {
            1 => Some(Self::F32Le),
            2 => Some(Self::U8),
            3 => Some(Self::S16Le),
            4 => Some(Self::S24Le),
            5 => Some(Self::S32Le),
            _ => None,
        }
    }

    /// Bytes per sample of one channel.
    pub const fn bytes_per_sample(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::S16Le => 2,
            Self::S24Le => 3,
            Self::F32Le | Self::S32Le => 4,
        }
    }

    /// The §4.4 wire value (what the Kotlin side compares against).
    pub const fn wire_value(self) -> i32 {
        match self {
            Self::F32Le => 1,
            Self::U8 => 2,
            Self::S16Le => 3,
            Self::S24Le => 4,
            Self::S32Le => 5,
        }
    }
}

/// The PCM geometry of the active configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmGeometry {
    pub sample_rate: u32,
    pub channels: u32,
    pub sample_format: SampleFormat,
}

impl PcmGeometry {
    /// Bytes of one sample frame (all channels).
    pub fn frame_bytes(&self) -> usize {
        self.sample_format.bytes_per_sample() * self.channels as usize
    }

    /// Frames in `bytes`, or `None` when not frame-aligned.
    pub fn frames(&self, bytes: usize) -> Option<u64> {
        let frame_bytes = self.frame_bytes();
        if frame_bytes == 0 || !bytes.is_multiple_of(frame_bytes) {
            return None;
        }
        Some((bytes / frame_bytes) as u64)
    }

    fn from_configuration(configuration: &Configuration) -> Option<Self> {
        let pcm = configuration.pcm.as_ref()?;
        if pcm.sample_rate == 0 || pcm.channels == 0 {
            return None;
        }
        Some(Self {
            sample_rate: pcm.sample_rate,
            channels: pcm.channels,
            sample_format: SampleFormat::from_wire(pcm.sample_format)?,
        })
    }
}

/// What one datagram did, for logs and tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatagramOutcome {
    /// PCM written to the ring.
    Delivered,
    /// Duplicate inside the dedup window.
    Duplicate,
    /// Malformed: truncated header, impossible field, or misaligned payload.
    Malformed,
    /// Generation older than the current one — dropped and counted (§7.5).
    StaleGeneration,
    /// Generation newer than anything published: held (§7.6).
    HeldUnknownConfig,
    /// Arrived after its playout deadline (§8): dropped, counted separately
    /// from network loss. Never written to the ring.
    LateDropped,
    /// Not playing (no active stream): ignored.
    Ignored,
}

/// The playback facts Kotlin observes. Only `submitted_bytes` (bytes the
/// `AudioTrack` actually accepted), `underruns` and the current ring fill are
/// known there; everything else is native receiver state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaybackReport {
    pub submitted_bytes: u64,
    pub underruns: u64,
    pub buffer_current_ms: u32,
}

/// Which platform clock feeds the §8 drift measurement.
///
/// M5-11: `AudioTrack.write` acceptance is a staircase (the reader hands the
/// platform whole blocks), so it can swing a 10 s window by hundreds of ppm
/// even when its long-run mean is exact. The hardware playout head is the
/// real clock. Never mix the two in one measurement window: they differ by a
/// platform-buffer latency and by different position-offset accounting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlaybackClockSource {
    /// Bytes the platform API accepted (`submitted_position`; pre-M5-11).
    AcceptedBytes,
    /// Frames the platform has actually played (`anchor + head`).
    HardwareHead,
}

/// Local diagnostics for the drift clock source (M5-11).
///
/// - `hardware_reports`: reports carrying an already-extended hardware head.
/// - `hardware_missing_reports`: reports with no head while the hardware clock
///   was active. A transient miss is skipped, not treated as position 0.
/// - `accepted_fallback_reports`: reports actually observed on the accepted
///   clock (the legacy path, or after a sustained hardware outage).
/// - `head_reanchor_events`: a head regression (sink rebuild/flush) that
///   re-anchored the base and restarted the window.
/// - `head_give_up_events`: a sustained missing-head run that switched from the
///   hardware clock to accepted bytes and restarted the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaybackClockStats {
    pub hardware_reports: u64,
    pub hardware_missing_reports: u64,
    pub accepted_fallback_reports: u64,
    pub head_reanchor_events: u64,
    pub head_give_up_events: u64,
}

/// Local-only summary of all playback reports since the last snapshot.
/// An unavailable estimate is still a report; missing reports instead widen
/// `max_report_gap_ms`. Bounds and gaps use the playback report clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DriftInterval {
    pub start_ms: u64,
    pub end_ms: u64,
    pub reports: u64,
    pub valid_estimates: u64,
    pub extrema_ppm: Option<(i32, i32)>,
    pub max_report_gap_ms: u64,
}

/// The four §8 intervention kinds, kept as a breakdown. `cumulative_concealments`
/// remains the single §9 field; the breakdown is for the local L1 log line so
/// the kind recorded by [`MediaSession::record_concealment`] is not lost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConcealmentCounts {
    pub gap: u64,
    pub underrun: u64,
    pub drift: u64,
    pub reanchor: u64,
}

impl ConcealmentCounts {
    pub const fn total(self) -> u64 {
        self.gap + self.underrun + self.drift + self.reanchor
    }
}

/// One datagram held under §7.6: the instant it really arrived (so releasing
/// it cannot manufacture a late drop) and its untouched wire bytes.
struct HeldDatagram {
    arrival_ms: u64,
    bytes: Vec<u8>,
}

/// The §7.6 hold for one unknown generation. A newer unknown generation
/// replaces it; overflow drops the oldest datagram.
struct HeldMedia {
    generation: u32,
    datagrams: VecDeque<HeldDatagram>,
}

/// One Client session's media state (§7.5–§7.9).
pub struct MediaSession {
    instance: u32,
    generation: u32,
    geometry: PcmGeometry,
    /// Capture-clock position of the first frame the receiver should get.
    anchor: u64,
    receiver: MediaReceiver,
    ring: Option<MediaRing>,
    /// Generation of held datagrams that could not be decoded yet (§7.6).
    /// The session instant at which each datagram actually arrived is kept so
    /// releasing the hold cannot manufacture a late drop (its deadline was
    /// judged when it arrived, not when the configuration caught up).
    holding: Option<HeldMedia>,
    /// Session-clock ms at which the current hold began (§7.6 2 s bound).
    held_since_ms: Option<u64>,
    report: PlaybackReport,
    /// Interval counters snapshotted since the last diagnostics report.
    interval: IntervalCounters,
    /// §9's "at most one send pending" rule is owned by `MediaControl` (the
    /// session-level cadence), not by the receiver state; it lives in one
    /// place so the two cannot disagree.
    /// Set when the unknown configuration stayed unknown too long.
    gave_up_unknown_config: bool,
    /// Frames delivered to the ring since the instance started.
    delivered_frames: u64,
    /// The §8 bounded jitter buffer (reorder + late drop + silence gap fill).
    jitter: JitterBuffer,
    /// Cumulative concealment count (all four §8 interventions). `concealed_frames`
    /// is inserted − deleted, signed (proto sint64).
    concealments: u64,
    concealed_frames: i64,
    /// Frames of silence accepted by the platform specifically for underrun
    /// fills (M5-08). `concealed_frames` also includes gap/drift/re-anchor, so
    /// this isolates the underrun contribution for the `playback.concealments`
    /// log line: `underrun_frames / underrun` is the mean accepted frames per
    /// fill.
    underrun_frames: u64,
    /// Late drops (§8): arrivals after their playout deadline. Separate from
    /// network loss (`receiver.cumulative_lost`).
    late_drops: u64,
    /// Frames/batches trimmed by the Kotlin reader's §8 hard-limit re-anchor.
    /// Also counted as `ReAnchor` concealment (M5-04).
    trimmed_frames: u64,
    trim_events: u64,
    /// Frames/batches trimmed by the reader's *soft* target trim (backlog
    /// above the §8 target). Counted separately and NOT as concealment.
    soft_trimmed_frames: u64,
    soft_trim_events: u64,
    /// Per-kind §8 intervention breakdown behind `concealments`.
    concealment_counts: ConcealmentCounts,
    /// Latest capture-clock position seen on the wire (wire `timestamp` +
    /// frames). The capture side of the §8 drift measurement.
    capture_position: u64,
    /// Whether any media datagram has established a capture position.
    saw_capture: bool,
    /// §8 10 s rolling drift estimator and its smooth ±100 ppm controller.
    drift: DriftEstimator,
    drift_controller: DriftController,
    drift_ppm: Option<i32>,
    drift_interval: DriftInterval,
    last_playback_report_ms: u64,
    /// Cumulative artificial playback-position jumps (soft trim, hard
    /// re-anchor, drift deletion) removed from the estimate's playback side
    /// (M5-08). A discontinuity is a position step, not a rate: subtracting it
    /// keeps the 10 s window continuous instead of starving it on every trim.
    drift_playback_step: i64,
    /// Cumulative server re-base jumps removed from the estimate's capture
    /// side (M5-08).
    drift_capture_step: u64,
    /// A whole-frame adjustment the playback path still has to apply and
    /// acknowledge (positive insert, negative delete). Only one is outstanding.
    pending_drift_frames: i32,
    /// Position offset separating the platform-accepted bytes from the capture
    /// clock: re-anchor/deleted frames advance it, inserted silence retreats it.
    position_adjust_frames: i64,
    /// Base added to a raw hardware head that regressed (32-bit wrap inside a
    /// track, or a sink rebuild whose head restarted at 0).
    playback_head_base: u64,
    /// Last raw hardware head seen, for wrap/rebuild detection.
    last_raw_head: Option<u64>,
    /// Effective monotonic hardware-played frame position (frames, not bytes)
    /// from the latest report, or `None` when that report had no head.
    playback_head_frames: Option<u64>,
    /// Clock the current drift window is built from. `None` until the first
    /// report; a change starts a fresh window.
    drift_clock_source: Option<PlaybackClockSource>,
    /// Playback-clock instant of the last report that carried a hardware head,
    /// used to distinguish a transient miss from a sustained outage.
    last_hardware_report_ms: Option<u64>,
    /// Local L1 diagnostics for the clock source (M5-11).
    playback_clock_stats: PlaybackClockStats,
    /// Monotonic reference for the non-test `record_playback` entry point.
    clock_start: std::time::Instant,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct IntervalCounters {
    received: u64,
    duplicates: u64,
    reordered: u64,
    lost: i64,
    unknown_config_drops: u64,
    late_drops: u64,
    concealments: u64,
}

impl MediaSession {
    /// Start delivering instance `instance` generation `generation` with the
    /// geometry of `configuration`, anchored at `initial_timestamp`.
    ///
    /// Returns `None` for a configuration this Client cannot decode: the
    /// session layer turns that into Error(403)+Close(403), never a silent
    /// format switch or a stalled `AudioTrack` (§5.3 / §6.2).
    pub fn start(
        instance: u32,
        generation: u32,
        initial_timestamp: u64,
        configuration: &Configuration,
        ring: Option<MediaRing>,
    ) -> Option<Self> {
        if !crate::control::configuration_decodable(configuration) {
            return None;
        }
        let geometry = PcmGeometry::from_configuration(configuration)?;
        let drift_config = DriftConfig::default();
        Some(Self {
            instance,
            generation,
            geometry,
            anchor: initial_timestamp,
            receiver: MediaReceiver::new(),
            ring,
            holding: None,
            held_since_ms: None,
            report: PlaybackReport::default(),
            interval: IntervalCounters::default(),
            gave_up_unknown_config: false,
            delivered_frames: 0,
            jitter: JitterBuffer::new(
                JitterConfig::default(),
                geometry.sample_rate,
                geometry.frame_bytes(),
            ),
            concealments: 0,
            concealed_frames: 0,
            underrun_frames: 0,
            late_drops: 0,
            trimmed_frames: 0,
            trim_events: 0,
            soft_trimmed_frames: 0,
            soft_trim_events: 0,
            concealment_counts: ConcealmentCounts::default(),
            capture_position: 0,
            saw_capture: false,
            drift: DriftEstimator::new(drift_config.window_ms, geometry.sample_rate),
            drift_controller: DriftController::new(drift_config),
            drift_ppm: None,
            drift_interval: DriftInterval::default(),
            last_playback_report_ms: 0,
            drift_playback_step: 0,
            drift_capture_step: 0,
            pending_drift_frames: 0,
            position_adjust_frames: 0,
            playback_head_base: 0,
            last_raw_head: None,
            playback_head_frames: None,
            drift_clock_source: None,
            last_hardware_report_ms: None,
            playback_clock_stats: PlaybackClockStats::default(),
            clock_start: std::time::Instant::now(),
        })
    }

    /// Start with an explicit jitter configuration (host tests / future tuning).
    pub fn start_with_config(
        instance: u32,
        generation: u32,
        initial_timestamp: u64,
        configuration: &Configuration,
        ring: Option<MediaRing>,
        jitter_config: JitterConfig,
    ) -> Option<Self> {
        let mut session =
            Self::start(instance, generation, initial_timestamp, configuration, ring)?;
        session.jitter = JitterBuffer::new(
            jitter_config,
            session.geometry.sample_rate,
            session.geometry.frame_bytes(),
        );
        session.drift_controller = DriftController::new(DriftConfig {
            target_ms: jitter_config.target_ms,
            ..DriftConfig::default()
        });
        Some(session)
    }

    /// The active instance.
    pub const fn stream_instance_id(&self) -> u32 {
        self.instance
    }

    /// The active generation.
    pub const fn config_generation(&self) -> u32 {
        self.generation
    }

    /// The published PCM geometry (what the playback side should configure).
    pub const fn geometry(&self) -> PcmGeometry {
        self.geometry
    }

    /// The capture-clock anchor announced by `StreamStarted`.
    pub const fn anchor(&self) -> u64 {
        self.anchor
    }

    /// The active §8 jitter-buffer bounds. The Kotlin reader asks for these
    /// instead of duplicating the 60/200 ms constants.
    pub const fn jitter_config(&self) -> JitterConfig {
        self.jitter.config()
    }

    /// Attach/replace the ring once the JVM hands over its direct ByteBuffer.
    pub fn set_ring(&mut self, ring: MediaRing) {
        self.ring = Some(ring);
    }

    /// A new decode-affecting configuration became current (§5.3 PLAYING).
    /// Datagrams of the previous generation are dropped from then on; held
    /// datagrams of the new generation are released.
    pub fn apply_configuration(&mut self, configuration: &Configuration) {
        let generation = configuration.config_generation;
        if generation > self.generation {
            self.generation = generation;
            if let Some(geometry) = PcmGeometry::from_configuration(configuration) {
                if geometry != self.geometry {
                    self.geometry = geometry;
                    // The drift window is expressed in frames at the published
                    // rate: a geometry change invalidates it.
                    let config = self.drift_controller.config();
                    self.drift = DriftEstimator::new(config.window_ms, geometry.sample_rate);
                    self.drift_controller = DriftController::new(config);
                    self.drift_ppm = None;
                    self.drift_clock_source = None;
                    self.playback_head_frames = None;
                    // The new estimator starts from a fresh origin; the old
                    // position steps belong to the previous frame base.
                    self.drift_playback_step = 0;
                    self.drift_capture_step = 0;
                    self.pending_drift_frames = 0;
                    self.capture_position = 0;
                    self.saw_capture = false;
                }
            }
            // Rebuild the jitter buffer for the new generation. Its pending
            // media and frontier belong to the superseded generation, and the
            // target-frame / silence-fill / deadline math must use the new
            // sample_rate and frame_bytes, not the previous configuration's.
            // The configured target/hard limit are preserved.
            let jitter_config = self.jitter.config();
            self.jitter = JitterBuffer::new(
                jitter_config,
                self.geometry.sample_rate,
                self.geometry.frame_bytes(),
            );
            if let Some(held) = self.holding.take() {
                if held.generation == generation {
                    for datagram in held.datagrams {
                        self.deliver(datagram.arrival_ms, &datagram.bytes);
                    }
                }
            }
            // The hold is gone: its 2 s clock must not fire later (§7.6).
            self.held_since_ms = None;
            self.gave_up_unknown_config = false;
        }
    }

    /// Fold one received DATAGRAM at session-clock `now_ms`. Never blocks and
    /// never panics on hostile input: every rejection is counted and reported.
    pub fn on_datagram(&mut self, now_ms: u64, datagram: &[u8]) -> DatagramOutcome {
        let Ok((header, payload)) = MediaHeader::parse(datagram) else {
            self.receiver.malformed_drops += 1;
            return DatagramOutcome::Malformed;
        };
        if header.stream_instance_id != self.instance {
            // A datagram of another instance is a new epoch only when the
            // reliable control stream says so (§7.5: the push is the anchor).
            self.receiver.malformed_drops += 1;
            return DatagramOutcome::Malformed;
        }
        if self.geometry.frames(payload.len()).is_none() {
            self.receiver.malformed_drops += 1;
            return DatagramOutcome::Malformed;
        }
        if header.config_generation < self.generation {
            self.receiver.note_stale_generation();
            self.interval.unknown_config_drops += 1;
            return DatagramOutcome::StaleGeneration;
        }
        if header.config_generation > self.generation {
            return self.hold(now_ms, datagram, header.config_generation);
        }
        self.accept_observed(now_ms, &header, payload, true)
    }

    /// Record what the platform playback API actually accepted (§9 evidence).
    /// Uses the session's monotonic clock; host tests drive the deterministic
    /// [`Self::record_playback_at`] entry point instead.
    pub fn record_playback(&mut self, report: PlaybackReport) {
        self.record_playback_at(self.playback_clock_ms(), report);
    }

    /// Record accepted playback facts plus the hardware playout head on the
    /// live session clock. `playback_head_frames` is in frames and `None`
    /// falls back to the accepted-bytes clock (M5-11).
    pub fn record_playback_with_head(
        &mut self,
        report: PlaybackReport,
        playback_head_frames: Option<u64>,
    ) {
        self.record_playback_with_head_at(self.playback_clock_ms(), report, playback_head_frames);
    }

    fn playback_clock_ms(&self) -> u64 {
        self.clock_start
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64
    }

    /// Record the platform playback facts at an injected session instant.
    ///
    /// Besides updating the §9 report, this is where the §8 drift machinery
    /// advances: the paired capture/playback clocks are sampled, the smooth
    /// ±100 ppm rate decision is taken, and (when a buffer deviation has been
    /// sustained) a whole-frame intervention is queued for the player. The
    /// intervention is counted only when the player acknowledges it through
    /// [`Self::record_drift_intervention`].
    ///
    /// This entry point carries no hardware playout position, so it keeps the
    /// accepted-bytes playback clock used before M5-11. It remains the
    /// backward-compatible path for platforms that cannot report a head.
    pub fn record_playback_at(&mut self, now_ms: u64, report: PlaybackReport) {
        self.record_playback_with_head_at(now_ms, report, None);
    }

    /// Record the platform playback facts plus the hardware playout position
    /// at an injected session instant.
    ///
    /// `playback_head_frames` is the number of audio frames the platform has
    /// actually **played** since the sink opened, in frames (never bytes). The
    /// sender owns the 32-bit `getPlaybackHeadPosition()` wrap and always
    /// reports an already-extended, monotonic value; native treats any
    /// regression as a sink rebuild and re-anchors. `None` means this report
    /// could not report a head: a transient miss keeps the hardware window and
    /// skips only the observation; after [`HARDWARE_HEAD_GIVE_UP_MS`] the
    /// accepted-bytes clock is used. A miss is never position 0.
    ///
    /// Unlike the accepted-bytes clock, the head already excludes frames the
    /// reader skipped (trim/re-anchor/soft trim) and includes silence it
    /// inserted, so `position_adjust_frames` / `drift_playback_step` must NOT
    /// also be applied to it — that would double-compensate.
    pub fn record_playback_with_head_at(
        &mut self,
        now_ms: u64,
        report: PlaybackReport,
        playback_head_frames: Option<u64>,
    ) {
        self.report = report;
        let observation = self.update_playback_clock(now_ms, playback_head_frames);
        if self.saw_capture {
            match observation {
                Some(source) => {
                    // The hardware head already reflects what the platform
                    // played: skipped frames are absent and inserted silence is
                    // present. The accepted-bytes path keeps today's M5-08
                    // position-step folding. Applying `position_adjust_frames`
                    // to the head as well would double-compensate (M5-11).
                    if let Some(playback) = self.drift_playback_position(source) {
                        let capture = self
                            .capture_position
                            .saturating_sub(self.drift_capture_step);
                        self.drift.observe(now_ms, capture, playback);
                        self.drift_ppm = self.drift.ppm();
                    }
                }
                None => {
                    // Hardware source active but this report had no head:
                    // keep the window and skip only this observation. Do not
                    // publish the previous estimate, which is now stale.
                    self.drift_ppm = None;
                }
            }
        }
        let interval = &mut self.drift_interval;
        interval.reports = interval.reports.saturating_add(1);
        interval.max_report_gap_ms = interval
            .max_report_gap_ms
            .max(now_ms.saturating_sub(self.last_playback_report_ms));
        self.last_playback_report_ms = now_ms;
        if let Some(ppm) = self.drift_ppm {
            interval.valid_estimates = interval.valid_estimates.saturating_add(1);
            interval.extrema_ppm = Some(match interval.extrema_ppm {
                Some((min, max)) => (min.min(ppm), max.max(ppm)),
                None => (ppm, ppm),
            });
        }
        if self.pending_drift_frames == 0 {
            let decision = self.drift_controller.update(
                now_ms,
                self.drift_ppm,
                report.buffer_current_ms,
                self.geometry.sample_rate,
            );
            if decision.frame_adjust != 0 {
                // The intervention is queued for the player; the position
                // offset it causes is folded out when it is acknowledged, not
                // by resetting the window.
                self.pending_drift_frames = decision.frame_adjust;
            }
        }
    }

    /// Select the playback clock for this report and fold in an extended head.
    ///
    /// Returns the clock to observe, or `None` when the hardware source is
    /// active but this report has no head: the window is kept and only this
    /// observation is skipped. A source change or a head re-anchor clears the
    /// window: the two sources are separated by the platform buffer latency
    /// and by different position-offset accounting, so samples from both must
    /// never share one slope.
    ///
    /// A single missing head is a transient report failure (the reader may not
    /// have a timestamp for one report). Only a run of misses lasting at least
    /// [`HARDWARE_HEAD_GIVE_UP_MS`] switches to the accepted-bytes clock.
    fn update_playback_clock(
        &mut self,
        now_ms: u64,
        playback_head_frames: Option<u64>,
    ) -> Option<PlaybackClockSource> {
        match playback_head_frames {
            Some(raw) => {
                self.playback_clock_stats.hardware_reports =
                    self.playback_clock_stats.hardware_reports.saturating_add(1);
                self.last_hardware_report_ms = Some(now_ms);
                let (effective, reanchored) = self.extend_playback_head(raw);
                if reanchored {
                    self.drift.reset();
                    self.drift_ppm = None;
                }
                self.playback_head_frames = Some(effective);
                self.use_drift_clock(PlaybackClockSource::HardwareHead);
                Some(PlaybackClockSource::HardwareHead)
            }
            None => {
                // Do not keep a stale head: `None` means this report has no
                // hardware evidence, not a zero position.
                self.playback_head_frames = None;
                if self.drift_clock_source == Some(PlaybackClockSource::HardwareHead) {
                    self.playback_clock_stats.hardware_missing_reports = self
                        .playback_clock_stats
                        .hardware_missing_reports
                        .saturating_add(1);
                    let missing_ms = self
                        .last_hardware_report_ms
                        .map_or(u64::MAX, |last| now_ms.saturating_sub(last));
                    if missing_ms < HARDWARE_HEAD_GIVE_UP_MS {
                        // Transient miss: keep the hardware window, skip this
                        // observation, publish no stale estimate.
                        return None;
                    }
                    self.playback_clock_stats.head_give_up_events = self
                        .playback_clock_stats
                        .head_give_up_events
                        .saturating_add(1);
                }
                self.playback_clock_stats.accepted_fallback_reports = self
                    .playback_clock_stats
                    .accepted_fallback_reports
                    .saturating_add(1);
                self.use_drift_clock(PlaybackClockSource::AcceptedBytes);
                Some(PlaybackClockSource::AcceptedBytes)
            }
        }
    }

    /// Switch the measurement window's clock, discarding samples from the
    /// previous source (they are separated by latency and offset accounting).
    fn use_drift_clock(&mut self, source: PlaybackClockSource) {
        if self.drift_clock_source != Some(source) {
            self.drift.reset();
            self.drift_ppm = None;
            self.drift_clock_source = Some(source);
        }
    }

    /// Continue an **already-extended** hardware head across a sink rebuild.
    ///
    /// Kotlin owns the 32-bit `getPlaybackHeadPosition()` wrap and always
    /// reports a monotonic frame count; the native side must not extend it a
    /// second time (that would misread a long-run rebuild as a wrap and add
    /// 2^32). A regression here can therefore only mean a rebuilt/flushed
    /// sink whose head restarted at 0: continue from the last effective
    /// position and start a fresh window. Returns the effective monotonic
    /// position and whether the window broke.
    fn extend_playback_head(&mut self, raw: u64) -> (u64, bool) {
        let mut reanchored = false;
        if let Some(last) = self.last_raw_head {
            if raw < last {
                self.playback_head_base = self.playback_head_base.saturating_add(last);
                self.playback_clock_stats.head_reanchor_events = self
                    .playback_clock_stats
                    .head_reanchor_events
                    .saturating_add(1);
                reanchored = true;
            }
        }
        self.last_raw_head = Some(raw);
        (self.playback_head_base.saturating_add(raw), reanchored)
    }

    /// The playback side of the drift observation for `source`, or `None` when
    /// a hardware source somehow has no head. Callers skip the observation
    /// rather than substituting frame 0.
    fn drift_playback_position(&self, source: PlaybackClockSource) -> Option<u64> {
        match source {
            PlaybackClockSource::HardwareHead => self
                .playback_head_frames
                .map(|head| self.anchor.saturating_add(head)),
            PlaybackClockSource::AcceptedBytes => {
                // M5-08: fold the known position offsets out of the accepted
                // series so a trim/re-anchor/re-base does not reset the window.
                let playback = (i128::from(self.submitted_position())
                    - i128::from(self.drift_playback_step))
                .max(0);
                Some(u64::try_from(playback).unwrap_or(u64::MAX))
            }
        }
    }

    /// Local clock-source diagnostics (M5-11): hardware reports, accepted-byte
    /// fallbacks and head re-anchors.
    pub const fn playback_clock_stats(&self) -> PlaybackClockStats {
        self.playback_clock_stats
    }

    /// Effective hardware-played frame position from the latest report, or
    /// `None` when that report had no head.
    pub const fn playback_head_frames(&self) -> Option<u64> {
        self.playback_head_frames
    }

    /// The clock the current drift window uses: `"hardware"`, `"accepted"`, or
    /// `"none"` before the first report.
    pub fn drift_clock_source(&self) -> &'static str {
        match self.drift_clock_source {
            Some(PlaybackClockSource::HardwareHead) => "hardware",
            Some(PlaybackClockSource::AcceptedBytes) => "accepted",
            None => "none",
        }
    }

    /// The latest §8 drift estimate in ppm, or `None` before a full 10 s
    /// window (or after a reset). Exposed for L1 tooling: the accepted §9
    /// `PlaybackDiagnostics` has no drift field, and adding one is a protocol
    /// change requiring maintainer adjudication.
    pub fn drift_ppm(&self) -> Option<i32> {
        self.drift_ppm
    }

    /// The retained drift window's span and sample count, logged alongside
    /// every `playback.drift` so short/quantised estimates are recognisable
    /// (M5-08).
    pub fn drift_window(&self) -> (u64, usize) {
        self.drift.window_stats()
    }

    /// Drain one local diagnostic interval on the live playback clock.
    pub(crate) fn take_drift_interval(&mut self) -> DriftInterval {
        self.take_drift_interval_at(self.playback_clock_ms())
    }

    /// Drain one local diagnostic interval at an injected playback-clock instant.
    pub fn take_drift_interval_at(&mut self, now_ms: u64) -> DriftInterval {
        let mut interval = std::mem::replace(
            &mut self.drift_interval,
            DriftInterval {
                start_ms: now_ms,
                ..DriftInterval::default()
            },
        );
        interval.end_ms = now_ms;
        interval.max_report_gap_ms = interval
            .max_report_gap_ms
            .max(now_ms.saturating_sub(self.last_playback_report_ms));
        interval
    }

    /// The number of rejected adjacent clock observations in this measurement
    /// series. This counts intervals, not stalls, trims or re-anchors.
    pub(crate) const fn drift_discontinuous_intervals(&self) -> u64 {
        self.drift.discontinuous_intervals()
    }

    /// The smooth playback-rate correction currently commanded, in ppm
    /// (clamped to ±100). The Kotlin player applies it through
    /// `AudioTrack.setPlaybackRate`.
    pub const fn drift_correction_ppm(&self) -> i32 {
        self.drift_controller.applied_ppm()
    }

    /// Take the whole-frame adjustment the playback path must apply:
    /// positive = insert that many silent frames, negative = delete.
    /// Taking it clears the pending value; the count is recorded only on
    /// [`Self::record_drift_intervention`].
    pub fn take_drift_intervention(&mut self) -> i32 {
        std::mem::take(&mut self.pending_drift_frames)
    }

    /// Acknowledge that the playback path applied `frames` (positive insert,
    /// negative delete). This is the §8 concealment accounting for drift.
    pub fn record_drift_intervention(&mut self, frames: i32) {
        if frames == 0 {
            return;
        }
        self.record_concealment(ConcealmentKind::Drift, i64::from(frames));
        // Inserted silence is not a capture position; deleted frames advanced
        // the playout cursor over real capture positions.
        self.position_adjust_frames -= i64::from(frames);
        // Mirror the position-offset change so the rate series stays
        // continuous: inserted silence advances the platform clock
        // (accepted bytes rise, the position offset falls); deleted frames skip
        // media. Either way a full reset is not needed (M5-08).
        self.drift_playback_step = self.drift_playback_step.saturating_sub(i64::from(frames));
    }

    /// Whether the unknown configuration exceeded §7.6's 2 s bound.
    pub const fn gave_up_unknown_config(&self) -> bool {
        self.gave_up_unknown_config
    }

    /// The diagnostics snapshot for a periodic or closing report (§9). Builds
    /// the message and resets the interval counters.
    pub fn diagnostics(&mut self, buffer_target_ms: u32) -> PlaybackDiagnostics {
        let diagnostics = PlaybackDiagnostics {
            stream_instance_id: self.instance,
            config_generation: self.generation,
            extended_highest_sequence: self.receiver.extended_highest,
            cumulative_lost: self.receiver.cumulative_lost,
            interval_lost: self.interval.lost,
            cumulative_received: self.receiver.cumulative_received,
            interval_received: self.interval.received,
            cumulative_duplicates: self.receiver.cumulative_duplicates,
            interval_duplicates: self.interval.duplicates,
            cumulative_reordered: self.receiver.cumulative_reordered,
            interval_reordered: self.interval.reordered,
            max_reorder_distance: self.receiver.max_reorder_distance,
            cumulative_late_drops: self.late_drops,
            interval_late_drops: self.interval.late_drops,
            cumulative_unknown_config_drops: self.receiver.unknown_config_drops,
            interval_unknown_config_drops: self.interval.unknown_config_drops,
            cumulative_concealments: self.concealments,
            interval_concealments: self.interval.concealments,
            concealed_frames: self.concealed_frames,
            underruns: self.report.underruns,
            // §9: RFC 3550 A.8 estimate converted at the published sample_rate.
            jitter_microseconds: self.jitter.jitter_microseconds(),
            buffer_current_ms: self.report.buffer_current_ms,
            buffer_target_ms,
            submitted_position: self.submitted_position(),
        };
        self.interval = IntervalCounters::default();
        diagnostics
    }

    /// `anchor + frames actually submitted to the platform API` (§9: a socket
    /// receive or ring write is never playback evidence), plus the §8
    /// position offsets: a hard-limit re-anchor and a drift deletion advance
    /// the playout position over media that was skipped, and inserted drift
    /// silence is subtracted because it has no capture position.
    pub fn submitted_position(&self) -> u64 {
        let frame_bytes = self.geometry.frame_bytes() as u64;
        if frame_bytes == 0 {
            return self.anchor;
        }
        let accepted = (self.report.submitted_bytes / frame_bytes) as i128;
        let position = self.anchor as i128 + accepted + i128::from(self.position_adjust_frames);
        position.max(0) as u64
    }

    /// Frames written to the ring (native delivery, not playback).
    pub const fn delivered_frames(&self) -> u64 {
        self.delivered_frames
    }

    /// Record one §8 concealment intervention.
    ///
    /// All four §8 interventions share the one §9 `concealments` counter
    /// (fields 17/18) and the signed `concealed_frames` (field 19); `kind` is
    /// kept in a per-kind breakdown that the `playback.concealments` log line
    /// reports, so the caller's classification is not discarded.
    pub fn record_concealment(&mut self, kind: ConcealmentKind, frames_delta: i64) {
        match kind {
            ConcealmentKind::Gap => self.concealment_counts.gap += 1,
            ConcealmentKind::Underrun => self.concealment_counts.underrun += 1,
            ConcealmentKind::Drift => self.concealment_counts.drift += 1,
            ConcealmentKind::ReAnchor => self.concealment_counts.reanchor += 1,
        }
        self.concealments += 1;
        self.interval.concealments += 1;
        self.concealed_frames = self.concealed_frames.saturating_add(frames_delta);
    }

    /// Per-kind §8 intervention counts (the local L1 breakdown; §9 only has
    /// the total).
    pub const fn concealment_counts(&self) -> ConcealmentCounts {
        self.concealment_counts
    }

    /// Signed frames inserted (+) minus deleted (−) for this instance (§9
    /// field 19). Exposed so the local `playback.concealments` log can print
    /// the number that turns an event count into seconds of silence.
    pub const fn concealed_frames(&self) -> i64 {
        self.concealed_frames
    }

    /// Frames of silence accepted by the platform for underrun fills only
    /// (M5-08). Divide by `concealment_counts().underrun` for per-fill frames.
    pub const fn underrun_frames(&self) -> u64 {
        self.underrun_frames
    }

    /// A playback-loop underrun: the reader had no data for one playback
    /// period while playing, so it fed `inserted_frames` of silence. This is
    /// the §8 "underrun fill" intervention — distinct from the platform's
    /// `underruns` counter, which stays as reported in §9 field 20.
    ///
    /// Inserted silence carries no capture position, so the platform-accepted
    /// bytes it contributes are offset out of `submitted_position` (the
    /// playback cursor advanced in wall time, not on the capture clock).
    pub fn record_underrun(&mut self, inserted_frames: u64) {
        if inserted_frames == 0 {
            return;
        }
        self.record_concealment(ConcealmentKind::Underrun, inserted_frames as i64);
        self.underrun_frames = self.underrun_frames.saturating_add(inserted_frames);
        self.position_adjust_frames = self
            .position_adjust_frames
            .saturating_sub(inserted_frames as i64);
        // The platform clock advanced over the inserted silence, so credit it
        // in the rate series too. Mirroring the position-offset change keeps
        // `submitted_position - drift_playback_step` continuous (M5-08): without
        // this, every accepted fill leaves a step that reads as a bogus ppm.
        self.drift_playback_step = self
            .drift_playback_step
            .saturating_sub(inserted_frames as i64);
    }

    /// Record frames trimmed from the ring by the reader's *soft* target trim
    /// (backlog above the target + a 20 ms hysteresis band, still below the
    /// 200 ms hard limit). Ticket M5-03: the oldest frames are dropped and
    /// counted separately — this is normal buffer steering (the deliberate
    /// M5-03 deviation from §8's four-intervention list, which governs
    /// concealment accounting, not buffer management), NOT a concealment
    /// intervention, so it must not touch `concealments`/`concealed_frames`.
    pub fn record_soft_trim(&mut self, frames: u64) {
        if frames == 0 {
            return;
        }
        self.soft_trimmed_frames = self.soft_trimmed_frames.saturating_add(frames);
        self.soft_trim_events += 1;
        // The skipped media had capture positions, so the playout cursor
        // advanced over them even though it was not a concealment event.
        self.position_adjust_frames = self.position_adjust_frames.saturating_add(frames as i64);
        // Fold the position step out of the drift series rather than clearing
        // the whole 10 s window (M5-08): a trim is not clock drift.
        self.drift_playback_step = self.drift_playback_step.saturating_add(frames as i64);
    }

    /// Total frames dropped by soft target trims.
    pub const fn soft_trimmed_frames(&self) -> u64 {
        self.soft_trimmed_frames
    }

    /// Number of soft target-trim batches.
    pub const fn soft_trim_events(&self) -> u64 {
        self.soft_trim_events
    }

    /// Record frames trimmed from the ring by the Kotlin reader's §8 hard-limit
    /// re-anchor. M5-04 owns this accounting: the deletion is counted as
    /// `ReAnchor` concealment, and the submitted/playout position jumps
    /// forward over the deleted media so latency never accumulates.
    pub fn record_trim(&mut self, frames: u64) {
        self.trimmed_frames = self.trimmed_frames.saturating_add(frames);
        self.trim_events += 1;
        if frames == 0 {
            return;
        }
        self.record_concealment(ConcealmentKind::ReAnchor, -(frames as i64));
        self.position_adjust_frames = self.position_adjust_frames.saturating_add(frames as i64);
        // A re-anchor is a position discontinuity, not clock drift: fold the
        // step out of the drift series instead of discarding the window
        // (M5-08).
        self.drift_playback_step = self.drift_playback_step.saturating_add(frames as i64);
    }

    /// Total frames trimmed by the reader (re-anchor, M5-04 boundary).
    pub const fn trimmed_frames(&self) -> u64 {
        self.trimmed_frames
    }

    /// Number of trim/resync batches the reader performed.
    pub const fn trim_events(&self) -> u64 {
        self.trim_events
    }

    /// Bytes written into the native ring so far, or `None` without a ring.
    pub fn ring_write_position(&self) -> Option<u64> {
        self.ring.as_ref().map(MediaRing::write_pos)
    }

    /// Whether held media is still waiting for its configuration (§7.6).
    pub fn holding_unknown_config(&self) -> bool {
        self.holding.is_some()
    }

    /// Periodic tick: enforces §7.6's 2 s give-up bound even when no further
    /// datagram arrives. Returns `true` exactly when this tick crosses the
    /// bound while a hold is still active, so the session can emit 403 once.
    /// After `apply_configuration` releases the hold, no later tick fires.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        self.check_hold_give_up(now_ms)
    }

    // ---- internals -------------------------------------------------------

    fn hold(&mut self, now_ms: u64, datagram: &[u8], generation: u32) -> DatagramOutcome {
        let arrival = || HeldDatagram {
            arrival_ms: now_ms,
            bytes: datagram.to_vec(),
        };
        // §7.6: bounded hold — ≤ 32 datagrams or ≤ 100 ms of media; overflow
        // drops the oldest held datagram. Never decoded with the old config.
        match &mut self.holding {
            Some(held) if held.generation == generation => {
                held.datagrams.push_back(arrival());
                self.trim_hold();
            }
            Some(held) if generation < held.generation => {
                // Older than what we are already holding for: superseded.
                self.receiver.note_stale_generation();
                self.interval.unknown_config_drops += 1;
                return DatagramOutcome::StaleGeneration;
            }
            Some(_) => {
                // A newer unknown generation replaces the hold.
                self.holding = Some(HeldMedia {
                    generation,
                    datagrams: VecDeque::from([arrival()]),
                });
                self.held_since_ms = Some(now_ms);
            }
            None => {
                self.holding = Some(HeldMedia {
                    generation,
                    datagrams: VecDeque::from([arrival()]),
                });
                self.held_since_ms = Some(now_ms);
            }
        }
        let _ = self.check_hold_give_up(now_ms);
        DatagramOutcome::HeldUnknownConfig
    }

    fn trim_hold(&mut self) {
        let max_hold_frames = u64::from(self.geometry.sample_rate) * UNKNOWN_CONFIG_MAX_MS / 1_000;
        let frame_bytes = self.geometry.frame_bytes().max(1);
        let Some(held) = self.holding.as_mut() else {
            return;
        };
        let mut held_frames: u64 = held
            .datagrams
            .iter()
            .map(|d| (d.bytes.len().saturating_sub(HEADER_SIZE) / frame_bytes) as u64)
            .sum();
        while held.datagrams.len() > UNKNOWN_CONFIG_MAX_DATAGRAMS || held_frames > max_hold_frames {
            let Some(dropped) = held.datagrams.pop_front() else {
                break;
            };
            let frames = (dropped.bytes.len().saturating_sub(HEADER_SIZE) / frame_bytes) as u64;
            held_frames = held_frames.saturating_sub(frames);
            self.receiver.note_stale_generation();
            self.interval.unknown_config_drops += 1;
        }
    }

    /// §7.6 step 3: still unknown after 2 s → the session sends
    /// Error(403)+Close(403). The clock is the session's monotonic millisecond
    /// counter, injected so the bound is testable without sleeping.
    fn check_hold_give_up(&mut self, now_ms: u64) -> bool {
        // Only a live, still-unknown hold can give up. `apply_configuration`
        // clears `holding` and `held_since_ms`, so a configuration that arrived
        // before the bound must never later trigger a spurious 403.
        if self.holding.is_none() {
            return false;
        }
        let Some(since) = self.held_since_ms else {
            return false;
        };
        if now_ms.saturating_sub(since) >= UNKNOWN_CONFIG_GIVE_UP_MS {
            self.gave_up_unknown_config = true;
            return self.gave_up_unknown_config;
        }
        false
    }

    fn deliver_payload(&mut self, payload: &[u8]) {
        let frames = self.geometry.frames(payload.len()).unwrap_or(0);
        if let Some(ring) = self.ring.as_mut() {
            if ring.write(payload) {
                self.delivered_frames += frames;
            }
        } else {
            self.delivered_frames += frames;
        }
    }

    fn deliver(&mut self, arrival_ms: u64, datagram: &[u8]) {
        if let Ok((header, payload)) = MediaHeader::parse(datagram) {
            if header.config_generation == self.generation {
                // Held media released by a configuration change must cross the
                // SAME playout frontier as live media, or the frontier is left
                // behind and the next live datagram triggers bogus silence.
                //
                // It is *not* a new arrival: `arrival_ms` is the instant the
                // datagram really arrived, and A.8 must not observe it again
                // (that would inject a bogus 0/old-time sample at release).
                self.accept_observed(arrival_ms, &header, payload, false);
            }
        }
    }

    /// The shared observe → jitter → deliver path for both live datagrams and
    /// held media released by `apply_configuration`. Keeping one path is what
    /// keeps the playout frontier in sync across a config change (§7.6/§8).
    ///
    /// `observe_arrival` is true only for a live receive: it is the only case
    /// where `now_ms` is a genuine arrival instant for the RFC 3550 A.8
    /// estimate. Replaying held media passes the original arrival time and
    /// `false`.
    fn accept_observed(
        &mut self,
        now_ms: u64,
        header: &MediaHeader,
        payload: &[u8],
        observe_arrival: bool,
    ) -> DatagramOutcome {
        let verdict = self.receiver.observe(header);
        let (extended, outcome) = match verdict {
            ObserveVerdict::Duplicate => {
                self.interval.duplicates += 1;
                return DatagramOutcome::Duplicate;
            }
            ObserveVerdict::Ok { gap } => {
                self.interval.received += 1;
                self.interval.lost += i64::from(gap);
                (self.receiver.extended_highest, DatagramOutcome::Delivered)
            }
            ObserveVerdict::Reordered { distance } => {
                self.interval.received += 1;
                self.interval.reordered += 1;
                (
                    self.receiver
                        .extended_highest
                        .saturating_sub(u64::from(distance)),
                    DatagramOutcome::Delivered,
                )
            }
        };

        // §8: RFC 3550 A.8 jitter is measured on every accepted *live*
        // arrival in arrival order, independent of the playout scheduling
        // below. Held media replay is not a new arrival.
        if observe_arrival {
            self.jitter.observe_arrival(now_ms, header.timestamp);
        }
        let frames = self.geometry.frames(payload.len()).unwrap_or(0);
        let mut chunks: Vec<Vec<u8>> = Vec::new();
        let result = {
            let jitter = &mut self.jitter;
            let mut write = |bytes: &[u8]| chunks.push(bytes.to_vec());
            if observe_arrival {
                jitter.submit(
                    now_ms,
                    extended,
                    header.timestamp,
                    frames,
                    payload,
                    &mut write,
                )
            } else {
                // Released held media keeps its real arrival instant for the
                // deadline check but must not move the live playout anchor.
                jitter.submit_replay(
                    now_ms,
                    extended,
                    header.timestamp,
                    frames,
                    payload,
                    &mut write,
                )
            }
        };
        for chunk in &chunks {
            self.deliver_payload(chunk);
        }
        for (kind, delta) in result.concealments {
            self.record_concealment(kind, delta);
        }
        if !result.late {
            let new_capture = header.timestamp.saturating_add(frames);
            if result.rebased {
                // M5-05/M5-08: a contiguous sequence with a jumped timestamp
                // is a server-side drop. The playout cursor never advances
                // over the lost media, so the capture−playback difference
                // takes a one-time step. Fold that step out of the capture
                // series instead of clearing the 10 s window; the jitter
                // arrival window still restarts (this is not interarrival
                // jitter).
                let gap = header.timestamp.saturating_sub(self.capture_position);
                self.drift_capture_step = self.drift_capture_step.saturating_add(gap);
            }
            // The capture side of the §8 drift measurement: the wire timestamp
            // is the Sender's capture-clock position.
            self.capture_position = self.capture_position.max(new_capture);
            self.saw_capture = true;
        }
        if result.rebased {
            self.jitter.reset_arrival();
        }
        if result.late {
            self.interval.late_drops += 1;
            self.late_drops += 1;
            return DatagramOutcome::LateDropped;
        }
        outcome
    }
}
