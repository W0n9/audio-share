//! Media-plane golden vectors through the public packetizer seam (spec §7.1,
//! §7.3). Expected bytes come from `fixtures/wire-protocol-v2/media/*.json`,
//! never from our own encoder.

use v2_transport::media::{MediaHeader, MediaPacketizer, FLAG_CONFIG_CHANGE, FLAG_STREAM_START};

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn fixture(id: &str) -> serde_json::Value {
    let path = format!(
        "{}/../../fixtures/wire-protocol-v2/media/{id}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("fixture json")
}

fn datagrams(value: &serde_json::Value) -> Vec<(String, usize)> {
    value["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .filter(|step| step["kind"] == "datagram")
        .map(|step| {
            (
                step["datagram_hex"].as_str().expect("hex").to_owned(),
                step["length"].as_u64().expect("length") as usize,
            )
        })
        .collect()
}

/// The first datagram of a fresh instance is byte-identical to the golden
/// vector: STREAM_START, sequence 0, timestamp 0, payload untouched.
#[test]
fn header_happy_path_first_datagram_matches_golden_bytes() {
    let expected = datagrams(&fixture("header-happy-path"));
    let (hex, length) = &expected[0];
    let expected_bytes = unhex(hex);
    assert_eq!(expected_bytes.len(), *length);

    let payload = expected_bytes[24..].to_vec();
    let mut packetizer = MediaPacketizer::new(1, 1, 8);
    let datagrams = packetizer
        .packetize(&payload, 88)
        .expect("one datagram at mds 88");
    assert_eq!(datagrams.len(), 1);
    assert_eq!(datagrams[0], expected_bytes);
}

/// Frame alignment and per-send limits: mds 1165 → 142 whole frames
/// (1136 bytes), never the 5-byte remainder as a partial frame; the next
/// datagram continues at timestamp 142 with sequence 1.
#[test]
fn reduced_mtu_chunks_on_whole_frames_and_continues_the_clock() {
    let expected = datagrams(&fixture("reduced-mtu-alignment-floor"));
    let first = unhex(&expected[0].0);
    let second = unhex(&expected[1].0);
    assert_eq!(first.len(), expected[0].1);
    assert_eq!(second.len(), expected[1].1);

    // The fixture's payload is a synthetic ramp; the independent oracle is the
    // header + the chunk boundaries, so feed the concatenated payloads back.
    let mut pcm = first[24..].to_vec();
    pcm.extend_from_slice(&second[24..]);

    let mut packetizer = MediaPacketizer::new(6, 1, 8);
    let datagrams = packetizer.packetize(&pcm, 1165).expect("two datagrams");
    assert_eq!(datagrams.len(), 2, "142 + 137 frames at 8 bytes each");
    assert_eq!(datagrams[0], first);
    assert_eq!(datagrams[1][..24], second[..24], "header of datagram 2");
}

/// A full-size datagram at the reported send-time limit: 1192 − 24 = 1168
/// payload bytes = 146 frames; never a cached 1464/1500 assumption.
#[test]
fn reduced_mtu_full_size_datagram_is_sized_from_the_current_limit() {
    let expected = datagrams(&fixture("reduced-mtu-full-size"));
    let bytes = unhex(&expected[0].0);
    let payload = bytes[24..].to_vec();
    let mut packetizer = MediaPacketizer::new(5, 1, 8);
    let datagrams = packetizer
        .packetize(&payload, 1192)
        .expect("single full-size datagram");
    assert_eq!(datagrams[0], bytes);
}

/// §7.1: CONFIG_CHANGE rides the first datagram of a greater generation, and
/// the generation field is authoritative.
#[test]
fn config_change_flag_marks_the_first_datagram_of_a_new_generation() {
    let fixture = fixture("config-change-mid-stream");
    let steps: Vec<_> = fixture["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|step| step["kind"] == "datagram")
        .collect();
    let gen2 = unhex(steps[1]["datagram_hex"].as_str().unwrap());
    assert_eq!(
        MediaHeader::parse(&gen2).unwrap().0.flags & FLAG_CONFIG_CHANGE,
        FLAG_CONFIG_CHANGE,
        "fixture pins CONFIG_CHANGE on the first gen-2 datagram"
    );

    let gen1 = unhex(steps[0]["datagram_hex"].as_str().unwrap());
    // The instance started earlier: consume STREAM_START, then line the
    // packetizer up with the fixture's gen-1 coordinates (seq 41, frame 1200).
    let mut packetizer = MediaPacketizer::new(1, 1, 8);
    packetizer.packetize(&[0u8; 8], 264).unwrap();
    packetizer.seek(41, 1200).expect("seek");
    let unchanged = packetizer.packetize(&gen1[24..], 264).unwrap();
    assert_eq!(unchanged[0], gen1, "identical configuration keeps gen 1");

    packetizer
        .set_generation(2)
        .expect("decode-affecting change");
    let switched = packetizer.packetize(&gen2[24..], 264).unwrap();
    assert_eq!(switched[0], gen2, "first gen-2 datagram is byte-exact");
}

/// Sequence wraps mod 2^32 and the timestamp keeps advancing by frames.
#[test]
fn sequence_wraps_mod_u32_and_timestamps_advance_by_frames() {
    let mut packetizer = MediaPacketizer::new(9, 1, 8);
    packetizer.seek(0xffff_ffff, 7).expect("seek");
    let datagrams = packetizer
        .packetize(&[0u8; 32], 40)
        .expect("two datagrams");
    assert_eq!(
        MediaHeader::parse(&datagrams[0]).unwrap().0.sequence,
        0xffff_ffff
    );
    let second = MediaHeader::parse(&datagrams[1]).unwrap().0;
    assert_eq!(second.sequence, 0, "wrap mod 2^32");
    assert_eq!(second.timestamp, 9, "two frames per datagram from 7");
}

/// §7.3: a limit below header + one frame cannot carry media → the sender
/// must fail explicitly (602 at the session layer), never fragment.
#[test]
fn a_limit_below_one_frame_is_an_explicit_error() {
    let mut packetizer = MediaPacketizer::new(1, 1, 8);
    assert!(packetizer.packetize(&[0u8; 32], 24 + 7).is_err());
}

/// §7.1: reserved flag bits are sent as zero; STREAM_START is set only on the
/// first datagram of an instance.
#[test]
fn flags_follow_the_spec_table() {
    let mut packetizer = MediaPacketizer::new(4, 2, 8);
    let datagrams = packetizer.packetize(&[1u8; 16], 32).unwrap();
    let first = MediaHeader::parse(&datagrams[0]).unwrap().0;
    let second = MediaHeader::parse(&datagrams[1]).unwrap().0;
    assert_eq!(first.flags, FLAG_STREAM_START);
    assert_eq!(second.flags, 0);
    assert_eq!(first.flags & !(FLAG_STREAM_START | FLAG_CONFIG_CHANGE), 0);
}

/// Empty PCM produces no datagram (never an empty media payload).
#[test]
fn empty_pcm_produces_nothing() {
    let mut packetizer = MediaPacketizer::new(1, 1, 8);
    assert!(packetizer.packetize(&[], 1200).unwrap().is_empty());
}

/// A payload whose length is not a whole number of frames is a caller bug
/// (the capture frame geometry must divide): report it, never emit a
/// misaligned datagram.
#[test]
fn misaligned_pcm_is_rejected() {
    let mut packetizer = MediaPacketizer::new(1, 1, 8);
    assert!(packetizer.packetize(&[0u8; 13], 1200).is_err());
}
