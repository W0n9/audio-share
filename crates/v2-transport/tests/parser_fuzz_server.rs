//! M5-06 §11.3 parser fuzz — Server/host half.
//!
//! Fixed-count (10 000), seeded, deterministic fuzz of:
//!
//! 1. the control-plane schema decoder (`framing::decode_body`),
//! 2. the streaming control-frame reader (`framing::FrameReader`),
//! 3. the server-side 24-byte media header parser (`media::MediaHeader`), and
//! 4. the §5.2 state machine fed mutated valid requests.
//!
//! Every framing failure must carry §6 `401`; state violations `402`; version
//! conflicts `101`; authentication failures `203`. No panic, no silent ignore
//! of a request frame, and the same seed replays identically. Seeds print to
//! stderr (run with `--nocapture`); override with `M5_FUZZ_SEED`.

use std::sync::Arc;

use prost::Message as _;
use v2_transport::auth_store::{AuthorizationStore, ClientState, StoredClient};
use v2_transport::codes::ErrorCode;
use v2_transport::framing::{decode_body, FrameError, FrameReader};
use v2_transport::media::{MediaHeader, PacketError};
use v2_transport::schema::{
    self, frame, AuthInit, AuthProof, Close, Error, Frame, Negotiate, RequestConfiguration, Status,
    Subscribe, Unsubscribe,
};
use v2_transport::state_machine::{
    CaptureSnapshot, Event, Outcome, ServerConfig, ServerState, StateMachine,
};

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
        .unwrap_or(0x4D35_0006_5E51_0001);
    eprintln!("m5-06 server fuzz seed = {seed}");
    seed
}

const CASES: usize = 10_000;

// ---------------------------------------------------------------------------
// Fixture identities (the accepted golden-vector constants; test-support)
// ---------------------------------------------------------------------------

const CLIENT_PUB: &str = "03d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d";
const SERVER_PUB: &str = "11b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c";
const VALID_SIG: &str = "e77e55090b72af3cb8f42a8e6696b4797170b8c8236944ce4c17164e5d97762f8686dfcd64f4a21f5768fa6c94e6a816029a566fb849fa27cbe98385f4df060c";
const INVALID_SIG: &str = "e77e55090b72af3cb8f42a8e6696b4797170b8c8236944ce4c17164e5d97762f8686dfcd64f4a21f5768fa6c94e6a816029a566fb849fa27cbe98385f4df060b";

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn store_with_client() -> Arc<AuthorizationStore> {
    let store = AuthorizationStore::in_memory();
    store.insert_for_tests(StoredClient {
        client_id: "cf7940b474310246".to_owned(),
        public_key: unhex(CLIENT_PUB),
        display_name: "Test Phone".to_owned(),
        state: ClientState::Active,
        created_at: String::new(),
        last_seen_at: String::new(),
    });
    Arc::new(store)
}

fn machine_with(store: &Arc<AuthorizationStore>) -> StateMachine {
    StateMachine::new(
        ServerConfig {
            server_public_key: unhex(SERVER_PUB),
            protocol_minor: 0,
            capture: CaptureSnapshot {
                configuration: Some(schema::Configuration {
                    config_generation: 1,
                    codec: schema::Codec::Pcm as i32,
                    pcm: Some(schema::PcmParameters {
                        sample_rate: 48_000,
                        channels: 2,
                        sample_format: schema::SampleFormat::F32Le as i32,
                    }),
                }),
                stream_active: true,
                ..CaptureSnapshot::absent()
            },
            max_auth_failures_per_connection: 5,
            server_random_override: Some(unhex(
                "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            )),
            auth_exporter: Some(unhex(
                "c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf",
            )),
            session_id_base: 1,
            pairing: None,
            pairing_exporter: None,
            pairing_server_random_override: None,
        },
        store,
    )
}

fn machine() -> StateMachine {
    machine_with(&store_with_client())
}

fn frame(body: frame::Body) -> Frame {
    Frame { body: Some(body) }
}

/// Drive a fresh machine to NEGOTIATING with the recorded happy-path exchange.
fn negotiating_machine() -> StateMachine {
    let store = store_with_client();
    let mut machine = machine_with(&store);
    let challenge = machine.handle(Event::Frame {
        frame: Box::new(frame(frame::Body::AuthInit(AuthInit {
            request_id: 1,
            client_public_key: unhex(CLIENT_PUB),
        }))),
        source: Vec::new(),
    });
    assert_eq!(challenge.len(), 1);
    let proof = machine.handle(Event::Frame {
        frame: Box::new(frame(frame::Body::AuthProof(AuthProof {
            request_id: 1,
            signature: unhex(VALID_SIG),
        }))),
        source: Vec::new(),
    });
    assert_eq!(proof.len(), 1, "auth exchange completes");
    assert_eq!(machine.state(), ServerState::Negotiating);
    machine
}

// ---------------------------------------------------------------------------
// Outcome helpers
// ---------------------------------------------------------------------------

fn allowed_codes() -> Vec<u32> {
    ErrorCode::ALL.iter().map(|entry| entry.code).collect()
}

/// Every nonzero §6 code an outcome can carry.
fn outcome_codes(outcome: &Outcome) -> Vec<u32> {
    let mut codes = Vec::new();
    if let Some(message) = &outcome.frame {
        match &message.body {
            Some(frame::Body::Error(error)) => codes.push(error.status.code),
            Some(frame::Body::Close(close)) => codes.push(close.code),
            Some(frame::Body::AuthResult(result)) => codes.push(result.status.code),
            Some(frame::Body::NegotiateResponse(result)) => codes.push(result.status.code),
            Some(frame::Body::SubscribeResponse(result)) => codes.push(result.status.code),
            Some(frame::Body::UnsubscribeResponse(result)) => codes.push(result.status.code),
            Some(frame::Body::PairingResponse(result)) => codes.push(result.status.code),
            Some(frame::Body::PairingResult(result)) => codes.push(result.status.code),
            Some(frame::Body::StreamEnded(ended)) => codes.push(ended.code),
            _ => {}
        }
    }
    if let Some(code) = outcome.close_code {
        codes.push(code.code);
    }
    if let Some(code) = outcome.transport_close_code {
        codes.push(code.code);
    }
    codes
}

/// A request body must never be silently ignored in CONNECTING. The defined
/// silent cases are the advisory form of a non-fatal protocol message.
fn defined_silent(body: &frame::Body) -> bool {
    match body {
        frame::Body::Close(close) => close.code != 0,
        frame::Body::Error(error) => error.status.code != 0,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Target 1 — control schema decoder
// ---------------------------------------------------------------------------

/// 10 000 random bodies through `decode_body`. A decode failure is always the
/// §6 `401` class; a success always has a oneof body set. Replayed seed
/// equality proves determinism.
#[test]
fn fuzz_decode_body_never_panics_and_maps_to_401() {
    let run = |seed: u64| {
        let mut rng = Rng::new(seed);
        let corpus = request_corpus();
        let mut ok = 0u64;
        let mut empty = 0u64;
        let mut malformed = 0u64;
        let mut unset = 0u64;
        for _ in 0..CASES {
            // Half raw garbage; half mutations of valid encodings so the
            // unknown-field / length-prefix evolution rules are exercised.
            let body = if rng.below(2) == 0 {
                let len = rng.below(4_096) as usize;
                rng.bytes(len)
            } else {
                let index = rng.below(corpus.len() as u64) as usize;
                mutate(&mut rng, &corpus[index])
            };
            match decode_body(&body) {
                Ok(frame) => {
                    ok += 1;
                    assert!(
                        frame.body.is_some(),
                        "decode_body never returns an unset body"
                    );
                }
                Err(error) => {
                    assert_eq!(
                        error.error_code().code,
                        401,
                        "every schema/framing failure is §6 401: {error:?}"
                    );
                    match error {
                        FrameError::Empty => empty += 1,
                        FrameError::Malformed => malformed += 1,
                        FrameError::UnsetBody => unset += 1,
                        FrameError::Oversize => {
                            panic!("decode_body does not check the prefix length")
                        }
                        FrameError::LengthOutOfBounds(_) | FrameError::Encode => {
                            panic!("outbound-only error from an inbound decode")
                        }
                    }
                }
            }
        }
        (ok, empty, malformed, unset)
    };

    let seed = seed();
    let first = run(seed);
    assert_eq!(first, run(seed), "same seed must replay identically");
    assert_eq!(first.0 + first.1 + first.2 + first.3, CASES as u64);
    assert!(first.0 > 0, "mutated valid encodings must decode sometimes");
    assert!(
        first.2 + first.3 > 0,
        "hostile bytes are rejected, not ignored"
    );
    eprintln!(
        "decode_body fuzz: ok={} empty={} malformed={} unset={}",
        first.0, first.1, first.2, first.3
    );
}

// ---------------------------------------------------------------------------
// Target 2 — streaming control-frame reader
// ---------------------------------------------------------------------------

#[test]
fn fuzz_frame_reader_never_panics_and_maps_to_401() {
    let run = |seed: u64| {
        let mut rng = Rng::new(seed);
        let corpus = request_corpus();
        let mut frames = 0u64;
        let mut errors = 0u64;
        for _ in 0..CASES {
            let mut reader = FrameReader::default();
            // Half raw stream chunks, half a valid length-prefixed corpus
            // frame delivered in random-sized pieces (with occasional body
            // corruption or a hostile prefix).
            let chunks: Vec<Vec<u8>> = if rng.below(2) == 0 {
                let count = 1 + rng.below(4);
                (0..count)
                    .map(|_| {
                        let len = rng.below(70_000) as usize;
                        rng.bytes(len)
                    })
                    .collect()
            } else {
                let index = rng.below(corpus.len() as u64) as usize;
                let mut body = mutate(&mut rng, &corpus[index]);
                let declared = if rng.below(8) == 0 {
                    match rng.below(2) {
                        0 => 0u32,
                        _ => 65_537,
                    }
                } else {
                    body.len() as u32
                };
                let mut framed = declared.to_be_bytes().to_vec();
                framed.append(&mut body);
                let split = rng.below(framed.len() as u64 + 1) as usize;
                vec![framed[..split].to_vec(), framed[split..].to_vec()]
            };
            for bytes in chunks {
                match reader.push(&bytes) {
                    Ok(decoded) => frames += decoded.len() as u64,
                    Err(error) => {
                        assert_eq!(error.error_code().code, 401);
                        errors += 1;
                        break;
                    }
                }
            }
        }
        (frames, errors)
    };

    let seed = seed();
    let first = run(seed);
    assert_eq!(first, run(seed), "same seed must replay identically");
    assert!(first.0 > 0, "valid framed corpus frames are decoded");
    assert!(first.1 > 0, "random streams include framing violations");
    eprintln!(
        "frame_reader fuzz: frames={} framing_errors={}",
        first.0, first.1
    );
}

// ---------------------------------------------------------------------------
// Target 3 — server media header parser
// ---------------------------------------------------------------------------

#[test]
fn fuzz_server_media_header_never_panics_and_is_deterministic() {
    let run = |seed: u64| {
        let mut rng = Rng::new(seed);
        let mut truncated = 0u64;
        let mut zero = 0u64;
        let mut parsed = 0u64;
        for _ in 0..CASES {
            let len = rng.below(301) as usize;
            let bytes = rng.bytes(len);
            match MediaHeader::parse(&bytes) {
                Ok((header, payload)) => {
                    parsed += 1;
                    assert_eq!(payload.len(), bytes.len() - 24);
                    assert_ne!(header.stream_instance_id, 0);
                    assert_ne!(header.config_generation, 0);
                }
                Err(PacketError::MisalignedPcm) => {
                    truncated += 1;
                    assert!(bytes.len() < 24, "truncated header only");
                }
                Err(PacketError::ZeroField) => {
                    zero += 1;
                    assert!(bytes.len() >= 24);
                }
                other => panic!("unexpected parse error for a header: {other:?}"),
            }
        }
        (truncated, zero, parsed)
    };

    let seed = seed();
    let first = run(seed);
    assert_eq!(first, run(seed));
    assert_eq!(first.0 + first.1 + first.2, CASES as u64);
    eprintln!(
        "server media-header fuzz: truncated={} zero_field={} parsed={}",
        first.0, first.1, first.2
    );
}

// ---------------------------------------------------------------------------
// Target 4 — state machine on mutated valid requests
// ---------------------------------------------------------------------------

fn request_corpus() -> Vec<Vec<u8>> {
    let frames = vec![
        frame(frame::Body::AuthInit(AuthInit {
            request_id: 1,
            client_public_key: unhex(CLIENT_PUB),
        })),
        frame(frame::Body::AuthProof(AuthProof {
            request_id: 1,
            signature: vec![0u8; 64],
        })),
        frame(frame::Body::Negotiate(Negotiate {
            request_id: 2,
            protocol_major: 2,
            protocol_minor: 0,
            capabilities: Vec::new(),
        })),
        frame(frame::Body::Subscribe(Subscribe { request_id: 3 })),
        frame(frame::Body::Unsubscribe(Unsubscribe { request_id: 4 })),
        frame(frame::Body::RequestConfiguration(RequestConfiguration {
            request_id: 5,
        })),
        frame(frame::Body::Close(Close {
            code: 701,
            detail: String::new(),
        })),
        frame(frame::Body::Error(Error {
            request_id: 0,
            status: Status::registry(401),
        })),
    ];
    frames
        .iter()
        .map(|frame| frame.encode_to_vec())
        .filter(|body| !body.is_empty())
        .collect()
}

fn mutate(rng: &mut Rng, base: &[u8]) -> Vec<u8> {
    let mut bytes = base.to_vec();
    match rng.below(6) {
        0 => {
            let keep = rng.below(bytes.len() as u64 + 1) as usize;
            bytes.truncate(keep);
        }
        1 => {
            for _ in 0..(1 + rng.below(4)) {
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

/// 10 000 mutated valid requests through `decode_body` and a fresh §5.2 state
/// machine. Every outcome carries only §6 registry codes, a request is never
/// silently ignored, and the outcome signature replays from the seed.
#[test]
fn fuzz_state_machine_requests_are_deterministic_and_never_silently_ignored() {
    let run = |seed: u64| {
        let mut rng = Rng::new(seed);
        let corpus = request_corpus();
        let allowed = allowed_codes();
        let mut signatures = Vec::new();
        let mut decoded = 0u64;
        let mut rejected = 0u64;
        for _ in 0..CASES {
            let body = if rng.below(2) == 0 {
                let len = rng.below(256) as usize;
                rng.bytes(len)
            } else {
                let index = rng.below(corpus.len() as u64) as usize;
                mutate(&mut rng, &corpus[index])
            };
            let Ok(frame) = decode_body(&body) else {
                rejected += 1;
                continue;
            };
            decoded += 1;
            let request = frame
                .body
                .as_ref()
                .is_some_and(|body| !defined_silent(body));
            let mut machine = machine();
            let outcomes = machine.handle(Event::Frame {
                frame: Box::new(frame),
                source: Vec::new(),
            });
            for outcome in &outcomes {
                for code in outcome_codes(outcome) {
                    if code != 0 {
                        assert!(
                            allowed.contains(&code),
                            "outcome code {code} is not in the §6 registry"
                        );
                    }
                }
            }
            if request {
                assert!(
                    !outcomes.is_empty(),
                    "a request frame must produce a response or a fatal close"
                );
            }
            signatures.push(format!("{:?}|{:?}", machine.state(), outcomes));
        }
        (decoded, rejected, signatures)
    };

    let seed = seed();
    let first = run(seed);
    let second = run(seed);
    assert_eq!(first.0, second.0);
    assert_eq!(first.1, second.1);
    assert_eq!(first.2, second.2, "same seed must replay identically");
    assert!(first.0 > 500, "mutated corpus must decode in bulk");
    eprintln!(
        "state-machine fuzz: decoded={} rejected={}",
        first.0, first.1
    );
}

// ---------------------------------------------------------------------------
// §11.3 explicit negative mappings
// ---------------------------------------------------------------------------

#[test]
fn negative_malformed_lengths_and_truncation_are_401() {
    // Zero length and above-cap prefixes are rejected before allocation.
    let mut reader = FrameReader::default();
    assert_eq!(
        reader.push(&[0, 0, 0, 0]).unwrap_err().error_code().code,
        401
    );
    let mut reader = FrameReader::default();
    assert_eq!(
        reader.push(&[0, 1, 0, 1]).unwrap_err().error_code().code,
        401
    );
    // Empty body.
    assert_eq!(decode_body(&[]).unwrap_err(), FrameError::Empty);
    // Unset oneof.
    assert_eq!(
        decode_body(&[0x78, 0x00]).unwrap_err(),
        FrameError::UnsetBody
    );
    // Undecodable body (bad wire type end-group).
    assert_eq!(
        decode_body(&[0x0a, 0xff, 0x01]).unwrap_err(),
        FrameError::Malformed
    );
    // A truncated frame body: the reader waits (does not allocate the body).
    let mut reader = FrameReader::default();
    assert!(reader
        .push(&[0, 0, 0, 5, 0x32])
        .expect("incomplete")
        .is_empty());
    // Truncated / undersized media datagrams.
    for len in 0..24 {
        assert_eq!(
            MediaHeader::parse(&vec![1u8; len]),
            Err(PacketError::MisalignedPcm),
            "len {len}"
        );
    }
    // Impossible media fields.
    let mut zero = vec![0u8; 32];
    zero[4] = 1; // generation 1, instance 0
    assert_eq!(MediaHeader::parse(&zero), Err(PacketError::ZeroField));
}

#[test]
fn negative_unknown_fields_are_ignored_but_unset_body_is_401() {
    // Unknown field alongside a known Subscribe (proto3 evolution) is ignored.
    // Subscribe { request_id: 1 } = field 32 (tag 82 02), len 9,
    // content = field 1 fixed64 (tag 09) + 1u64 LE.
    let mut body = vec![0x82, 0x02, 0x09, 0x09];
    body.extend_from_slice(&1u64.to_le_bytes());
    body.extend_from_slice(&[0x78, 0x00]); // unknown field 15, outside Subscribe
    let frame = decode_body(&body).expect("known body wins");
    match frame.body {
        Some(frame::Body::Subscribe(subscribe)) => assert_eq!(subscribe.request_id, 1),
        other => panic!("expected Subscribe, got {other:?}"),
    }
    // Unknown message/field alone leaves the oneof unset → 401, never silent.
    assert_eq!(
        decode_body(&[0x78, 0x00]).unwrap_err(),
        FrameError::UnsetBody
    );
}

#[test]
fn negative_state_violation_is_402() {
    let mut machine = machine();
    assert_eq!(machine.state(), ServerState::Connecting);
    let outcomes = machine.handle(Event::Frame {
        frame: Box::new(frame(frame::Body::Subscribe(Subscribe { request_id: 1 }))),
        source: Vec::new(),
    });
    let codes: Vec<u32> = outcomes.iter().flat_map(outcome_codes).collect();
    assert!(
        codes.contains(&402),
        "a request illegal for CONNECTING is fatal 402, got {codes:?}"
    );
}

#[test]
fn negative_version_and_capability_conflicts_are_101_and_401() {
    // major 1 → 101.
    let mut machine = negotiating_machine();
    let outcomes = machine.handle(Event::Frame {
        frame: Box::new(frame(frame::Body::Negotiate(Negotiate {
            request_id: 2,
            protocol_major: 1,
            protocol_minor: 0,
            capabilities: Vec::new(),
        }))),
        source: Vec::new(),
    });
    let codes: Vec<u32> = outcomes.iter().flat_map(outcome_codes).collect();
    assert!(
        codes.contains(&101),
        "version mismatch → 101, got {codes:?}"
    );

    // Duplicated / non-ascending capabilities → 401.
    let mut machine = negotiating_machine();
    let outcomes = machine.handle(Event::Frame {
        frame: Box::new(frame(frame::Body::Negotiate(Negotiate {
            request_id: 2,
            protocol_major: 2,
            protocol_minor: 0,
            capabilities: vec![2, 1],
        }))),
        source: Vec::new(),
    });
    let codes: Vec<u32> = outcomes.iter().flat_map(outcome_codes).collect();
    assert!(
        codes.contains(&401),
        "capability conflict → 401, got {codes:?}"
    );
}

#[test]
fn negative_authentication_failure_is_203() {
    let mut machine = machine();
    let _ = machine.handle(Event::Frame {
        frame: Box::new(frame(frame::Body::AuthInit(AuthInit {
            request_id: 1,
            client_public_key: unhex(CLIENT_PUB),
        }))),
        source: Vec::new(),
    });
    let outcomes = machine.handle(Event::Frame {
        frame: Box::new(frame(frame::Body::AuthProof(AuthProof {
            request_id: 1,
            signature: unhex(INVALID_SIG),
        }))),
        source: Vec::new(),
    });
    let codes: Vec<u32> = outcomes.iter().flat_map(outcome_codes).collect();
    assert!(
        codes.contains(&203),
        "a bad proof is AuthResult(203), got {codes:?}"
    );
    assert_eq!(
        machine.take_auth_failure_code(),
        Some(ErrorCode::AUTHENTICATION_FAILED)
    );
}
