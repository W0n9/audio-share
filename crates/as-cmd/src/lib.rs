//! v2 控制平面的只读生产 capture 注入。
//! v1 保持协商后的 immutable 格式；这里不发送 v2 媒体。

use std::sync::Arc;

use audio_share_core::capture::CaptureObserver;
use audio_share_core::protocol::AudioFormat;
use v2_transport::driver::{CaptureSnapshot, CaptureStateSource};

/// 将 Server Core 的真实 capture 状态提供给 v2 会话层。
pub fn v2_capture_source(capture: CaptureObserver) -> Arc<CaptureStateSource> {
    let snapshot = v2_capture_snapshot(&capture.format());
    Arc::new(move || {
        let mut current = snapshot.clone();
        current.healthy = capture.is_running();
        current
    })
}

/// 将协商后的格式映射为 v2 PCM 配置；无效几何信息保持 capture absent（501）。
/// 本阶段没有活动 v2 媒体流。
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
