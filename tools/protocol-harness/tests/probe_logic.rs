//! Public behavior of the acceptance probe's PCM analysis helpers.
//!
//! These tests feed known byte patterns and independently generated sine
//! waves; they do not reuse the probe's own encoding or tone-generation code.

use audio_share_core::protocol::{AudioFormat, Encoding};
use protocol_harness::probe::{
    analyze_tone, decode_mono_f32, dominant_frequency, goertzel_amplitude, validate_datagram,
};

/// Independently generate `amplitude * sin(2*pi*f*t)` at `sample_rate`.
fn sine(frequency: f32, amplitude: f32, sample_rate: u32, seconds: f32) -> Vec<f32> {
    let samples = (sample_rate as f32 * seconds) as usize;
    (0..samples)
        .map(|index| {
            amplitude
                * (2.0 * std::f32::consts::PI * frequency * index as f32 / sample_rate as f32).sin()
        })
        .collect()
}

#[test]
fn goertzel_measures_the_amplitude_of_an_independent_sine() {
    for sample_rate in [48_000, 44_100] {
        let samples = sine(440.0, 0.5, sample_rate, 0.25);
        let at_tone = goertzel_amplitude(&samples, sample_rate, 440.0);
        let off_tone = goertzel_amplitude(&samples, sample_rate, 1_000.0);
        assert!(
            (at_tone - 0.5).abs() < 0.02,
            "{sample_rate} Hz: measured {at_tone}"
        );
        assert!(
            off_tone < 0.02,
            "{sample_rate} Hz: 1000 Hz leaked {off_tone}"
        );
    }
}

#[test]
fn dominant_frequency_finds_a_440_hz_sine_within_two_hertz() {
    let samples = sine(440.0, 0.25, 48_000, 0.5);
    let detected = dominant_frequency(&samples, 48_000, 300.0, 600.0);
    assert!((detected - 440.0).abs() <= 2.0, "detected {detected} Hz");
}

#[test]
fn datagram_validation_enforces_the_1464_byte_mtu_and_block_alignment() {
    // Full 1464-byte payloads are valid when block_align divides them.
    assert!(validate_datagram(1464, 8).is_ok());
    assert!(validate_datagram(1464, 3).is_ok());
    assert!(validate_datagram(1464, 1).is_ok());

    // Short final datagrams must still contain whole sample frames.
    assert!(validate_datagram(280, 8).is_ok());
    assert!(validate_datagram(1460, 8).is_err());
    assert!(validate_datagram(1463, 8).is_err());

    // 1464 is the hard MTU-derived ceiling even if the length is aligned.
    assert!(validate_datagram(1465, 1).is_err());
    assert!(validate_datagram(1464, 5).is_err());
    assert!(validate_datagram(1456, 7).is_ok());

    // Empty datagrams and unusable formats are not valid PCM.
    assert!(validate_datagram(0, 8).is_err());
    assert!(validate_datagram(8, 0).is_err());
    assert!(validate_datagram(8, 1465).is_err());
}

/// Interleave one sample per channel as little-endian PCM.
fn interleave(samples: &[f32], encoding: Encoding, channels: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    for &sample in samples {
        let sample_bytes = match encoding {
            Encoding::PcmFloat => sample.to_le_bytes().to_vec(),
            Encoding::Pcm16Bit => ((sample * 32_768.0) as i16).to_le_bytes().to_vec(),
            _ => unreachable!("test helper only encodes f32 and s16"),
        };
        for _ in 0..channels {
            bytes.extend_from_slice(&sample_bytes);
        }
    }
    bytes
}

#[test]
fn tone_analysis_reports_the_expected_tone_and_rejects_silence() {
    let format = AudioFormat::new(Encoding::PcmFloat, 2, 48_000);
    let samples = sine(440.0, 0.25, 48_000, 0.5);

    let tone = analyze_tone(
        &interleave(&samples, Encoding::PcmFloat, 2),
        &format,
        440.0,
        0.01,
    );
    assert!(tone.present, "{tone:?}");
    assert!((tone.detected_hz - 440.0).abs() <= 2.0, "{tone:?}");
    assert!((tone.amplitude - 0.25).abs() < 0.02, "{tone:?}");
    assert!(tone.reference_amplitude < tone.amplitude / 10.0, "{tone:?}");

    let silence = vec![0u8; 48_000 * 8];
    let quiet = analyze_tone(&silence, &format, 440.0, 0.01);
    assert!(!quiet.present, "{quiet:?}");

    // The same analysis follows the negotiated integer encoding and channels.
    let s16 = AudioFormat::new(Encoding::Pcm16Bit, 1, 44_100);
    let samples = sine(440.0, 0.25, 44_100, 0.5);
    let tone = analyze_tone(
        &interleave(&samples, Encoding::Pcm16Bit, 1),
        &s16,
        440.0,
        0.01,
    );
    assert!(tone.present, "{tone:?}");
    assert!((tone.amplitude - 0.25).abs() < 0.02, "{tone:?}");
}

#[test]
fn decodes_the_first_channel_of_every_protocol_encoding() {
    // Each frame is two samples; the analysis keeps channel 0.
    let f32_frame = [
        0.5f32.to_le_bytes(),
        0.25f32.to_le_bytes(),
        (-0.5f32).to_le_bytes(),
        0.25f32.to_le_bytes(),
    ]
    .concat();
    assert_eq!(
        decode_mono_f32(&f32_frame, Encoding::PcmFloat, 2),
        vec![0.5, -0.5]
    );

    // Unsigned 8-bit: 128 is silence, 255 is +127/128, 0 is -1.
    let s8_frame = [128u8, 7, 255, 7, 0, 7];
    let decoded = decode_mono_f32(&s8_frame, Encoding::Pcm8Bit, 2);
    assert_eq!(decoded[0], 0.0);
    assert!((decoded[1] - 127.0 / 128.0).abs() < f32::EPSILON);
    assert_eq!(decoded[2], -1.0);

    // Signed 16-bit: 16384 is +0.5, -32768 is -1.
    let s16_frame = [
        16384i16.to_le_bytes(),
        1i16.to_le_bytes(),
        (-32768i16).to_le_bytes(),
        1i16.to_le_bytes(),
    ]
    .concat();
    assert_eq!(
        decode_mono_f32(&s16_frame, Encoding::Pcm16Bit, 2),
        vec![0.5, -1.0]
    );

    // Packed signed 24-bit: 0x400000 is +0.5, 0x800000 is -1.
    let s24_frame: Vec<u8> = vec![
        0x00, 0x00, 0x40, // +0.5, channel 0
        0x01, 0x00, 0x00, // channel 1
        0x00, 0x00, 0x80, // -1.0, channel 0
        0x01, 0x00, 0x00, // channel 1
    ];
    assert_eq!(
        decode_mono_f32(&s24_frame, Encoding::Pcm24Bit, 2),
        vec![0.5, -1.0]
    );

    // Signed 32-bit: 0x40000000 is +0.5, i32::MIN is -1.
    let s32_frame = [
        0x4000_0000i32.to_le_bytes(),
        1i32.to_le_bytes(),
        i32::MIN.to_le_bytes(),
        1i32.to_le_bytes(),
    ]
    .concat();
    assert_eq!(
        decode_mono_f32(&s32_frame, Encoding::Pcm32Bit, 2),
        vec![0.5, -1.0]
    );
}
