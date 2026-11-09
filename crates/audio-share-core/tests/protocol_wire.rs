//! Wire Protocol behavior at the public `audio_share_core::protocol` seam.

use audio_share_core::protocol::{
    decode_command, decode_get_format_response, decode_heartbeat, decode_start_play_response,
    decode_udp_bind, encode_command, encode_get_format_response, encode_heartbeat,
    encode_start_play_response, encode_udp_bind, AudioFormat, Command, Encoding, ProtocolError,
};
use prost::Message;

#[test]
fn commands_encode_as_little_endian_u32() {
    assert_eq!(encode_command(Command::GetFormat), [0x01, 0x00, 0x00, 0x00]);
    assert_eq!(encode_command(Command::StartPlay), [0x02, 0x00, 0x00, 0x00]);
    assert_eq!(encode_command(Command::Heartbeat), [0x03, 0x00, 0x00, 0x00]);
}

#[test]
fn encoding_values_match_client_proto() {
    assert_eq!(Encoding::Invalid as i32, 0);
    assert_eq!(Encoding::PcmFloat as i32, 1);
    assert_eq!(Encoding::Pcm8Bit as i32, 2);
    assert_eq!(Encoding::Pcm16Bit as i32, 3);
    assert_eq!(Encoding::Pcm24Bit as i32, 4);
    assert_eq!(Encoding::Pcm32Bit as i32, 5);
}

#[test]
fn audio_format_encodes_exact_v034_protobuf_wire_bytes() {
    let cases: &[(AudioFormat, &[u8])] = &[
        // f32, stereo, 48 kHz
        (
            AudioFormat::new(Encoding::PcmFloat, 2, 48_000),
            &[0x08, 0x01, 0x10, 0x02, 0x18, 0x80, 0xf7, 0x02],
        ),
        // f32, mono, 48 kHz
        (
            AudioFormat::new(Encoding::PcmFloat, 1, 48_000),
            &[0x08, 0x01, 0x10, 0x01, 0x18, 0x80, 0xf7, 0x02],
        ),
        // s16, stereo, 44.1 kHz
        (
            AudioFormat::new(Encoding::Pcm16Bit, 2, 44_100),
            &[0x08, 0x03, 0x10, 0x02, 0x18, 0xc4, 0xd8, 0x02],
        ),
        // s24, stereo, 48 kHz
        (
            AudioFormat::new(Encoding::Pcm24Bit, 2, 48_000),
            &[0x08, 0x04, 0x10, 0x02, 0x18, 0x80, 0xf7, 0x02],
        ),
        // s32, mono, 44.1 kHz
        (
            AudioFormat::new(Encoding::Pcm32Bit, 1, 44_100),
            &[0x08, 0x05, 0x10, 0x01, 0x18, 0xc4, 0xd8, 0x02],
        ),
        // s8, stereo, 48 kHz
        (
            AudioFormat::new(Encoding::Pcm8Bit, 2, 48_000),
            &[0x08, 0x02, 0x10, 0x02, 0x18, 0x80, 0xf7, 0x02],
        ),
        // invalid encoding is proto3 default and therefore omitted
        (
            AudioFormat::new(Encoding::Invalid, 1, 48_000),
            &[0x10, 0x01, 0x18, 0x80, 0xf7, 0x02],
        ),
    ];

    for (format, expected) in cases {
        assert_eq!(&format.encode_to_vec(), expected, "format: {format:?}");
    }
}

#[test]
fn audio_format_round_trips_through_protobuf() {
    for format in [
        AudioFormat::new(Encoding::PcmFloat, 2, 48_000),
        AudioFormat::new(Encoding::Pcm16Bit, 2, 44_100),
        AudioFormat::new(Encoding::Pcm24Bit, 1, 48_000),
        AudioFormat::new(Encoding::Pcm32Bit, 2, 96_000),
        AudioFormat::new(Encoding::Pcm8Bit, 1, 8_000),
        AudioFormat::new(Encoding::Invalid, 0, 0),
    ] {
        let bytes = format.encode_to_vec();
        assert_eq!(AudioFormat::decode(bytes.as_slice()).unwrap(), format);
    }
}

#[test]
fn block_align_follows_encoding_sample_width_and_channels() {
    let case = |encoding, channels| AudioFormat::new(encoding, channels, 48_000).block_align();
    assert_eq!(case(Encoding::PcmFloat, 2), 8);
    assert_eq!(case(Encoding::PcmFloat, 1), 4);
    assert_eq!(case(Encoding::Pcm8Bit, 2), 2);
    assert_eq!(case(Encoding::Pcm8Bit, 1), 1);
    assert_eq!(case(Encoding::Pcm16Bit, 2), 4);
    assert_eq!(case(Encoding::Pcm16Bit, 1), 2);
    assert_eq!(case(Encoding::Pcm24Bit, 2), 6);
    assert_eq!(case(Encoding::Pcm24Bit, 1), 3);
    assert_eq!(case(Encoding::Pcm32Bit, 2), 8);
    assert_eq!(case(Encoding::Pcm32Bit, 1), 4);
    assert_eq!(case(Encoding::Invalid, 2), 0);
    assert_eq!(case(Encoding::PcmFloat, 0), 0);
    assert_eq!(case(Encoding::PcmFloat, -1), 0);
}

#[test]
fn block_align_is_zero_for_unknown_encoding_value() {
    let mut format = AudioFormat::new(Encoding::PcmFloat, 2, 48_000);
    format.encoding = 99;
    assert_eq!(format.block_align(), 0);
}

#[test]
fn decode_command_reads_little_endian_u32() {
    assert_eq!(
        decode_command(&[0x01, 0x00, 0x00, 0x00]).unwrap(),
        Command::GetFormat
    );
    assert_eq!(
        decode_command(&[0x02, 0x00, 0x00, 0x00, 0xff]).unwrap(),
        Command::StartPlay
    );
    assert!(matches!(
        decode_command(&[0x00, 0x00, 0x00, 0x00]),
        Err(ProtocolError::InvalidCommand(0))
    ));
    assert!(matches!(
        decode_command(&[0x04, 0x00, 0x00, 0x00]),
        Err(ProtocolError::InvalidCommand(4))
    ));
    assert!(matches!(
        decode_command(&[0x01, 0x00]),
        Err(ProtocolError::UnexpectedEof {
            needed: 4,
            available: 2
        })
    ));
}

#[test]
fn get_format_response_has_command_size_and_protobuf_payload() {
    let format = AudioFormat::new(Encoding::PcmFloat, 2, 48_000);
    let frame = encode_get_format_response(&format);
    assert_eq!(
        frame,
        vec![
            0x01, 0x00, 0x00, 0x00, // GET_FORMAT
            0x08, 0x00, 0x00, 0x00, // size = 8
            0x08, 0x01, 0x10, 0x02, 0x18, 0x80, 0xf7, 0x02, // AudioFormat
        ]
    );

    let (decoded, consumed) = decode_get_format_response(&frame).unwrap();
    assert_eq!(decoded, format);
    assert_eq!(consumed, frame.len());

    // A decoder may be handed a stream buffer with more frames after it.
    let mut stream = frame.clone();
    stream.extend_from_slice(&encode_heartbeat());
    let (decoded, consumed) = decode_get_format_response(&stream).unwrap();
    assert_eq!(decoded, format);
    assert_eq!(consumed, frame.len());
}

#[test]
fn get_format_response_decode_rejects_bad_frames() {
    let frame = encode_get_format_response(&AudioFormat::new(Encoding::Pcm16Bit, 1, 44_100));

    let mut wrong_command = frame.clone();
    wrong_command[0] = 0x02;
    assert!(matches!(
        decode_get_format_response(&wrong_command),
        Err(ProtocolError::UnexpectedCommand {
            expected: Command::GetFormat,
            actual: Command::StartPlay
        })
    ));

    let mut truncated = frame.clone();
    truncated.truncate(frame.len() - 1);
    assert!(matches!(
        decode_get_format_response(&truncated),
        Err(ProtocolError::UnexpectedEof { .. })
    ));

    let mut lying_size = frame;
    lying_size[4..8].copy_from_slice(&99u32.to_le_bytes());
    assert!(matches!(
        decode_get_format_response(&lying_size),
        Err(ProtocolError::UnexpectedEof { .. })
    ));

    assert!(matches!(
        decode_get_format_response(&[0x01, 0x00, 0x00]),
        Err(ProtocolError::UnexpectedEof { needed: 8, .. })
    ));
}

#[test]
fn start_play_response_is_command_then_id() {
    let frame = encode_start_play_response(1);
    assert_eq!(frame, [0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00]);
    let (id, consumed) = decode_start_play_response(&frame).unwrap();
    assert_eq!(id, 1);
    assert_eq!(consumed, 8);

    let negative = encode_start_play_response(-7);
    assert_eq!(decode_start_play_response(&negative).unwrap().0, -7);

    let mut wrong_command = frame;
    wrong_command[0] = 0x03;
    assert!(matches!(
        decode_start_play_response(&wrong_command),
        Err(ProtocolError::UnexpectedCommand {
            expected: Command::StartPlay,
            actual: Command::Heartbeat
        })
    ));
    assert!(matches!(
        decode_start_play_response(&frame[..7]),
        Err(ProtocolError::UnexpectedEof { needed: 8, .. })
    ));
}

#[test]
fn heartbeat_is_a_bare_little_endian_command() {
    assert_eq!(encode_heartbeat(), [0x03, 0x00, 0x00, 0x00]);
    assert_eq!(decode_heartbeat(&encode_heartbeat()).unwrap(), 4);
    assert_eq!(
        decode_heartbeat(&[0x03, 0x00, 0x00, 0x00, 0xff]).unwrap(),
        4
    );
    assert!(matches!(
        decode_heartbeat(&[0x01, 0x00, 0x00, 0x00]),
        Err(ProtocolError::UnexpectedCommand {
            expected: Command::Heartbeat,
            actual: Command::GetFormat
        })
    ));
    assert!(matches!(
        decode_heartbeat(&[0x03, 0x00]),
        Err(ProtocolError::UnexpectedEof { .. })
    ));
}

#[test]
fn udp_bind_packet_is_a_little_endian_play_id() {
    assert_eq!(encode_udp_bind(1), [0x01, 0x00, 0x00, 0x00]);
    assert_eq!(encode_udp_bind(0x0102_0304), [0x04, 0x03, 0x02, 0x01]);
    assert_eq!(
        decode_udp_bind(&[0x04, 0x03, 0x02, 0x01]).unwrap(),
        0x0102_0304
    );
    assert_eq!(decode_udp_bind(&[0xfe, 0xff, 0xff, 0xff]).unwrap(), -2);
    assert!(matches!(
        decode_udp_bind(&[0x01, 0x00, 0x00]),
        Err(ProtocolError::UnexpectedEof {
            needed: 4,
            available: 3
        })
    ));
}

#[test]
fn mtu_constants_match_v034() {
    use audio_share_core::protocol::{IP_HEADER_BYTES, MAX_PAYLOAD_BYTES, MTU, UDP_HEADER_BYTES};
    assert_eq!(MTU, 1492);
    assert_eq!(IP_HEADER_BYTES, 20);
    assert_eq!(UDP_HEADER_BYTES, 8);
    assert_eq!(MAX_PAYLOAD_BYTES, 1464);
}

#[test]
fn segment_payload_aligns_down_to_block_align() {
    use audio_share_core::protocol::max_segment_payload;
    assert_eq!(max_segment_payload(1), 1464);
    assert_eq!(max_segment_payload(2), 1464);
    assert_eq!(max_segment_payload(3), 1464);
    assert_eq!(max_segment_payload(4), 1464);
    assert_eq!(max_segment_payload(5), 1460);
    assert_eq!(max_segment_payload(6), 1464);
    assert_eq!(max_segment_payload(8), 1464);
    assert_eq!(max_segment_payload(16), 1456);
    assert_eq!(max_segment_payload(32), 1440);
    assert_eq!(max_segment_payload(1464), 1464);
}

#[test]
fn invalid_block_align_produces_no_segments_instead_of_panicking() {
    use audio_share_core::protocol::{max_segment_payload, segment_audio};
    for block_align in [0, 1465, 2048, usize::MAX] {
        assert_eq!(max_segment_payload(block_align), 0);
        assert!(
            segment_audio(&[0u8; 100], block_align).is_empty(),
            "block_align={block_align}"
        );
    }
}

#[test]
fn segment_audio_uses_aligned_chunks_and_preserves_the_remainder() {
    use audio_share_core::protocol::{max_segment_payload, segment_audio};

    let data: Vec<u8> = (0..5000).map(|i| (i % 251) as u8).collect();
    for block_align in [1usize, 2, 3, 4, 6, 8, 32] {
        let max = max_segment_payload(block_align);
        let segments = segment_audio(&data, block_align);
        assert_eq!(segments.concat(), data, "block_align={block_align}");
        assert!(segments.iter().all(|segment| !segment.is_empty()));

        let mut rebuilt = Vec::new();
        for (index, segment) in segments.iter().enumerate() {
            if index + 1 < segments.len() {
                assert_eq!(
                    segment.len(),
                    max,
                    "only the final segment may be short (block_align={block_align})"
                );
            }
            assert!(
                segment.len() <= max,
                "segment exceeds aligned payload (block_align={block_align})"
            );
            rebuilt.extend_from_slice(segment);
        }
        assert_eq!(rebuilt, data);
    }
}

#[test]
fn segment_audio_keeps_short_and_empty_buffers_intact() {
    use audio_share_core::protocol::segment_audio;

    let short = [1u8, 2, 3];
    let segments = segment_audio(&short, 8);
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0], &short);

    assert!(segment_audio(&[], 8).is_empty());
}
