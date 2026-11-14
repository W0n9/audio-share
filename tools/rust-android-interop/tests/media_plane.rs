//! Ticket 15: Rust Server → real Android Client media interop.
//!
//! The Server's production `MediaPump` (capture → paced QUIC DATAGRAMs) is
//! driven with capture buffers built from the frozen golden-vector PCM, and
//! every resulting datagram is handed to the Android Client's real
//! `MediaSession`, which owns header parse / dedup / generation protection /
//! ring delivery / §9 diagnostics. Neither end's own encoder is the oracle:
//! `fixtures/wire-protocol-v2/media/*.json` plus the normative §7 header
//! layout are.

use std::time::Instant;

use audio_share_client_v2_native::media_session::{DatagramOutcome, MediaSession, PlaybackReport};
use audio_share_client_v2_native::proto::{Configuration, PcmParameters};
use audio_share_client_v2_native::ring::MediaRing;
use rust_android_interop::{media_datagrams, media_fixture, unhex};
use v2_transport::media::{MediaHeader, MediaPacketizer, HEADER_SIZE};
use v2_transport::media_pump::MediaPump;
use v2_transport::media_source::{MediaChunk, StreamFormat, StreamState};

const FRAME_BYTES: usize = 8; // F32LE stereo
const GENERATION: u32 = 1;

fn configuration() -> Configuration {
    Configuration {
        config_generation: GENERATION,
        codec: 1,
        pcm: Some(PcmParameters {
            sample_rate: 48_000,
            channels: 2,
            sample_format: 1,
        }),
    }
}

/// The instance id and the first datagram's capture-clock position as pinned
/// by the vector itself (never from our own encoder).
fn fixture_coordinates(id: &str) -> (u32, u64) {
    let fixture = media_fixture(id);
    let step = &fixture["steps"][0];
    (
        step["decoded"]["stream_instance_id"].as_u64().expect("instance") as u32,
        step["decoded"]["timestamp"].as_u64().expect("timestamp"),
    )
}

fn pump(instance: u32) -> MediaPump {
    MediaPump::new(StreamState {
        stream_instance_id: instance,
        config_generation: GENERATION,
        next_frame: 0,
        format: StreamFormat {
            sample_rate: 48_000,
            channels: 2,
            bytes_per_sample: 4,
        },
    })
}

/// Reassemble the golden payloads into one capture buffer, let the REAL Server
/// pump packetize them at the given send-time limit, and assert the resulting
/// bytes are the frozen vector's bytes.
fn pump_golden_datagrams(
    id: &str,
    max_datagram_size: usize,
) -> (u32, Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let (instance, start_frame) = fixture_coordinates(id);
    let fixture = media_fixture(id);
    let expected: Vec<Vec<u8>> = media_datagrams(&fixture)
        .iter()
        .map(|(hex, length)| {
            let bytes = unhex(hex);
            assert_eq!(bytes.len(), *length, "fixture length");
            bytes
        })
        .collect();
    let pcm: Vec<u8> = expected
        .iter()
        .flat_map(|datagram| datagram[HEADER_SIZE..].to_vec())
        .collect();
    let frames = (pcm.len() / FRAME_BYTES) as u64;
    let mut pump = pump(instance);
    pump.accept(MediaChunk {
        pcm,
        start_frame,
        end_frame: start_frame + frames,
        stream_instance_id: instance,
        config_generation: GENERATION,
    })
    .expect("capture chunk accepted");
    let produced = pump
        .take_sendable(Instant::now(), max_datagram_size, expected.len())
        .expect("packetize");
    assert_eq!(produced.len(), expected.len(), "one datagram per fixture step");
    assert_eq!(produced, expected, "Server bytes are the golden vector: {id}");
    (instance, expected, produced)
}

fn android_session(instance: u32, anchor: u64) -> (Vec<u8>, MediaSession) {
    let mut backing = vec![0u8; 4096];
    let ring = unsafe { MediaRing::attach(backing.as_mut_ptr(), backing.len()) };
    let media = MediaSession::start(instance, GENERATION, anchor, &configuration(), Some(ring))
        .expect("Android accepts the published F32LE stereo geometry");
    (backing, media)
}

/// §7.1/§7.2/§7.3: the Server's real packetizer emits the frozen happy-path
/// bytes, and the Android Client's own header parser reads back the same
/// instance/generation/sequence/timestamp/flags and delivers every frame.
#[test]
fn server_packetizer_bytes_are_accepted_by_the_android_receiver() {
    let (instance, _expected, produced) = pump_golden_datagrams("header-happy-path", 88);
    assert_eq!(instance, 1);

    let (_backing, mut media) = android_session(instance, 0);
    let mut previous: Option<(u32, u64, u64)> = None;
    for (index, server_datagram) in produced.iter().enumerate() {
        // Independent normative oracle: decode the 24-byte BE header by hand.
        let header = MediaHeader::parse(server_datagram).expect("valid header");
        assert_eq!(header.0.stream_instance_id, instance);
        assert_eq!(header.0.config_generation, GENERATION);
        assert_eq!(header.0.sequence as usize, index);
        if index == 0 {
            assert_eq!(header.0.flags & 0x1, 0x1, "STREAM_START on sequence 0");
        } else {
            assert_eq!(header.0.flags & 0x1, 0, "STREAM_START once per instance");
        }
        let frames = (header.1.len() / FRAME_BYTES) as u64;
        if let Some((prev_seq, prev_ts, prev_frames)) = previous {
            assert_eq!(header.0.sequence, prev_seq + 1);
            assert_eq!(
                header.0.timestamp,
                prev_ts + prev_frames,
                "§7.2: ts(next) = ts(prev) + frames(prev)"
            );
        }
        previous = Some((header.0.sequence, header.0.timestamp, frames));

        assert_eq!(
            media.on_datagram(0, server_datagram),
            DatagramOutcome::Delivered,
            "Android delivers datagram {index}"
        );
    }
    assert_eq!(media.delivered_frames(), 16, "two datagrams × 8 frames");
    let diagnostics = media.diagnostics(0);
    assert_eq!(diagnostics.cumulative_received, 2);
    assert_eq!(diagnostics.extended_highest_sequence, 1);
    assert_eq!(diagnostics.cumulative_lost, 0);
    assert_eq!(diagnostics.cumulative_duplicates, 0);
}

/// §7.3 interop floor: a Server datagram carrying 1176 payload bytes (147
/// whole frames) must be accepted by the Android receiver; it never assumes
/// a fixed local maximum.
#[test]
fn server_interop_floor_datagram_is_delivered_by_android() {
    let (instance, _, produced) = pump_golden_datagrams("interop-floor-1176", 1200);
    assert_eq!(instance, 9);
    assert_eq!(produced[0].len(), 1200);

    let (_backing, mut media) = android_session(instance, 0);
    assert_eq!(media.on_datagram(0, &produced[0]), DatagramOutcome::Delivered);
    assert_eq!(media.delivered_frames(), 147);
    assert_eq!(media.diagnostics(0).cumulative_received, 1);
}

/// §7.3 D2: a reduced send-time limit produces smaller, still frame-aligned
/// datagrams that the Android receiver accepts without a fixed-MTU assumption.
#[test]
fn reduced_path_limit_still_interoperates_with_android() {
    let (instance, _, produced) = pump_golden_datagrams("reduced-mtu-alignment-floor", 1165);
    assert_eq!(instance, 6);

    let (_backing, mut media) = android_session(instance, 0);
    for datagram in &produced {
        assert!(datagram.len() <= 1165, "every datagram respects the current limit");
        assert_eq!((datagram.len() - HEADER_SIZE) % FRAME_BYTES, 0, "whole frames");
        assert_eq!(media.on_datagram(0, datagram), DatagramOutcome::Delivered);
    }
    assert_eq!(media.delivered_frames(), 284, "142 + 142 frames");
    assert_eq!(media.diagnostics(0).cumulative_received, 2);
}

/// §7.5/§7.6: Server generation changes are protected by the Android
/// receiver — a stale-generation datagram is never decoded with the old
/// configuration, and a newer one is held until the configuration arrives.
#[test]
fn android_generation_protection_matches_server_mid_stream_change() {
    let golden = media_datagrams(&media_fixture("config-change-mid-stream"));
    // Steps 2..5: gen1(seq41) → gen2(seq42, CONFIG_CHANGE) → stale gen1(seq43) → gen2(seq44).
    let gen1 = unhex(&golden[0].0);
    let gen2_first = unhex(&golden[1].0);
    assert_eq!(
        MediaHeader::parse(&gen2_first).unwrap().0.flags & 0x2,
        0x2,
        "fixture pins CONFIG_CHANGE"
    );

    let mut server_pump = pump(1);
    server_pump
        .accept(MediaChunk {
            pcm: gen1[HEADER_SIZE..].to_vec(),
            start_frame: 1_200,
            end_frame: 1_230,
            stream_instance_id: 1,
            config_generation: GENERATION,
        })
        .expect("gen1 accepted");
    server_pump
        .take_sendable(Instant::now(), 264, 1)
        .expect("gen1 sent");
    // The Server advances the generation (CONFIG_CHANGE on the next datagram)
    // and refuses capture buffers of the old generation.
    server_pump.set_generation(2).expect("decode-affecting change");

    let (_backing, mut media) = android_session(1, 1_200);
    // A datagram of the future generation is held, never decoded.
    assert_eq!(
        media.on_datagram(0, &gen2_first),
        DatagramOutcome::HeldUnknownConfig
    );
    assert!(media.holding_unknown_config());
    assert_eq!(media.delivered_frames(), 0);

    // The reliable ConfigurationPublished releases it.
    let gen2_configuration = Configuration {
        config_generation: 2,
        ..configuration()
    };
    media.apply_configuration(&gen2_configuration);
    assert!(!media.holding_unknown_config());
    assert_eq!(media.delivered_frames(), 30, "held gen-2 frames delivered");

    // A late gen-1 datagram is dropped, never decoded with gen 2.
    let stale = unhex(&golden[2].0);
    assert_eq!(media.on_datagram(0, &stale), DatagramOutcome::StaleGeneration);
    assert_eq!(media.delivered_frames(), 30);
    let diagnostics = media.diagnostics(0);
    assert_eq!(diagnostics.config_generation, 2);
    assert_eq!(diagnostics.cumulative_unknown_config_drops, 1);
}

/// §7.1/§7.5: reserved flag bits are ignored by the Android receiver, and
/// malformed media produced by no Server path is dropped instead of crashing.
#[test]
fn android_receiver_ignores_reserved_bits_and_drops_impossible_media() {
    for id in ["flags-reserved-ignored", "flags-reserved-bit31-ignored"] {
        let (instance, start_frame) = fixture_coordinates(id);
        let bytes = unhex(&media_datagrams(&media_fixture(id))[0].0);
        let (_backing, mut media) = android_session(instance, start_frame);
        assert_eq!(
            media.on_datagram(0, &bytes),
            DatagramOutcome::Delivered,
            "{id}: reserved bits are not malformed"
        );
    }

    let malformed = unhex(&media_datagrams(&media_fixture("malformed-drop"))[0].0);
    let (_backing, mut media) = android_session(1, 0);
    assert_eq!(
        media.on_datagram(0, &malformed),
        DatagramOutcome::Malformed,
        "zero instance id is impossible (§7.1)"
    );
    assert_eq!(media.delivered_frames(), 0);
    assert_eq!(media.diagnostics(0).cumulative_received, 0);
}

/// §4.4/§9: a late Subscriber's anchor is the mid-stream capture position;
/// the Android receiver reports `submitted_position` relative to it using
/// only what the platform `AudioTrack` actually accepted.
#[test]
fn late_subscriber_anchor_and_real_playback_position_round_trip() {
    const ANCHOR: u64 = 96_000;
    let mut server_pump = pump(1);
    let pcm = vec![0x5au8; 960]; // 120 frames
    server_pump
        .accept(MediaChunk {
            pcm,
            start_frame: ANCHOR,
            end_frame: ANCHOR + 120,
            stream_instance_id: 1,
            config_generation: GENERATION,
        })
        .expect("accepted");
    let datagrams = server_pump
        .take_sendable(Instant::now(), 1_192, 8)
        .expect("send");
    let first = MediaHeader::parse(&datagrams[0]).unwrap().0;
    assert_eq!(first.timestamp, ANCHOR, "Server starts at the announced anchor");
    assert_eq!(first.sequence, 0);

    let (_backing, mut media) = android_session(1, ANCHOR);
    for datagram in &datagrams {
        assert_eq!(media.on_datagram(0, datagram), DatagramOutcome::Delivered);
    }
    assert_eq!(media.anchor(), ANCHOR);
    assert_eq!(
        media.submitted_position(),
        ANCHOR,
        "receiving/ring delivery is not playback evidence"
    );

    // The AudioTrack accepted 64 whole frames = 512 bytes.
    media.record_playback(PlaybackReport {
        submitted_bytes: 64 * FRAME_BYTES as u64,
        underruns: 1,
        buffer_current_ms: 25,
    });
    assert_eq!(media.submitted_position(), ANCHOR + 64);
    let diagnostics = media.diagnostics(0);
    assert_eq!(diagnostics.stream_instance_id, 1);
    assert_eq!(diagnostics.submitted_position, ANCHOR + 64);
    assert_eq!(diagnostics.underruns, 1);
    assert_eq!(diagnostics.buffer_current_ms, 25);
    // M5-only interventions are explicit zeros, never invented.
    assert_eq!(diagnostics.cumulative_late_drops, 0);
    assert_eq!(diagnostics.cumulative_concealments, 0);
    assert_eq!(diagnostics.jitter_microseconds, 0);
    assert_eq!(diagnostics.buffer_target_ms, 0);
}

/// §7.5/RFC 3550 A.1: the Server's sequence wrap is folded by the Android
/// receiver's extended sequence without a false loss burst.
///
/// NOTE (frozen-fixture inconsistency, recorded rather than "fixed"): the
/// `media.sequence-wrap` vector's step 2 carries timestamp 3_840_080, i.e.
/// step 1 + 80 *payload bytes*, while §7.2 and the vector's own assertion
/// ("3840000 + 10 = 3840080") say the clock advances by *sample frames* (10).
/// Every other media vector (header-happy-path, reduced-mtu-*, config-change)
/// is frame-based. This harness therefore uses the golden BYTES as the
/// independent wire input for the A.1 folding assertion, and verifies the
/// real Server packetizer against the normative frame-based clock. The frozen
/// fixture is left untouched (compatibility asset); see evidence/ticket-15.
#[test]
fn server_sequence_wrap_is_folded_by_android() {
    let (instance, start_frame) = fixture_coordinates("sequence-wrap");
    let golden = media_datagrams(&media_fixture("sequence-wrap"));
    let first = unhex(&golden[0].0);
    let second = unhex(&golden[1].0);
    let first_header = MediaHeader::parse(&first).unwrap().0;
    let second_header = MediaHeader::parse(&second).unwrap().0;
    assert_eq!(first_header.sequence, 0xffff_ffff);
    assert_eq!(first_header.timestamp, 3_840_000);
    assert_eq!(second_header.sequence, 0);
    assert_eq!(first_header.flags, 0);
    assert_eq!(second_header.flags, 0);

    // (1) Frozen golden bytes → the REAL Android receiver: cycle folding.
    let (_backing, mut media) = android_session(instance, start_frame);
    assert_eq!(media.on_datagram(0, &first), DatagramOutcome::Delivered);
    assert_eq!(media.on_datagram(0, &second), DatagramOutcome::Delivered);
    let diagnostics = media.diagnostics(0);
    assert_eq!(diagnostics.cumulative_received, 2);
    assert_eq!(diagnostics.cumulative_duplicates, 0);
    assert_eq!(diagnostics.cumulative_lost, 0, "wrap is in-order, not a loss burst");
    assert_eq!(
        diagnostics.extended_highest_sequence,
        0x1_0000_0000,
        "RFC 3550 A.1 folds the cycle"
    );

    // (2) The real Server packetizer at the same instance/sequence. Consume
    // one datagram first so STREAM_START (a per-instance hint) is not set on
    // this mid-stream vector; both golden headers carry flags 0.
    let mut packetizer = MediaPacketizer::new(instance, GENERATION, FRAME_BYTES);
    packetizer.packetize(&[0u8; 8], 104).expect("consume STREAM_START");
    packetizer.seek(0xffff_ffff, start_frame).expect("seek");
    let produced = packetizer.packetize(&[0u8; 160], 104).expect("two datagrams");
    assert_eq!(produced.len(), 2);
    assert_eq!(
        produced[0][..HEADER_SIZE],
        first[..HEADER_SIZE],
        "first wrap datagram is byte-exact with the golden vector"
    );
    let second_server = MediaHeader::parse(&produced[1]).unwrap().0;
    assert_eq!(second_server.sequence, 0);
    assert_eq!(second_server.flags, 0);
    assert_eq!(
        second_server.timestamp,
        first_header.timestamp + 10,
        "§7.2: timestamp advances by the 10 payload frames, not 80 bytes"
    );

    // (3) Server-produced wrap → the same Android receiver folds it too.
    let (_backing, mut media) = android_session(instance, start_frame);
    assert_eq!(media.on_datagram(0, &produced[0]), DatagramOutcome::Delivered);
    assert_eq!(media.on_datagram(0, &produced[1]), DatagramOutcome::Delivered);
    let diagnostics = media.diagnostics(0);
    assert_eq!(diagnostics.extended_highest_sequence, 0x1_0000_0000);
    assert_eq!(diagnostics.cumulative_lost, 0);
}
