//! Ticket 15: real Rust Server ↔ real Android Client control-plane interop.
//!
//! The Android `ClientMachine` and the Server `StateMachine` are wired to each
//! other only through their own framing + prost codecs. The harness never
//! fabricates a decoded `Frame` for either end: every message crosses the
//! `u32 BE length | Frame` wire, encoded by one end's `encode_frame` and
//! decoded by the other end's `FrameDecoder` / `FrameReader`. Expected frame
//! bytes and field values come from the frozen golden vectors.

use std::sync::Arc;

use audio_share_client_v2_native::auth::sec1_compressed;
use audio_share_client_v2_native::control::{ClientMachine, ClientState, Event, Verdict};
use audio_share_client_v2_native::frame::{encode_frame as android_encode, FrameDecoder};
use audio_share_client_v2_native::proto::frame::Body as AndroidBody;
use prost::Message as _;
use rust_android_interop::{control_fixture, control_frames, unhex};
use v2_transport::auth_store::{AuthorizationStore, ClientState as StoreState, StoredClient};
use v2_transport::codes::ErrorCode;
use v2_transport::framing::{encode_frame as server_encode, FrameReader};
use v2_transport::schema::{frame, Frame as ServerFrame, Status};
use v2_transport::state_machine::{
    CaptureSnapshot, Event as ServerEvent, ServerConfig, ServerState, StateMachine, StreamLifecycle,
};

const CLIENT_PUB_HEX: &str = "03d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d";
const SERVER_PUB_HEX: &str = "11b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c";
const SERVER_RANDOM_HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const AUTH_EXPORTER_HEX: &str = "c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf";

fn client_identity() -> p256::ecdsa::SigningKey {
    let d_hex = "87fb33cdf19dc9ec6324570648e8f354d26d5262e4930182382220bec2fdfac7";
    let mut d = [0u8; 32];
    for i in 0..32 {
        d[i] = u8::from_str_radix(&d_hex[2 * i..2 * i + 2], 16).unwrap();
    }
    p256::ecdsa::SigningKey::from_bytes(&d.into()).expect("fixture scalar")
}

fn server_configuration() -> v2_transport::schema::Configuration {
    v2_transport::schema::Configuration {
        config_generation: 1,
        codec: v2_transport::schema::Codec::Pcm as i32,
        pcm: Some(v2_transport::schema::PcmParameters {
            sample_rate: 48_000,
            channels: 2,
            sample_format: v2_transport::schema::SampleFormat::F32Le as i32,
        }),
    }
}

/// A live stream with a mid-stream anchor: a late Subscriber must see 96_000.
fn active_capture() -> CaptureSnapshot {
    CaptureSnapshot {
        configuration: Some(server_configuration()),
        healthy: true,
        stream_active: true,
        stream_instance_id: 7,
        next_frame_timestamp: 96_000,
    }
}

fn server_machine() -> StateMachine {
    let store = AuthorizationStore::in_memory();
    store.insert_for_tests(StoredClient {
        client_id: "cf7940b474310246".to_owned(),
        public_key: unhex(CLIENT_PUB_HEX),
        display_name: "Test Phone".to_owned(),
        state: StoreState::Active,
        created_at: String::new(),
        last_seen_at: String::new(),
    });
    StateMachine::new(
        ServerConfig {
            server_public_key: unhex(SERVER_PUB_HEX),
            protocol_minor: 0,
            capture: active_capture(),
            max_auth_failures_per_connection: 5,
            // Golden-vector replay constants (test-keys.json).
            server_random_override: Some(unhex(SERVER_RANDOM_HEX)),
            auth_exporter: Some(unhex(AUTH_EXPORTER_HEX)),
            session_id_base: 1,
            pairing: None,
            pairing_exporter: None,
            pairing_server_random_override: None,
        },
        &Arc::new(store),
    )
}

fn android_machine() -> ClientMachine {
    let mut machine = ClientMachine::new();
    machine
        .set_auth_context(
            unhex(SERVER_PUB_HEX),
            unhex(AUTH_EXPORTER_HEX),
            std::sync::Arc::new(audio_share_client_v2_native::signer::SoftwareSigner::new(
                client_identity(),
            )),
        )
        .expect("auth context");
    machine
}

/// Encode a Server `Frame` and decode it with the Android Client's real
/// `FrameDecoder`; assert the wire bytes first when a golden vector pins them.
fn server_to_android(
    bytes: &[u8],
    decoder: &mut FrameDecoder,
) -> audio_share_client_v2_native::proto::Frame {
    let frames = decoder.push(bytes).expect("Android framing decoder");
    assert_eq!(frames.len(), 1, "one framed message");
    frames.into_iter().next().unwrap()
}

/// Encode an Android `Frame` and decode it with the Server's real `FrameReader`.
fn android_to_server(bytes: &[u8], reader: &mut FrameReader) -> ServerFrame {
    let frames = reader.push(bytes).expect("Server framing reader");
    assert_eq!(frames.len(), 1, "one framed message");
    frames.into_iter().next().unwrap().frame
}

/// Feed one decoded Android frame into the Server machine and return the
/// ordered wire frames it emits (encoded with the Server's own encoder).
fn server_step(machine: &mut StateMachine, frame: ServerFrame) -> Vec<Vec<u8>> {
    machine
        .handle(ServerEvent::Frame {
            frame: Box::new(frame),
            source: unhex(CLIENT_PUB_HEX),
        })
        .into_iter()
        .filter_map(|outcome| outcome.frame)
        .map(|frame| server_encode(&frame).expect("Server encode"))
        .collect()
}

/// Feed one decoded Server frame into the Android machine and return the wire
/// frames the machine wants to send (encoded with the Android encoder).
fn android_step(
    machine: &mut ClientMachine,
    frame: audio_share_client_v2_native::proto::Frame,
) -> Vec<Vec<u8>> {
    let (result, _event) = machine.on_frame(&frame);
    let verdict = result.expect("no local close");
    match verdict {
        Verdict::Send(frame) => vec![android_encode(&frame).expect("Android encode")],
        Verdict::Ok | Verdict::Reject { .. } | Verdict::Fatal { .. } | Verdict::Stop(_) => {
            Vec::new()
        }
    }
}

/// §9.1 / §5: the whole handshake crosses the real wire in both directions.
/// The Android Client signs its AuthProof; the Server verifies it. Neither
/// side's decoder sees a hand-constructed `Frame`.
#[test]
fn real_auth_negotiate_subscribe_reaches_playing_on_both_ends() {
    let mut server = server_machine();
    let mut android = android_machine();
    let mut to_server = FrameReader::default();
    let mut to_android = FrameDecoder::new();

    // Android: CONNECTING → AUTHENTICATING, AuthInit (request_id 1).
    // `auth_init` only builds the frame; `enter_authenticating` is the real
    // state-machine row that records the pending request and changes state.
    let init = android.auth_init(sec1_compressed(&client_identity()));
    let init_wire = match android
        .enter_authenticating(init)
        .expect("enter authenticating")
    {
        Verdict::Send(frame) => android_encode(&frame).expect("encode AuthInit"),
        other => panic!("expected AuthInit Send, got {other:?}"),
    };
    assert_eq!(
        init_wire,
        unhex(&control_frames(&control_fixture("auth-happy-path"))[0].1)
    );
    let init_frame = android_to_server(&init_wire, &mut to_server);
    let challenge_wire = server_step(&mut server, init_frame);
    assert_eq!(challenge_wire.len(), 1, "AuthChallenge only");
    assert_eq!(
        challenge_wire[0],
        unhex(&control_frames(&control_fixture("auth-happy-path"))[1].1),
        "golden AuthChallenge bytes"
    );

    // Android verifies nothing about the challenge itself; it signs the raw
    // auth_context with its real identity key, then the Server verifies.
    let challenge = server_to_android(&challenge_wire[0], &mut to_android);
    let (result, _) = android.on_frame(&challenge);
    let proof_wire = match result.expect("auth proof") {
        Verdict::Send(frame) => android_encode(&frame).expect("encode AuthProof"),
        other => panic!("expected AuthProof Send, got {other:?}"),
    };
    let proof = android_to_server(&proof_wire, &mut to_server);
    let auth_result_wire = server_step(&mut server, proof);
    assert_eq!(auth_result_wire.len(), 1, "AuthResult only");
    assert_eq!(server.state(), ServerState::Negotiating);

    // Android: AuthResult OK → Negotiate → Server: NegotiateResponse +
    // ConfigurationPublished (the READY entry push).
    let auth_result = server_to_android(&auth_result_wire[0], &mut to_android);
    let negotiate_wire = android_step(&mut android, auth_result);
    assert_eq!(negotiate_wire.len(), 1);
    let negotiate = android_to_server(&negotiate_wire[0], &mut to_server);
    let ready_wire = server_step(&mut server, negotiate);
    assert_eq!(
        ready_wire.len(),
        2,
        "NegotiateResponse + ConfigurationPublished"
    );
    assert_eq!(server.state(), ServerState::Ready);

    for wire in &ready_wire {
        let frame = server_to_android(wire, &mut to_android);
        assert!(android_step(&mut android, frame).is_empty());
    }
    assert_eq!(android.state, ClientState::Ready);
    assert_eq!(android.config.as_ref().unwrap().config_generation, 1);

    // READY + valid configuration → Subscribe (§5.3 READY row). The Server
    // answers with SubscribeResponse then StreamStarted(real anchor).
    let subscribe_wire = match android.request_subscribe() {
        Verdict::Send(frame) => android_encode(&frame).expect("encode Subscribe"),
        other => panic!("expected Subscribe, got {other:?}"),
    };
    let subscribe = android_to_server(&subscribe_wire, &mut to_server);
    let playing_wire = server_step(&mut server, subscribe);
    assert_eq!(playing_wire.len(), 2, "SubscribeResponse + StreamStarted");
    assert_eq!(server.state(), ServerState::Playing);

    let responses: Vec<_> = playing_wire
        .iter()
        .map(|wire| server_to_android(wire, &mut to_android))
        .collect();
    let (result, event) = android.on_frame(&responses[0]);
    assert!(matches!(result, Ok(Verdict::Ok)));
    assert_eq!(event, Some(Event::Subscribed));
    let (result, event) = android.on_frame(&responses[1]);
    assert!(matches!(result, Ok(Verdict::Ok)));
    match event {
        Some(Event::StreamStarted(anchor)) => {
            assert_eq!(anchor.stream_instance_id, 7);
            assert_eq!(anchor.config_generation, 1);
            assert_eq!(anchor.initial_timestamp, 96_000, "late-subscriber anchor");
        }
        other => panic!("expected StreamStarted anchor, got {other:?}"),
    }
    assert_eq!(android.state, ClientState::Playing);
    assert!(android.subscribed);
}

/// §5.2/§5.4/§9: Unsubscribe pushes StreamEnded(502) before the response,
/// StreamEnded retains the subscription, a new instance starts a fresh epoch,
/// diagnostics are observable on the Server, and Close(701) is never echoed.
#[test]
fn stream_lifecycle_diagnostics_and_one_way_close() {
    let mut server = server_machine();
    let mut android = android_machine();
    let mut to_server = FrameReader::default();
    let mut to_android = FrameDecoder::new();

    // Drive both real machines to PLAYING using the same wire-only exchange.
    let init = android.auth_init(sec1_compressed(&client_identity()));
    let init_wire = match android
        .enter_authenticating(init)
        .expect("enter authenticating")
    {
        Verdict::Send(frame) => android_encode(&frame).unwrap(),
        other => panic!("expected AuthInit Send, got {other:?}"),
    };
    let init = android_to_server(&init_wire, &mut to_server);
    let challenge = server_step(&mut server, init).remove(0);
    let challenge = server_to_android(&challenge, &mut to_android);
    let proof = match android.on_frame(&challenge).0.unwrap() {
        Verdict::Send(frame) => android_to_server(&android_encode(&frame).unwrap(), &mut to_server),
        other => panic!("expected AuthProof, got {other:?}"),
    };
    let auth_result = server_step(&mut server, proof).remove(0);
    let auth_result = server_to_android(&auth_result, &mut to_android);
    let negotiate = android_step(&mut android, auth_result).remove(0);
    let negotiate = android_to_server(&negotiate, &mut to_server);
    for wire in server_step(&mut server, negotiate) {
        let frame = server_to_android(&wire, &mut to_android);
        android_step(&mut android, frame);
    }
    let subscribe = match android.request_subscribe() {
        Verdict::Send(frame) => android_to_server(&android_encode(&frame).unwrap(), &mut to_server),
        other => panic!("expected Subscribe, got {other:?}"),
    };
    for wire in server_step(&mut server, subscribe) {
        let frame = server_to_android(&wire, &mut to_android);
        android_step(&mut android, frame);
    }
    assert_eq!(server.state(), ServerState::Playing);
    assert_eq!(android.state, ClientState::Playing);

    // §9: the Android Client reports a full diagnostics snapshot built from
    // the golden vector; the Server records it silently (no reply).
    let golden = control_fixture("subscribe-config-diagnostics");
    let diag_body = unhex(&control_frames(&golden)[3].0);
    let android_diag = audio_share_client_v2_native::proto::Frame::decode(diag_body.as_slice())
        .expect("Android decodes the golden diagnostics body");
    let diag_wire = android_encode(&android_diag).expect("Android re-encodes diagnostics");
    assert_eq!(
        diag_wire,
        unhex(&control_frames(&golden)[3].1),
        "golden §9 bytes"
    );
    let diag_frame = android_to_server(&diag_wire, &mut to_server);
    assert!(
        server_step(&mut server, diag_frame).is_empty(),
        "no reply to diagnostics"
    );
    let recorded = server
        .last_playback_diagnostics()
        .expect("Server observed the Client-reported diagnostics");
    assert_eq!(recorded.stream_instance_id, 1);
    assert_eq!(recorded.extended_highest_sequence, 63);
    assert_eq!(
        recorded.cumulative_lost, -2,
        "signed sint64 crosses the wire"
    );
    assert_eq!(recorded.submitted_position, 307_200);
    assert_eq!(recorded.concealed_frames, 1_440);

    // §5.2 capture end: StreamEnded(502) → READY, subscription survives.
    let ended_wire = server
        .handle(ServerEvent::StreamLifecycle(StreamLifecycle::Ended {
            stream_instance_id: 7,
            code: ErrorCode::STREAM_ENDED,
        }))
        .into_iter()
        .filter_map(|o| o.frame)
        .map(|f| server_encode(&f).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ended_wire.len(), 1);
    let ended = server_to_android(&ended_wire[0], &mut to_android);
    let (result, event) = android.on_frame(&ended);
    assert!(matches!(result, Ok(Verdict::Ok)));
    assert_eq!(event, Some(Event::StreamEnded(502)));
    assert_eq!(android.state, ClientState::Ready);
    assert!(
        android.subscribed,
        "StreamEnded does not clear the subscription"
    );

    // A capture restart with a NEW instance pushes StreamStarted again; the
    // Android Client re-enters PLAYING with the new epoch anchor.
    let restarted_wire = server
        .handle(ServerEvent::StreamLifecycle(StreamLifecycle::Started {
            stream_instance_id: 8,
            config_generation: 2,
            initial_timestamp: 0,
        }))
        .into_iter()
        .filter_map(|o| o.frame)
        .map(|f| server_encode(&f).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(restarted_wire.len(), 1);
    let started = server_to_android(&restarted_wire[0], &mut to_android);
    let (result, event) = android.on_frame(&started);
    assert!(matches!(result, Ok(Verdict::Ok)));
    match event {
        Some(Event::StreamStarted(anchor)) => {
            assert_eq!(anchor.stream_instance_id, 8);
            assert_eq!(anchor.config_generation, 2);
            assert_eq!(anchor.initial_timestamp, 0);
        }
        other => panic!("expected new-instance anchor, got {other:?}"),
    }
    assert_eq!(android.state, ClientState::Playing);

    // §5.2 PLAYING Unsubscribe: StreamEnded(502,"unsubscribed") FIRST, then
    // UnsubscribeResponse(OK). Both are real Server frames on the real wire.
    let unsubscribe = match android.request_unsubscribe() {
        Verdict::Send(frame) => android_to_server(&android_encode(&frame).unwrap(), &mut to_server),
        other => panic!("expected Unsubscribe, got {other:?}"),
    };
    let unsubscribe_wire = server_step(&mut server, unsubscribe);
    assert_eq!(unsubscribe_wire.len(), 2, "ordered push then response");
    let first = server_to_android(&unsubscribe_wire[0], &mut to_android);
    match first.body.as_ref().unwrap() {
        AndroidBody::StreamEnded(e) => {
            assert_eq!(e.code, 502);
            assert_eq!(e.detail, "unsubscribed");
        }
        other => panic!("expected StreamEnded first, got {other:?}"),
    }
    let (_, event) = android.on_frame(&first);
    assert_eq!(event, Some(Event::StreamEnded(502)));
    let second = server_to_android(&unsubscribe_wire[1], &mut to_android);
    assert!(matches!(
        second.body,
        Some(AndroidBody::UnsubscribeResponse(_))
    ));
    android_step(&mut android, second);
    assert!(
        !android.subscribed,
        "only user Unsubscribe clears the subscription"
    );
    assert_eq!(server.state(), ServerState::Ready);
    assert_eq!(android.state, ClientState::Ready);

    // §5.4 invariant 4 / §6 701: Close is one-way. The Android Client's real
    // user_stop emits the golden Close(701) bytes; the Server emits NO frames.
    let close = match android.user_stop() {
        Verdict::Send(frame) => android_encode(&frame).expect("encode Close"),
        other => panic!("expected Close(701), got {other:?}"),
    };
    assert_eq!(
        close,
        unhex(&control_frames(&control_fixture("error-701-client-stopping"))[0].1),
        "golden Close(701) bytes"
    );
    let close_frame = android_to_server(&close, &mut to_server);
    assert!(
        server_step(&mut server, close_frame).is_empty(),
        "Close is never echoed"
    );
    assert_eq!(server.state(), ServerState::Closing);
    assert_eq!(android.state, ClientState::Closed);
}

/// §4.4/§5.3: a decode-affecting configuration change published by the
/// Server is applied by the Android Client's real machine, and a configuration
/// the Client cannot decode is refused with 403 (no silent format switch).
#[test]
fn configuration_change_and_unsupported_media_are_cross_checked() {
    let mut server = server_machine();
    let mut android = android_machine();
    let mut to_server = FrameReader::default();
    let mut to_android = FrameDecoder::new();

    // READY (not subscribed): a publication with a new generation is stored.
    let init = android.auth_init(sec1_compressed(&client_identity()));
    let init_wire = match android
        .enter_authenticating(init)
        .expect("enter authenticating")
    {
        Verdict::Send(frame) => android_encode(&frame).unwrap(),
        other => panic!("expected AuthInit Send, got {other:?}"),
    };
    let init = android_to_server(&init_wire, &mut to_server);
    let challenge = server_step(&mut server, init).remove(0);
    let challenge = server_to_android(&challenge, &mut to_android);
    let proof = match android.on_frame(&challenge).0.unwrap() {
        Verdict::Send(frame) => android_to_server(&android_encode(&frame).unwrap(), &mut to_server),
        other => panic!("expected AuthProof, got {other:?}"),
    };
    let auth_result = server_step(&mut server, proof).remove(0);
    let auth_result = server_to_android(&auth_result, &mut to_android);
    let negotiate = android_step(&mut android, auth_result).remove(0);
    let negotiate = android_to_server(&negotiate, &mut to_server);
    for wire in server_step(&mut server, negotiate) {
        let frame = server_to_android(&wire, &mut to_android);
        android_step(&mut android, frame);
    }
    assert_eq!(android.state, ClientState::Ready);

    let mut changed = server_configuration();
    changed.config_generation = 2;
    let publication = ServerFrame {
        body: Some(frame::Body::ConfigurationPublished(
            v2_transport::schema::ConfigurationPublished {
                request_id: 0,
                configuration: Some(changed),
            },
        )),
    };
    let wire = server_encode(&publication).expect("Server encode publication");
    let frame = server_to_android(&wire, &mut to_android);
    let (result, event) = android.on_frame(&frame);
    assert!(matches!(result, Ok(Verdict::Ok)));
    assert_eq!(event, Some(Event::ConfigurationPublished(2)));
    assert_eq!(android.config.as_ref().unwrap().config_generation, 2);

    // S24LE (wire value 4) has no AudioTrack encoding on API 26. The Client
    // must refuse with 403 rather than silently switching formats.
    let mut unsupported = server_configuration();
    unsupported.config_generation = 3;
    unsupported.pcm.as_mut().unwrap().sample_format = 4;
    let publication = ServerFrame {
        body: Some(frame::Body::ConfigurationPublished(
            v2_transport::schema::ConfigurationPublished {
                request_id: 0,
                configuration: Some(unsupported),
            },
        )),
    };
    let wire = server_encode(&publication).expect("Server encode publication");
    let frame = server_to_android(&wire, &mut to_android);
    let (result, _) = android.on_frame(&frame);
    assert!(
        matches!(result, Ok(Verdict::Fatal { code: 403 })),
        "unsupported geometry is a 403-class condition, got {result:?}"
    );
}

/// §3: the two framing implementations agree on the wire. A Server-produced
/// frame is exactly the golden bytes, and the Android decoder accepts it in
/// arbitrary chunk boundaries (streaming, not a whole-frame assumption).
#[test]
fn control_framing_is_byte_identical_across_the_two_implementations() {
    let fixture = control_fixture("subscribe-config-diagnostics");
    let frames = control_frames(&fixture);
    // Golden Subscribe / StreamEnded / UnsubscribeResponse bodies.
    for (index, message) in [
        (0usize, "subscribe"),
        (5, "stream_ended"),
        (6, "unsubscribe_response"),
    ] {
        let body = unhex(&frames[index].0);
        let expected = unhex(&frames[index].1);
        let server_frame = ServerFrame::decode(body.as_slice()).expect("Server decodes body");
        let server_wire = server_encode(&server_frame).expect("Server re-encodes");
        assert_eq!(
            server_wire, expected,
            "{message}: both ends match the golden frame"
        );

        // Android's streaming decoder accepts the same bytes split in three.
        let mut decoder = FrameDecoder::new();
        assert!(decoder.push(&server_wire[..2]).unwrap().is_empty());
        assert!(decoder.push(&server_wire[2..5]).unwrap().is_empty());
        let decoded = decoder.push(&server_wire[5..]).unwrap();
        assert_eq!(decoded.len(), 1, "{message}: one complete frame");
        assert_eq!(android_encode(&decoded[0]).unwrap(), expected);
    }

    // §4.4: the golden SubscribeResponse's Status is OK (code 0).
    let response_body = unhex(&frames[1].0);
    let response = ServerFrame::decode(response_body.as_slice()).unwrap();
    match response.body.unwrap() {
        frame::Body::SubscribeResponse(r) => assert_eq!(r.status.code, Status::ok().code),
        other => panic!("expected SubscribeResponse, got {other:?}"),
    }
}
