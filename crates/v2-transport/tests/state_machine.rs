//! The §5.2 Server transition table, exercised row by row through the pure
//! domain machine (`v2_transport::state_machine`). Fixture constants come from
//! `fixtures/wire-protocol-v2/` (test-keys.json); bytes are asserted against
//! the recorded frame bodies where the fixtures pin them.

use v2_transport::auth_store::{AuthorizationStore, ClientState, StoredClient};
use v2_transport::codes::ErrorCode;
use v2_transport::schema::{frame, Frame, Status};
use v2_transport::state_machine::{Event, ServerConfig, ServerState, StateMachine};

const CLIENT_PUB: &str = "03d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d";
const SERVER_PUB: &str = "11b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c";

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn auth_init(id: u64) -> Frame {
    Frame {
        body: Some(frame::Body::AuthInit(v2_transport::schema::AuthInit {
            request_id: id,
            client_public_key: unhex(CLIENT_PUB),
        })),
    }
}

fn auth_proof(id: u64, sig_hex: &str) -> Frame {
    Frame {
        body: Some(frame::Body::AuthProof(v2_transport::schema::AuthProof {
            request_id: id,
            signature: unhex(sig_hex),
        })),
    }
}

fn negotiate(id: u64, capabilities: &[u32]) -> Frame {
    Frame {
        body: Some(frame::Body::Negotiate(v2_transport::schema::Negotiate {
            request_id: id,
            protocol_major: 2,
            protocol_minor: 0,
            capabilities: capabilities.to_vec(),
        })),
    }
}

fn subscribe(id: u64) -> Frame {
    Frame {
        body: Some(frame::Body::Subscribe(v2_transport::schema::Subscribe {
            request_id: id,
        })),
    }
}

fn unsubscribe(id: u64) -> Frame {
    Frame {
        body: Some(frame::Body::Unsubscribe(
            v2_transport::schema::Unsubscribe { request_id: id },
        )),
    }
}

fn request_configuration(id: u64) -> Frame {
    Frame {
        body: Some(frame::Body::RequestConfiguration(
            v2_transport::schema::RequestConfiguration { request_id: id },
        )),
    }
}

fn close_frame(code: u32) -> Frame {
    Frame {
        body: Some(frame::Body::Close(v2_transport::schema::Close {
            code,
            detail: String::new(),
        })),
    }
}

fn malformed_error(request_id: u64) -> Frame {
    // §4.4: status.code == 0 is malformed in every state.
    Frame {
        body: Some(frame::Body::Error(v2_transport::schema::Error {
            request_id,
            status: Status::ok(),
        })),
    }
}

/// The recorded happy-path signature (control.auth-happy-path step 3).
const VALID_SIG: &str = "e77e55090b72af3cb8f42a8e6696b4797170b8c8236944ce4c17164e5d97762f8686dfcd64f4a21f5768fa6c94e6a816029a566fb849fa27cbe98385f4df060c";
/// error-203's corrupted signature (final byte flipped).
const INVALID_SIG: &str = "e77e55090b72af3cb8f42a8e6696b4797170b8c8236944ce4c17164e5d97762f8686dfcd64f4a21f5768fa6c94e6a816029a566fb849fa27cbe98385f4df060b";

fn store_with_client() -> AuthorizationStore {
    let store = AuthorizationStore::in_memory();
    store.insert_for_tests(StoredClient {
        client_id: "cf7940b474310246".to_owned(),
        public_key: unhex(CLIENT_PUB),
        display_name: "Test Phone".to_owned(),
        state: ClientState::Active,
    });
    store
}

fn machine(store: &AuthorizationStore) -> StateMachine {
    machine_on(std::sync::Arc::new(clone_store(store)))
}

/// A machine sharing one store (revocation is visible across machines).
fn machine_on(store: std::sync::Arc<AuthorizationStore>) -> StateMachine {
    StateMachine::new(
        ServerConfig {
            server_public_key: unhex(SERVER_PUB),
            protocol_minor: 0,
            current_configuration: Some(v2_transport::schema::Configuration {
                config_generation: 1,
                codec: v2_transport::schema::Codec::Pcm as i32,
                pcm: Some(v2_transport::schema::PcmParameters {
                    sample_rate: 48_000,
                    channels: 2,
                    sample_format: v2_transport::schema::SampleFormat::F32Le as i32,
                }),
            }),
            capture_active: true,
            capture_healthy: true,
            max_auth_failures_per_connection: 5,
            // Golden-vector replay constants (test-keys.json): the pinned
            // challenge randomness and the TLS-Exporter stand-in.
            server_random_override: Some(unhex(
                "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            )),
            auth_exporter: Some(unhex(
                "c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf",
            )),
            session_id_base: 1,
        },
        &store,
    )
}

fn clone_store(store: &AuthorizationStore) -> AuthorizationStore {
    // Tests construct an Arc-shareable store per machine; records carry over.
    let fresh = AuthorizationStore::in_memory();
    for client in store.clients() {
        fresh.insert_for_tests(client);
    }
    fresh
}

/// Drive CONNECTING → NEGOTIATING with the fixture's valid signature.
fn authenticate(machine: &mut StateMachine) {
    let out = machine.handle(Event::Frame {
        frame: Box::new(auth_init(1)),
        source: unhex(CLIENT_PUB),
    });
    assert!(
        matches!(
            out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
            Some(frame::Body::AuthChallenge(_))
        ),
        "challenge first, got {out:?}"
    );
    let out = machine.handle(Event::Frame {
        frame: Box::new(auth_proof(1, VALID_SIG)),
        source: unhex(CLIENT_PUB),
    });
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::AuthResult(r)) if r.status.code == 0),
        "auth ok, got {out:?}"
    );
}

// ---------------------------------------------------------------------------
// Row-by-row: the exchange works end to end.
// ---------------------------------------------------------------------------

#[test]
fn connect_auth_negotiate_reaches_ready() {
    let store = store_with_client();
    let mut machine = machine(&store);
    assert_eq!(machine.state(), ServerState::Connecting);
    authenticate(&mut machine);
    assert_eq!(machine.state(), ServerState::Negotiating);
    // control.negotiate-happy-path: request_id 2 (auth consumed 1).
    let out = machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Ready);
    assert_eq!(
        out.len(),
        2,
        "NegotiateResponse + ConfigurationPublished push"
    );
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::NegotiateResponse(r)) if r.status.code == 0 && r.protocol_major == 2 && r.protocol_minor == 0 && r.capabilities.is_empty())
    );
    // The push carries request_id 0 and the current configuration (fixture
    // control.negotiate-happy-path step 3: 48 kHz stereo F32LE, generation 1).
    assert!(
        matches!(out[1].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::ConfigurationPublished(p)) if p.request_id == 0)
    );
}

// ---------------------------------------------------------------------------
// §5.2 CONNECTING rows.
// ---------------------------------------------------------------------------

#[test]
fn connecting_valid_subscribe_is_fatal_402_with_echo() {
    // control.connecting-valid-subscribe-402: valid frame, state-forbidden →
    // Error(402) echoing request_id 1 + Close(402) + transport close 402.
    let store = store_with_client();
    let mut machine = machine(&store);
    let out = machine.handle(Event::Frame {
        frame: Box::new(subscribe(1)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Closing);
    assert_eq!(out.len(), 2);
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::Error(e)) if e.request_id == 1 && e.status.code == 402)
    );
    assert_eq!(out[1].close_code, Some(ErrorCode::INVALID_STATE));
}

#[test]
fn connecting_malformed_frame_is_fatal_401() {
    // Generic validation failure: an Error frame with status.code == 0 (§4.4
    // validation table) is malformed → 401.
    let store = store_with_client();
    let mut machine = machine(&store);
    let bad = Frame {
        body: Some(frame::Body::Error(v2_transport::schema::Error {
            request_id: 0,
            status: Status::ok(),
        })),
    };
    let out = machine.handle(Event::Frame {
        frame: Box::new(bad),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Closing);
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::Error(e)) if e.status.code == 401 && e.request_id == 0)
    );
    assert_eq!(out[1].close_code, Some(ErrorCode::MALFORMED_MESSAGE));
}

fn assert_error_code_zero_is_fatal_401(mut machine: StateMachine, request_id: u64) {
    let out = machine.handle(Event::Frame {
        frame: Box::new(malformed_error(request_id)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Closing);
    assert_eq!(out.len(), 2, "malformed input earns an Error + Close pair");
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::Error(e)) if e.status.code == 401 && e.request_id == request_id)
    );
    assert_eq!(out[1].close_code, Some(ErrorCode::MALFORMED_MESSAGE));
}

#[test]
fn authenticating_error_code_zero_is_fatal_401() {
    // §4.4: code==0 is malformed regardless of the state's request table.
    let store = store_with_client();
    let mut machine = machine(&store);
    machine.handle(Event::Frame {
        frame: Box::new(auth_init(1)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Authenticating);
    assert_error_code_zero_is_fatal_401(machine, 7);
}

#[test]
fn negotiating_error_code_zero_is_fatal_401() {
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    assert_eq!(machine.state(), ServerState::Negotiating);
    assert_error_code_zero_is_fatal_401(machine, 8);
}

#[test]
fn ready_error_code_zero_is_fatal_401() {
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Ready);
    assert_error_code_zero_is_fatal_401(machine, 9);
}

// ---------------------------------------------------------------------------
// §5.2 AUTHENTICATING rows.
// ---------------------------------------------------------------------------

#[test]
fn valid_proof_unknown_identity_is_202_connection_stays() {
    // control.error-202-authorization-unknown: valid signature, no record →
    // AuthResult(202, requires_repairing=true), NO Close, state stays.
    let store = AuthorizationStore::in_memory();
    let mut machine = machine(&store);
    let out = machine.handle(Event::Frame {
        frame: Box::new(auth_init(1)),
        source: unhex(CLIENT_PUB),
    });
    assert!(matches!(
        out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::AuthChallenge(_))
    ));
    let out = machine.handle(Event::Frame {
        frame: Box::new(auth_proof(1, VALID_SIG)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Authenticating);
    assert_eq!(out.len(), 1, "non-fatal: no Close rides along");
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::AuthResult(r)) if r.status.code == 202 && r.status.requires_repairing && r.session_id == 0)
    );
}

#[test]
fn valid_proof_revoked_identity_is_204() {
    // control.error-204-authorization-denied: known but revoked → 204 with
    // requires_repairing=false (admin action), connection maintained.
    let store = AuthorizationStore::in_memory();
    store.insert_for_tests(StoredClient {
        client_id: "whatever".to_owned(),
        public_key: unhex(CLIENT_PUB),
        display_name: "Test Phone".to_owned(),
        state: ClientState::Revoked,
    });
    let mut machine = machine(&store);
    machine.handle(Event::Frame {
        frame: Box::new(auth_init(1)),
        source: unhex(CLIENT_PUB),
    });
    let out = machine.handle(Event::Frame {
        frame: Box::new(auth_proof(1, VALID_SIG)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Authenticating);
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::AuthResult(r)) if r.status.code == 204 && !r.status.requires_repairing)
    );
}

#[test]
fn invalid_signature_is_203_and_counts_a_failure() {
    // control.error-203-authentication-failed: AuthResult(203, requires_
    // repairing=true); the connection stays for a retry (below 5 failures).
    let store = store_with_client();
    let mut machine = machine(&store);
    machine.handle(Event::Frame {
        frame: Box::new(auth_init(1)),
        source: unhex(CLIENT_PUB),
    });
    let out = machine.handle(Event::Frame {
        frame: Box::new(auth_proof(1, INVALID_SIG)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Authenticating);
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::AuthResult(r)) if r.status.code == 203 && r.status.requires_repairing)
    );
    assert_eq!(machine.auth_failures(), 1);
}

#[test]
fn auth_failure_log_code_matches_the_reason() {
    // The driver logs the §6 code of the actual failure (ticket 09: 202/204
    // were invisible because every line said 203).
    let store = store_with_client();
    let mut checked = machine(&store);
    checked.handle(Event::Frame {
        frame: Box::new(auth_init(1)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(
        checked.take_auth_failure_code(),
        None,
        "a challenge is not a failure"
    );
    checked.handle(Event::Frame {
        frame: Box::new(auth_proof(1, INVALID_SIG)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(
        checked.take_auth_failure_code().map(|c| c.code),
        Some(203),
        "invalid proof"
    );
    assert_eq!(checked.take_auth_failure_code(), None, "drained once");

    let mut unknown = machine(&AuthorizationStore::in_memory());
    unknown.handle(Event::Frame {
        frame: Box::new(auth_init(1)),
        source: unhex(CLIENT_PUB),
    });
    unknown.handle(Event::Frame {
        frame: Box::new(auth_proof(1, VALID_SIG)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(
        unknown.take_auth_failure_code().map(|c| c.code),
        Some(202),
        "unknown identity"
    );

    let revoked = AuthorizationStore::in_memory();
    revoked.insert_for_tests(StoredClient {
        client_id: "cf7940b474310246".to_owned(),
        public_key: unhex(CLIENT_PUB),
        display_name: "Test Phone".to_owned(),
        state: ClientState::Revoked,
    });
    let mut denied = machine(&revoked);
    denied.handle(Event::Frame {
        frame: Box::new(auth_init(1)),
        source: unhex(CLIENT_PUB),
    });
    denied.handle(Event::Frame {
        frame: Box::new(auth_proof(1, VALID_SIG)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(
        denied.take_auth_failure_code().map(|c| c.code),
        Some(204),
        "revoked identity"
    );

    let mut malformed = machine(&store_with_client());
    let bad_key = Frame {
        body: Some(frame::Body::AuthInit(v2_transport::schema::AuthInit {
            request_id: 1,
            client_public_key: vec![0xff; 33],
        })),
    };
    malformed.handle(Event::Frame {
        frame: Box::new(bad_key),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(
        malformed.take_auth_failure_code().map(|c| c.code),
        Some(401),
        "malformed client key"
    );
}

#[test]
fn auth_init_retry_gets_a_fresh_challenge() {
    // §5.2 AUTHENTICATING AuthInit (retry): a fresh challenge, still
    // AUTHENTICATING.
    let store = store_with_client();
    let mut machine = machine(&store);
    machine.handle(Event::Frame {
        frame: Box::new(auth_init(1)),
        source: unhex(CLIENT_PUB),
    });
    let out = machine.handle(Event::Frame {
        frame: Box::new(auth_init(2)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Authenticating);
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::AuthChallenge(c)) if c.request_id == 2)
    );
}

#[test]
fn fifth_auth_failure_closes_with_203() {
    // §5.2/TM-4: 5 failures per connection → Close(203). Four 203s first, the
    // 5th carries the Close.
    let store = AuthorizationStore::in_memory();
    let mut machine = machine(&store);
    for _ in 0..4 {
        machine.handle(Event::Frame {
            frame: Box::new(auth_init(1)),
            source: unhex(CLIENT_PUB),
        });
        let out = machine.handle(Event::Frame {
            frame: Box::new(auth_proof(1, INVALID_SIG)),
            source: unhex(CLIENT_PUB),
        });
        assert_eq!(out.len(), 1, "failures 1–4 are non-fatal AuthResult(203)");
    }
    machine.handle(Event::Frame {
        frame: Box::new(auth_init(1)),
        source: unhex(CLIENT_PUB),
    });
    let out = machine.handle(Event::Frame {
        frame: Box::new(auth_proof(1, INVALID_SIG)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Closing);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].close_code, Some(ErrorCode::AUTHENTICATION_FAILED));
}

#[test]
fn auth_proof_without_challenge_is_402_fatal() {
    // A well-formed AuthProof in CONNECTING (no AuthInit first): the auth
    // rows need a pending challenge; a stray AuthProof is state-forbidden.
    let store = store_with_client();
    let mut machine = machine(&store);
    let out = machine.handle(Event::Frame {
        frame: Box::new(auth_proof(1, VALID_SIG)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Closing);
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::Error(e)) if e.status.code == 402 && e.request_id == 1)
    );
    assert_eq!(out[1].close_code, Some(ErrorCode::INVALID_STATE));
}

// ---------------------------------------------------------------------------
// §5.2 NEGOTIATING rows.
// ---------------------------------------------------------------------------

#[test]
fn negotiate_wrong_major_is_101_fatal() {
    // control.error-101-version-mismatch: major 1 → Error(101)+Close(101).
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    let bad = Frame {
        body: Some(frame::Body::Negotiate(v2_transport::schema::Negotiate {
            request_id: 2,
            protocol_major: 1,
            protocol_minor: 0,
            capabilities: Vec::new(),
        })),
    };
    let out = machine.handle(Event::Frame {
        frame: Box::new(bad),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Closing);
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::Error(e)) if e.request_id == 2 && e.status.code == 101)
    );
    assert_eq!(out[1].close_code, Some(ErrorCode::VERSION_MISMATCH));
}

#[test]
fn negotiate_minor_above_servers_is_101_fatal() {
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    let bad = Frame {
        body: Some(frame::Body::Negotiate(v2_transport::schema::Negotiate {
            request_id: 2,
            protocol_major: 2,
            protocol_minor: 1, // Server's minor is 0
            capabilities: Vec::new(),
        })),
    };
    let out = machine.handle(Event::Frame {
        frame: Box::new(bad),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Closing);
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::Error(e)) if e.status.code == 101)
    );
}

#[test]
fn negotiate_non_ascending_capabilities_is_401_fatal() {
    // control.malformed-capability-list-negotiate-401: [2, 1] →
    // Error(401) echoing request_id 2 + Close(401).
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    let out = machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[2, 1])),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Closing);
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::Error(e)) if e.request_id == 2 && e.status.code == 401)
    );
    assert_eq!(out[1].close_code, Some(ErrorCode::MALFORMED_MESSAGE));
}

#[test]
fn negotiate_unknown_capability_is_ignored_intersection_empty() {
    // control.negotiate-unknown-capability-ignored: id 7 unknown-but-optional
    // → accepted; the honored intersection is [] (no ids exist in v2.0).
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    let out = machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[7])),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Ready);
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::NegotiateResponse(r)) if r.capabilities.is_empty() && r.status.code == 0)
    );
}

#[test]
fn negotiating_subscribe_is_402_fatal() {
    // control.subscribe-before-ready-402: Subscribe in NEGOTIATING →
    // Error(402) + Close(402).
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    let out = machine.handle(Event::Frame {
        frame: Box::new(subscribe(1)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Closing);
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::Error(e)) if e.request_id == 1 && e.status.code == 402)
    );
    assert_eq!(out[1].close_code, Some(ErrorCode::INVALID_STATE));
}

#[test]
fn negotiating_auth_init_is_402_fatal() {
    // Once authenticated, re-authenticating in place is never legal (§5.2:
    // "Re-authentication of a live session is never in-place"); a fresh
    // session supersedes via 303 instead.
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    let out = machine.handle(Event::Frame {
        frame: Box::new(auth_init(9)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Closing);
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::Error(e)) if e.status.code == 402 && e.request_id == 9)
    );
}

// ---------------------------------------------------------------------------
// §5.2 READY rows.
// ---------------------------------------------------------------------------

#[test]
fn ready_subscribe_ok_with_configuration() {
    // control.subscribe-config-diagnostics step 1–2.
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    let out = machine.handle(Event::Frame {
        frame: Box::new(subscribe(3)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Ready);
    assert!(machine.subscribed());
    assert_eq!(
        out.len(),
        2,
        "SubscribeResponse + StreamStarted push (capture active)"
    );
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::SubscribeResponse(r)) if r.status.code == 0 && r.configuration.is_some())
    );
    assert!(
        matches!(out[1].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::StreamStarted(s)) if s.stream_instance_id == 1 && s.config_generation == 1 && s.initial_timestamp == 0)
    );
}

#[test]
fn ready_duplicate_subscribe_is_nonfatal_402_connection_maintained() {
    // control.error-402-duplicate-subscribe-nonfatal: Error(402) ONLY (no
    // Close), state unchanged, the next request is still served.
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    machine.handle(Event::Frame {
        frame: Box::new(subscribe(3)),
        source: unhex(CLIENT_PUB),
    });
    let out = machine.handle(Event::Frame {
        frame: Box::new(subscribe(4)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Ready, "no state change");
    assert_eq!(out.len(), 1, "non-fatal: no Close");
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::Error(e)) if e.request_id == 4 && e.status.code == 402 && !e.status.retryable)
    );
    // The connection is maintained: RequestConfiguration is still served
    // (the fixture's follow-up proves it).
    let out = machine.handle(Event::Frame {
        frame: Box::new(request_configuration(5)),
        source: unhex(CLIENT_PUB),
    });
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::ConfigurationPublished(p)) if p.request_id == 5)
    );
}

#[test]
fn ready_unsubscribe_is_idempotent() {
    // §5.2: Unsubscribe → OK, including duplicate/late Unsubscribe.
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    let out = machine.handle(Event::Frame {
        frame: Box::new(unsubscribe(3)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(out.len(), 1);
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::UnsubscribeResponse(r)) if r.status.code == 0)
    );
    // Duplicate/late Unsubscribe also gets OK (idempotent).
    let out = machine.handle(Event::Frame {
        frame: Box::new(unsubscribe(4)),
        source: unhex(CLIENT_PUB),
    });
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::UnsubscribeResponse(r)) if r.status.code == 0)
    );
    assert_eq!(machine.state(), ServerState::Ready);
}

#[test]
fn ready_unsubscribe_returns_to_unsubscribed_and_allows_resubscribe() {
    // §5.2: READY Unsubscribe clears the subscription, so the next Subscribe
    // is a fresh request rather than a duplicate (402). Regression: the flag
    // used to stay true and the re-Subscribe was refused.
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    machine.handle(Event::Frame {
        frame: Box::new(subscribe(3)),
        source: unhex(CLIENT_PUB),
    });
    assert!(machine.subscribed(), "precondition: subscribed");

    let out = machine.handle(Event::Frame {
        frame: Box::new(unsubscribe(4)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Ready);
    assert!(
        !machine.subscribed(),
        "Unsubscribe must clear READY's subscription"
    );
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::UnsubscribeResponse(r)) if r.request_id == 4 && r.status.code == 0)
    );

    let out = machine.handle(Event::Frame {
        frame: Box::new(subscribe(5)),
        source: unhex(CLIENT_PUB),
    });
    assert!(machine.subscribed());
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::SubscribeResponse(r)) if r.request_id == 5 && r.status.code == 0)
    );
}

#[test]
fn ready_request_configuration_echoes_request_id() {
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    let out = machine.handle(Event::Frame {
        frame: Box::new(request_configuration(3)),
        source: unhex(CLIENT_PUB),
    });
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::ConfigurationPublished(p)) if p.request_id == 3 && p.configuration.is_some())
    );
}

#[test]
fn ready_capture_unhealthy_subscribe_is_501_retryable() {
    // control.error-501-capture-unhealthy: Error(501, retryable=true), no
    // Close, state unchanged.
    let store = store_with_client();
    let mut machine = StateMachine::new(
        ServerConfig {
            capture_healthy: false,
            ..machine(&store).config().clone()
        },
        &std::sync::Arc::new(clone_store(&store)),
    );
    authenticate(&mut machine);
    machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    let out = machine.handle(Event::Frame {
        frame: Box::new(subscribe(3)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Ready);
    assert_eq!(out.len(), 1, "non-fatal");
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::Error(e)) if e.request_id == 3 && e.status.code == 501 && e.status.retryable)
    );
}

#[test]
fn ready_close_received_moves_to_closing_without_echo() {
    // control.error-701-client-stopping: Close(701) → CLOSING, no echo.
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    let out = machine.handle(Event::Frame {
        frame: Box::new(close_frame(701)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Closing);
    assert!(out.is_empty(), "Close is one-way and never echoed (§5.4)");
}

#[test]
fn close_with_code_zero_is_malformed_401() {
    // §4.4 validation table + spec:264/:419: a Close whose code is 0 is
    // malformed, not a one-way close. The Server answers with the fatal
    // Error(401) + Close(401) pair and closes the transport with 401.
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    let out = machine.handle(Event::Frame {
        frame: Box::new(close_frame(0)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Closing);
    assert_eq!(out.len(), 2, "malformed input earns an Error + Close pair");
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::Error(e)) if e.status.code == 401)
    );
    assert_eq!(out[1].close_code, Some(ErrorCode::MALFORMED_MESSAGE));
}

// ---------------------------------------------------------------------------
// §5.2 "any" rows.
// ---------------------------------------------------------------------------

#[test]
fn unsolicited_nonfatal_error_is_advisory_only() {
    // §5.2 "any": non-fatal Error received (no paired Close) — advisory, no
    // state change. The Client's 403 report is the canonical instance.
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    let advisory = Frame {
        body: Some(frame::Body::Error(v2_transport::schema::Error {
            request_id: 0,
            status: v2_transport::schema::Status::registry(403),
        })),
    };
    let out = machine.handle(Event::Frame {
        frame: Box::new(advisory),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Ready, "advisory only");
    assert!(out.is_empty(), "no reply to an advisory Error");
    // The session still serves requests.
    let out = machine.handle(Event::Frame {
        frame: Box::new(request_configuration(3)),
        source: unhex(CLIENT_PUB),
    });
    assert!(matches!(
        out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::ConfigurationPublished(_))
    ));
}

#[test]
fn fatal_error_class_sends_error_plus_close_and_stops_processing() {
    // §5.2 "any": fatal error raised → matching Error + Close, and CLOSING
    // accepts no new requests.
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    let out = machine.handle(Event::Fatal(ErrorCode::INTERNAL_ERROR));
    assert_eq!(machine.state(), ServerState::Closing);
    assert_eq!(out.len(), 2);
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::Error(e)) if e.request_id == 0 && e.status.code == 599)
    );
    assert_eq!(out[1].close_code, Some(ErrorCode::INTERNAL_ERROR));
    // CLOSING accepts no new requests (§5.2).
    let out = machine.handle(Event::Frame {
        frame: Box::new(request_configuration(4)),
        source: unhex(CLIENT_PUB),
    });
    assert!(out.is_empty());
}

#[test]
fn closing_rejects_everything_and_transport_lost_absorbs() {
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    machine.handle(Event::Frame {
        frame: Box::new(close_frame(701)),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Closing);
    machine.handle(Event::TransportLost);
    assert_eq!(machine.state(), ServerState::Closed);
    // CLOSED absorbs everything (§5.4).
    machine.handle(Event::TransportLost);
    assert_eq!(machine.state(), ServerState::Closed);
}

#[test]
fn idle_timeout_closes_with_601() {
    // §5.2 "any": idle timeout → transport close carrying 601.
    let store = store_with_client();
    let mut machine = machine(&store);
    authenticate(&mut machine);
    machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    let out = machine.handle(Event::IdleTimeout);
    assert_eq!(machine.state(), ServerState::Closed);
    assert_eq!(
        out[0].transport_close_code,
        Some(ErrorCode::NETWORK_FAILURE)
    );
}

// ---------------------------------------------------------------------------
// §5.2 supersede (per-Client 1 session, TM-3; 06's leftover).
// ---------------------------------------------------------------------------

#[test]
fn supersede_closes_the_old_session_with_303_and_keeps_the_new() {
    // control.error-303-session-superseded: connection B authenticates the
    // same identity; connection A receives Close(303) alone and its transport
    // close carries 303. The new session proceeds normally.
    let store = store_with_client();
    let mut old = machine(&store);
    authenticate(&mut old);
    old.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(old.state(), ServerState::Ready);
    assert_eq!(old.session_id(), Some(1));

    let mut new = machine(&store);
    // The registry callback records supersede requests for the accept loop.
    let out = new.handle(Event::Frame {
        frame: Box::new(auth_init(1)),
        source: unhex(CLIENT_PUB),
    });
    assert!(matches!(
        out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::AuthChallenge(_))
    ));
    let out = new.handle(Event::Frame {
        frame: Box::new(auth_proof(1, VALID_SIG)),
        source: unhex(CLIENT_PUB),
    });
    // The new session is admitted (in production the session registry assigns
    // the next id — the fixture's 2; this per-machine counter model assigns
    // from 1 and the registry's numbering is asserted at the driver level).
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::AuthResult(r)) if r.status.code == 0 && r.session_id >= 1),
        "the new session is admitted: {out:?}"
    );
    assert_eq!(new.state(), ServerState::Negotiating);
    // The old session was marked superseded for the accept loop to close:
    // Close(303) on connection A, transport close 303 (spec §5.2, fixture).
    assert_eq!(
        new.supersede_requested(),
        Some(ErrorCode::SESSION_SUPERSEDED)
    );
}

// ---------------------------------------------------------------------------
// Revocation (pairing.md §9.2, TM-9): ≤ 5 s termination of active sessions.
// ---------------------------------------------------------------------------

#[test]
fn deleted_record_is_revoked_on_the_next_check() {
    // pairing.md §9.2: deleting the record (not just setting state=revoked)
    // must end the live session with the 204 pair within the poll window.
    let dir = tempfile::tempdir().expect("tempdir");
    let store = AuthorizationStore::open(dir.path()).expect("open");
    store
        .upsert(StoredClient {
            client_id: "cf7940b474310246".to_owned(),
            public_key: unhex(CLIENT_PUB),
            display_name: "Test Phone".to_owned(),
            state: ClientState::Active,
        })
        .expect("persist");
    let store = std::sync::Arc::new(store);
    let mut machine = machine_on(std::sync::Arc::clone(&store));
    authenticate(&mut machine);
    machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Ready);

    std::fs::remove_file(dir.path().join("pairing").join("clients.json")).expect("delete record");
    assert!(
        store.reload_if_changed().expect("reload"),
        "deletion is a change"
    );
    let out = machine.handle(Event::RevocationCheck);
    assert_eq!(machine.state(), ServerState::Closing);
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::Error(e)) if e.request_id == 0 && e.status.code == 204)
    );
    assert_eq!(out[1].close_code, Some(ErrorCode::AUTHORIZATION_DENIED));
}

#[test]
fn revoked_active_session_receives_fatal_204_pair() {
    // control.error-204-revoked-active-session: unsolicited Error(204) +
    // Close(204) + transport close 204.
    let store = std::sync::Arc::new(store_with_client());
    let mut machine = machine_on(std::sync::Arc::clone(&store));
    authenticate(&mut machine);
    machine.handle(Event::Frame {
        frame: Box::new(negotiate(2, &[])),
        source: unhex(CLIENT_PUB),
    });
    assert_eq!(machine.state(), ServerState::Ready);
    // The operator revokes while the session is live; the machine shares the
    // store, so the next §9.2 check sees it (the driver polls every ≤ 5 s).
    store.revoke(&unhex(CLIENT_PUB)).expect("revoke");
    let out = machine.handle(Event::RevocationCheck);
    assert_eq!(machine.state(), ServerState::Closing);
    assert_eq!(out.len(), 2);
    assert!(
        matches!(out[0].frame.as_ref().and_then(|f| f.body.as_ref()),
        Some(frame::Body::Error(e)) if e.request_id == 0 && e.status.code == 204)
    );
    assert_eq!(out[1].close_code, Some(ErrorCode::AUTHORIZATION_DENIED));
}
