//! v2 控制平面与共享 capture 的生产接线：只读 capture 状态，以及协议中立的
//! 媒体源（PCM chunks + 流实例）。
//!
//! v1 保持协商后的 immutable 格式，并继续按自己的 framing/MTU 发送；这里只
//! 观察同一批原始 PCM，不改变 v1 行为（ADR-0003/ADR-0004）。
//!
//! These adapters live in the shared runtime crate so `as-cmd` and the Server
//! GUI host v2 over the same v1 `CaptureObserver`/`MediaTap` without copying
//! protocol wiring.

use std::sync::Arc;

use audio_share_core::capture::media::{EndReason, MediaTap};
use audio_share_core::capture::CaptureObserver;
use audio_share_core::protocol::AudioFormat;
use v2_transport::codes::ErrorCode;
use v2_transport::driver::{CaptureSnapshot, CaptureStateSource};
use v2_transport::media_source::{MediaChunk, MediaSource, MediaStatus, StreamFormat, StreamState};

/// 将 Server Core 的真实 capture 状态提供给 v2 会话层。
///
/// 格式与线程健康来自 `CaptureObserver`；活动流实例、下一帧位置来自共享
/// capture 的媒体 tap，保证 `Subscribe` 推送的 `StreamStarted` 与真正发送的
/// 媒体使用同一份流状态。
pub fn v2_capture_source(
    capture: CaptureObserver,
    media_tap: &MediaTap,
) -> Arc<CaptureStateSource> {
    let snapshot = v2_capture_snapshot(&capture.format());
    let media_tap = media_tap.clone();
    Arc::new(move || {
        let mut current = snapshot.clone();
        current.healthy = capture.is_running();
        if let Some(stream) = media_tap.stream() {
            current.stream_active = true;
            current.stream_instance_id = stream.stream_instance_id;
            current.next_frame_timestamp = stream.next_frame;
        }
        current
    })
}

/// 将协商后的格式映射为 v2 PCM 配置；无效几何信息保持 capture absent（501）。
pub fn v2_capture_snapshot(format: &AudioFormat) -> CaptureSnapshot {
    use audio_share_core::protocol::Encoding;
    use v2_transport::schema::{Codec, Configuration, PcmParameters, SampleFormat};

    let mut snapshot = CaptureSnapshot::absent();
    let sample_format = match format.encoding_kind() {
        Some(Encoding::PcmFloat) => SampleFormat::F32Le,
        Some(Encoding::Pcm8Bit) => SampleFormat::U8,
        Some(Encoding::Pcm16Bit) => SampleFormat::S16Le,
        Some(Encoding::Pcm24Bit) => SampleFormat::S24Le,
        Some(Encoding::Pcm32Bit) => SampleFormat::S32Le,
        Some(Encoding::Invalid) | None => return snapshot,
    };
    let (Ok(channels), Ok(sample_rate)) = (
        u32::try_from(format.channels),
        u32::try_from(format.sample_rate),
    ) else {
        return snapshot;
    };
    if channels == 0 || sample_rate == 0 {
        return snapshot;
    }
    snapshot.configuration = Some(Configuration {
        config_generation: 1,
        codec: Codec::Pcm as i32,
        pcm: Some(PcmParameters {
            sample_rate,
            channels,
            sample_format: sample_format as i32,
        }),
    });
    snapshot
}

/// v2 媒体源：把 capture tap 的原始 PCM 与流实例状态暴露给会话层。
pub struct CaptureMediaSource {
    tap: MediaTap,
    observer: CaptureObserver,
}

impl CaptureMediaSource {
    /// 观察同一个 capture tap 与线程健康状态。
    pub fn new(tap: MediaTap, observer: CaptureObserver) -> Self {
        Self { tap, observer }
    }
}

impl MediaSource for CaptureMediaSource {
    fn status(&self) -> MediaStatus {
        if !self.observer.is_running() {
            // ADR-0003：capture 结束不自动恢复；订阅者看到 501。
            return MediaStatus::Ended {
                code: ErrorCode::CAPTURE_FAILURE.code,
            };
        }
        match self.tap.stream() {
            Some(stream) => MediaStatus::Active(StreamState {
                stream_instance_id: stream.stream_instance_id,
                config_generation: stream.config_generation,
                next_frame: stream.next_frame,
                format: StreamFormat {
                    sample_rate: stream.format.sample_rate,
                    channels: stream.format.channels,
                    bytes_per_sample: stream.format.bytes_per_sample,
                },
            }),
            None => match self.tap.end_reason() {
                Some(EndReason::Failed) => MediaStatus::Ended {
                    code: ErrorCode::CAPTURE_FAILURE.code,
                },
                Some(EndReason::Ended) => MediaStatus::Ended {
                    code: ErrorCode::STREAM_ENDED.code,
                },
                // 尚未开始/已清空且没有结束原因：视作没有活动流。
                None => MediaStatus::Absent,
            },
        }
    }

    fn subscribe(&self) -> tokio::sync::mpsc::Receiver<MediaChunk> {
        // The capture tap speaks `audio-share-core`'s protocol-neutral chunk
        // type; the v2 session speaks the protocol crate's own. Forward
        // between them on a bounded channel, converting in place — the data
        // types stay owned by their crates, no shared DTO is invented.
        let mut source = self.tap.subscribe();
        let capacity = 8;
        let (tx, rx) = tokio::sync::mpsc::channel(capacity);
        tokio::spawn(async move {
            while let Some(chunk) = source.recv().await {
                let converted = MediaChunk {
                    pcm: chunk.pcm,
                    start_frame: chunk.start_frame,
                    end_frame: chunk.end_frame,
                    stream_instance_id: chunk.stream_instance_id,
                    config_generation: chunk.config_generation,
                };
                // Non-blocking by construction: a stalled consumer loses the
                // chunk here rather than applying backpressure to capture.
                if tx.try_send(converted).is_err() {
                    continue;
                }
            }
        });
        rx
    }
}

/// 构造生产媒体源。
pub fn capture_media_source(tap: &MediaTap, capture: CaptureObserver) -> Arc<dyn MediaSource> {
    Arc::new(CaptureMediaSource::new(tap.clone(), capture))
}
