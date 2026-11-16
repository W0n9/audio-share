//! Pairing rows at the §5.2 state-machine seam (ticket 19).
//!
//! Fixtures: `fixtures/wire-protocol-v2/control/pairing-happy-path-qr.json`,
//! `pairing-happy-path-sas.json`, `error-201-pairing-declined.json`,
//! `error-201-pairing-proof-invalid.json`, and `test-keys.json`. The recorded
//! `client_proof` values are verified, never regenerated.

use std::sync::Arc;
use std::time::{Duration, Instant};

use v2_transport::auth_store::{AuthorizationStore, ClientState};
use v2_transport::codes::ErrorCode;
use v2_transport::pairing::{self, Clock as _, PairingCore, PairingExporter};
use v2_transport::schema::{
    frame, Frame, PairingConfirm, PairingMode, PairingRequest, PairingResponse,
};
use v2_transport::state_machine::{
    CaptureSnapshot, Event, Outcome, ServerConfig, ServerState, StateMachine,
};

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

const SERVER_PUB: &str = "11b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c";
const CLIENT_PUB: &str = "03d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d";
const NONCE: &str = "808182838485868788898a8b8c8d8e8f";
const CLIENT_RANDOM: &str = "606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f";
const SERVER_RANDOM: &str = "404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f";
const QR_BINDING: &str = "909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf";
const SAS_BINDING: &str = "d0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff";
const SAS_SEED: &str = "e0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff";
const QR_PROOF: &str = "c7ae8b6474ebb7b6d654febfce10e38d3a479b4534b5436bbb7714ae3652bfc30b0d8ffbf1582ecb2af24e25ba47876b4a35dd00af0924b57648871f95baeee1";
const SAS_PROOF: &str = "41cb672ee38b878ced50dfae60f0adeb3c4822fa615361392ce7d5ff65e55989012093a2e71125a0edfc1149cb1cb5470899f273c6ab66daf18735f2cdf44f67";

/// The fixture stand-ins for the connection-scoped TLS exporter outputs.
#[derive(Debug)]
struct FixtureExporter {
    binding: &'static str,
}

impl FixtureExporter {
    const QR: Self = Self {
        binding: QR_BINDING,
    };
    const SAS: Self = Self {
        binding: SAS_BINDING,
    };
}

impl PairingExporter for FixtureExporter {
    fn export(&self, label: &[u8], context: &[u8], len: usize) -> Option<Vec<u8>> {
        // The raw transcript is never re-derived here; the fixture's
        // exported stand-ins are keyed by the exchange's mode.
        let _ = context;
        if label == pairing::PAIRING_EXPORTER_LABEL {
            assert_eq!(len, 48);
            Some(unhex(self.binding))
        } else if label == pairing::SAS_EXPORTER_LABEL {
            assert_eq!(len, 32);
            Some(unhex(SAS_SEED))
        } else {
            None
        }
    }
}

#[derive(Debug)]
struct TestClock(std::sync::Mutex<Instant>);

impl TestClock {
    fn new() -> Arc<Self> {
        Arc::new(Self(std::sync::Mutex::new(Instant::now())))
    }
}

impl pairing::Clock for TestClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

fn core_with_nonce(clock: Arc<TestClock>) -> Arc<PairingCore> {
    let store = Arc::new(AuthorizationStore::in_memory());
    let core = Arc::new(PairingCore::with_clock(
        unhex(SERVER_PUB),
        store,
        clock.clone(),
    ));
    core.insert_nonce_for_tests(
        unhex(NONCE).try_into().unwrap(),
        clock.now() + Duration::from_secs(600),
    );
    core
}

fn config(core: Arc<PairingCore>) -> ServerConfig {
    config_with_exporter(core, FixtureExporter::QR)
}

fn config_with_exporter(core: Arc<PairingCore>, exporter: FixtureExporter) -> ServerConfig {
    ServerConfig {
        server_public_key: unhex(SERVER_PUB),
        protocol_minor: 0,
        capture: CaptureSnapshot::absent(),
        max_auth_failures_per_connection: 5,
        server_random_override: Some(unhex(
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        )),
        auth_exporter: Some(unhex(
            "c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf",
        )),
        session_id_base: 1,
        pairing: Some(core),
        pairing_exporter: Some(Arc::new(exporter)),
        pairing_server_random_override: Some(unhex(SERVER_RANDOM)),
    }
}

fn machine(config: ServerConfig) -> StateMachine {
    let store = config
        .pairing
        .as_ref()
        .expect("pairing core")
        .store()
        .clone();
    StateMachine::new(config, &store)
}

fn request_qr() -> PairingRequest {
    PairingRequest {
        request_id: 1,
        pairing_mode: PairingMode::Qr as i32,
        pairing_nonce: unhex(NONCE),
        client_random: unhex(CLIENT_RANDOM),
        client_public_key: unhex(CLIENT_PUB),
        protocol_major: 2,
        capabilities: Vec::new(),
        client_name: "Test Phone".to_owned(),
    }
}

fn request_sas() -> PairingRequest {
    PairingRequest {
        pairing_nonce: Vec::new(),
        pairing_mode: PairingMode::Sas as i32,
        ..request_qr()
    }
}

fn frame_event(body: frame::Body, source: &str) -> Event {
    Event::Frame {
        frame: Box::new(Frame { body: Some(body) }),
        source: source.as_bytes().to_vec(),
    }
}

fn body(outcome: &Outcome) -> &frame::Body {
    outcome
        .frame
        .as_ref()
        .and_then(|frame| frame.body.as_ref())
        .expect("frame")
}

fn expect_pairing_response(outcomes: &[Outcome]) -> &PairingResponse {
    assert_eq!(outcomes.len(), 1, "{outcomes:?}");
    match body(&outcomes[0]) {
        frame::Body::PairingResponse(response) => response,
        other => panic!("expected PairingResponse, got {other:?}"),
    }
}

#[test]
fn qr_happy_path_verifies_then_waits_for_local_approval() {
    let clock = TestClock::new();
    let core = core_with_nonce(clock);
    let store = core.store().clone();
    let mut machine = machine(config(Arc::clone(&core)));

    let out = machine.handle(frame_event(
        frame::Body::PairingRequest(request_qr()),
        "192.168.1.20",
    ));
    let response = expect_pairing_response(&out);
    assert_eq!(response.request_id, 1);
    assert_eq!(response.pairing_mode, PairingMode::Qr as i32);
    assert_eq!(hex(&response.pairing_nonce), NONCE);
    assert_eq!(hex(&response.client_random), CLIENT_RANDOM);
    assert_eq!(hex(&response.server_random), SERVER_RANDOM);
    assert_eq!(hex(&response.server_public_key), SERVER_PUB);
    assert_eq!(response.protocol_major, 2);
    assert_eq!(response.capabilities, Vec::<u32>::new());
    assert_eq!(hex(&response.binding), QR_BINDING);
    assert_eq!(hex(&response.client_id), "cf7940b474310246");
    assert_eq!(response.display_name, "Test Phone");
    assert_eq!(machine.state(), ServerState::Authenticating);

    // Paired but not persisted: the proof verifies and the machine parks the
    // exchange for the operator.
    let out = machine.handle(frame_event(
        frame::Body::PairingConfirm(PairingConfirm {
            request_id: 1,
            binding_echo: unhex(QR_BINDING),
            client_proof: unhex(QR_PROOF),
        }),
        "192.168.1.20",
    ));
    assert!(out.is_empty(), "no PairingResult before approval: {out:?}");
    assert!(
        store.clients().is_empty(),
        "nothing persisted before approval"
    );
    let approval = machine
        .take_pairing_approval_request()
        .expect("verified pairing surfaces an approval request");
    assert_eq!(approval.client_id, "cf7940b474310246");
    assert_eq!(approval.display_name, "Test Phone");
    assert_eq!(approval.mode, PairingMode::Qr);
    assert_eq!(approval.sas.as_deref(), Some("900 067"));
    assert!(
        machine.take_pairing_approval_request().is_none(),
        "one request is surfaced exactly once"
    );

    let out = machine.handle(Event::PairingApproved { request_id: 1 });
    assert_eq!(out.len(), 1);
    match body(&out[0]) {
        frame::Body::PairingResult(result) => {
            assert_eq!(result.request_id, 1);
            assert_eq!(result.status.code, 0);
        }
        other => panic!("expected PairingResult(OK), got {other:?}"),
    }
    assert_eq!(machine.state(), ServerState::Negotiating);
    assert_eq!(machine.identity(), Some(unhex(CLIENT_PUB).as_slice()));
    assert!(machine.session_id().is_some(), "fresh session registered");
    assert_eq!(
        machine.supersede_requested(),
        Some(ErrorCode::SESSION_SUPERSEDED)
    );
    let clients = store.clients();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].client_id, "cf7940b474310246");
    assert_eq!(clients[0].state, ClientState::Active);
    assert_eq!(clients[0].created_at.len(), 20);
    assert!(clients[0].created_at.ends_with('Z'));
    assert_eq!(clients[0].last_seen_at.len(), 20);
}

#[test]
fn declined_approval_is_201_and_close_with_nothing_persisted() {
    let clock = TestClock::new();
    let core = core_with_nonce(clock);
    let store = core.store().clone();
    let mut machine = machine(config(Arc::clone(&core)));
    machine.handle(frame_event(
        frame::Body::PairingRequest(request_qr()),
        "192.168.1.20",
    ));
    machine.handle(frame_event(
        frame::Body::PairingConfirm(PairingConfirm {
            request_id: 1,
            binding_echo: unhex(QR_BINDING),
            client_proof: unhex(QR_PROOF),
        }),
        "192.168.1.20",
    ));
    let out = machine.handle(Event::PairingDeclined { request_id: 1 });
    assert_eq!(out.len(), 2, "{out:?}");
    match body(&out[0]) {
        frame::Body::PairingResult(result) => {
            assert_eq!(result.status.code, 201);
            assert!(result.status.detail.is_empty());
        }
        other => panic!("expected PairingResult(201), got {other:?}"),
    }
    match body(&out[1]) {
        frame::Body::Close(close) => {
            assert_eq!(close.code, 201);
            assert!(close.detail.is_empty());
        }
        other => panic!("expected Close(201), got {other:?}"),
    }
    assert_eq!(machine.state(), ServerState::Closing);
    assert!(
        store.clients().is_empty(),
        "declined pairing never persists"
    );
}

#[test]
fn invalid_client_proof_is_201_and_close_with_nothing_persisted() {
    let clock = TestClock::new();
    let core = core_with_nonce(clock);
    let store = core.store().clone();
    let mut machine = machine(config(Arc::clone(&core)));
    machine.handle(frame_event(
        frame::Body::PairingRequest(request_qr()),
        "192.168.1.20",
    ));
    // error-201-pairing-proof-invalid: final proof byte flipped.
    let mut proof = unhex(QR_PROOF);
    let last = proof.len() - 1;
    proof[last] ^= 0x01;
    let out = machine.handle(frame_event(
        frame::Body::PairingConfirm(PairingConfirm {
            request_id: 1,
            binding_echo: unhex(QR_BINDING),
            client_proof: proof,
        }),
        "192.168.1.20",
    ));
    assert_eq!(out.len(), 2, "{out:?}");
    match body(&out[0]) {
        frame::Body::PairingResult(result) => assert_eq!(result.status.code, 201),
        other => panic!("expected PairingResult(201), got {other:?}"),
    }
    match body(&out[1]) {
        frame::Body::Close(close) => assert_eq!(close.code, 201),
        other => panic!("expected Close(201), got {other:?}"),
    }
    assert!(store.clients().is_empty());
    assert!(machine.take_pairing_approval_request().is_none());
}

#[test]
fn malformed_capabilities_are_401_without_burning_the_nonce() {
    let clock = TestClock::new();
    let core = core_with_nonce(clock);
    let mut bad = request_qr();
    bad.request_id = 1;
    bad.capabilities = vec![2, 1];
    let mut first = machine(config(Arc::clone(&core)));
    let out = first.handle(frame_event(
        frame::Body::PairingRequest(bad),
        "192.168.1.20",
    ));
    assert_eq!(out.len(), 2, "{out:?}");
    match body(&out[0]) {
        frame::Body::Error(error) => assert_eq!(error.status.code, 401),
        other => panic!("expected Error(401), got {other:?}"),
    }
    // The same nonce is still admissible on a new connection.
    let mut second = machine(config(Arc::clone(&core)));
    let out = second.handle(frame_event(
        frame::Body::PairingRequest(request_qr()),
        "192.168.1.20",
    ));
    expect_pairing_response(&out);
}

#[test]
fn a_well_formed_failure_burns_the_nonce_for_replay() {
    let clock = TestClock::new();
    let core = core_with_nonce(clock);
    let mut first = machine(config(Arc::clone(&core)));
    expect_pairing_response(&first.handle(frame_event(
        frame::Body::PairingRequest(request_qr()),
        "192.168.1.20",
    )));
    // The first exchange never confirmed; a second connection replays it.
    let mut replay = machine(config(Arc::clone(&core)));
    let out = replay.handle(frame_event(
        frame::Body::PairingRequest(request_qr()),
        "192.168.1.21",
    ));
    assert_eq!(out.len(), 2, "{out:?}");
    match body(&out[0]) {
        frame::Body::PairingResult(result) => assert_eq!(result.status.code, 201),
        other => panic!("expected PairingResult(201), got {other:?}"),
    }
}

#[test]
fn expired_nonce_is_201() {
    let clock = TestClock::new();
    let store = Arc::new(AuthorizationStore::in_memory());
    let core = Arc::new(PairingCore::with_clock(
        unhex(SERVER_PUB),
        store.clone(),
        clock.clone(),
    ));
    core.insert_nonce_for_tests(
        unhex(NONCE).try_into().unwrap(),
        // Already expired at admission time.
        clock.now(),
    );
    let mut machine = machine(config(core));
    let out = machine.handle(frame_event(
        frame::Body::PairingRequest(request_qr()),
        "192.168.1.20",
    ));
    assert_eq!(out.len(), 2, "{out:?}");
    match body(&out[0]) {
        frame::Body::PairingResult(result) => assert_eq!(result.status.code, 201),
        other => panic!("expected PairingResult(201), got {other:?}"),
    }
}

#[test]
fn pairing_deadline_is_206_and_close() {
    let clock = TestClock::new();
    let core = core_with_nonce(clock);
    let mut machine = machine(config(core));
    machine.handle(frame_event(
        frame::Body::PairingRequest(request_qr()),
        "192.168.1.20",
    ));
    let out = machine.handle(Event::PairingDeadline);
    assert_eq!(out.len(), 2, "{out:?}");
    match body(&out[0]) {
        frame::Body::Error(error) => {
            assert_eq!(error.status.code, 206);
            assert!(error.status.detail.is_empty());
        }
        other => panic!("expected Error(206), got {other:?}"),
    }
    match body(&out[1]) {
        frame::Body::Close(close) => assert_eq!(close.code, 206),
        other => panic!("expected Close(206), got {other:?}"),
    }
    assert_eq!(machine.state(), ServerState::Closing);
}

#[test]
fn five_pairing_failures_from_one_address_close_302() {
    let clock = TestClock::new();
    let core = core_with_nonce(clock);
    // No nonce is inserted for these attempts: every request fails admission.
    let mut outcomes_by_attempt = Vec::new();
    for attempt in 0..5 {
        let mut machine = machine(config(Arc::clone(&core)));
        let mut request = request_qr();
        request.request_id = attempt + 1;
        request.pairing_nonce = vec![0xee; 16];
        let out = machine.handle(frame_event(
            frame::Body::PairingRequest(request),
            "192.168.1.20",
        ));
        outcomes_by_attempt.push(out);
    }
    for out in &outcomes_by_attempt[..4] {
        match body(&out[0]) {
            frame::Body::PairingResult(result) => assert_eq!(result.status.code, 201),
            other => panic!("expected PairingResult(201), got {other:?}"),
        }
    }
    // TM-5: the 5th failure trips the address budget → Close(302), no
    // PairingResult.
    let fifth = &outcomes_by_attempt[4];
    assert_eq!(fifth.len(), 1, "{fifth:?}");
    match body(&fifth[0]) {
        frame::Body::Close(close) => assert_eq!(close.code, 302),
        other => panic!("expected Close(302), got {other:?}"),
    }
    assert!(core.is_limited("192.168.1.20".parse().unwrap()));
}

#[test]
fn sas_window_pairs_without_a_nonce_and_derives_the_fixture_sas() {
    let clock = TestClock::new();
    let store = Arc::new(AuthorizationStore::in_memory());
    let core = Arc::new(PairingCore::with_clock(
        unhex(SERVER_PUB),
        store.clone(),
        clock.clone(),
    ));
    core.begin_sas();
    let mut machine = machine(config_with_exporter(
        Arc::clone(&core),
        FixtureExporter::SAS,
    ));
    let out = machine.handle(frame_event(
        frame::Body::PairingRequest(request_sas()),
        "192.168.1.20",
    ));
    let response = expect_pairing_response(&out);
    assert_eq!(response.pairing_mode, PairingMode::Sas as i32);
    assert!(response.pairing_nonce.is_empty());
    assert_eq!(hex(&response.binding), SAS_BINDING);
    machine.handle(frame_event(
        frame::Body::PairingConfirm(PairingConfirm {
            request_id: 1,
            binding_echo: unhex(SAS_BINDING),
            client_proof: unhex(SAS_PROOF),
        }),
        "192.168.1.20",
    ));
    let approval = machine.take_pairing_approval_request().expect("approval");
    assert_eq!(approval.sas.as_deref(), Some("900 067"));
    let out = machine.handle(Event::PairingApproved { request_id: 1 });
    match body(&out[0]) {
        frame::Body::PairingResult(result) => assert_eq!(result.status.code, 0),
        other => panic!("expected PairingResult(OK), got {other:?}"),
    }
    assert_eq!(store.clients().len(), 1);
}

#[test]
fn pairing_confirm_without_an_admitted_exchange_is_201() {
    let clock = TestClock::new();
    let core = core_with_nonce(clock);
    let mut machine = machine(config(core));
    // Enter AUTHENTICATING through the auth exchange, but never admit a
    // pairing. A PairingConfirm is then a pairing failure (201+Close(201)).
    machine.handle(frame_event(
        frame::Body::AuthInit(v2_transport::schema::AuthInit {
            request_id: 9,
            client_public_key: unhex(CLIENT_PUB),
        }),
        "192.168.1.20",
    ));
    assert_eq!(machine.state(), ServerState::Authenticating);
    let out = machine.handle(frame_event(
        frame::Body::PairingConfirm(PairingConfirm {
            request_id: 1,
            binding_echo: unhex(QR_BINDING),
            client_proof: unhex(QR_PROOF),
        }),
        "192.168.1.20",
    ));
    assert_eq!(out.len(), 2, "{out:?}");
    match body(&out[0]) {
        frame::Body::PairingResult(result) => assert_eq!(result.status.code, 201),
        other => panic!("expected PairingResult(201), got {other:?}"),
    }
}
