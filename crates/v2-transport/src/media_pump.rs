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

    /// Buffered capture audio in sample frames.
    pub fn pending_frames(&self) -> u64 {
        let frame_bytes = self.format.frame_bytes();
        if frame_bytes == 0 {
            return 0;
        }
        (self.pending.iter().map(|chunk| chunk.pcm.len()).sum::<usize>() / frame_bytes) as u64
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
        self.pending.push_back(PendingChunk { pcm: chunk.pcm });
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
            let pcm: Vec<u8> = front.pcm.drain(..take_bytes).collect();
            if front.pcm.is_empty() {
                self.pending.pop_front();
            }
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
        let sent = pump
            .take_sendable(Instant::now(), 1_192, 8)
            .expect("send");
        let first = MediaHeader::parse(&sent[0]).expect("header").0;
        assert_eq!(first.timestamp, 96_000);
        assert_eq!(
            first.sequence, 0,
            "the instance's sequence space starts here"
        );
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
        let sent = pump
            .take_sendable(Instant::now(), 1_192, 64)
            .expect("send");
        let frames: Vec<u64> = sent
            .iter()
            .map(|d| MediaHeader::parse(d).expect("header").0.timestamp)
            .collect();
        let expected: Vec<u64> = (0..sent.len() as u64)
            .map(|i| i * 120)
            .collect();
        assert_eq!(frames, expected);
        assert_eq!(pump.next_frame(), 240);
    }
}
