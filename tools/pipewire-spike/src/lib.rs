//! Platform-neutral helpers for the Linux PipeWire capture spike.
//!
//! The pure logic lives here so it can be unit-tested on any host: encoding
//! mapping, default-endpoint selection, blocking alignment, statistics and
//! PCM tone analysis. The actual PipeWire client lives in `linux.rs`.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use clap::ValueEnum;

/// PCM encodings exposed by `--encoding`.
///
/// `S8` intentionally maps to PipeWire's unsigned `U8`, matching the old
/// server (`ENCODING_PCM_8BIT` is unsigned 8-bit on Android too).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PcmEncoding {
    F32,
    S8,
    S16,
    S24,
    S32,
}

impl PcmEncoding {
    /// Bytes occupied by one sample in the negotiated PipeWire format.
    ///
    /// `S24` is packed 24-bit (3 bytes), not a 32-bit container.
    pub fn bytes_per_sample(self) -> u32 {
        match self {
            PcmEncoding::S8 => 1,
            PcmEncoding::S16 => 2,
            PcmEncoding::S24 => 3,
            PcmEncoding::F32 | PcmEncoding::S32 => 4,
        }
    }

    /// `block_align = bytes_per_sample * channels`.
    pub fn block_align(self, channels: u32) -> u32 {
        self.bytes_per_sample() * channels
    }

    pub fn as_str(self) -> &'static str {
        match self {
            PcmEncoding::F32 => "f32",
            PcmEncoding::S8 => "s8",
            PcmEncoding::S16 => "s16",
            PcmEncoding::S24 => "s24",
            PcmEncoding::S32 => "s32",
        }
    }
}

impl fmt::Display for PcmEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PcmEncoding {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "f32" | "f32le" => Ok(PcmEncoding::F32),
            "s8" | "u8" => Ok(PcmEncoding::S8),
            "s16" | "s16le" => Ok(PcmEncoding::S16),
            "s24" | "s24le" => Ok(PcmEncoding::S24),
            "s32" | "s32le" => Ok(PcmEncoding::S32),
            other => Err(format!(
                "unknown encoding {other:?}; expected one of f32, s8, s16, s24, s32"
            )),
        }
    }
}

/// One `Audio/Sink` discovered through the PipeWire registry.
///
/// `serial` is `object.serial` (the Wire Protocol endpoint id), not the
/// registry/global object id. Display names follow the old server:
/// `node.nick` -> `node.description` -> `node.name` -> serial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointInfo {
    pub serial: String,
    pub node_name: Option<String>,
    pub node_nick: Option<String>,
    pub node_description: Option<String>,
    pub priority_session: i64,
}

impl EndpointInfo {
    pub fn display_name(&self) -> &str {
        self.node_nick
            .as_deref()
            .or(self.node_description.as_deref())
            .or(self.node_name.as_deref())
            .unwrap_or(&self.serial)
    }
}

/// Pick the default sink exactly like the old C++ server: start at priority
/// -1, replace only on a strictly greater `priority.session`, first wins ties.
/// When every sink has priority <= -1 there is no default.
pub fn pick_default(endpoints: &[EndpointInfo]) -> Option<&EndpointInfo> {
    let mut best: Option<&EndpointInfo> = None;
    for endpoint in endpoints {
        let best_priority = best.map_or(-1, |candidate| candidate.priority_session);
        if endpoint.priority_session > best_priority {
            best = Some(endpoint);
        }
    }
    best
}

pub fn validate_audio_params(channels: u32, sample_rate: u32) -> Result<(), String> {
    if channels > 2 {
        return Err(format!(
            "unsupported channel count {channels}; expected 0 (endpoint default), 1, or 2"
        ));
    }
    if !matches!(sample_rate, 0 | 44_100 | 48_000) {
        return Err(format!(
            "unsupported sample rate {sample_rate}; expected 0 (endpoint default), 44100, or 48000"
        ));
    }
    Ok(())
}

/// Summary of a set of time intervals (callback spacing).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntervalStats {
    pub count: usize,
    pub min: Duration,
    pub max: Duration,
    pub mean: Duration,
    pub p50: Duration,
    pub p95: Duration,
}

/// Summary of a set of integer sample counts (frames per callback / quantum).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValueStats {
    pub count: usize,
    pub min: u64,
    pub max: u64,
    pub mean: f64,
    pub p50: u64,
    pub p95: u64,
}

/// Nearest-rank percentile over an already sorted slice.
fn percentile(sorted: &[u64], percent: f64) -> u64 {
    let rank = ((percent / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

/// `None` for an empty slice. Input order does not matter.
pub fn summarize_intervals(intervals: &[Duration]) -> Option<IntervalStats> {
    if intervals.is_empty() {
        return None;
    }
    let mut nanos: Vec<u64> = intervals
        .iter()
        .map(|interval| interval.as_nanos().min(u64::MAX as u128) as u64)
        .collect();
    nanos.sort_unstable();
    let count = nanos.len();
    let total: u128 = nanos.iter().map(|&value| value as u128).sum();
    let mean_nanos = (total / count as u128).min(u64::MAX as u128) as u64;
    Some(IntervalStats {
        count,
        min: Duration::from_nanos(nanos[0]),
        max: Duration::from_nanos(nanos[count - 1]),
        mean: Duration::from_nanos(mean_nanos),
        p50: Duration::from_nanos(percentile(&nanos, 50.0)),
        p95: Duration::from_nanos(percentile(&nanos, 95.0)),
    })
}

/// `None` for an empty slice. Input order does not matter.
pub fn summarize_values(values: &[u64]) -> Option<ValueStats> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let count = sorted.len();
    let total: u128 = sorted.iter().map(|&value| value as u128).sum();
    Some(ValueStats {
        count,
        min: sorted[0],
        max: sorted[count - 1],
        mean: total as f64 / count as f64,
        p50: percentile(&sorted, 50.0),
        p95: percentile(&sorted, 95.0),
    })
}

/// Whole frames contained in `bytes` at `block_align` bytes per frame.
/// Returns 0 when the alignment is unknown or there is not a full frame.
pub fn frames_in_bytes(bytes: u64, block_align: u32) -> u64 {
    if block_align == 0 {
        0
    } else {
        bytes / u64::from(block_align)
    }
}

/// Approximate peak amplitude of `frequency` in `samples` (Goertzel).
/// A pure sine of amplitude A yields approximately A.
pub fn goertzel_amplitude(samples: &[f32], sample_rate: u32, frequency: f32) -> f32 {
    if samples.is_empty() || sample_rate == 0 || !frequency.is_finite() {
        return 0.0;
    }
    let omega = 2.0 * std::f32::consts::PI * frequency / sample_rate as f32;
    let coefficient = 2.0 * omega.cos();
    let mut previous = 0.0f32;
    let mut previous_previous = 0.0f32;
    for &sample in samples {
        let current = coefficient * previous - previous_previous + sample;
        previous_previous = previous;
        previous = current;
    }
    let power = previous * previous + previous_previous * previous_previous
        - coefficient * previous * previous_previous;
    power.max(0.0).sqrt() * 2.0 / samples.len() as f32
}

/// Coarse-to-fine Goertzel scan for the strongest frequency in
/// `[min_hz, max_hz]`, refined to 1 Hz. `None` for unusable input.
pub fn dominant_frequency(
    samples: &[f32],
    sample_rate: u32,
    min_hz: f32,
    max_hz: f32,
) -> Option<f32> {
    if samples.is_empty()
        || sample_rate == 0
        || !min_hz.is_finite()
        || !max_hz.is_finite()
        || min_hz >= max_hz
    {
        return None;
    }
    let mut best = (min_hz, f32::NEG_INFINITY);
    let mut frequency = min_hz;
    while frequency <= max_hz {
        let amplitude = goertzel_amplitude(samples, sample_rate, frequency);
        if amplitude > best.1 {
            best = (frequency, amplitude);
        }
        frequency += 10.0;
    }
    let coarse = best.0;
    let refine_min = (coarse - 10.0).max(min_hz);
    let refine_max = (coarse + 10.0).min(max_hz);
    let mut frequency = refine_min;
    while frequency <= refine_max {
        let amplitude = goertzel_amplitude(samples, sample_rate, frequency);
        if amplitude > best.1 {
            best = (frequency, amplitude);
        }
        frequency += 1.0;
    }
    Some(best.0)
}

/// Decode interleaved PCM to mono f32 (channel 0 only), normalized to [-1, 1].
///
/// `PcmEncoding::S8` is unsigned 8-bit (PipeWire `U8`), like the old server.
pub fn decode_mono_f32(bytes: &[u8], encoding: PcmEncoding, channels: u32) -> Vec<f32> {
    let channels = channels.max(1) as usize;
    let bytes_per_sample = encoding.bytes_per_sample() as usize;
    let frame_size = bytes_per_sample * channels;
    if frame_size == 0 {
        return Vec::new();
    }
    bytes
        .chunks_exact(frame_size)
        .map(|frame| match encoding {
            PcmEncoding::F32 => f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]),
            PcmEncoding::S8 => (f32::from(frame[0]) - 128.0) / 128.0,
            PcmEncoding::S16 => i16::from_le_bytes([frame[0], frame[1]]) as f32 / 32_768.0,
            PcmEncoding::S24 => {
                let high = if frame[2] & 0x80 != 0 { 0xff } else { 0x00 };
                let value = i32::from_le_bytes([frame[0], frame[1], frame[2], high]);
                value as f32 / 8_388_608.0
            }
            PcmEncoding::S32 => {
                i32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]) as f32
                    / 2_147_483_648.0
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_stats_summarize_spacing() {
        assert!(summarize_intervals(&[]).is_none());

        let single = summarize_intervals(&[Duration::from_millis(20)]).unwrap();
        assert_eq!(single.count, 1);
        assert_eq!(single.min, Duration::from_millis(20));
        assert_eq!(single.mean, Duration::from_millis(20));

        let stats = summarize_intervals(&[
            Duration::from_millis(30),
            Duration::from_millis(10),
            Duration::from_millis(20),
        ])
        .unwrap();
        assert_eq!(stats.count, 3);
        assert_eq!(stats.min, Duration::from_millis(10));
        assert_eq!(stats.max, Duration::from_millis(30));
        assert_eq!(stats.mean, Duration::from_millis(20));
        assert_eq!(stats.p50, Duration::from_millis(20));
        assert_eq!(stats.p95, Duration::from_millis(30));
    }

    #[test]
    fn value_stats_summarize_vector_counts() {
        assert!(summarize_values(&[]).is_none());

        let stats = summarize_values(&[2048, 1024, 4096]).unwrap();
        assert_eq!(stats.count, 3);
        assert_eq!(stats.min, 1024);
        assert_eq!(stats.max, 4096);
        assert!((stats.mean - (1024.0 + 2048.0 + 4096.0) / 3.0).abs() < 0.001);
        assert_eq!(stats.p50, 2048);
        assert_eq!(stats.p95, 4096);
    }

    #[test]
    fn frames_in_bytes_uses_block_align() {
        assert_eq!(frames_in_bytes(8192, 8), 1024);
        assert_eq!(frames_in_bytes(12288, 6), 2048);
        assert_eq!(frames_in_bytes(7412, 4), 1853);
        assert_eq!(frames_in_bytes(10, 4), 2);
        assert_eq!(frames_in_bytes(0, 8), 0);
        assert_eq!(frames_in_bytes(100, 0), 0);
    }

    fn sine(frequency: f32, sample_rate: u32, seconds: f32, amplitude: f32) -> Vec<f32> {
        let count = (sample_rate as f32 * seconds) as usize;
        (0..count)
            .map(|index| {
                amplitude
                    * (2.0 * std::f32::consts::PI * frequency * index as f32 / sample_rate as f32)
                        .sin()
            })
            .collect()
    }

    fn encode(samples: &[f32], encoding: PcmEncoding) -> Vec<u8> {
        let mut bytes = Vec::new();
        for &sample in samples {
            match encoding {
                PcmEncoding::F32 => bytes.extend_from_slice(&sample.to_le_bytes()),
                PcmEncoding::S8 => {
                    let value = ((sample * 128.0).round() + 128.0).clamp(0.0, 255.0) as u8;
                    bytes.push(value);
                }
                PcmEncoding::S16 => {
                    let value = (sample * 32768.0).clamp(-32768.0, 32767.0) as i16;
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                PcmEncoding::S24 => {
                    let value = (sample * 8_388_608.0).clamp(-8_388_608.0, 8_388_607.0) as i32;
                    bytes.extend_from_slice(&value.to_le_bytes()[..3]);
                }
                PcmEncoding::S32 => {
                    let value =
                        (sample * 2_147_483_648.0).clamp(-2_147_483_648.0, 2_147_483_647.0) as i32;
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        bytes
    }

    #[test]
    fn decode_mono_f32_handles_every_encoding() {
        for encoding in [
            PcmEncoding::F32,
            PcmEncoding::S8,
            PcmEncoding::S16,
            PcmEncoding::S24,
            PcmEncoding::S32,
        ] {
            let source = sine(440.0, 48_000, 0.01, 0.5);
            let decoded = decode_mono_f32(&encode(&source, encoding), encoding, 1);
            assert_eq!(decoded.len(), source.len(), "{encoding:?}");
            for (got, want) in decoded.iter().zip(&source) {
                let tolerance = if encoding == PcmEncoding::S8 {
                    0.02
                } else {
                    0.001
                };
                assert!(
                    (got - want).abs() <= tolerance,
                    "{encoding:?}: got {got}, want {want}"
                );
            }
        }
    }

    #[test]
    fn decode_mono_f32_extracts_channel_zero() {
        let mut bytes = Vec::new();
        for (left, right) in [(0.25f32, -0.5f32), (0.75, -0.25)] {
            bytes.extend_from_slice(&left.to_le_bytes());
            bytes.extend_from_slice(&right.to_le_bytes());
        }
        let samples = decode_mono_f32(&bytes, PcmEncoding::F32, 2);
        assert_eq!(samples, vec![0.25, 0.75]);
    }

    #[test]
    fn goertzel_measures_a_440_hz_tone() {
        let samples = sine(440.0, 48_000, 1.0, 0.5);
        let at_440 = goertzel_amplitude(&samples, 48_000, 440.0);
        let at_1000 = goertzel_amplitude(&samples, 48_000, 1000.0);
        assert!((at_440 - 0.5).abs() < 0.05, "amplitude at 440: {at_440}");
        assert!(at_1000 < 0.05, "amplitude at 1000: {at_1000}");
    }

    #[test]
    fn dominant_frequency_finds_the_tone() {
        let samples = sine(440.0, 48_000, 1.0, 0.5);
        let detected = dominant_frequency(&samples, 48_000, 300.0, 600.0).unwrap();
        assert!((detected - 440.0).abs() <= 2.0, "detected {detected}");

        let high = sine(1000.0, 48_000, 1.0, 0.5);
        let detected = dominant_frequency(&high, 48_000, 800.0, 1200.0).unwrap();
        assert!((detected - 1000.0).abs() <= 2.0, "detected {detected}");
    }

    #[test]
    fn dominant_frequency_rejects_unusable_input() {
        assert!(dominant_frequency(&[], 48_000, 300.0, 600.0).is_none());
        assert!(dominant_frequency(&[0.0; 100], 0, 300.0, 600.0).is_none());
        assert!(dominant_frequency(&[0.0; 100], 48_000, 600.0, 300.0).is_none());
    }

    fn endpoint(
        serial: &str,
        nick: Option<&str>,
        description: Option<&str>,
        name: Option<&str>,
        priority: i64,
    ) -> EndpointInfo {
        EndpointInfo {
            serial: serial.to_owned(),
            node_name: name.map(str::to_owned),
            node_nick: nick.map(str::to_owned),
            node_description: description.map(str::to_owned),
            priority_session: priority,
        }
    }

    #[test]
    fn display_name_falls_back_in_old_server_order() {
        let with_nick = endpoint("1", Some("Nick"), Some("Desc"), Some("Name"), 1);
        assert_eq!(with_nick.display_name(), "Nick");

        let without_nick = endpoint("2", None, Some("Desc"), Some("Name"), 1);
        assert_eq!(without_nick.display_name(), "Desc");

        let only_name = endpoint("3", None, None, Some("Name"), 1);
        assert_eq!(only_name.display_name(), "Name");

        let bare = endpoint("4", None, None, None, 1);
        assert_eq!(bare.display_name(), "4");
    }

    #[test]
    fn default_is_highest_priority() {
        let sinks = [
            endpoint("10", None, None, Some("low"), 500),
            endpoint("11", None, None, Some("high"), 2000),
            endpoint("12", None, None, Some("mid"), 1000),
        ];
        assert_eq!(pick_default(&sinks).unwrap().serial, "11");
    }

    #[test]
    fn default_requires_priority_strictly_greater_than_minus_one() {
        let sinks = [
            endpoint("20", None, None, Some("missing"), -1),
            endpoint("21", None, None, Some("negative"), -5),
        ];
        // Old server initializes default_priority to -1 and never selects a
        // sink whose priority is not strictly greater.
        assert!(pick_default(&sinks).is_none());
        assert!(pick_default(&[]).is_none());
    }

    #[test]
    fn default_is_first_seen_on_priority_ties() {
        let sinks = [
            endpoint("30", None, None, Some("first"), 1000),
            endpoint("31", None, None, Some("second"), 1000),
        ];
        assert_eq!(pick_default(&sinks).unwrap().serial, "30");
    }

    #[test]
    fn default_picks_priority_zero_over_missing_priority() {
        let sinks = [
            endpoint("40", None, None, Some("missing"), -1),
            endpoint("41", None, None, Some("zero"), 0),
        ];
        assert_eq!(pick_default(&sinks).unwrap().serial, "41");
    }

    #[test]
    fn audio_params_accept_the_documented_values() {
        assert!(validate_audio_params(0, 0).is_ok());
        assert!(validate_audio_params(1, 44100).is_ok());
        assert!(validate_audio_params(2, 48000).is_ok());
        assert!(validate_audio_params(3, 48000).is_err());
        assert!(validate_audio_params(2, 96000).is_err());
    }

    #[test]
    fn bytes_per_sample_matches_old_server() {
        assert_eq!(PcmEncoding::F32.bytes_per_sample(), 4);
        assert_eq!(PcmEncoding::S8.bytes_per_sample(), 1);
        assert_eq!(PcmEncoding::S16.bytes_per_sample(), 2);
        // Packed 24-bit, not a 32-bit container.
        assert_eq!(PcmEncoding::S24.bytes_per_sample(), 3);
        assert_eq!(PcmEncoding::S32.bytes_per_sample(), 4);
    }

    #[test]
    fn block_align_is_bytes_per_sample_times_channels() {
        assert_eq!(PcmEncoding::F32.block_align(0), 0);
        assert_eq!(PcmEncoding::F32.block_align(1), 4);
        assert_eq!(PcmEncoding::F32.block_align(2), 8);
        assert_eq!(PcmEncoding::S16.block_align(2), 4);
        assert_eq!(PcmEncoding::S24.block_align(2), 6);
        assert_eq!(PcmEncoding::S8.block_align(1), 1);
    }

    #[test]
    fn encoding_parsing_accepts_cli_spellings() {
        for (text, expected) in [
            ("f32", PcmEncoding::F32),
            ("F32LE", PcmEncoding::F32),
            ("s8", PcmEncoding::S8),
            ("s16", PcmEncoding::S16),
            ("s24le", PcmEncoding::S24),
            ("s32", PcmEncoding::S32),
        ] {
            assert_eq!(text.parse::<PcmEncoding>().unwrap(), expected);
        }
        assert!("s64".parse::<PcmEncoding>().is_err());
    }
}
