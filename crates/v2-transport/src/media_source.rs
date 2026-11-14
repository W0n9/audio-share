//! The media source a session subscribes to, independent of where capture
//! lives.
//!
//! `audio-share-core` owns the capture backends; this crate owns the wire
//! protocol. The adapter that connects them implements [`MediaSource`] over
//! the capture tap (see `as_cmd`), so neither crate has to know the other's
//! capture model. The types here are the media-domain vocabulary the session
//! layer needs: a stream identity plus PCM chunks carrying their capture-clock
//! position.

/// PCM geometry of a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamFormat {
    /// Hz.
    pub sample_rate: u32,
    /// Interleaved channels.
    pub channels: u16,
    /// Bytes per sample of one channel.
    pub bytes_per_sample: u16,
}

impl StreamFormat {
    /// Bytes of one sample frame.
    pub fn frame_bytes(&self) -> usize {
        usize::from(self.bytes_per_sample) * usize::from(self.channels)
    }
}

/// A live stream's identity and next position (what Subscribe and
/// `StreamStarted` publish).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamState {
    /// §7.2 stream instance id.
    pub stream_instance_id: u32,
    /// §7.2 configuration generation.
    pub config_generation: u32,
    /// Capture-clock position of the next payload frame.
    pub next_frame: u64,
    /// PCM geometry.
    pub format: StreamFormat,
}

/// One capture buffer in protocol terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaChunk {
    /// Interleaved PCM bytes (Transfer Format encoding).
    pub pcm: Vec<u8>,
    /// Capture-clock position of the first frame.
    pub start_frame: u64,
    /// Position just past the last frame.
    pub end_frame: u64,
    /// Instance id carried by the chunk.
    pub stream_instance_id: u32,
    /// Generation carried by the chunk.
    pub config_generation: u32,
}

/// What the shared capture currently offers a subscriber.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaStatus {
    /// No Transfer Format is current; nothing can be streamed (§5.2 absent).
    Absent,
    /// A stream instance is active and delivering chunks.
    Active(StreamState),
    /// The instance ended. `code` is the §6 code a subscribed Client sees:
    /// 502 STREAM_ENDED for a normal end, 501 CAPTURE_FAILURE for a failure.
    Ended {
        /// §6 code (501 or 502).
        code: u32,
    },
}

/// A bounded, non-blocking source of one capture stream.
///
/// Implementations must never block the capture path: a slow subscriber loses
/// chunks rather than stalling capture (spec §7.4).
pub trait MediaSource: Send + Sync {
    /// The current capture status.
    fn status(&self) -> MediaStatus;

    /// Subscribe to chunks from now on. A subscriber that stops draining
    /// loses chunks; it is never waited for.
    fn subscribe(&self) -> tokio::sync::mpsc::Receiver<MediaChunk>;
}
