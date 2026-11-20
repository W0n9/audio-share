//! The per-session media pump: capture chunks → paced DATAGRAMs.
//!
//! Deliberately free of I/O. The pump holds the capture's raw PCM, and the
//! driver hands it the *current* `max_datagram_size()` at send time (§7.3,
//! D2) — never a cached limit — so a path-limit reduction simply produces
//! smaller datagrams without any re-chunking of encoded output. The §7.4
//! send-ahead window is enforced as a media-rate token bucket on an injected
//! clock.
//!
//! Keeping the PCM undivided until send time is what makes the per-send
//! sizing honest: there is no encoded datagram in flight that a smaller path
//! could no longer carry.

use std::collections::VecDeque;
use std::time::Instant;

use crate::media::{MediaPacketizer, PacketError};
use crate::media_source::{MediaChunk, StreamFormat, StreamState};
use crate::send_queue::MediaPacer;

/// The capture contract: every buffer is a whole number of sample frames and
/// belongs to the stream this pump sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaPumpError {
    /// The datagram limit cannot carry the header plus one sample frame; the
    /// session closes with 602 (§7.3).
    DatagramUnsupported,
    /// The chunk is not frame-aligned, or belongs to another stream instance
    /// or configuration generation.
    MisalignedPcm,
}

/// Undelivered capture audio for one session.
#[derive(Debug)]
struct PendingChunk {
    pcm: Vec<u8>,
    /// Capture-clock position of the first frame still in `pcm`. Advanced as
    /// the chunk is drained so the packetizer can re-anchor even after a
    /// pre-packetize drop (§7.2).
    start_frame: u64,
}

/// Frames of capture audio a stalled Client may accumulate before the newest
/// capture buffer is dropped (§7.4: bounded per-Client queue). M4 allows
/// 500 ms of unencoded capture so a short scheduler stall does not lose audio;
/// the separate §7.4 send-ahead invariant (≤ 100 ms) is enforced by
/// `MediaPacer`, not by this buffer bound.
pub const MAX_PENDING_MS: u64 = 500;

/// One session's media send state. Owned exclusively by the session's media
/// task; never shared.
#[derive(Debug)]
pub struct MediaPump {
    packetizer: MediaPacketizer,
    pending: VecDeque<PendingChunk>,
    pacer: MediaPacer,
    format: StreamFormat,
    stream: StreamState,
    dropped_chunks: u64,
    started: bool,
}

impl MediaPump {
    /// Start sending `stream` in the announced geometry.
    pub fn new(stream: StreamState) -> Self {
        Self {
            packetizer: MediaPacketizer::new(
                stream.stream_instance_id,
                stream.config_generation,
                stream.format.frame_bytes(),
            ),
            pending: VecDeque::new(),
            pacer: MediaPacer::new(stream.format.sample_rate),
            format: stream.format,
            stream,
            dropped_chunks: 0,
            started: false,
        }
    }

    /// The instance this pump is sending.
    pub const fn stream_instance_id(&self) -> u32 {
        self.stream.stream_instance_id
    }

    /// The generation this pump is sending.
    pub const fn config_generation(&self) -> u32 {
        self.stream.config_generation
    }

    /// The capture-clock position of the next frame to send.
    pub const fn next_frame(&self) -> u64 {
        self.packetizer.next_timestamp()
    }

    /// Capture buffers dropped because the pending bound was reached.
    pub const fn dropped_chunks(&self) -> u64 {
        self.dropped_chunks
    }

    /// Frames the §7.4 pacer has released (M5-08 periodic instrumentation).
    pub const fn pacer_consumed_frames(&self) -> u64 {
        self.pacer.consumed_frames()
    }

    /// `try_consume` invocations so far (M5-08 periodic instrumentation).
    pub const fn pacer_calls(&self) -> u64 {
        self.pacer.calls()
    }

    /// Buffered capture audio in sample frames.
    pub fn pending_frames(&self) -> u64 {
        let frame_bytes = self.format.frame_bytes();
        if frame_bytes == 0 {
            return 0;
        }
        (self
            .pending
            .iter()
            .map(|chunk| chunk.pcm.len())
            .sum::<usize>()
            / frame_bytes) as u64
    }

    /// Accept one capture buffer. Non-blocking and bounded: beyond
    /// [`MAX_PENDING_MS`] of audio the newest buffer is dropped and counted,
    /// so a stalled Client can never grow this session without bound.
    ///
    /// The first accepted buffer positions the media clock at its capture
    /// position, so a late Subscriber starts exactly at the anchor its
    /// `StreamStarted` announced.
    pub fn accept(&mut self, chunk: MediaChunk) -> Result<(), MediaPumpError> {
        if chunk.stream_instance_id != self.stream.stream_instance_id
            || chunk.config_generation != self.stream.config_generation
        {
            return Err(MediaPumpError::MisalignedPcm);
        }
        let frame_bytes = self.format.frame_bytes();
        if frame_bytes == 0 || !chunk.pcm.len().is_multiple_of(frame_bytes) {
            return Err(MediaPumpError::MisalignedPcm);
        }
        if !self.started {
            self.packetizer
                .seek(0, chunk.start_frame)
                .map_err(map_packet_error)?;
            self.started = true;
        }
        let max_frames = u64::from(self.format.sample_rate) * MAX_PENDING_MS / 1_000;
        let incoming = (chunk.pcm.len() / frame_bytes) as u64;
        if self.pending_frames() + incoming > max_frames {
            self.dropped_chunks += 1;
            return Ok(());
        }
        self.pending.push_back(PendingChunk {
            pcm: chunk.pcm,
            start_frame: chunk.start_frame,
        });
        Ok(())
    }

    /// Whether a capture buffer has positioned the media clock.
    pub const fn has_started(&self) -> bool {
        self.started
    }

    /// Record a decode-affecting configuration change: the next datagram
    /// carries the new generation and `CONFIG_CHANGE` (§7.1/§7.2). Buffers of
    /// an older generation are refused from then on.
    pub fn set_generation(&mut self, generation: u32) -> Result<(), MediaPumpError> {
        self.packetizer
            .set_generation(generation)
            .map_err(map_packet_error)?;
        self.stream.config_generation = generation;
        Ok(())
    }

    /// Produce the datagrams that may be sent now at the given transport
    /// limit, enforcing the send-ahead window. At most `max` datagrams are
    /// produced so one tick cannot monopolize the runtime.
    pub fn take_sendable(
        &mut self,
        now: Instant,
        max_datagram_size: usize,
        max: usize,
    ) -> Result<Vec<Vec<u8>>, MediaPumpError> {
        let frame_bytes = self.format.frame_bytes();
        if frame_bytes == 0 {
            return Err(MediaPumpError::MisalignedPcm);
        }
        let Some(payload_max) = max_datagram_size.checked_sub(crate::media::HEADER_SIZE) else {
            return Err(MediaPumpError::DatagramUnsupported);
        };
        let frames_per_datagram = payload_max / frame_bytes;
        if frames_per_datagram == 0 {
            return Err(MediaPumpError::DatagramUnsupported);
        }
        let payload_max = frames_per_datagram * frame_bytes;

        let mut out = Vec::new();
        while out.len() < max {
            let Some(front) = self.pending.front_mut() else {
                break;
            };
            let take_bytes = front.pcm.len().min(payload_max);
            // Only whole frames leave the pump.
            let take_bytes = take_bytes - (take_bytes % frame_bytes);
            if take_bytes == 0 {
                break;
            }
            let frames = (take_bytes / frame_bytes) as u64;
            if self.pacer.try_consume(now, frames).is_err() {
                break;
            }
            // §7.2: the header timestamp is the capture-clock position of the
            // first payload frame. Re-anchor before packetizing so a chunk
            // dropped pre-packetize (pending bound) leaves an honest timestamp
            // gap on the wire — sequence stays +1 per datagram sent.
            let chunk_start = front.start_frame;
            let pcm: Vec<u8> = front.pcm.drain(..take_bytes).collect();
            front.start_frame = front.start_frame.saturating_add(frames);
            if front.pcm.is_empty() {
                self.pending.pop_front();
            }
            // The capture clock never moves backwards; a chunk arriving out of
            // order would be a contract violation (MediaTap positions chunks).
            debug_assert!(chunk_start >= self.packetizer.next_timestamp());
            self.packetizer
                .seek(self.packetizer.next_sequence(), chunk_start)
                .map_err(map_packet_error)?;
            let datagrams = self
                .packetizer
                .packetize(&pcm, max_datagram_size)
                .map_err(map_packet_error)?;
            out.extend(datagrams);
        }
        Ok(out)
    }

    /// Whether there is nothing to send.
    pub fn idle(&self) -> bool {
        self.pending.is_empty()
    }

    /// Release buffered media on teardown (§5.4 invariant 6).
    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

/// A snapshot of the two server-side drop counters, for the diagnostics log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropReport {
    /// Capture buffers dropped at the pending bound (`MediaPump`).
    pub pump_dropped_chunks: u64,
    /// Encoded datagrams dropped at the queue bound (`MediaSendQueue`).
    pub queue_dropped_newest: u64,
    /// Buffered capture audio at report time, in sample frames.
    pub pending_frames: u64,
}

/// Bounded-cadence reporter for server-side drops. Emits only when the
/// diagnostics interval elapsed *and* a counter changed since the last report,
/// so `server.log` distinguishes pre-packetize/server drops from network loss
/// without flooding the log. Pure: the clock is injected, never read here.
#[derive(Debug)]
pub struct DropReporter {
    cadence: std::time::Duration,
    last_report: Option<Instant>,
    last_pump: u64,
    last_queue: u64,
}

impl DropReporter {
    /// A reporter that logs at most once per `cadence`.
    pub fn new(cadence: std::time::Duration) -> Self {
        Self {
            cadence,
            last_report: None,
            last_pump: 0,
            last_queue: 0,
        }
    }

    /// Return a report when `now` is at least one cadence past the last report
    /// and either drop counter changed. The first poll establishes the baseline
    /// (no report) so a zero-valued startup line is never emitted.
    pub fn poll(
        &mut self,
        now: Instant,
        pump_dropped_chunks: u64,
        queue_dropped_newest: u64,
        pending_frames: u64,
    ) -> Option<DropReport> {
        let due = match self.last_report {
            None => {
                self.last_report = Some(now);
                false
            }
            Some(last) => now.saturating_duration_since(last) >= self.cadence,
        };
        let changed =
            pump_dropped_chunks != self.last_pump || queue_dropped_newest != self.last_queue;
        if !due || !changed {
            return None;
        }
        self.last_report = Some(now);
        self.last_pump = pump_dropped_chunks;
        self.last_queue = queue_dropped_newest;
        Some(DropReport {
            pump_dropped_chunks,
            queue_dropped_newest,
            pending_frames,
        })
    }
}

/// A periodic snapshot of the media pump, emitted even when nothing was
/// dropped (M5-08). `DropReport` only fires at drop instants, so its
/// `pending_frames` is a drop-time-biased sample; this reporter samples every
/// cadence so support can see whether backlog saw-tooths or is pinned, and can
/// measure the pacer call rate and delivered-frame rate directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaStats {
    /// Buffered capture audio at report time, in sample frames.
    pub pending_frames: u64,
    /// Frames released by the §7.4 pacer since the session started.
    pub consumed_frames: u64,
    /// `MediaPacer::try_consume` invocations since the session started.
    pub pacer_calls: u64,
    /// `try_consume` invocations per second over the reporting interval.
    pub pacer_calls_per_sec: u64,
}

/// Fixed-cadence sampler for [`MediaStats`]. The first poll establishes the
/// baseline (no line) so a startup zero is never emitted; every later poll at
/// or past the cadence emits, whether or not counters changed.
#[derive(Debug)]
pub struct MediaStatsReporter {
    cadence: std::time::Duration,
    last_report: Option<Instant>,
    last_calls: u64,
}

impl MediaStatsReporter {
    /// A reporter that logs at most once per `cadence`.
    pub fn new(cadence: std::time::Duration) -> Self {
        Self {
            cadence,
            last_report: None,
            last_calls: 0,
        }
    }

    /// Return a snapshot when `now` is at least one cadence past the last
    /// report. The first poll establishes the baseline and returns `None`.
    pub fn poll(
        &mut self,
        now: Instant,
        pending_frames: u64,
        consumed_frames: u64,
        pacer_calls: u64,
    ) -> Option<MediaStats> {
        let last = match self.last_report {
            None => {
                self.last_report = Some(now);
                self.last_calls = pacer_calls;
                return None;
            }
            Some(last) => last,
        };
        let elapsed = now.saturating_duration_since(last);
        if elapsed < self.cadence {
            return None;
        }
        let elapsed_ms = elapsed.as_millis().max(1);
        let calls_delta = u128::from(pacer_calls.saturating_sub(self.last_calls));
        let per_sec = u64::try_from(calls_delta * 1_000 / elapsed_ms).unwrap_or(u64::MAX);
        self.last_report = Some(now);
        self.last_calls = pacer_calls;
        Some(MediaStats {
            pending_frames,
            consumed_frames,
            pacer_calls,
            pacer_calls_per_sec: per_sec,
        })
    }
}

fn map_packet_error(error: PacketError) -> MediaPumpError {
    match error {
        PacketError::DatagramTooSmall { .. } => MediaPumpError::DatagramUnsupported,
        _ => MediaPumpError::MisalignedPcm,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::MediaHeader;

    fn stream(instance: u32, generation: u32) -> StreamState {
        StreamState {
            stream_instance_id: instance,
            config_generation: generation,
            next_frame: 0,
            format: StreamFormat {
                sample_rate: 48_000,
                channels: 2,
                bytes_per_sample: 4,
            },
        }
    }

    fn chunk(instance: u32, generation: u32, start: u64, bytes: usize) -> MediaChunk {
        MediaChunk {
            pcm: vec![0x5a; bytes],
            start_frame: start,
            end_frame: start + (bytes / 8) as u64,
            stream_instance_id: instance,
            config_generation: generation,
        }
    }

    /// Drain every pending frame through the pacer, advancing the injected
    /// clock 10 ms per tick so a full 100 ms window is always available.
    fn drain_all(pump: &mut MediaPump, now: &mut Instant) -> Vec<Vec<u8>> {
        let mut all = Vec::new();
        for _ in 0..512 {
            let datagrams = pump.take_sendable(*now, 1_192, 64).expect("send");
            *now += std::time::Duration::from_millis(10);
            if datagrams.is_empty() {
                if pump.idle() {
                    break;
                }
                continue;
            }
            all.extend(datagrams);
        }
        all
    }

    /// Captured audio becomes golden-vector-shaped datagrams sized from the
    /// limit the caller supplies at send time.
    #[test]
    fn captured_audio_is_packetized_at_the_current_limit() {
        let mut pump = MediaPump::new(stream(1, 1));
        pump.accept(chunk(1, 1, 0, 9_600)).expect("accept");
        let now = Instant::now();
        let sent = pump.take_sendable(now, 1_192, 64).expect("send");
        assert!(!sent.is_empty());
        for datagram in &sent {
            assert!(datagram.len() <= 1_192);
            let (header, payload) = MediaHeader::parse(datagram).expect("header");
            assert_eq!(header.stream_instance_id, 1);
            assert_eq!(header.config_generation, 1);
            assert_eq!(payload.len() % 8, 0, "whole frames only");
        }
        let first = MediaHeader::parse(&sent[0]).expect("header").0;
        assert_eq!(first.sequence, 0);
        assert_eq!(first.timestamp, 0);
    }

    /// A reduced path limit produces smaller datagrams on the next call, with
    /// no encoded datagram left over that no longer fits.
    #[test]
    fn a_reduced_limit_takes_effect_on_the_next_send() {
        let mut pump = MediaPump::new(stream(1, 1));
        pump.accept(chunk(1, 1, 0, 9_600)).expect("accept");
        let now = Instant::now();
        let wide = pump.take_sendable(now, 1_500, 64).expect("send");
        assert!(wide.iter().any(|d| d.len() > 1_192));
        let narrow = pump.take_sendable(now, 1_192, 64).expect("send");
        assert!(
            narrow.iter().all(|d| d.len() <= 1_192),
            "every datagram respects the new limit"
        );
    }

    /// A datagram limit below header + one frame is an explicit 602-class
    /// failure, never fragmentation.
    #[test]
    fn an_unsupported_datagram_path_is_explicit() {
        let mut pump = MediaPump::new(StreamState {
            stream_instance_id: 1,
            config_generation: 1,
            next_frame: 0,
            format: StreamFormat {
                sample_rate: 48_000,
                channels: 2,
                bytes_per_sample: 1_024,
            },
        });
        pump.accept(chunk(1, 1, 0, 4_096)).expect("accept");
        assert_eq!(
            pump.take_sendable(Instant::now(), 1_200, 4),
            Err(MediaPumpError::DatagramUnsupported)
        );
    }

    /// Media from another instance or generation never enters this pump.
    #[test]
    fn foreign_stream_chunks_are_refused() {
        let mut pump = MediaPump::new(stream(2, 1));
        assert_eq!(
            pump.accept(chunk(3, 1, 0, 96)),
            Err(MediaPumpError::MisalignedPcm)
        );
        assert_eq!(
            pump.accept(chunk(2, 2, 0, 96)),
            Err(MediaPumpError::MisalignedPcm)
        );
    }

    /// A stalled Client cannot grow the pump without bound: the newest
    /// buffers are dropped once the pending bound is reached.
    #[test]
    fn a_stalled_client_drops_newest_and_counts_it() {
        let mut pump = MediaPump::new(stream(1, 1));
        // 500 ms at 48 kHz = 24000 frames = 192000 bytes; push 40 × 8192.
        for index in 0..40u64 {
            pump.accept(chunk(1, 1, index * 1_024, 8_192))
                .expect("accept");
        }
        assert!(pump.dropped_chunks() > 0, "the bound must be enforced");
        assert!(pump.pending_frames() <= 24_000);
    }

    /// The send-ahead window caps how much media one tick may burst.
    #[test]
    fn one_tick_never_sends_more_than_one_window() {
        let mut pump = MediaPump::new(stream(1, 1));
        pump.accept(chunk(1, 1, 0, 48_000 * 8 * 2)).expect("accept");
        let now = Instant::now();
        let sent = pump.take_sendable(now, 1_200, 10_000).expect("send");
        let frames: u64 = sent
            .iter()
            .map(|d| (d.len() - crate::media::HEADER_SIZE) as u64 / 8)
            .sum();
        assert!(frames <= 4_800, "one 100 ms window at 48 kHz: {frames}");
    }

    /// §4.4: a late Subscriber's first datagram starts at the capture position
    /// the reliable `StreamStarted` anchor announced.
    #[test]
    fn a_late_subscriber_starts_at_the_announced_anchor() {
        let mut pump = MediaPump::new(stream(1, 1));
        pump.accept(chunk(1, 1, 96_000, 960)).expect("accept");
        let sent = pump.take_sendable(Instant::now(), 1_192, 8).expect("send");
        let first = MediaHeader::parse(&sent[0]).expect("header").0;
        assert_eq!(first.timestamp, 96_000);
        assert_eq!(
            first.sequence, 0,
            "the instance's sequence space starts here"
        );
    }

    #[test]
    fn drop_reporter_fires_only_on_change_at_the_diagnostics_cadence() {
        use std::time::Duration;
        let mut reporter = super::DropReporter::new(Duration::from_secs(5));
        let t0 = Instant::now();
        // Initial poll at t0 with zero counters logs nothing (nothing changed).
        assert!(reporter.poll(t0, 0, 0, 0).is_none());
        // A drop happens but no diagnostics tick has elapsed.
        assert!(reporter
            .poll(t0 + Duration::from_secs(1), 3, 0, 0)
            .is_none());
        // At the 5 s tick the changed counters are reported once.
        let report = reporter
            .poll(t0 + Duration::from_secs(5), 3, 2, 100)
            .expect("report at tick");
        assert_eq!(report.pump_dropped_chunks, 3);
        assert_eq!(report.queue_dropped_newest, 2);
        assert_eq!(report.pending_frames, 100);
        // Same counters at the next tick: nothing changed, no repeat log.
        assert!(reporter
            .poll(t0 + Duration::from_secs(10), 3, 2, 100)
            .is_none());
        // A fresh drop at the next tick is reported.
        assert!(reporter
            .poll(t0 + Duration::from_secs(15), 4, 2, 100)
            .is_some());
    }

    /// M5-08: the periodic line must appear at the 5 s cadence even when no
    /// drop occurred, so support never sees a drop-time-biased sample.
    #[test]
    fn periodic_stats_are_emitted_even_with_zero_drops() {
        use std::time::Duration;
        let mut reporter = super::MediaStatsReporter::new(Duration::from_secs(5));
        let t0 = Instant::now();
        // Baseline: no startup zero line.
        assert!(reporter.poll(t0, 0, 0, 0).is_none());
        // No drop counters exist on this reporter; it still emits at 5 s.
        let stats = reporter
            .poll(t0 + Duration::from_secs(5), 480, 24_000, 500)
            .expect("periodic line at the cadence");
        assert_eq!(stats.pending_frames, 480);
        assert_eq!(stats.consumed_frames, 24_000);
        assert_eq!(stats.pacer_calls, 500);
        assert_eq!(stats.pacer_calls_per_sec, 100, "500 calls over 5 s");
        // A zero-activity interval (idle session) still emits.
        assert!(reporter
            .poll(t0 + Duration::from_secs(10), 0, 24_000, 500)
            .is_some());
    }

    /// §7.4 guard (characterization): when a whole capture callback's worth
    /// of media is offered at once, the pacer releases at most one 100 ms
    /// window up front and then no more than 100 ms of media per 100 ms of
    /// wall time — never an unbounded whole-callback burst. Thresholds are
    /// literals from the spec (48 kHz × 100 ms = 4800 frames; 10 ms = 480).
    #[test]
    fn send_ahead_never_exceeds_window_plus_rate_times_elapsed() {
        let mut pump = MediaPump::new(stream(1, 1));
        let t0 = Instant::now();
        // Offer one full 500 ms pending bound at once: 24000 frames at 48 kHz.
        // An unbounded sender would dump all 24000 in the first tick.
        pump.accept(chunk(1, 1, 0, 24_000 * 8)).expect("accept");
        let mut sent_frames = 0u64;
        for k in 0..100u64 {
            let now = t0 + std::time::Duration::from_millis(10 * k);
            let datagrams = pump.take_sendable(now, 1_192, 10_000).expect("send");
            sent_frames += datagrams
                .iter()
                .map(|d| ((d.len() - crate::media::HEADER_SIZE) / 8) as u64)
                .sum::<u64>();
            // By tick k (k×10 ms elapsed) the pacer may have sent at most one
            // 4800-frame window up front plus 480 frames per 10 ms elapsed.
            assert!(
                sent_frames <= 4_800 + 480 * k,
                "tick {k}: sent {sent_frames} exceeds the §7.4 window"
            );
        }
        // Not vacuous: the whole 500 ms burst eventually drains at the paced
        // rate (24000 frames ÷ 480/tick = 50 ticks, well within 100).
        assert_eq!(sent_frames, 24_000, "the full burst must drain, paced");
    }

    /// §7.2: a chunk dropped pre-packetize leaves a wire timestamp gap equal
    /// to its frame count, while sequence stays contiguous per datagram sent.
    #[test]
    fn a_dropped_chunk_jumps_the_wire_timestamp_not_the_sequence() {
        let mut pump = MediaPump::new(stream(1, 1));
        let mut now = Instant::now();
        // Fill the 500 ms pending bound: 24 chunks × 1024 frames = 24576 > 24000.
        // The last chunk that crosses the bound is dropped and counted.
        for i in 0..24u64 {
            pump.accept(chunk(1, 1, i * 1_024, 1_024 * 8))
                .expect("accept");
        }
        assert!(pump.dropped_chunks() > 0, "a chunk must have been dropped");
        let dropped_before = pump.dropped_chunks();

        // Drain everything accepted so far, then accept the next contiguous
        // chunk with start_frame = 24 * 1024 (the true capture position).
        let first_batch = drain_all(&mut pump, &mut now);
        assert!(!first_batch.is_empty());
        pump.accept(chunk(1, 1, 24 * 1_024, 1_024 * 8))
            .expect("accept");
        assert_eq!(pump.dropped_chunks(), dropped_before, "no new drop");
        let second_batch = drain_all(&mut pump, &mut now);
        assert!(!second_batch.is_empty());

        // The first datagram after the drop must carry the real capture-frame
        // position of its first payload frame, not a clock that skipped drops.
        let first_after = MediaHeader::parse(&second_batch[0]).expect("header").0;
        assert_eq!(
            first_after.timestamp,
            24 * 1_024,
            "timestamp must re-anchor to the first payload frame's capture position"
        );

        // Sequence remains +1 per datagram across the whole stream.
        let all: Vec<&Vec<u8>> = first_batch.iter().chain(second_batch.iter()).collect();
        for (index, datagram) in all.iter().enumerate() {
            let header = MediaHeader::parse(datagram).expect("header").0;
            assert_eq!(header.sequence, index as u32, "sequence +1 per datagram");
        }
    }

    /// Teardown releases buffered media.
    #[test]
    fn clear_releases_buffered_media() {
        let mut pump = MediaPump::new(stream(1, 1));
        pump.accept(chunk(1, 1, 0, 9_600)).expect("accept");
        pump.clear();
        assert!(pump.idle());
    }

    /// Sequence/timestamp keep advancing across several capture buffers.
    #[test]
    fn the_media_clock_continues_across_capture_buffers() {
        let mut pump = MediaPump::new(stream(1, 1));
        pump.accept(chunk(1, 1, 0, 960)).expect("accept");
        pump.accept(chunk(1, 1, 120, 960)).expect("accept");
        let sent = pump.take_sendable(Instant::now(), 1_192, 64).expect("send");
        let frames: Vec<u64> = sent
            .iter()
            .map(|d| MediaHeader::parse(d).expect("header").0.timestamp)
            .collect();
        let expected: Vec<u64> = (0..sent.len() as u64).map(|i| i * 120).collect();
        assert_eq!(frames, expected);
        assert_eq!(pump.next_frame(), 240);
    }
}
