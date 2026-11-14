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

use crate::media::{MediaHeader, MediaReceiver, ObserveVerdict, HEADER_SIZE};
use crate::proto::{Configuration, PlaybackDiagnostics};
use crate::ring::MediaRing;

/// §7.6 bounds: hold at most 100 ms of media or 32 datagrams, whichever fills
/// first; overflow drops the oldest held datagram.
pub const UNKNOWN_CONFIG_MAX_DATAGRAMS: usize = 32;
pub const UNKNOWN_CONFIG_MAX_MS: u64 = 100;

/// How long a held generation may stay unknown before the session gives up
/// (§7.6 step 3: 2 s → Error(403)+Close(403)).
pub const UNKNOWN_CONFIG_GIVE_UP_MS: u64 = 2_000;

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
    holding: Option<(u32, VecDeque<Vec<u8>>)>,
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
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct IntervalCounters {
    received: u64,
    duplicates: u64,
    reordered: u64,
    lost: i64,
    unknown_config_drops: u64,
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
        })
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
                self.geometry = geometry;
            }
            if let Some((held_generation, held)) = self.holding.take() {
                if held_generation == generation {
                    for datagram in held {
                        self.deliver(&datagram);
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
        match self.receiver.observe(&header) {
            ObserveVerdict::Duplicate => {
                self.interval.duplicates += 1;
                DatagramOutcome::Duplicate
            }
            ObserveVerdict::Ok { gap } => {
                self.interval.received += 1;
                self.interval.lost += i64::from(gap);
                self.deliver_payload(payload);
                DatagramOutcome::Delivered
            }
            ObserveVerdict::Reordered { distance: _ } => {
                self.interval.received += 1;
                self.interval.reordered += 1;
                // M4 has no playout scheduling: a reordered-but-unseen packet
                // is still delivered in arrival order. M5 owns late drops.
                self.deliver_payload(payload);
                DatagramOutcome::Delivered
            }
        }
    }

    /// Record what the platform playback API actually accepted (§9 evidence).
    pub fn record_playback(&mut self, report: PlaybackReport) {
        self.report = report;
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
            cumulative_late_drops: 0,
            interval_late_drops: 0,
            cumulative_unknown_config_drops: self.receiver.unknown_config_drops,
            interval_unknown_config_drops: self.interval.unknown_config_drops,
            cumulative_concealments: 0,
            interval_concealments: 0,
            concealed_frames: 0,
            underruns: self.report.underruns,
            // §9 explicit-zero semantics: RFC 3550 A.8 jitter estimation and
            // the M5 playout interventions below are NOT implemented in M4.
            // Reporting 0 (not a fabricated estimate) is the ticket-14 rule;
            // M5 owns the estimator, late drops, concealment and target buffer.
            jitter_microseconds: 0,
            buffer_current_ms: self.report.buffer_current_ms,
            buffer_target_ms,
            submitted_position: self.submitted_position(),
        };
        self.interval = IntervalCounters::default();
        diagnostics
    }

    /// `anchor + frames actually submitted to the platform API` (§9: a socket
    /// receive or ring write is never playback evidence).
    pub fn submitted_position(&self) -> u64 {
        let frame_bytes = self.geometry.frame_bytes() as u64;
        if frame_bytes == 0 {
            return self.anchor;
        }
        self.anchor + self.report.submitted_bytes / frame_bytes
    }

    /// Frames written to the ring (native delivery, not playback).
    pub const fn delivered_frames(&self) -> u64 {
        self.delivered_frames
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
        // §7.6: bounded hold — ≤ 32 datagrams or ≤ 100 ms of media; overflow
        // drops the oldest held datagram. Never decoded with the old config.
        match &mut self.holding {
            Some((held_generation, held)) if *held_generation == generation => {
                held.push_back(datagram.to_vec());
                self.trim_hold();
            }
            Some((held_generation, held)) if generation < *held_generation => {
                // Older than what we are already holding for: superseded.
                self.receiver.note_stale_generation();
                self.interval.unknown_config_drops += 1;
                let _ = held;
                return DatagramOutcome::StaleGeneration;
            }
            Some(_) => {
                // A newer unknown generation replaces the hold.
                self.holding = Some((generation, VecDeque::from([datagram.to_vec()])));
                self.held_since_ms = Some(now_ms);
            }
            None => {
                self.holding = Some((generation, VecDeque::from([datagram.to_vec()])));
                self.held_since_ms = Some(now_ms);
            }
        }
        let _ = self.check_hold_give_up(now_ms);
        DatagramOutcome::HeldUnknownConfig
    }

    fn trim_hold(&mut self) {
        let max_hold_frames =
            u64::from(self.geometry.sample_rate) * UNKNOWN_CONFIG_MAX_MS / 1_000;
        let frame_bytes = self.geometry.frame_bytes().max(1);
        let Some((_, held)) = self.holding.as_mut() else {
            return;
        };
        let mut held_frames: u64 = held
            .iter()
            .map(|datagram| (datagram.len().saturating_sub(HEADER_SIZE) / frame_bytes) as u64)
            .sum();
        while held.len() > UNKNOWN_CONFIG_MAX_DATAGRAMS || held_frames > max_hold_frames {
            let Some(dropped) = held.pop_front() else {
                break;
            };
            let frames = (dropped.len().saturating_sub(HEADER_SIZE) / frame_bytes) as u64;
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

    fn deliver(&mut self, datagram: &[u8]) {
        if let Ok((header, payload)) = MediaHeader::parse(datagram) {
            if header.config_generation == self.generation {
                self.receiver.observe(&header);
                self.deliver_payload(payload);
            }
        }
    }
}
