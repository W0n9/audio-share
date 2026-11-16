//! M4 §5.2 PLAYING rows and stream lifecycle, through the pure domain machine.
//! Expected frame order comes from the spec table and
//! `fixtures/wire-protocol-v2/control/subscribe-config-diagnostics.json`.

use std::sync::Arc;

use v2_transport::auth_store::{AuthorizationStore, ClientState as StoreState, StoredClient};
use v2_transport::codes::ErrorCode;
use v2_transport::schema::{frame, PlaybackDiagnostics, StreamEnded, StreamStarted};
use v2_transport::state_machine::{
    CaptureSnapshot, Event, ServerConfig, ServerState, StateMachine, StreamLifecycle,
};

const CLIENT_PUB: &str = "03d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d";
const SERVER_PUB: &str = "11b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c";
const VALID_SIG: &str = "e77e55090b72af3cb8f42a8e6696b4797170b8c8236944ce4c17164e5d97762f8686dfcd64f4a21f5768fa6c94e6a816029a566fb849fa27cbe98385f4df060c";

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn body(frame: Option<&frame::Body>) -> Option<&frame::Body> {
    frame
}

fn active_snapshot(instance: u32, generation: u32, next_frame: u64) -> CaptureSnapshot {
    CaptureSnapshot {
        configuration: Some(v2_transport::schema::Configuration {
            config_generation: generation,
            codec: v2_transport::schema::Codec::Pcm as i32,
            pcm: Some(v2_transport::schema::PcmParameters {
                sample_rate: 48_000,
                channels: 2,
                sample_format: v2_transport::schema::SampleFormat::F32Le as i32,
            }),
        }),
        healthy: true,
        stream_active: true,
        stream_instance_id: instance,
        next_frame_timestamp: next_frame,
    }
}

fn machine_at(stream_active: bool) -> StateMachine {
    let store = AuthorizationStore::in_memory();
    store.insert_for_tests(StoredClient {
        client_id: "cf7940b474310246".to_owned(),
        public_key: unhex(CLIENT_PUB),
        display_name: "Test Phone".to_owned(),
        state: StoreState::Active,
        created_at: String::new(),
        last_seen_at: String::new(),
    });
    let capture = if stream_active {
        active_snapshot(1, 1, 0)
    } else {
        CaptureSnapshot {
            configuration: active_snapshot(1, 1, 0).configuration,
            stream_active: false,
            ..CaptureSnapshot::absent()
        }
    };
    StateMachine::new(
        ServerConfig {
            server_public_key: unhex(SERVER_PUB),
            protocol_minor: 0,
            capture,
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
        &Arc::new(store),
    )
}

fn frame(id: u64) -> frame::Body {
    frame::Body::AuthInit(v2_transport::schema::AuthInit {
        request_id: id,
        client_public_key: unhex(CLIENT_PUB),
    })
}

fn ready(stream_active: bool) -> StateMachine {
    let mut machine = machine_at(stream_active);
    machine.handle(Event::Frame {
        frame: Box::new(v2_transport::schema::Frame {
            body: Some(frame(1)),
        }),
        source: Vec::new(),
    });
    machine.handle(Event::Frame {
        frame: Box::new(v2_transport::schema::Frame {
            body: Some(frame::Body::AuthProof(v2_transport::schema::AuthProof {
                request_id: 1,
                signature: unhex(VALID_SIG),
            })),
        }),
        source: Vec::new(),
    });
    machine.handle(Event::Frame {
        frame: Box::new(v2_transport::schema::Frame {
            body: Some(frame::Body::Negotiate(v2_transport::schema::Negotiate {
                request_id: 2,
                protocol_major: 2,
                protocol_minor: 0,
                capabilities: Vec::new(),
            })),
        }),
        source: Vec::new(),
    });
    assert_eq!(machine.state(), ServerState::Ready);
    machine
}

fn subscribe(machine: &mut StateMachine, id: u64) -> Vec<v2_transport::state_machine::Outcome> {
    machine.handle(Event::Frame {
        frame: Box::new(v2_transport::schema::Frame {
            body: Some(frame::Body::Subscribe(v2_transport::schema::Subscribe {
                request_id: id,
            })),
        }),
        source: Vec::new(),
    })
}

fn playing() -> StateMachine {
    let mut machine = ready(true);
    let out = subscribe(&mut machine, 3);
    assert!(
        matches!(
            body(out[1].frame.as_ref().and_then(|f| f.body.as_ref())),
            Some(frame::Body::StreamStarted(_))
        ),
        "Subscribe to an active stream pushes StreamStarted: {out:?}"
    );
    assert_eq!(machine.state(), ServerState::Playing);
    machine
}

/// §5.2 READY: Subscribe to an active stream is answered first, then
/// StreamStarted carries the live instance, generation and next-frame anchor.
#[test]
fn subscribe_to_an_active_stream_enters_playing_with_the_live_anchor() {
    let mut machine = ready(true);
    machine.update_capture(active_snapshot(4, 2, 96_000));
    let out = subscribe(&mut machine, 3);
    assert_eq!(out.len(), 2);
    assert!(
        matches!(
            body(out[0].frame.as_ref().and_then(|f| f.body.as_ref())),
            Some(frame::Body::SubscribeResponse(r)) if r.request_id == 3 && r.status.code == 0
        ),
        "{out:?}"
    );
    match body(out[1].frame.as_ref().and_then(|f| f.body.as_ref())) {
        Some(frame::Body::StreamStarted(started)) => {
            assert_eq!(started.stream_instance_id, 4);
            assert_eq!(started.config_generation, 2);
            assert_eq!(started.initial_timestamp, 96_000, "late-subscriber anchor");
        }
        other => panic!("expected StreamStarted, got {other:?}"),
    }
    assert_eq!(machine.state(), ServerState::Playing);
}

/// §5.2 PLAYING: a duplicate Subscribe is a non-fatal 402 (connection kept).
#[test]
fn duplicate_subscribe_in_playing_is_non_fatal() {
    let mut machine = playing();
    let out = subscribe(&mut machine, 4);
    assert_eq!(out.len(), 1);
    assert!(matches!(
        body(out[0].frame.as_ref().and_then(|f| f.body.as_ref())),
        Some(frame::Body::Error(e)) if e.request_id == 4 && e.status.code == 402
    ));
    assert_eq!(out[0].close_code, None, "non-fatal: no paired Close");
    assert_eq!(machine.state(), ServerState::Playing);
}

/// §5.2 PLAYING: Unsubscribe pushes StreamEnded(502, "unsubscribed") FIRST,
/// then UnsubscribeResponse(OK); the subscription clears.
#[test]
fn unsubscribe_in_playing_ends_the_stream_first() {
    let mut machine = playing();
    let out = machine.handle(Event::Frame {
        frame: Box::new(v2_transport::schema::Frame {
            body: Some(frame::Body::Unsubscribe(
                v2_transport::schema::Unsubscribe { request_id: 5 },
            )),
        }),
        source: Vec::new(),
    });
    assert_eq!(out.len(), 2, "{out:?}");
    match body(out[0].frame.as_ref().and_then(|f| f.body.as_ref())) {
        Some(frame::Body::StreamEnded(ended)) => {
            assert_eq!(ended.stream_instance_id, 1);
            assert_eq!(ended.code, ErrorCode::STREAM_ENDED.code);
            assert_eq!(ended.detail, "unsubscribed");
        }
        other => panic!("expected StreamEnded first, got {other:?}"),
    }
    assert!(matches!(
        body(out[1].frame.as_ref().and_then(|f| f.body.as_ref())),
        Some(frame::Body::UnsubscribeResponse(r)) if r.request_id == 5 && r.status.code == 0
    ));
    assert_eq!(machine.state(), ServerState::Ready);
    assert!(!machine.subscribed());
}

/// §5.2 PLAYING: capture end (502) returns to READY and keeps the
/// subscription, so a later capture start reaches the same Client.
#[test]
fn capture_end_returns_to_ready_and_keeps_the_subscription() {
    let mut machine = playing();
    let out = machine.handle(Event::StreamLifecycle(StreamLifecycle::Ended {
        stream_instance_id: 1,
        code: ErrorCode::STREAM_ENDED,
    }));
    assert_eq!(machine.state(), ServerState::Ready);
    assert!(
        machine.subscribed(),
        "capture end is not a user unsubscribe"
    );
    match body(out[0].frame.as_ref().and_then(|f| f.body.as_ref())) {
        Some(frame::Body::StreamEnded(ended)) => {
            assert_eq!(ended.stream_instance_id, 1);
            assert_eq!(ended.code, 502);
        }
        other => panic!("expected StreamEnded(502), got {other:?}"),
    }

    // A new capture instance reaches the still-subscribed Client.
    machine.update_capture(active_snapshot(2, 1, 0));
    let out = machine.handle(Event::StreamLifecycle(StreamLifecycle::Started {
        stream_instance_id: 2,
        config_generation: 1,
        initial_timestamp: 0,
    }));
    assert_eq!(machine.state(), ServerState::Playing);
    match body(out[0].frame.as_ref().and_then(|f| f.body.as_ref())) {
        Some(frame::Body::StreamStarted(started)) => {
            assert_eq!(started.stream_instance_id, 2);
        }
        other => panic!("expected StreamStarted, got {other:?}"),
    }
}

/// §5.2 PLAYING: capture failure pushes StreamEnded(501) and returns to READY.
#[test]
fn capture_failure_returns_to_ready_with_501() {
    let mut machine = playing();
    let out = machine.handle(Event::StreamLifecycle(StreamLifecycle::Ended {
        stream_instance_id: 1,
        code: ErrorCode::CAPTURE_FAILURE,
    }));
    match body(out[0].frame.as_ref().and_then(|f| f.body.as_ref())) {
        Some(frame::Body::StreamEnded(ended)) => assert_eq!(ended.code, 501),
        other => panic!("expected StreamEnded(501), got {other:?}"),
    }
    assert_eq!(machine.state(), ServerState::Ready);
    assert!(machine.subscribed());
}

/// §5.2 PLAYING: a capture restart announces the end of the old instance and
/// the start of the new one, in that order.
#[test]
fn capture_restart_ends_then_starts() {
    let mut machine = playing();
    machine.update_capture(active_snapshot(2, 1, 0));
    let out = machine.handle(Event::StreamLifecycle(StreamLifecycle::Restarted {
        old_stream_instance_id: 1,
        stream_instance_id: 2,
        config_generation: 1,
        initial_timestamp: 0,
    }));
    assert_eq!(out.len(), 2);
    assert!(matches!(
        body(out[0].frame.as_ref().and_then(|f| f.body.as_ref())),
        Some(frame::Body::StreamEnded(e)) if e.stream_instance_id == 1 && e.code == 502
    ));
    assert!(matches!(
        body(out[1].frame.as_ref().and_then(|f| f.body.as_ref())),
        Some(frame::Body::StreamStarted(s)) if s.stream_instance_id == 2
    ));
    assert_eq!(machine.state(), ServerState::Playing);
}

/// §5.2 READY: a capture start reaches a subscribed Client even when it was
/// subscribed before the stream began.
#[test]
fn capture_start_reaches_a_client_subscribed_before_the_stream() {
    let mut machine = ready(false);
    let out = subscribe(&mut machine, 3);
    assert_eq!(out.len(), 1, "no StreamStarted while nothing is streaming");
    assert_eq!(machine.state(), ServerState::Ready);

    machine.update_capture(active_snapshot(1, 1, 0));
    let out = machine.handle(Event::StreamLifecycle(StreamLifecycle::Started {
        stream_instance_id: 1,
        config_generation: 1,
        initial_timestamp: 0,
    }));
    assert_eq!(out.len(), 1);
    assert!(matches!(
        body(out[0].frame.as_ref().and_then(|f| f.body.as_ref())),
        Some(frame::Body::StreamStarted(s)) if s.stream_instance_id == 1
    ));
    assert_eq!(machine.state(), ServerState::Playing);
}

/// §5.2 READY: capture starts but nobody is subscribed → no push.
#[test]
fn capture_start_without_a_subscriber_pushes_nothing() {
    let mut machine = ready(false);
    machine.update_capture(active_snapshot(1, 1, 0));
    let out = machine.handle(Event::StreamLifecycle(StreamLifecycle::Started {
        stream_instance_id: 1,
        config_generation: 1,
        initial_timestamp: 0,
    }));
    assert!(out.is_empty(), "{out:?}");
    assert_eq!(machine.state(), ServerState::Ready);
}

/// §9: diagnostics are recorded as the only playback evidence and stay
/// readable through a low-frequency observation.
#[test]
fn playback_diagnostics_are_recorded_not_replied_to() {
    let mut machine = playing();
    let diagnostics = PlaybackDiagnostics {
        stream_instance_id: 1,
        config_generation: 1,
        extended_highest_sequence: 63,
        cumulative_lost: -2,
        cumulative_received: 66,
        submitted_position: 307_200,
        ..PlaybackDiagnostics::default()
    };
    let out = machine.handle(Event::Frame {
        frame: Box::new(v2_transport::schema::Frame {
            body: Some(frame::Body::PlaybackDiagnostics(diagnostics.clone())),
        }),
        source: Vec::new(),
    });
    assert!(
        out.is_empty(),
        "no reply; §9 says statistics never block media"
    );
    assert_eq!(machine.last_playback_diagnostics(), Some(&diagnostics));
    assert_eq!(machine.state(), ServerState::Playing);
}

/// Diagnostics are also legal in READY (§5.3 says a Client reports in PLAYING;
/// a late/best-effort final report after StreamEnded must not kill the
/// session).
#[test]
fn a_late_diagnostics_report_in_ready_is_accepted() {
    let mut machine = playing();
    machine.handle(Event::StreamLifecycle(StreamLifecycle::Ended {
        stream_instance_id: 1,
        code: ErrorCode::STREAM_ENDED,
    }));
    assert_eq!(machine.state(), ServerState::Ready);
    let out = machine.handle(Event::Frame {
        frame: Box::new(v2_transport::schema::Frame {
            body: Some(frame::Body::PlaybackDiagnostics(
                PlaybackDiagnostics::default(),
            )),
        }),
        source: Vec::new(),
    });
    assert!(out.is_empty());
    assert_eq!(machine.state(), ServerState::Ready);
}

/// §5.2 PLAYING: Close is one-way, no echo; state moves to CLOSING.
#[test]
fn close_in_playing_is_not_echoed() {
    let mut machine = playing();
    let out = machine.handle(Event::Frame {
        frame: Box::new(v2_transport::schema::Frame {
            body: Some(frame::Body::Close(v2_transport::schema::Close {
                code: 701,
                detail: String::new(),
            })),
        }),
        source: Vec::new(),
    });
    assert!(out.is_empty());
    assert_eq!(machine.state(), ServerState::Closing);
}

/// §5.2 PLAYING: a capture configuration change bumps the generation and
/// pushes ConfigurationPublished.
#[test]
fn a_config_change_in_playing_publishes_the_new_generation() {
    let mut machine = playing();
    let mut changed = active_snapshot(1, 2, 0);
    changed
        .configuration
        .as_mut()
        .unwrap()
        .pcm
        .as_mut()
        .unwrap()
        .sample_rate = 96_000;
    machine.update_capture(changed.clone());
    let out = machine.handle(Event::StreamLifecycle(StreamLifecycle::ConfigChanged {
        config_generation: 2,
    }));
    assert_eq!(out.len(), 1);
    assert!(matches!(
        body(out[0].frame.as_ref().and_then(|f| f.body.as_ref())),
        Some(frame::Body::ConfigurationPublished(p))
            if p.request_id == 0
                && p.configuration.as_ref().unwrap().config_generation == 2
    ));
    assert_eq!(machine.state(), ServerState::Playing);
}

/// §5.4: after CLOSED a lifecycle event produces nothing (no frames into a
/// dead session).
#[test]
fn lifecycle_events_after_close_are_ignored() {
    let mut machine = playing();
    machine.handle(Event::CloseReceived);
    machine.handle(Event::TransportLost);
    assert_eq!(machine.state(), ServerState::Closed);
    let out = machine.handle(Event::StreamLifecycle(StreamLifecycle::Started {
        stream_instance_id: 9,
        config_generation: 1,
        initial_timestamp: 0,
    }));
    assert!(out.is_empty());
}

/// The StreamEnded/StreamStarted shapes used by the fixture round-trip
/// through the machine unchanged (no extra fields invented).
#[test]
fn stream_messages_carry_only_the_spec_fields() {
    let started = StreamStarted {
        stream_instance_id: 1,
        config_generation: 1,
        initial_timestamp: 0,
    };
    let mut machine = ready(true);
    machine.update_capture(active_snapshot(
        started.stream_instance_id,
        started.config_generation,
        started.initial_timestamp,
    ));
    let out = subscribe(&mut machine, 3);
    assert_eq!(
        body(out[1].frame.as_ref().and_then(|f| f.body.as_ref())),
        Some(&frame::Body::StreamStarted(started))
    );

    let ended = StreamEnded {
        stream_instance_id: 1,
        code: 502,
        detail: "unsubscribed".to_owned(),
    };
    let out = machine.handle(Event::Frame {
        frame: Box::new(v2_transport::schema::Frame {
            body: Some(frame::Body::Unsubscribe(
                v2_transport::schema::Unsubscribe { request_id: 4 },
            )),
        }),
        source: Vec::new(),
    });
    assert_eq!(
        body(out[0].frame.as_ref().and_then(|f| f.body.as_ref())),
        Some(&frame::Body::StreamEnded(ended))
    );
}
