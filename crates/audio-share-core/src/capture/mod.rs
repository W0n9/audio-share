//! Audio capture backends.
//!
//! Platform backends (WASAPI on Windows, PipeWire on Linux) arrive in later
//! tickets behind `#[cfg]`. This module always provides [`SyntheticCapture`], a
//! deterministic backend used by tests, `as-cmd` before real capture lands, and
//! the debug/fake-capture path.
//!
//! Capture runs on its own thread and hands PCM to the network side through a
//! [`FrameSink`]. A capture session publishes the actual negotiated
//! [`AudioFormat`]; the Server must treat that format as immutable for the
//! lifetime of the session.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::error::{Error, Result};
use crate::protocol::{AudioFormat, Encoding};

/// Windows WASAPI capture backend and its platform-neutral format logic.
///
/// The `backend` module and `WasapiCapture` exist only when building for
/// Windows; the pure format rules are compiled everywhere so they can be
/// unit-tested on the Linux development host.
pub mod windows;

/// The encoding requested from a capture backend.
///
/// `Default` asks the endpoint for its native format; the explicit variants
/// request a specific PCM encoding. Backends may negotiate a closest match and
/// must report the actual format through [`CaptureSession::format`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EncodingPreference {
    /// Let the endpoint choose (F32 on the Linux PipeWire baseline).
    #[default]
    Default,
    /// 32-bit floating-point PCM.
    F32,
    /// 8-bit integer PCM.
    S8,
    /// 16-bit integer PCM.
    S16,
    /// 24-bit integer PCM.
    S24,
    /// 32-bit integer PCM.
    S32,
}

impl EncodingPreference {
    /// The protocol encoding this preference requests, or `None` for `Default`.
    pub const fn requested_encoding(self) -> Option<Encoding> {
        match self {
            Self::Default => None,
            Self::F32 => Some(Encoding::PcmFloat),
            Self::S8 => Some(Encoding::Pcm8Bit),
            Self::S16 => Some(Encoding::Pcm16Bit),
            Self::S24 => Some(Encoding::Pcm24Bit),
            Self::S32 => Some(Encoding::Pcm32Bit),
        }
    }
}

/// What to capture and in which format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureConfig {
    /// Endpoint ID; empty or `"default"` selects the backend default.
    pub endpoint_id: String,
    /// Requested encoding.
    pub encoding: EncodingPreference,
    /// Requested channels; `0` means the endpoint default.
    pub channels: u16,
    /// Requested sample rate in Hz; `0` means the endpoint default.
    pub sample_rate: u32,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            endpoint_id: "default".to_owned(),
            encoding: EncodingPreference::Default,
            channels: 0,
            sample_rate: 0,
        }
    }
}

/// An audio endpoint that can be captured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    /// Stable backend identifier (WASAPI device ID, PipeWire `object.serial`).
    pub id: String,
    /// Human-readable display name.
    pub name: String,
}

/// Channel to which a capture backend delivers raw PCM buffers.
///
/// Each item is one capture callback's worth of interleaved PCM. Sending is
/// bounded to provide backpressure instead of unbounded memory growth.
pub type FrameSink = mpsc::Sender<Vec<u8>>;

/// A platform capture backend.
pub trait CaptureBackend: Send + Sync + 'static {
    /// List endpoints available for capture.
    fn endpoints(&self) -> Result<Vec<Endpoint>>;
    /// The backend's default endpoint, if it has one.
    fn default_endpoint(&self) -> Result<Option<Endpoint>>;
    /// Start capture, delivering PCM to `sink` until the session is stopped.
    fn start(&self, config: &CaptureConfig, sink: FrameSink) -> Result<CaptureSession>;
}

/// A running capture session.
///
/// `format` is the actual format the backend negotiated. It is published before
/// this session is returned, so network handlers never observe an unknown
/// format.
pub struct CaptureSession {
    /// Actual negotiated capture format. Immutable for the session lifetime.
    pub format: AudioFormat,
    signal: Arc<StopSignal>,
    thread: Option<JoinHandle<()>>,
}

impl CaptureSession {
    /// Stop capture, joining the capture thread. Idempotent by construction:
    /// a second call has no thread left to stop.
    pub fn stop(mut self) -> Result<()> {
        self.stop_inner()
    }

    fn stop_inner(&mut self) -> Result<()> {
        if let Some(thread) = self.thread.take() {
            self.signal.stop();
            thread
                .join()
                .map_err(|_| Error::Capture("capture thread panicked".to_owned()))?;
        }
        Ok(())
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        let _ = self.stop_inner();
    }
}

impl std::fmt::Debug for CaptureSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureSession")
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

/// Cancellation signal shared with the capture thread.
///
/// The flag itself is an atomic so the capture loop can poll it cheaply between
/// device periods; the condition variable only wakes an idle thread promptly on
/// stop. A backend that polls on its own may use [`StopSignal::is_stopped`]
/// directly without waiting.
#[derive(Debug, Default)]
struct StopSignal {
    stopped: AtomicBool,
    wait_lock: Mutex<()>,
    wake: Condvar,
}

impl StopSignal {
    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        // Locking before notifying prevents a waiter that checked the flag just
        // before the store from missing the wake-up.
        let _guard = self
            .wait_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.wake.notify_all();
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// Wait for up to `duration`; returns `false` when capture was stopped.
    fn wait(&self, duration: Duration) -> bool {
        if self.is_stopped() {
            return false;
        }
        let guard = self
            .wait_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.is_stopped() {
            return false;
        }
        let (_guard, _) = self
            .wake
            .wait_timeout(guard, duration)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        !self.is_stopped()
    }
}

#[derive(Clone)]
enum FrameSource {
    Generated { frame_bytes: usize },
    Explicit(Arc<Vec<Vec<u8>>>),
}

/// Deterministic capture backend for tests and hardware-free operation.
///
/// The generator emits interleaved PCM built from a 32-frame sawtooth using
/// integer arithmetic only, so the same format and frame index always produce
/// the same bytes on every platform. Construct with [`SyntheticCapture::new`]
/// for generated PCM, or [`SyntheticCapture::with_frames`] to emit exact frames
/// in order (cycling through them) on demand.
pub struct SyntheticCapture {
    format: AudioFormat,
    source: FrameSource,
    interval: Duration,
}

impl SyntheticCapture {
    /// Create a capture that emits `frame_bytes` of deterministic PCM every
    /// `interval`.
    pub fn new(format: AudioFormat, frame_bytes: usize, interval: Duration) -> Result<Self> {
        validate_format_and_interval(&format, interval)?;
        validate_frame_length(&format, frame_bytes)?;
        Ok(Self {
            format,
            source: FrameSource::Generated { frame_bytes },
            interval,
        })
    }

    /// Create a capture that emits the given exact frames in order, cycling,
    /// one every `interval`.
    pub fn with_frames(
        format: AudioFormat,
        frames: Vec<Vec<u8>>,
        interval: Duration,
    ) -> Result<Self> {
        validate_format_and_interval(&format, interval)?;
        if frames.is_empty() {
            return Err(Error::InvalidConfig(
                "synthetic capture needs at least one frame".to_owned(),
            ));
        }
        for frame in &frames {
            validate_frame_length(&format, frame.len())?;
        }
        Ok(Self {
            format,
            source: FrameSource::Explicit(Arc::new(frames)),
            interval,
        })
    }
}

impl CaptureBackend for SyntheticCapture {
    fn endpoints(&self) -> Result<Vec<Endpoint>> {
        Ok(vec![Endpoint {
            id: "synthetic".to_owned(),
            name: "Synthetic Capture".to_owned(),
        }])
    }

    fn default_endpoint(&self) -> Result<Option<Endpoint>> {
        Ok(self.endpoints()?.into_iter().next())
    }

    fn start(&self, _config: &CaptureConfig, sink: FrameSink) -> Result<CaptureSession> {
        let signal = Arc::new(StopSignal::default());
        let thread_signal = Arc::clone(&signal);
        let format = self.format.clone();
        let thread_format = format.clone();
        let source = self.source.clone();
        let interval = self.interval;
        let thread = thread::Builder::new()
            .name("audio-share-synthetic-capture".to_owned())
            .spawn(move || {
                let mut frame_index: u64 = 0;
                loop {
                    if thread_signal.is_stopped() {
                        break;
                    }
                    let Some(frame) = next_frame(&thread_format, &source, frame_index) else {
                        break;
                    };
                    frame_index = frame_index.wrapping_add(1);
                    if sink.blocking_send(frame).is_err() {
                        break;
                    }
                    if !thread_signal.wait(interval) {
                        break;
                    }
                }
            })
            .map_err(|error| Error::io("spawn synthetic capture thread", error))?;
        Ok(CaptureSession {
            format,
            signal,
            thread: Some(thread),
        })
    }
}

/// v0.3.4's Linux default: F32, stereo, 48 kHz, 1024-frame callbacks every
/// 10 ms. Used by the synthetic fallback backend.
const SYNTHETIC_FORMAT: AudioFormat = AudioFormat::new(Encoding::PcmFloat, 2, 48_000);
const SYNTHETIC_FRAME_BYTES: usize = 8192;
const SYNTHETIC_FRAME_INTERVAL: Duration = Duration::from_millis(10);

/// The deterministic synthetic backend used by tests and `--fake-capture`.
pub fn synthetic_backend() -> Result<SyntheticCapture> {
    SyntheticCapture::new(
        SYNTHETIC_FORMAT,
        SYNTHETIC_FRAME_BYTES,
        SYNTHETIC_FRAME_INTERVAL,
    )
}

/// The default capture backend for the current platform.
///
/// Windows uses WASAPI shared-mode loopback. Every other platform currently
/// falls back to [`SyntheticCapture`]; the Linux PipeWire backend (ticket 06)
/// will replace only the non-Windows branch.
pub fn platform_backend() -> Result<Arc<dyn CaptureBackend>> {
    platform_backend_impl()
}

#[cfg(windows)]
fn platform_backend_impl() -> Result<Arc<dyn CaptureBackend>> {
    Ok(Arc::new(windows::WasapiCapture::new()))
}

#[cfg(not(windows))]
fn platform_backend_impl() -> Result<Arc<dyn CaptureBackend>> {
    Ok(Arc::new(synthetic_backend()?))
}

fn next_frame(format: &AudioFormat, source: &FrameSource, frame_index: u64) -> Option<Vec<u8>> {
    match source {
        FrameSource::Generated { frame_bytes } => {
            generated_frame(format, *frame_bytes, frame_index)
        }
        FrameSource::Explicit(frames) => {
            let index = (frame_index % frames.len() as u64) as usize;
            Some(frames[index].clone())
        }
    }
}

fn generated_frame(format: &AudioFormat, frame_bytes: usize, frame_index: u64) -> Option<Vec<u8>> {
    let encoding = format.encoding_kind()?;
    let block_align = format.block_align();
    if block_align == 0 || format.channels <= 0 {
        return None;
    }
    let frames = frame_bytes / block_align;
    let start_frame = frame_index.wrapping_mul(frames as u64);
    let mut out = Vec::with_capacity(frame_bytes);
    for frame in 0..frames {
        let saw = ((start_frame.wrapping_add(frame as u64) % 32) as i32) - 16;
        for _ in 0..format.channels {
            push_sample(&mut out, encoding, saw);
        }
    }
    Some(out)
}

fn push_sample(out: &mut Vec<u8>, encoding: Encoding, saw: i32) {
    match encoding {
        Encoding::Invalid => {}
        Encoding::PcmFloat => out.extend_from_slice(&(saw as f32 / 32.0).to_le_bytes()),
        Encoding::Pcm8Bit => out.push((saw * 4) as u8),
        Encoding::Pcm16Bit => out.extend_from_slice(&((saw * 1024) as i16).to_le_bytes()),
        Encoding::Pcm24Bit => out.extend_from_slice(&(saw * 65_536).to_le_bytes()[..3]),
        Encoding::Pcm32Bit => out.extend_from_slice(&(saw * 16_777_216).to_le_bytes()),
    }
}

fn validate_format_and_interval(format: &AudioFormat, interval: Duration) -> Result<()> {
    if format.block_align() == 0 {
        return Err(Error::InvalidConfig(format!(
            "AudioFormat {format:?} has no usable block alignment"
        )));
    }
    if format.sample_rate <= 0 {
        return Err(Error::InvalidConfig(format!(
            "AudioFormat {format:?} has a non-positive sample rate"
        )));
    }
    if interval.is_zero() {
        return Err(Error::InvalidConfig(
            "capture interval must be greater than zero".to_owned(),
        ));
    }
    Ok(())
}

fn validate_frame_length(format: &AudioFormat, frame_bytes: usize) -> Result<()> {
    let block_align = format.block_align();
    if frame_bytes == 0 {
        return Err(Error::InvalidConfig(
            "capture frame must not be empty".to_owned(),
        ));
    }
    if frame_bytes % block_align != 0 {
        return Err(Error::InvalidConfig(format!(
            "capture frame of {frame_bytes} bytes is not a multiple of block_align {block_align}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_preference_maps_to_protocol_encodings() {
        assert_eq!(EncodingPreference::Default.requested_encoding(), None);
        assert_eq!(
            EncodingPreference::F32.requested_encoding(),
            Some(Encoding::PcmFloat)
        );
        assert_eq!(
            EncodingPreference::S8.requested_encoding(),
            Some(Encoding::Pcm8Bit)
        );
        assert_eq!(
            EncodingPreference::S16.requested_encoding(),
            Some(Encoding::Pcm16Bit)
        );
        assert_eq!(
            EncodingPreference::S24.requested_encoding(),
            Some(Encoding::Pcm24Bit)
        );
        assert_eq!(
            EncodingPreference::S32.requested_encoding(),
            Some(Encoding::Pcm32Bit)
        );
    }

    #[test]
    fn generated_frames_are_deterministic_aligned_and_nonempty() {
        for (encoding, channels) in [
            (Encoding::PcmFloat, 2),
            (Encoding::Pcm8Bit, 1),
            (Encoding::Pcm16Bit, 2),
            (Encoding::Pcm24Bit, 2),
            (Encoding::Pcm32Bit, 1),
        ] {
            let format = AudioFormat::new(encoding, channels, 48_000);
            let block_align = format.block_align();
            let first = generated_frame(&format, block_align * 8, 0).unwrap();
            let again = generated_frame(&format, block_align * 8, 0).unwrap();
            let next = generated_frame(&format, block_align * 8, 1).unwrap();
            assert_eq!(first, again);
            assert_eq!(first.len(), block_align * 8);
            assert_ne!(first, next, "consecutive frames must differ");
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn platform_backend_is_synthetic_on_non_windows() {
        let backend = platform_backend().unwrap();
        let endpoints = backend.endpoints().unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].id, "synthetic");
        assert_eq!(
            backend.default_endpoint().unwrap().unwrap().id,
            endpoints[0].id
        );
    }
}
