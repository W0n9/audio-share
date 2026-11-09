//! Platform-neutral WASAPI format negotiation.
//!
//! These rules mirror the old C++ server (`audio_manager_impl.cpp`) and the
//! ticket-02 spike. They are pure so they can be tested on any host; the
//! Windows-only backend converts `wasapi::WaveFormat` to and from
//! [`FormatSpec`].

use crate::capture::EncodingPreference;
use crate::protocol::{AudioFormat, Encoding};

/// The `WAVEFORMATEXTENSIBLE` subformat, reduced to the v0.3.4 encodings.
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
/// advertised as `ENCODING_PCM_FLOAT` because the Android Client expects 32-bit
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
pub fn expected_block_align(container_bits: u16, channels: u16) -> u32 {
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

impl EncodingPreference {
    /// The requested sample representation; `None` keeps the mix format.
    ///
    /// `S24` is packed 3-byte PCM (`storebits == validbits == 24`), which is
    /// what Android's `ENCODING_PCM_24BIT` expects.
    fn layout(self) -> Option<(SampleSubtype, u16, u16)> {
        match self {
            Self::Default => None,
            Self::F32 => Some((SampleSubtype::Float, 32, 32)),
            Self::S8 => Some((SampleSubtype::Int, 8, 8)),
            Self::S16 => Some((SampleSubtype::Int, 16, 16)),
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
/// mix channel mask is kept only when the channel count is unchanged - the
/// `wasapi` crate supplies a default mask otherwise.
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
        block_align: expected_block_align(container_bits, channels),
        channel_mask,
    }
}

/// Outcome of `IsFormatSupported`: the request itself or the closest match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NegotiatedFormat {
    /// The format the endpoint will actually deliver.
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

/// Why a negotiated WASAPI format cannot be advertised to the v0.3.4 client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatError {
    /// The subformat or container width has no v0.3.4 encoding.
    UnsupportedEncoding {
        subtype: SampleSubtype,
        container_bits: u16,
    },
    /// The OS container disagrees with the mapped encoding's frame size.
    BlockAlignMismatch { expected: u32, actual: u32 },
    /// A stream must have at least one channel.
    InvalidChannels,
    /// A sample rate of zero or above `i32::MAX` cannot be advertised.
    InvalidSampleRate,
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
            Self::InvalidChannels => write!(f, "the negotiated format has no channels"),
            Self::InvalidSampleRate => {
                write!(f, "the negotiated sample rate cannot be represented")
            }
        }
    }
}

impl std::error::Error for FormatError {}

/// Validate a negotiated format and reduce it to the values advertised in
/// `GET_FORMAT`.
///
/// The encoding is derived from the container width, not from the requested
/// preference: 24 valid bits in a 32-bit container is advertised as
/// `PCM_32BIT`. A container whose `nBlockAlign` disagrees with its encoding is
/// rejected instead of mis-advertised.
pub fn advertised_format(format: &FormatSpec) -> Result<AudioFormat, FormatError> {
    if format.channels == 0 {
        return Err(FormatError::InvalidChannels);
    }
    if format.sample_rate == 0 || format.sample_rate > i32::MAX as u32 {
        return Err(FormatError::InvalidSampleRate);
    }
    let encoding = encoding_for(format.subtype, format.container_bits);
    if encoding == Encoding::Invalid {
        return Err(FormatError::UnsupportedEncoding {
            subtype: format.subtype,
            container_bits: format.container_bits,
        });
    }
    let expected = expected_block_align(format.container_bits, format.channels);
    if format.block_align != expected {
        return Err(FormatError::BlockAlignMismatch {
            expected,
            actual: format.block_align,
        });
    }
    Ok(AudioFormat::new(
        encoding,
        i32::from(format.channels),
        format.sample_rate as i32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(expected_block_align(8, 1), 1);
        assert_eq!(expected_block_align(16, 2), 4);
        assert_eq!(expected_block_align(32, 2), 8);
        assert_eq!(expected_block_align(32, 6), 24);
    }

    #[test]
    fn packed_s24_uses_three_bytes_per_sample() {
        assert_eq!(expected_block_align(24, 2), 6);
        assert_eq!(expected_block_align(24, 1), 3);
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
            block_align: expected_block_align(bits, channels),
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
                expected_block_align(8, 2),
            ),
            (
                EncodingPreference::S16,
                SampleSubtype::Int,
                16,
                16,
                expected_block_align(16, 2),
            ),
            (
                EncodingPreference::S24,
                SampleSubtype::Int,
                24,
                24,
                expected_block_align(24, 2),
            ),
            (
                EncodingPreference::S32,
                SampleSubtype::Int,
                32,
                32,
                expected_block_align(32, 2),
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
    fn advertised_format_maps_encoding_and_stream_properties() {
        assert_eq!(
            advertised_format(&spec(SampleSubtype::Float, 32, 2)),
            Ok(AudioFormat::new(Encoding::PcmFloat, 2, 48_000))
        );
        assert_eq!(
            advertised_format(&spec(SampleSubtype::Int, 8, 1)),
            Ok(AudioFormat::new(Encoding::Pcm8Bit, 1, 48_000))
        );
        assert_eq!(
            advertised_format(&spec(SampleSubtype::Int, 16, 2)),
            Ok(AudioFormat::new(Encoding::Pcm16Bit, 2, 48_000))
        );
        assert_eq!(
            advertised_format(&spec(SampleSubtype::Int, 32, 6)),
            Ok(AudioFormat::new(Encoding::Pcm32Bit, 6, 48_000))
        );
    }

    #[test]
    fn packed_s24_is_advertised_as_three_bytes_per_sample() {
        let advertised = advertised_format(&spec(SampleSubtype::Int, 24, 2)).unwrap();
        assert_eq!(advertised, AudioFormat::new(Encoding::Pcm24Bit, 2, 48_000));
        assert_eq!(advertised.block_align(), 6);
    }

    #[test]
    fn twenty_four_valid_bits_in_a_32_bit_container_are_pcm32() {
        let mut format = spec(SampleSubtype::Int, 32, 2);
        format.valid_bits = 24;
        let advertised = advertised_format(&format).unwrap();
        assert_eq!(advertised, AudioFormat::new(Encoding::Pcm32Bit, 2, 48_000));
        assert_eq!(advertised.block_align(), 8);
    }

    #[test]
    fn advertised_format_rejects_a_block_align_mismatch() {
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

    #[test]
    fn advertised_format_rejects_zero_channels_or_sample_rate() {
        let mut no_channels = spec(SampleSubtype::Int, 16, 2);
        no_channels.channels = 0;
        no_channels.block_align = 0;
        assert_eq!(
            advertised_format(&no_channels),
            Err(FormatError::InvalidChannels)
        );

        let mut no_rate = spec(SampleSubtype::Int, 16, 2);
        no_rate.sample_rate = 0;
        assert_eq!(
            advertised_format(&no_rate),
            Err(FormatError::InvalidSampleRate)
        );

        let mut huge_rate = spec(SampleSubtype::Int, 16, 2);
        huge_rate.sample_rate = i32::MAX as u32 + 1;
        assert_eq!(
            advertised_format(&huge_rate),
            Err(FormatError::InvalidSampleRate)
        );
    }
}
