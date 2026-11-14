#![cfg(unix)]
//! The M4 end-to-end seam: one authenticated v2 Client subscribes to the
//! Server's shared capture and receives real QUIC DATAGRAM media, then ends
//! the stream and closes.
//!
//! Everything here goes through production wiring: `Server::start` with a
//! synthetic capture, `as_cmd::v2_capture_source` + `as_cmd::capture_media_source`
//! (the exact injection `as-cmd` performs), a real QUIC endpoint and the real
//! §5.2 machine. Expected bytes come from the normative header layout
//! (`docs/wire-protocol-v2/spec.md` §7.1), never from the Server's own encoder.

use std::sync::Arc;
use std::time::Duration;

use audio_share_core::capture::{CaptureConfig, SyntheticCapture};
use audio_share_core::protocol::{AudioFormat, Encoding};
use audio_share_core::server::{Server, ServerConfig};
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use v2_transport::auth::build_auth_context;
use v2_transport::auth_store::{AuthorizationStore, ClientState, StoredClient};
use v2_transport::codes::ALPN_PROTOCOL;
use v2_transport::driver::{SessionServer, AUTH_EXPORTER_LABEL};
use v2_transport::framing::{check_length_prefix, decode_body, encode_frame, LengthCheck};
use v2_transport::identity::{IdentityConfig, ServerIdentity};
use v2_transport::limits::{CapacityGuard, Limits};
use v2_transport::media::HEADER_SIZE;
use v2_transport::schema::{
    frame, AuthInit, AuthProof, Frame, Negotiate, Subscribe, Unsubscribe,
};
use v2_transport::transport::{Transport, TransportConfig};

const CLIENT_NAME: &str = "m4 media client";
const FRAME_BYTES: usize = 8 * 64; // 64 F32LE stereo frames per callback

async fn send_control(send: &mut quinn::SendStream, body: frame::Body) {
    send.write_all(&encode_frame(&Frame { body: Some(body) }).expect("encode"))
        .await
        .expect("write control frame");
}

async fn read_control(recv: &mut quinn::RecvStream) -> Frame {
    let mut prefix = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut prefix))
        .await
        .expect("control prefix timeout")
        .expect("control prefix");
    let body_len = match check_length_prefix(prefix) {
        LengthCheck::Accept { body_len } => body_len,
        other => panic!("invalid control length: {other:?}"),
    };
    let mut body = vec![0; body_len];
    tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut body))
        .await
        .expect("control body timeout")
        .expect("control body");
    decode_body(&body).expect("control frame")
}

async fn authenticate_and_negotiate(
    address: std::net::SocketAddr,
    key: &SigningKey,
    server_key: &[u8],
) -> (quinn::Connection, quinn::SendStream, quinn::RecvStream) {
    let connection = v2_transport::test_support::connect_with_alpn(address, ALPN_PROTOCOL)
        .expect("test endpoint")
        .await
        .expect("QUIC handshake");
    let max = connection.max_datagram_size();
    assert!(
        max.is_some(),
        "the test transport must negotiate DATAGRAM support"
    );
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    let client_key = key
        .verifying_key()
        .to_encoded_point(true)
        .as_bytes()
        .to_vec();
    send_control(
        &mut send,
        frame::Body::AuthInit(AuthInit {
            request_id: 1,
            client_public_key: client_key.clone(),
        }),
    )
    .await;
    let random = match read_control(&mut recv).await.body {
        Some(frame::Body::AuthChallenge(challenge)) => challenge.server_random,
        _ => panic!("expected AuthChallenge"),
    };
    let mut exporter = [0u8; 32];
    connection
        .export_keying_material(&mut exporter, AUTH_EXPORTER_LABEL, b"")
        .expect("TLS exporter");
    let signature: Signature = key.sign(&build_auth_context(
        &random,
        &client_key,
        server_key,
        &exporter,
    ));
    send_control(
        &mut send,
        frame::Body::AuthProof(AuthProof {
            request_id: 1,
            signature: signature.to_vec(),
        }),
    )
    .await;
    match read_control(&mut recv).await.body {
        Some(frame::Body::AuthResult(result)) => assert_eq!(result.status.code, 0),
        _ => panic!("expected AuthResult"),
    }
    send_control(
        &mut send,
        frame::Body::Negotiate(Negotiate {
            request_id: 2,
            protocol_major: 2,
            protocol_minor: 0,
            capabilities: Vec::new(),
        }),
    )
    .await;
    match read_control(&mut recv).await.body {
        Some(frame::Body::NegotiateResponse(result)) => assert_eq!(result.status.code, 0),
        _ => panic!("expected NegotiateResponse"),
    }
    (connection, send, recv)
}

struct RunningServer {
    core: audio_share_core::server::ServerHandle,
    transport: Arc<Transport>,
    accept_task: tokio::task::JoinHandle<()>,
    sessions: Arc<SessionServer>,
}

/// Production wiring: synthetic capture + the exact as-cmd injections.
async fn start_server(key: &SigningKey) -> RunningServer {
    let format = AudioFormat::new(Encoding::PcmFloat, 2, 48_000);
    let core = Server::start(
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
            ..ServerConfig::default()
        },
        Arc::new(
            SyntheticCapture::new(format, FRAME_BYTES, Duration::from_millis(20))
                .expect("synthetic capture"),
        ),
        CaptureConfig::default(),
    )
    .await
    .expect("Server Core");
    let observer = core.capture_observer();
    let dir = tempfile::tempdir().expect("identity dir");
    let identity = ServerIdentity::open(&IdentityConfig {
        data_dir: dir.path().into(),
        allow_loose_permissions: false,
    })
    .expect("fresh identity");
    let store = Arc::new(AuthorizationStore::in_memory());
    store.insert_for_tests(StoredClient {
        client_id: "0000000000000001".to_owned(),
        public_key: key
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
        display_name: CLIENT_NAME.to_owned(),
        state: ClientState::Active,
    });
    let mut sessions = SessionServer::new(&identity, store);
    sessions.capture_source = Some(as_cmd::v2_capture_source(observer.clone(), core.media_tap()));
    sessions.media_source = Some(as_cmd::capture_media_source(core.media_tap(), observer));
    let sessions = Arc::new(sessions);
    let transport = Arc::new(
        Transport::start(
            TransportConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                limits: Limits::defaults(),
            },
            &identity,
        )
        .expect("QUIC Server"),
    );
    let accepting = Arc::clone(&transport);
    let serving = Arc::clone(&sessions);
    let accept_task = tokio::spawn(async move {
        let mut guard = CapacityGuard::new(Limits::defaults());
        loop {
            let accepted = accepting.accept_one(&mut guard).await;
            let Some(connection) = accepted.connection else {
                continue;
            };
            let server = Arc::clone(&serving);
            tokio::spawn(async move {
                server
                    .serve_connection(connection, accepted.remote_address)
                    .await
            });
        }
    });
    RunningServer {
        core,
        transport,
        accept_task,
        sessions,
    }
}

impl RunningServer {
    async fn stop(self) {
        self.transport.stop();
        self.accept_task.abort();
        let _ = self.accept_task.await;
        self.core.stop().await.expect("Server Core stop");
    }
}

fn new_key() -> SigningKey {
    SigningKey::random(&mut rand_core::OsRng)
}

/// The M4 tracer bullet: READY → Subscribe → StreamStarted → media DATAGRAMs
/// with the §7.1 header → Unsubscribe (StreamEnded first) → READY.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribed_client_receives_real_media_and_can_end_the_stream() {
    let key = new_key();
    let server = start_server(&key).await;
    let identity_pin = {
        // The identity public key is what the auth transcript binds; the test
        // client reads it from the same fresh identity the server uses.
        server.sessions.server_public_key.clone()
    };
    let (connection, mut send, mut recv) =
        authenticate_and_negotiate(server.transport.local_addr(), &key, &identity_pin).await;

    // READY entry: ConfigurationPublished (request_id 0).
    let configuration = match read_control(&mut recv).await.body {
        Some(frame::Body::ConfigurationPublished(push)) => {
            push.configuration.expect("current configuration")
        }
        other => panic!("expected ConfigurationPublished, got {other:?}"),
    };
    assert_eq!(
        configuration.config_generation, 1,
        "the immutable capture format is generation 1"
    );

    send_control(&mut send, frame::Body::Subscribe(Subscribe { request_id: 3 })).await;
    match read_control(&mut recv).await.body {
        Some(frame::Body::SubscribeResponse(response)) => {
            assert_eq!(response.request_id, 3);
            assert_eq!(response.status.code, 0);
        }
        other => panic!("expected SubscribeResponse, got {other:?}"),
    }
    let started = match read_control(&mut recv).await.body {
        Some(frame::Body::StreamStarted(started)) => started,
        other => panic!("expected StreamStarted, got {other:?}"),
    };
    assert_ne!(started.stream_instance_id, 0);
    assert_eq!(started.config_generation, 1);
    // §4.4: 0 for a fresh instance, a mid-stream anchor for a late Subscriber.
    // The capture starts with the Server, so this Subscriber joins a running
    // stream; the anchor is the next frame position at Subscribe time.
    let anchor = started.initial_timestamp;

    // Media: real DATAGRAMs with the normative 24-byte BE header.
    let mut observed = Vec::new();
    let mut last_timestamp = None;
    while observed.len() < 4 {
        let datagram = tokio::time::timeout(Duration::from_secs(10), connection.read_datagram())
            .await
            .expect("media datagram timeout")
            .expect("datagram");
        assert!(datagram.len() > HEADER_SIZE, "header + payload");
        let stream = u32::from_be_bytes(datagram[0..4].try_into().unwrap());
        let generation = u32::from_be_bytes(datagram[4..8].try_into().unwrap());
        let sequence = u32::from_be_bytes(datagram[8..12].try_into().unwrap());
        let timestamp = u64::from_be_bytes(datagram[12..20].try_into().unwrap());
        let flags = u32::from_be_bytes(datagram[20..24].try_into().unwrap());
        let payload = &datagram[HEADER_SIZE..];
        assert_eq!(stream, started.stream_instance_id);
        assert_eq!(generation, started.config_generation);
        assert_eq!(payload.len() % 8, 0, "frame-aligned (F32LE stereo)");
        assert_eq!(sequence as usize, observed.len(), "sequence from 0");
        if let Some(previous) = last_timestamp {
            let frames = (payload.len() / 8) as u64;
            assert_eq!(
                timestamp,
                previous + frames,
                "timestamp advances by whole frames (§7.2)"
            );
        } else {
            assert!(
                timestamp >= anchor,
                "the first datagram is at or after the announced anchor ({timestamp} >= {anchor})"
            );
        }
        last_timestamp = Some(timestamp);
        observed.push(flags);
    }
    assert_eq!(observed[0] & 0x1, 0x1, "STREAM_START on the first datagram");
    assert!(
        observed[1..].iter().all(|flags| flags & 0x1 == 0),
        "STREAM_START is set once per instance"
    );

    // Unsubscribe: StreamEnded(502, "unsubscribed") BEFORE UnsubscribeResponse.
    send_control(
        &mut send,
        frame::Body::Unsubscribe(Unsubscribe { request_id: 4 }),
    )
    .await;
    match read_control(&mut recv).await.body {
        Some(frame::Body::StreamEnded(ended)) => {
            assert_eq!(ended.stream_instance_id, started.stream_instance_id);
            assert_eq!(ended.code, 502);
            assert_eq!(ended.detail, "unsubscribed");
        }
        other => panic!("expected StreamEnded first, got {other:?}"),
    }
    match read_control(&mut recv).await.body {
        Some(frame::Body::UnsubscribeResponse(response)) => {
            assert_eq!(response.request_id, 4);
            assert_eq!(response.status.code, 0);
        }
        other => panic!("expected UnsubscribeResponse, got {other:?}"),
    }

    // No media after the end.
    let next = tokio::time::timeout(Duration::from_millis(300), connection.read_datagram()).await;
    if let Ok(Ok(datagram)) = next {
        let instance = u32::from_be_bytes(datagram[0..4].try_into().unwrap());
        assert_eq!(
            instance, started.stream_instance_id,
            "only in-flight datagrams may still arrive"
        );
    }

    // Explicit close: one-way, no echo; the transport close is the ack.
    connection.close(quinn::VarInt::from_u32(701), b"client stopping");
    server.stop().await;
}

/// §9: diagnostics reported by the Client are recorded as playback evidence.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reported_playback_diagnostics_are_observable() {
    let key = new_key();
    let server = start_server(&key).await;
    let (connection, mut send, mut recv) =
        authenticate_and_negotiate(server.transport.local_addr(), &key, &server.sessions.server_public_key.clone())
            .await;
    match read_control(&mut recv).await.body {
        Some(frame::Body::ConfigurationPublished(_)) => {}
        other => panic!("expected ConfigurationPublished, got {other:?}"),
    }
    send_control(&mut send, frame::Body::Subscribe(Subscribe { request_id: 3 })).await;
    let session_id = match read_control(&mut recv).await.body {
        Some(frame::Body::SubscribeResponse(response)) => {
            assert_eq!(response.status.code, 0);
            // AuthResult carried the Server-assigned session id (1 for the
            // first session on this endpoint).
            1
        }
        other => panic!("expected SubscribeResponse, got {other:?}"),
    };
    match read_control(&mut recv).await.body {
        Some(frame::Body::StreamStarted(_)) => {}
        other => panic!("expected StreamStarted, got {other:?}"),
    }

    let diagnostics = v2_transport::schema::PlaybackDiagnostics {
        stream_instance_id: 1,
        config_generation: 1,
        extended_highest_sequence: 63,
        cumulative_lost: -2,
        interval_lost: 0,
        cumulative_received: 66,
        interval_received: 66,
        cumulative_duplicates: 2,
        interval_duplicates: 2,
        cumulative_reordered: 4,
        interval_reordered: 4,
        max_reorder_distance: 3,
        cumulative_late_drops: 1,
        interval_late_drops: 1,
        cumulative_unknown_config_drops: 0,
        interval_unknown_config_drops: 0,
        cumulative_concealments: 3,
        interval_concealments: 3,
        concealed_frames: 1_440,
        underruns: 1,
        jitter_microseconds: 1_234,
        buffer_current_ms: 61,
        buffer_target_ms: 60,
        submitted_position: 307_200,
    };
    send_control(
        &mut send,
        frame::Body::PlaybackDiagnostics(diagnostics.clone()),
    )
    .await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(recorded) = server.sessions.playback_diagnostics(session_id) {
            assert_eq!(recorded, diagnostics);
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "diagnostics must be observable on the Server"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    connection.close(quinn::VarInt::from_u32(701), b"client stopping");
    server.stop().await;
}
