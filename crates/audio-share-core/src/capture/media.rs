//! The protocol-independent media view of the shared capture.
//!
//! The capture backend hands raw PCM buffers to [`crate::capture::FrameSink`].
//! The v1 server segments those buffers for its UDP peers; v2 needs the same
//! PCM with a capture-clock position and a stream-instance identity, and it
//! must never take the v1 framing, MTU, or Protobuf types as its model
//! (design baseline §Implementation Boundaries).
//!
//! [`MediaTap`] is that seam: a bounded, non-blocking fan-out. Publishing is
//! O(subscribers) `try_send` calls, so the capture/broadcast path can never be
//! blocked by a slow or stalled Client; a full subscriber drops the newest
//! chunk and counts it. Sample-frame positions advance by whole frames only —
//! a chunk that is not frame-aligned is a capture bug and is rejected.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

/// Process-wide stream-instance counter (spec §7.2: nonzero, +1 per Server
/// process, wrapping `2^32−1 → 1`). Every tap that starts a stream takes the
/// next value, so two captures never share an instance id.
static NEXT_STREAM_INSTANCE: AtomicU32 = AtomicU32::new(1);

fn next_stream_instance() -> u32 {
    loop {
        let current = NEXT_STREAM_INSTANCE.fetch_add(1, Ordering::SeqCst);
        // 0 is invalid on the wire (§7.1); skip it on wrap.
        let instance = if current == 0 { 1 } else { current };
        if instance != 0 {
            return instance;
        }
    }
}

/// The PCM geometry of one capture stream: what a receiver needs to interpret
/// payload bytes and to convert frame counts to time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaFormat {
    /// Hz.
    pub sample_rate: u32,
    /// Interleaved channels.
    pub channels: u16,
    /// Bytes per sample of one channel.
    pub bytes_per_sample: u16,
}

impl MediaFormat {
    /// Bytes of one sample frame (all channels).
    pub fn frame_bytes(&self) -> usize {
        usize::from(self.bytes_per_sample) * usize::from(self.channels)
    }

    /// Whole sample frames in `bytes`, or `None` when not frame-aligned.
    pub fn frames(&self, bytes: usize) -> Option<u64> {
        let frame_bytes = self.frame_bytes();
        if frame_bytes == 0 || !bytes.is_multiple_of(frame_bytes) {
            return None;
        }
        Some((bytes / frame_bytes) as u64)
    }
}

/// A read-only observation of the live stream (what Subscribe and
/// `StreamStarted` publish).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaStreamState {
    /// Stream instance id (§7.2).
    pub stream_instance_id: u32,
    /// Decode-affecting configuration generation (§7.2).
    pub config_generation: u32,
    /// Capture-clock position of the next payload frame.
    pub next_frame: u64,
    /// The stream's PCM geometry.
    pub format: MediaFormat,
}

/// One published capture buffer with its capture-clock coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaChunk {
    /// Interleaved PCM bytes in Transfer Format encoding.
    pub pcm: Vec<u8>,
    /// Capture-clock position of the first frame in `pcm`.
    pub start_frame: u64,
    /// Position just past the last frame.
    pub end_frame: u64,
    /// Stream instance id carried by this chunk.
    pub stream_instance_id: u32,
    /// Configuration generation carried by this chunk.
    pub config_generation: u32,
}

struct TapState {
    stream: Option<MediaStreamState>,
    subscribers: Vec<mpsc::Sender<MediaChunk>>,
    dropped_chunks: u64,
    end_reason: Option<EndReason>,
}

/// Why a stream ended, for the §5.2 StreamEnded code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    /// The Server is stopping; a normal end (502).
    Ended,
    /// The capture backend failed or its thread exited (501).
    Failed,
}

/// Bounded fan-out of one capture stream to any number of independent v2
/// sessions. Cheap to clone.
#[derive(Clone)]
pub struct MediaTap {
    inner: Arc<Mutex<TapState>>,
    capacity: usize,
}

impl MediaTap {
    /// Start a stream tap for `format`. The first tap in a process gets
    /// instance 1; later taps advance the process-wide counter.
    pub fn new(format: MediaFormat) -> Self {
        Self::with_instance(format, next_stream_instance())
    }

    /// Start a tap with an explicit instance id (tests, and restarts that
    /// already own an id).
    pub fn with_instance(format: MediaFormat, stream_instance_id: u32) -> Self {
        let instance = if stream_instance_id == 0 {
            1
        } else {
            stream_instance_id
        };
        let state = TapState {
            stream: Some(MediaStreamState {
                stream_instance_id: instance,
                config_generation: 1,
                next_frame: 0,
                format,
            }),
            subscribers: Vec::new(),
            dropped_chunks: 0,
            end_reason: None,
        };
        Self {
            inner: Arc::new(Mutex::new(state)),
            capacity: 8,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TapState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The current stream, or `None` once it ended (the immutable format is
    /// still kept by the Server's capture observer).
    pub fn stream(&self) -> Option<MediaStreamState> {
        self.lock().stream
    }

    /// Register a bounded subscriber for future chunks.
    pub fn subscribe(&self) -> mpsc::Receiver<MediaChunk> {
        let (tx, rx) = mpsc::channel(self.capacity);
        let mut state = self.lock();
        if state.stream.is_some() {
            state.subscribers.push(tx);
        }
        rx
    }

    /// Publish one capture buffer. Non-blocking: a full subscriber drops this
    /// chunk (newest) and the drop is counted. A chunk that is not a whole
    /// number of frames is rejected and counted — the capture contract says
    /// every buffer is frame-aligned.
    pub fn publish(&self, pcm: &[u8]) {
        if pcm.is_empty() {
            return;
        }
        let mut state = self.lock();
        let Some(stream) = state.stream else {
            return;
        };
        let Some(frames) = stream.format.frames(pcm.len()) else {
            state.dropped_chunks += 1;
            return;
        };
        let chunk = MediaChunk {
            pcm: pcm.to_vec(),
            start_frame: stream.next_frame,
            end_frame: stream.next_frame + frames,
            stream_instance_id: stream.stream_instance_id,
            config_generation: stream.config_generation,
        };
        state.stream = Some(MediaStreamState {
            next_frame: chunk.end_frame,
            ..stream
        });
        let mut dropped = 0u64;
        state.subscribers.retain(|subscriber| {
            match subscriber.try_send(chunk.clone()) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // Slow consumer: drop this chunk for it, keep it subscribed.
                    dropped += 1;
                    true
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
        });
        state.dropped_chunks += dropped;
    }

    /// End the stream: subscribers observe the closed channel and the
    /// snapshot becomes `None` (§5.2 capture-ended rows).
    pub fn end(&self) {
        self.finish(EndReason::Ended);
    }

    /// End the stream because capture failed (502 vs 501 for observers).
    pub fn fail(&self) {
        self.finish(EndReason::Failed);
    }

    fn finish(&self, reason: EndReason) {
        let mut state = self.lock();
        state.stream = None;
        state.end_reason = Some(reason);
        state.subscribers.clear();
    }

    /// Why the stream ended, once it has.
    pub fn end_reason(&self) -> Option<EndReason> {
        self.lock().end_reason
    }

    /// Whether a stream is currently active.
    pub fn is_active(&self) -> bool {
        self.lock().stream.is_some()
    }

    /// Chunks dropped because a subscriber's bounded queue was full.
    pub fn dropped_chunks(&self) -> u64 {
        self.lock().dropped_chunks
    }
}

impl std::fmt::Debug for MediaTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.lock();
        f.debug_struct("MediaTap")
            .field("stream", &state.stream)
            .field("subscribers", &state.subscribers.len())
            .field("dropped_chunks", &state.dropped_chunks)
            .finish()
    }
}
