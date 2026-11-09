//! Platform-neutral format and measurement logic.
//!
//! Everything here is pure so it can be unit-tested on any host. The Windows
//! module converts `wasapi` types into these types and back.

use std::time::Duration;

use audio_share_core::protocol::format::Encoding;

/// The `WAVEFORMATEXTENSIBLE` subformat, reduced to what this tool maps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleSubtype {
    /// `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT`.
    Float,
    /// `KSDATAFORMAT_SUBTYPE_PCM`.
    Int,
    /// Any other subformat; cannot be advertised as a v0.3.4 encoding.
    Other,
}

/// Map a WASAPI sample representation to the frozen v0.3.4 encoding.
///
/// Float is only valid as 32-bit; a 64-bit float container must not be
/// advertised as `ENCODING_PCM_FLOAT` because the Android client expects 32-bit
/// floats. Integer widths map by container width; 24-bit means packed 3-byte
/// samples (`WaveFormat::new(24, 24, Int, ..)`).
pub fn encoding_for(subtype: SampleSubtype, container_bits: u16) -> Encoding {
    match subtype {
        SampleSubtype::Float if container_bits == 32 => Encoding::PcmFloat,
        SampleSubtype::Float => Encoding::Invalid,
        SampleSubtype::Int => match container_bits {
            8 => Encoding::Pcm8Bit,
            16 => Encoding::Pcm16Bit,
            24 => Encoding::Pcm24Bit,
            32 => Encoding::Pcm32Bit,
            _ => Encoding::Invalid,
        },
        SampleSubtype::Other => Encoding::Invalid,
    }
}

/// `nBlockAlign` for a PCM container: bytes per sample times channel count.
///
/// The supported container widths are all byte-aligned, matching the old C++
/// `wBitsPerSample * nChannels / 8` computation. Packed s24 stereo is 6.
pub fn block_align(container_bits: u16, channels: u16) -> u32 {
    (u32::from(container_bits) * u32::from(channels)) / 8
}

/// A WASAPI format in platform-neutral terms.
///
/// `block_align` is carried separately because the OS closest match supplies
/// its own value; [`advertised_format`] cross-checks it against the encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatSpec {
    pub subtype: SampleSubtype,
    pub container_bits: u16,
    pub valid_bits: u16,
    pub channels: u16,
    pub sample_rate: u32,
    pub block_align: u32,
    pub channel_mask: Option<u32>,
}

/// The encoding the user asked for; `Default` keeps the endpoint mix format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum EncodingPreference {
    Default,
    F32,
    S8,
    S16,
    S24,
    S32,
}

impl EncodingPreference {
    fn layout(self) -> Option<(SampleSubtype, u16, u16)> {
        match self {
            Self::Default => None,
            Self::F32 => Some((SampleSubtype::Float, 32, 32)),
            Self::S8 => Some((SampleSubtype::Int, 8, 8)),
            Self::S16 => Some((SampleSubtype::Int, 16, 16)),
            // Packed 3-byte samples: storebits == validbits == 24.
            Self::S24 => Some((SampleSubtype::Int, 24, 24)),
            Self::S32 => Some((SampleSubtype::Int, 32, 32)),
        }
    }
}

/// Build the format to pass to `IsFormatSupported`.
///
/// This mirrors the old C++ `audio_manager_impl.cpp`: `Default` keeps the mix
/// format's sample representation; an explicit encoding overrides the subformat
/// and container width; `channels`/`sample_rate` of 0 keep the mix values. The
/// mix channel mask is kept only when the channel count is unchanged — the
/// crate supplies a default mask otherwise.
pub fn requested_format(
    mix: FormatSpec,
    preference: EncodingPreference,
    channels: u16,
    sample_rate: u32,
) -> FormatSpec {
    let (subtype, container_bits, valid_bits) =
        preference
            .layout()
            .unwrap_or((mix.subtype, mix.container_bits, mix.valid_bits));
    let channels = if channels == 0 {
        mix.channels
    } else {
        channels
    };
    let sample_rate = if sample_rate == 0 {
        mix.sample_rate
    } else {
        sample_rate
    };
    let channel_mask = if channels == mix.channels {
        mix.channel_mask
    } else {
        None
    };
    FormatSpec {
        subtype,
        container_bits,
        valid_bits,
        channels,
        sample_rate,
        block_align: block_align(container_bits, channels),
        channel_mask,
    }
}

/// Outcome of `IsFormatSupported`: the request itself or the closest match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NegotiatedFormat {
    pub format: FormatSpec,
    /// True when the OS returned `S_FALSE` plus a closest match.
    pub used_closest_match: bool,
}

/// Advertise the format the OS will actually deliver, never the requested one.
pub fn select_negotiated(requested: FormatSpec, closest: Option<FormatSpec>) -> NegotiatedFormat {
    match closest {
        Some(format) => NegotiatedFormat {
            format,
            used_closest_match: true,
        },
        None => NegotiatedFormat {
            format: requested,
            used_closest_match: false,
        },
    }
}

/// The v0.3.4 `AudioFormat` fields plus the PCM frame size used on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdvertisedFormat {
    pub encoding: Encoding,
    pub channels: u16,
    pub sample_rate: u32,
    pub block_align: u32,
}

/// Why a negotiated WASAPI format cannot be advertised to the Android client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatError {
    UnsupportedEncoding {
        subtype: SampleSubtype,
        container_bits: u16,
    },
    BlockAlignMismatch {
        expected: u32,
        actual: u32,
    },
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedEncoding {
                subtype,
                container_bits,
            } => write!(
                f,
                "cannot advertise a v0.3.4 encoding for {subtype:?} samples in a \
                 {container_bits}-bit container"
            ),
            Self::BlockAlignMismatch { expected, actual } => write!(
                f,
                "negotiated block align {actual} does not match the advertised \
                 encoding's expected block align {expected}"
            ),
        }
    }
}

impl std::error::Error for FormatError {}

/// Validate a negotiated format and reduce it to the values advertized in
/// `GET_FORMAT`. A 24-valid-bit sample in a 32-bit container is advertised as
/// `PCM_32BIT`; a container whose `nBlockAlign` disagrees with its encoding is
/// rejected instead of mis-advertised.
pub fn advertised_format(format: &FormatSpec) -> Result<AdvertisedFormat, FormatError> {
    let encoding = encoding_for(format.subtype, format.container_bits);
    if encoding == Encoding::Invalid {
        return Err(FormatError::UnsupportedEncoding {
            subtype: format.subtype,
            container_bits: format.container_bits,
        });
    }
    let expected = block_align(format.container_bits, format.channels);
    if format.block_align != expected {
        return Err(FormatError::BlockAlignMismatch {
            expected,
            actual: format.block_align,
        });
    }
    Ok(AdvertisedFormat {
        encoding,
        channels: format.channels,
        sample_rate: format.sample_rate,
        block_align: format.block_align,
    })
}

/// Per-packet `AUDCLNT_BUFFERFLAGS_*` observations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PacketFlags {
    pub silent: bool,
    pub data_discontinuity: bool,
    pub timestamp_error: bool,
}

/// Counters and interval samples from one capture session.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub packets: u64,
    pub frames: u64,
    pub bytes: u64,
    pub silent_packets: u64,
    pub discontinuities: u64,
    pub timestamp_errors: u64,
    /// Frames missing between the previous packet's end and this packet's start.
    pub index_gap_frames: u64,
    /// Number of packets that started after the expected device position.
    pub index_gap_events: u64,
    pub max_packet_frames: u32,
    /// Wall time between consecutive packets; the first packet has no interval.
    pub packet_intervals: Vec<Duration>,
    /// Wall time between consecutive poll wake-ups.
    pub poll_intervals: Vec<Duration>,
    expected_index: Option<u64>,
}

impl Stats {
    /// Record one packet read from `IAudioCaptureClient::GetBuffer`.
    ///
    /// `index` is the device position of the packet's first frame. A forward
    /// jump means frames were lost before we read them; a reset or overlap is
    /// ignored because the device position can restart after a stream restart.
    pub fn on_packet(
        &mut self,
        frames: u32,
        block_align: u32,
        flags: PacketFlags,
        index: u64,
        interval: Option<Duration>,
    ) {
        self.packets += 1;
        self.frames += u64::from(frames);
        self.bytes += u64::from(frames) * u64::from(block_align);
        self.max_packet_frames = self.max_packet_frames.max(frames);
        if flags.silent {
            self.silent_packets += 1;
        }
        if flags.data_discontinuity {
            self.discontinuities += 1;
        }
        if flags.timestamp_error {
            self.timestamp_errors += 1;
        }
        if let Some(expected) = self.expected_index {
            if index > expected {
                self.index_gap_frames += index - expected;
                self.index_gap_events += 1;
            }
        }
        self.expected_index = Some(index + u64::from(frames));
        if let Some(interval) = interval {
            self.packet_intervals.push(interval);
        }
    }

    pub fn on_poll(&mut self, interval: Duration) {
        self.poll_intervals.push(interval);
    }

    pub fn packet_interval_summary(&self) -> Option<IntervalSummary> {
        summarize(&self.packet_intervals)
    }

    pub fn poll_interval_summary(&self) -> Option<IntervalSummary> {
        summarize(&self.poll_intervals)
    }

    /// Poll wake-ups slower than `threshold` (callers pass 2x the device period).
    pub fn long_poll_count(&self, threshold: Duration) -> u64 {
        self.poll_intervals
            .iter()
            .filter(|interval| **interval > threshold)
            .count() as u64
    }
}

/// Min/avg/max/p99 of a set of durations. `p99` uses the nearest-rank method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntervalSummary {
    pub count: u64,
    pub min: Duration,
    pub avg: Duration,
    pub max: Duration,
    pub p99: Duration,
}

pub fn summarize(intervals: &[Duration]) -> Option<IntervalSummary> {
    if intervals.is_empty() {
        return None;
    }
    let mut sorted = intervals.to_vec();
    sorted.sort_unstable();
    let count = sorted.len();
    let total_nanos: u128 = sorted.iter().map(Duration::as_nanos).sum();
    let rank = (99 * count).div_ceil(100); // ceil(0.99 * count), at least 1
    Some(IntervalSummary {
        count: count as u64,
        min: sorted[0],
        avg: Duration::from_nanos((total_nanos / count as u128) as u64),
        max: sorted[count - 1],
        p99: sorted[rank.saturating_sub(1).min(count - 1)],
    })
}

/// CPU use as a percentage of one core: `cpu_time / wall_time * 100`.
pub fn cpu_percent(cpu_time: Duration, wall_time: Duration) -> Option<f64> {
    let wall = wall_time.as_secs_f64();
    if wall <= 0.0 {
        return None;
    }
    Some(cpu_time.as_secs_f64() / wall * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_share_core::protocol::format::Encoding;

    #[test]
    fn maps_ieee_float_32_to_pcm_float() {
        assert_eq!(encoding_for(SampleSubtype::Float, 32), Encoding::PcmFloat);
    }

    #[test]
    fn rejects_float_container_widths_other_than_32_bits() {
        assert_eq!(encoding_for(SampleSubtype::Float, 16), Encoding::Invalid);
        assert_eq!(encoding_for(SampleSubtype::Float, 64), Encoding::Invalid);
    }

    #[test]
    fn maps_integer_container_widths_to_pcm_encodings() {
        assert_eq!(encoding_for(SampleSubtype::Int, 8), Encoding::Pcm8Bit);
        assert_eq!(encoding_for(SampleSubtype::Int, 16), Encoding::Pcm16Bit);
        assert_eq!(encoding_for(SampleSubtype::Int, 24), Encoding::Pcm24Bit);
        assert_eq!(encoding_for(SampleSubtype::Int, 32), Encoding::Pcm32Bit);
    }

    #[test]
    fn rejects_unknown_subtypes_and_bit_widths() {
        assert_eq!(encoding_for(SampleSubtype::Other, 16), Encoding::Invalid);
        assert_eq!(encoding_for(SampleSubtype::Int, 12), Encoding::Invalid);
        assert_eq!(encoding_for(SampleSubtype::Int, 0), Encoding::Invalid);
    }

    #[test]
    fn block_align_is_container_bytes_times_channels() {
        assert_eq!(block_align(8, 1), 1);
        assert_eq!(block_align(16, 2), 4);
        assert_eq!(block_align(32, 2), 8);
        assert_eq!(block_align(32, 6), 24);
    }

    #[test]
    fn packed_s24_uses_three_bytes_per_sample() {
        assert_eq!(block_align(24, 2), 6);
    }

    fn stereo_float_mix() -> FormatSpec {
        FormatSpec {
            subtype: SampleSubtype::Float,
            container_bits: 32,
            valid_bits: 32,
            channels: 2,
            sample_rate: 48_000,
            block_align: 8,
            channel_mask: Some(0x3),
        }
    }

    fn spec(subtype: SampleSubtype, bits: u16, channels: u16) -> FormatSpec {
        FormatSpec {
            subtype,
            container_bits: bits,
            valid_bits: bits,
            channels,
            sample_rate: 48_000,
            block_align: block_align(bits, channels),
            channel_mask: Some(0x3),
        }
    }

    #[test]
    fn default_preference_preserves_the_mix_format_exactly() {
        let mix = stereo_float_mix();
        let requested = requested_format(mix, EncodingPreference::Default, 0, 0);
        assert_eq!(requested, mix);
    }

    #[test]
    fn default_preference_keeps_mix_representation_when_rate_or_channels_change() {
        let mix = stereo_float_mix();
        let requested = requested_format(mix, EncodingPreference::Default, 1, 44_100);
        assert_eq!(requested.subtype, SampleSubtype::Float);
        assert_eq!(requested.container_bits, 32);
        assert_eq!(requested.valid_bits, 32);
        assert_eq!(requested.channels, 1);
        assert_eq!(requested.sample_rate, 44_100);
        assert_eq!(requested.block_align, 4);
    }

    #[test]
    fn explicit_encodings_map_to_the_old_cpp_requested_layouts() {
        let mix = stereo_float_mix();
        let cases = [
            (
                EncodingPreference::F32,
                SampleSubtype::Float,
                32u16,
                32u16,
                8u32,
            ),
            (
                EncodingPreference::S8,
                SampleSubtype::Int,
                8,
                8,
                block_align(8, 2),
            ),
            (
                EncodingPreference::S16,
                SampleSubtype::Int,
                16,
                16,
                block_align(16, 2),
            ),
            (
                EncodingPreference::S24,
                SampleSubtype::Int,
                24,
                24,
                block_align(24, 2),
            ),
            (
                EncodingPreference::S32,
                SampleSubtype::Int,
                32,
                32,
                block_align(32, 2),
            ),
        ];
        for (preference, subtype, container_bits, valid_bits, expected_align) in cases {
            let requested = requested_format(mix, preference, 0, 0);
            assert_eq!(requested.subtype, subtype, "{preference:?}");
            assert_eq!(requested.container_bits, container_bits, "{preference:?}");
            assert_eq!(requested.valid_bits, valid_bits, "{preference:?}");
            assert_eq!(requested.channels, 2, "{preference:?}");
            assert_eq!(requested.sample_rate, 48_000, "{preference:?}");
            assert_eq!(requested.block_align, expected_align, "{preference:?}");
        }
    }

    #[test]
    fn zero_channels_and_rate_mean_endpoint_defaults() {
        let mix = stereo_float_mix();
        let requested = requested_format(mix, EncodingPreference::S16, 0, 0);
        assert_eq!(requested.channels, mix.channels);
        assert_eq!(requested.sample_rate, mix.sample_rate);
    }

    #[test]
    fn channel_mask_is_kept_only_when_the_channel_count_is_unchanged() {
        let mix = stereo_float_mix();
        assert_eq!(
            requested_format(mix, EncodingPreference::S16, 2, 0).channel_mask,
            Some(0x3)
        );
        assert_eq!(
            requested_format(mix, EncodingPreference::S16, 1, 0).channel_mask,
            None
        );
    }

    #[test]
    fn closest_match_replaces_the_requested_format() {
        let requested = spec(SampleSubtype::Int, 24, 2);
        let closest = stereo_float_mix();
        let negotiated = select_negotiated(requested, Some(closest));
        assert!(negotiated.used_closest_match);
        assert_eq!(negotiated.format, closest);
    }

    #[test]
    fn a_supported_request_is_advertised_unchanged() {
        let requested = spec(SampleSubtype::Int, 16, 2);
        let negotiated = select_negotiated(requested, None);
        assert!(!negotiated.used_closest_match);
        assert_eq!(negotiated.format, requested);
    }

    #[test]
    fn advertised_format_maps_encoding_and_block_align() {
        assert_eq!(
            advertised_format(&spec(SampleSubtype::Float, 32, 2)),
            Ok(AdvertisedFormat {
                encoding: Encoding::PcmFloat,
                channels: 2,
                sample_rate: 48_000,
                block_align: 8,
            })
        );
        assert_eq!(
            advertised_format(&spec(SampleSubtype::Int, 24, 2)),
            Ok(AdvertisedFormat {
                encoding: Encoding::Pcm24Bit,
                channels: 2,
                sample_rate: 48_000,
                block_align: 6,
            })
        );
    }

    #[test]
    fn advertised_format_rejects_a_container_width_mismatch() {
        // A 24-bit format in a 32-bit container must be advertised as 32-bit,
        // never as packed 24-bit with a 4-byte-per-sample block align.
        let mut odd = spec(SampleSubtype::Int, 32, 2);
        odd.valid_bits = 24;
        assert_eq!(
            advertised_format(&odd),
            Ok(AdvertisedFormat {
                encoding: Encoding::Pcm32Bit,
                channels: 2,
                sample_rate: 48_000,
                block_align: 8,
            })
        );

        let mut broken = spec(SampleSubtype::Int, 24, 2);
        broken.block_align = 8;
        assert_eq!(
            advertised_format(&broken),
            Err(FormatError::BlockAlignMismatch {
                expected: 6,
                actual: 8,
            })
        );
    }

    #[test]
    fn advertised_format_rejects_unsupported_subformats_and_float_widths() {
        assert_eq!(
            advertised_format(&spec(SampleSubtype::Other, 16, 2)),
            Err(FormatError::UnsupportedEncoding {
                subtype: SampleSubtype::Other,
                container_bits: 16,
            })
        );
        assert_eq!(
            advertised_format(&spec(SampleSubtype::Float, 64, 2)),
            Err(FormatError::UnsupportedEncoding {
                subtype: SampleSubtype::Float,
                container_bits: 64,
            })
        );
    }

    fn flags(silent: bool, data_discontinuity: bool, timestamp_error: bool) -> PacketFlags {
        PacketFlags {
            silent,
            data_discontinuity,
            timestamp_error,
        }
    }

    #[test]
    fn stats_count_packets_frames_bytes_and_flags() {
        let mut stats = Stats::default();
        stats.on_packet(480, 8, flags(true, false, false), 0, None);
        stats.on_packet(
            480,
            8,
            flags(false, true, false),
            480,
            Some(Duration::from_millis(10)),
        );
        stats.on_packet(
            240,
            8,
            flags(false, false, true),
            960,
            Some(Duration::from_millis(5)),
        );
        assert_eq!(stats.packets, 3);
        assert_eq!(stats.frames, 1_200);
        assert_eq!(stats.bytes, 9_600);
        assert_eq!(stats.silent_packets, 1);
        assert_eq!(stats.discontinuities, 1);
        assert_eq!(stats.timestamp_errors, 1);
        assert_eq!(stats.max_packet_frames, 480);
        assert_eq!(stats.packet_intervals.len(), 2);
    }

    #[test]
    fn stats_do_not_record_an_interval_for_the_first_packet() {
        let mut stats = Stats::default();
        stats.on_packet(480, 8, PacketFlags::default(), 0, None);
        assert!(stats.packet_intervals.is_empty());
        stats.on_packet(
            480,
            8,
            PacketFlags::default(),
            480,
            Some(Duration::from_millis(9)),
        );
        assert_eq!(stats.packet_intervals, vec![Duration::from_millis(9)]);
    }

    #[test]
    fn stats_detect_forward_index_gaps_in_frames() {
        let mut stats = Stats::default();
        stats.on_packet(10, 1, PacketFlags::default(), 100, None);
        stats.on_packet(10, 1, PacketFlags::default(), 120, None);
        assert_eq!(stats.index_gap_frames, 10);
        assert_eq!(stats.index_gap_events, 1);
        // Contiguous packet after the gap: no new gap.
        stats.on_packet(10, 1, PacketFlags::default(), 130, None);
        assert_eq!(stats.index_gap_frames, 10);
        assert_eq!(stats.index_gap_events, 1);
    }

    #[test]
    fn stats_ignore_index_resets_and_overlapping_packets() {
        let mut stats = Stats::default();
        stats.on_packet(10, 1, PacketFlags::default(), 1_000, None);
        stats.on_packet(10, 1, PacketFlags::default(), 0, None);
        assert_eq!(stats.index_gap_frames, 0);
        assert_eq!(stats.index_gap_events, 0);
        stats.on_packet(10, 1, PacketFlags::default(), 5, None);
        assert_eq!(stats.index_gap_frames, 0);
    }

    #[test]
    fn stats_record_poll_intervals_and_long_polls() {
        let mut stats = Stats::default();
        for ms in [10, 21, 40] {
            stats.on_poll(Duration::from_millis(ms));
        }
        assert_eq!(stats.poll_intervals.len(), 3);
        assert_eq!(stats.long_poll_count(Duration::from_millis(20)), 2);
    }

    #[test]
    fn stats_expose_packet_and_poll_interval_summaries() {
        let mut stats = Stats::default();
        stats.on_packet(10, 1, PacketFlags::default(), 0, None);
        stats.on_packet(
            10,
            1,
            PacketFlags::default(),
            10,
            Some(Duration::from_millis(4)),
        );
        stats.on_packet(
            10,
            1,
            PacketFlags::default(),
            20,
            Some(Duration::from_millis(6)),
        );
        let packets = stats.packet_interval_summary().unwrap();
        assert_eq!(packets.count, 2);
        assert_eq!(packets.min, Duration::from_millis(4));
        assert_eq!(packets.avg, Duration::from_millis(5));
        assert_eq!(packets.max, Duration::from_millis(6));

        stats.on_poll(Duration::from_millis(10));
        stats.on_poll(Duration::from_millis(20));
        let polls = stats.poll_interval_summary().unwrap();
        assert_eq!(polls.count, 2);
        assert_eq!(polls.min, Duration::from_millis(10));
        assert_eq!(polls.max, Duration::from_millis(20));
    }

    #[test]
    fn summarize_reports_min_avg_max_and_nearest_rank_p99() {
        let intervals: Vec<Duration> = (1..=100).map(Duration::from_millis).collect();
        let summary = summarize(&intervals).unwrap();
        assert_eq!(summary.count, 100);
        assert_eq!(summary.min, Duration::from_millis(1));
        assert_eq!(summary.max, Duration::from_millis(100));
        assert_eq!(summary.avg, Duration::from_micros(50_500));
        assert_eq!(summary.p99, Duration::from_millis(99));
    }

    #[test]
    fn summarize_p99_is_the_maximum_for_small_samples() {
        let intervals = vec![
            Duration::from_millis(1),
            Duration::from_millis(2),
            Duration::from_millis(3),
            Duration::from_millis(4),
        ];
        assert_eq!(summarize(&intervals).unwrap().p99, Duration::from_millis(4));
    }

    #[test]
    fn summarize_returns_none_for_no_samples() {
        assert_eq!(summarize(&[]), None);
    }

    #[test]
    fn cpu_percent_is_cpu_time_over_wall_time() {
        assert_eq!(
            cpu_percent(Duration::from_secs(5), Duration::from_secs(10)),
            Some(50.0)
        );
        assert_eq!(cpu_percent(Duration::from_secs(1), Duration::ZERO), None);
    }
}
