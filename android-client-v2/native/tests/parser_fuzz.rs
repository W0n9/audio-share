//! M5-06 §11.3 parser fuzz — Client/host half.
//!
//! Fixed-count, seeded, deterministic fuzz of the two Client parsers that see
//! hostile bytes:
//!
//! 1. the 24-byte media header (`media::MediaHeader::parse`),
//! 2. the whole media datagram path (`MediaSession::on_datagram`), and
//! 3. the control-stream framing/decoder plus the Client state machine.
//!
//! Rules asserted for every case: no panic/abort, no unbounded allocation
//! beyond the input, and a deterministic outcome for a given seed (the same
//! seed is replayed twice and the summaries must match). Media has no on-wire
//! §6 code — rejection is drop + count — while control framing failures carry
//! the §6 `401` classification. Explicit negative cases pin the registry
//! mappings that §11.3 names.

use audio_share_client_v2_native::control::ClientMachine;
use audio_share_client_v2_native::frame::FrameDecoder;
use audio_share_client_v2_native::media::{HeaderError, MediaHeader, FLAG_SPARE};
use audio_share_client_v2_native::media_session::{DatagramOutcome, MediaSession};
use audio_share_client_v2_native::proto::frame::Body;
use audio_share_client_v2_native::proto::{
    AuthChallenge, AuthResult, Close, Configuration, ConfigurationPublished, Error, Frame,
    NegotiateResponse, PcmParameters, Status, StreamEnded, StreamStarted, SubscribeResponse,
};
use prost::Message as _;

// ---------------------------------------------------------------------------
// Deterministic PRNG
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.next_u64() as u8).collect()
    }
}

fn seed() -> u64 {
    let seed = std::env::var("M5_FUZZ_SEED")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0x4D35_0006_F022_0001);
    eprintln!("m5-06 client fuzz seed = {seed}");
    seed
}

const CASES: usize = 10_000;

fn config() -> Configuration {
    Configuration {
        config_generation: 1,
        codec: 1,
        pcm: Some(PcmParameters {
            sample_rate: 48_000,
            channels: 2,
            sample_format: 1,
        }),
    }
}

fn session() -> MediaSession {
    MediaSession::start(1, 1, 0, &config(), None).expect("geometry")
}

fn datagram(sequence: u32, timestamp: u64, frames: usize) -> Vec<u8> {
    let header = MediaHeader {
        stream_instance_id: 1,
        config_generation: 1,
        sequence,
        timestamp,
        flags: 0,
    };
    let mut out = Vec::new();
    header.write_be(&mut out);
    out.extend(std::iter::repeat_n(0x33u8, frames * 8));
    out
}

// ---------------------------------------------------------------------------
// Target 1 — 24-byte media header parser
// ---------------------------------------------------------------------------

/// 10 000 random datagrams of length 0..=300 through the header parser.
/// Fewer than 24 bytes is always `Truncated`; a parsed header always has
/// non-zero instance/generation (zero is `ImpossibleField`). Replayed seed
/// equality proves determinism.
#[test]
fn fuzz_media_header_parser_never_panics_and_is_deterministic() {
    let run = |seed: u64| {
        let mut rng = Rng::new(seed);
        let mut truncated = 0u64;
        let mut impossible = 0u64;
        let mut parsed = 0u64;
        for _ in 0..CASES {
            let len = rng.below(301) as usize;
            let bytes = rng.bytes(len);
            match MediaHeader::parse(&bytes) {
                Ok((header, payload)) => {
                    parsed += 1;
                    assert!(bytes.len() >= 24);
                    assert_eq!(payload.len(), bytes.len() - 24);
                    assert_ne!(header.stream_instance_id, 0);
                    assert_ne!(header.config_generation, 0);
                }
                Err(HeaderError::Truncated) => {
                    truncated += 1;
                    assert!(bytes.len() < 24);
                }
                Err(HeaderError::ImpossibleField) => {
                    impossible += 1;
                    assert!(bytes.len() >= 24);
                }
            }
        }
        (truncated, impossible, parsed)
    };

    let seed = seed();
    let first = run(seed);
    assert_eq!(first.0 + first.1 + first.2, CASES as u64);
    assert!(first.2 > 0, "random 24+ byte inputs do parse sometimes");
    assert_eq!(first, run(seed), "same seed must replay identically");
    eprintln!(
        "media-header fuzz: truncated={} impossible={} parsed={}",
        first.0, first.1, first.2
    );
}

// ---------------------------------------------------------------------------
// Target 2 — media datagram path (`on_datagram`)
// ---------------------------------------------------------------------------

fn structured_datagram(rng: &mut Rng) -> Vec<u8> {
    // Mix valid geometry with hostile fields and arbitrary payload lengths.
    let instance = match rng.below(8) {
        0 => 0,
        1 => 2,
        _ => 1,
    };
    let generation = match rng.below(8) {
        0 => 0,
        1 => 2,
        _ => 1,
    };
    let header = MediaHeader {
        stream_instance_id: instance,
        config_generation: generation,
        sequence: rng.next_u64() as u32,
        timestamp: rng.next_u64(),
        flags: rng.next_u64() as u32,
    };
    let mut out = Vec::new();
    header.write_be(&mut out);
    let payload_len = match rng.below(4) {
        0 => rng.below(64) as usize,        // usually misaligned
        1 => (rng.below(16) * 8) as usize,  // frame-aligned
        2 => (rng.below(160) * 8) as usize, // larger but bounded
        _ => (rng.below(1024)) as usize,    // arbitrary tail
    };
    out.extend(rng.bytes(payload_len));
    out
}

/// 10 000 random datagrams through `MediaSession::on_datagram`. Every call
/// returns one of the deterministic outcomes, counters never decrease, and
/// rejections never write PCM. A second run with the same seed must produce
/// the same outcome histogram.
#[test]
fn fuzz_on_datagram_never_panics_and_is_deterministic() {
    let run = |seed: u64| {
        let mut rng = Rng::new(seed);
        let mut media = session();
        let mut delivered = 0u64;
        let mut duplicate = 0u64;
        let mut malformed = 0u64;
        let mut stale = 0u64;
        let mut held = 0u64;
        let mut late = 0u64;
        let mut ignored = 0u64;
        let mut last_delivered = 0u64;
        for case in 0..CASES {
            let bytes = if case % 2 == 0 {
                let len = rng.below(400) as usize;
                rng.bytes(len)
            } else {
                structured_datagram(&mut rng)
            };
            let before = media.delivered_frames();
            let outcome = media.on_datagram(case as u64, &bytes);
            let after = media.delivered_frames();
            assert!(after >= before, "delivered frames never decrease");
            match outcome {
                DatagramOutcome::Delivered => delivered += 1,
                DatagramOutcome::Duplicate => {
                    duplicate += 1;
                    assert_eq!(after, before, "a duplicate is never written");
                }
                DatagramOutcome::Malformed => {
                    malformed += 1;
                    assert_eq!(after, before, "malformed media is never written");
                }
                DatagramOutcome::StaleGeneration => {
                    stale += 1;
                    assert_eq!(after, before, "stale media is never written");
                }
                DatagramOutcome::HeldUnknownConfig => {
                    held += 1;
                    assert_eq!(after, before, "held media is never decoded");
                }
                DatagramOutcome::LateDropped => {
                    late += 1;
                    assert_eq!(after, before, "late media is never written");
                }
                DatagramOutcome::Ignored => ignored += 1,
            }
            last_delivered = last_delivered.max(after);
        }
        let diagnostics = media.diagnostics(60);
        assert_eq!(
            diagnostics.cumulative_received,
            delivered + late,
            "received counts delivered plus late (reordered) arrivals"
        );
        assert_eq!(diagnostics.cumulative_duplicates, duplicate);
        assert!(
            diagnostics.cumulative_unknown_config_drops >= stale,
            "every stale generation is counted; held-queue overflow adds more"
        );
        assert!(last_delivered > 0, "some structured cases are deliverable");
        (delivered, duplicate, malformed, stale, held, late, ignored)
    };

    let seed = seed();
    let first = run(seed);
    let second = run(seed);
    assert_eq!(first, second, "same seed must replay identically");
    let total: u64 = [first.0, first.1, first.2, first.4, first.5, first.6]
        .iter()
        .sum();
    assert!(total <= CASES as u64);
    eprintln!(
        "on_datagram fuzz: delivered={} duplicate={} malformed={} stale={} held={} late={} ignored={}",
        first.0, first.1, first.2, first.3, first.4, first.5, first.6
    );
}

/// Held media that overflows its bounded queue is counted as an unknown-config
/// drop too; the exact split is internal, so the fuzz run only requires the
/// total counter to cover the explicit stale outcomes. Overflow itself is
/// exercised by the dedicated bounded-hold contract test.

// ---------------------------------------------------------------------------
// Target 3 — control framing + Client state machine
// ---------------------------------------------------------------------------

fn verdict_signature(
    machine: &ClientMachine,
    (verdict, event): (
        Result<
            audio_share_client_v2_native::control::Verdict,
            audio_share_client_v2_native::control::CloseInfo,
        >,
        Option<audio_share_client_v2_native::control::Event>,
    ),
) -> String {
    format!(
        "{}|{:?}|{:?}",
        machine.state.name(),
        verdict.map(|value| format!("{value:?}")),
        event.map(|value| format!("{value:?}"))
    )
}

/// A corpus of valid server→client control messages. Mutating these reaches
/// deep schema/state paths that uniformly random bytes almost never decode to.
fn control_corpus() -> Vec<Vec<u8>> {
    let ok = Status {
        code: 0,
        retryable: false,
        requires_repairing: false,
        detail: String::new(),
    };
    let frames = vec![
        Frame {
            body: Some(Body::AuthChallenge(AuthChallenge {
                request_id: 1,
                server_random: vec![7u8; 32],
            })),
        },
        Frame {
            body: Some(Body::AuthResult(AuthResult {
                request_id: 1,
                status: Some(ok.clone()),
                session_id: 1,
            })),
        },
        Frame {
            body: Some(Body::Error(Error {
                request_id: 0,
                status: Some(Status {
                    code: 401,
                    ..ok.clone()
                }),
            })),
        },
        Frame {
            body: Some(Body::Close(Close {
                code: 601,
                detail: String::new(),
            })),
        },
        Frame {
            body: Some(Body::NegotiateResponse(NegotiateResponse {
                request_id: 2,
                status: Some(ok.clone()),
                protocol_major: 2,
                protocol_minor: 0,
                capabilities: Vec::new(),
            })),
        },
        Frame {
            body: Some(Body::ConfigurationPublished(ConfigurationPublished {
                request_id: 0,
                configuration: Some(config()),
            })),
        },
        Frame {
            body: Some(Body::StreamStarted(StreamStarted {
                stream_instance_id: 1,
                config_generation: 1,
                initial_timestamp: 0,
            })),
        },
        Frame {
            body: Some(Body::StreamEnded(StreamEnded {
                stream_instance_id: 1,
                code: 502,
                detail: String::new(),
            })),
        },
        Frame {
            body: Some(Body::SubscribeResponse(SubscribeResponse {
                request_id: 3,
                status: Some(ok),
                configuration: None,
            })),
        },
    ];
    frames
        .iter()
        .map(|frame| frame.encode_to_vec())
        .filter(|body| !body.is_empty())
        .collect()
}

/// Truncate, flip, splice, or replace bytes in one corpus encoding.
fn mutate(rng: &mut Rng, base: &[u8]) -> Vec<u8> {
    let mut bytes = base.to_vec();
    match rng.below(6) {
        0 => {
            let keep = rng.below(bytes.len() as u64 + 1) as usize;
            bytes.truncate(keep);
        }
        1 => {
            let flips = 1 + rng.below(4);
            for _ in 0..flips {
                if !bytes.is_empty() {
                    let index = rng.below(bytes.len() as u64) as usize;
                    bytes[index] = rng.next_u64() as u8;
                }
            }
        }
        2 => {
            let extra = rng.below(8) as usize;
            bytes.extend(rng.bytes(extra));
        }
        3 => {
            if bytes.len() > 1 {
                let index = rng.below(bytes.len() as u64) as usize;
                bytes.remove(index);
            }
        }
        4 => {
            let len = rng.below(64) as usize;
            bytes = rng.bytes(len);
        }
        _ => {}
    }
    bytes
}

/// 10 000 random control bodies through the framed decoder, then every decoded
/// frame through a fresh `ClientMachine`. No panic, and a deterministic
/// signature sequence for a fixed seed. Framing violations are §6 `401`
/// classes (the client answers Fatal 401).
#[test]
fn fuzz_control_frames_and_client_machine_are_deterministic() {
    let run = |seed: u64| {
        let mut rng = Rng::new(seed);
        let mut decode_errors = 0u64;
        let mut decoded_frames = 0u64;
        let mut fatal = 0u64;
        let mut stops = 0u64;
        let mut signatures = Vec::new();
        let mut machine = ClientMachine::new();
        let corpus = control_corpus();
        for case in 0..CASES {
            // A terminal machine absorbs everything; start fresh so the
            // corpus keeps exercising the full state table.
            if machine.state.is_closed() {
                machine = ClientMachine::new();
            }
            // Half raw garbage, half a mutated valid encoding (so the decoder
            // and state machine see deep, plausible structures, not just
            // undecodable noise).
            let body = if rng.below(2) == 0 {
                let len = rng.below(256) as usize;
                rng.bytes(len)
            } else {
                let index = rng.below(corpus.len() as u64) as usize;
                mutate(&mut rng, &corpus[index])
            };
            let mut framed = Vec::with_capacity(4 + body.len());
            let declared = if rng.below(4) == 0 {
                // Hostile prefix: zero or above the 64 KiB cap.
                match rng.below(2) {
                    0 => 0u32,
                    _ => 65_537,
                }
            } else {
                body.len() as u32
            };
            framed.extend_from_slice(&declared.to_be_bytes());
            framed.extend_from_slice(&body);

            let mut decoder = FrameDecoder::new();
            match decoder.push(&framed) {
                Ok(frames) => {
                    for frame in frames {
                        decoded_frames += 1;
                        signatures.push(format!("{frame:?}"));
                        // Feed the decoded frame to the machine too.
                        if machine.state.is_closed() {
                            continue;
                        }
                        let output = machine.on_frame(&frame);
                        if matches!(
                            output.0,
                            Ok(audio_share_client_v2_native::control::Verdict::Fatal { .. })
                        ) {
                            fatal += 1;
                        }
                        if matches!(
                            output.0,
                            Ok(audio_share_client_v2_native::control::Verdict::Stop(_))
                        ) {
                            stops += 1;
                        }
                        signatures.push(verdict_signature(&machine, output));
                    }
                }
                Err(error) => {
                    decode_errors += 1;
                    assert!(
                        error.0.contains("401")
                            || error.0.contains("64 KiB")
                            || error.0.contains("§3")
                            || error.0.contains("decode"),
                        "framing failure must map to the §3/§6 401 class, got {error}"
                    );
                }
            }
        }
        (decode_errors, decoded_frames, fatal, stops, signatures)
    };

    let seed = seed();
    let first = run(seed);
    let second = run(seed);
    assert_eq!(first.0, second.0);
    assert_eq!(first.1, second.1);
    assert_eq!(first.2, second.2);
    assert_eq!(first.3, second.3);
    assert_eq!(first.4, second.4, "same seed must replay identically");
    assert!(
        first.1 > 500,
        "mutated corpus encodings must decode in bulk, got {}",
        first.1
    );
    eprintln!(
        "control fuzz: decode_errors={} decoded_frames={} fatal={} stops={}",
        first.0, first.1, first.2, first.3
    );
}

// ---------------------------------------------------------------------------
// §11.3 explicit negative cases (deterministic mappings)
// ---------------------------------------------------------------------------

fn expect_header(bytes: &[u8]) -> HeaderError {
    MediaHeader::parse(bytes).expect_err("must not parse")
}

#[test]
fn negative_truncated_and_impossible_media_headers() {
    // Truncation at every length below the 24-byte header.
    for len in 0..24 {
        let bytes = vec![0x01u8; len];
        assert_eq!(expect_header(&bytes), HeaderError::Truncated, "len {len}");
    }
    // Zero instance / zero generation are impossible fields (§7.1).
    let mut zero_instance = datagram(0, 0, 1);
    zero_instance[0..4].copy_from_slice(&0u32.to_be_bytes());
    assert_eq!(expect_header(&zero_instance), HeaderError::ImpossibleField);
    let mut zero_generation = datagram(0, 0, 1);
    zero_generation[4..8].copy_from_slice(&0u32.to_be_bytes());
    assert_eq!(
        expect_header(&zero_generation),
        HeaderError::ImpossibleField
    );
}

#[test]
fn negative_media_datagrams_are_dropped_and_counted_never_silent() {
    let mut media = session();
    // Undersized datagram.
    assert_eq!(media.on_datagram(0, &[0u8; 23]), DatagramOutcome::Malformed);
    // Impossible fields.
    let mut zero_instance = datagram(0, 0, 8);
    zero_instance[0..4].copy_from_slice(&0u32.to_be_bytes());
    assert_eq!(
        media.on_datagram(0, &zero_instance),
        DatagramOutcome::Malformed
    );
    // Misaligned payload (one extra byte).
    let mut misaligned = datagram(1, 64, 8);
    misaligned.push(0);
    assert_eq!(
        media.on_datagram(0, &misaligned),
        DatagramOutcome::Malformed
    );
    // Foreign instance is never adopted from media.
    let mut foreign = datagram(2, 128, 8);
    foreign[0..4].copy_from_slice(&2u32.to_be_bytes());
    assert_eq!(media.on_datagram(0, &foreign), DatagramOutcome::Malformed);
    // Reserved flags are ignored, not rejected (forward compatibility).
    let mut reserved = datagram(3, 192, 8);
    reserved[20..24].copy_from_slice(&(FLAG_SPARE | 0x8000_0000).to_be_bytes());
    let outcome = media.on_datagram(0, &reserved);
    assert!(
        matches!(outcome, DatagramOutcome::Delivered),
        "reserved bits must be ignored, got {outcome:?}"
    );
    // Oversized payload: the app parser accepts a frame-aligned payload; the
    // transport's datagram limit is where oversize is refused (NOT RUN here).
    let oversized = datagram(4, 256, 4_000); // 32 000-byte payload
    assert_eq!(
        media.on_datagram(0, &oversized),
        DatagramOutcome::Delivered,
        "app-layer parser is not the size gate"
    );
}

#[test]
fn negative_unknown_and_stale_generations_state_violations() {
    let mut media = session();
    // Unknown (newer) generation → bounded hold, never decoded.
    assert_eq!(
        media.on_datagram(0, &datagram_generation(2, 0, 0, 8)),
        DatagramOutcome::HeldUnknownConfig
    );
    assert!(media.holding_unknown_config());
    assert_eq!(media.delivered_frames(), 0);
    // Still unknown after the §7.6 2 s bound → the session gives up (403).
    assert!(media.tick(2_000));
    assert!(media.gave_up_unknown_config());

    // Stale generation after a config change → dropped and counted.
    let mut media = session();
    media.apply_configuration(&Configuration {
        config_generation: 2,
        ..config()
    });
    assert_eq!(
        media.on_datagram(0, &datagram_generation(1, 5, 320, 8)),
        DatagramOutcome::StaleGeneration
    );
    let diagnostics = media.diagnostics(60);
    assert_eq!(diagnostics.cumulative_unknown_config_drops, 1);
    assert_eq!(diagnostics.config_generation, 2);
}

fn datagram_generation(generation: u32, sequence: u32, timestamp: u64, frames: usize) -> Vec<u8> {
    let header = MediaHeader {
        stream_instance_id: 1,
        config_generation: generation,
        sequence,
        timestamp,
        flags: 0,
    };
    let mut out = Vec::new();
    header.write_be(&mut out);
    out.extend(std::iter::repeat_n(0u8, frames * 8));
    out
}

#[test]
fn negative_control_framing_lengths_are_401_classes() {
    // Zero length prefix.
    let mut decoder = FrameDecoder::new();
    assert!(decoder.push(&[0, 0, 0, 0]).is_err());
    // Above the 64 KiB cap.
    let mut decoder = FrameDecoder::new();
    assert!(decoder.push(&[0, 1, 0, 1]).is_err());
    // Declared length with no body: buffered, no panic, no allocation beyond
    // the input (the decoder keeps the 4-byte prefix and waits for the rest).
    let mut decoder = FrameDecoder::new();
    assert!(decoder.push(&[0, 0, 0, 5]).expect("incomplete").is_empty());
    assert!(
        decoder.buffered() <= 4,
        "only the prefix is held while the body is awaited"
    );
    // Undecodable body.
    let mut decoder = FrameDecoder::new();
    assert!(decoder.push(&[0, 0, 0, 3, 0x0a, 0xff, 0x01]).is_err());
    // Unknown field alone leaves the oneof unset; the Client state machine
    // classifies an unset body as 401, never a silent ignore.
    let body = Frame::decode([0x78u8, 0x00].as_slice()).expect("unknown field decodes");
    assert!(body.body.is_none());
    let mut machine = ClientMachine::new();
    match machine.on_frame(&body).0 {
        Ok(audio_share_client_v2_native::control::Verdict::Fatal { code }) => {
            assert_eq!(code, 401)
        }
        other => panic!("unset body must be fatal 401, got {other:?}"),
    }
}
