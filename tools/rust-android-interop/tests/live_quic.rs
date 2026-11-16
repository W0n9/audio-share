#![cfg(unix)]
//! Ticket 15: real authenticated live-QUIC session, Rust Server → real
//! Android Client code.
//!
//! The Server side is the exact production wiring from
//! `crates/as-cmd/tests/v2_media.rs`: `Server::start` with a synthetic
//! capture, `as_cmd::v2_capture_source` / `capture_media_source`, a fresh
//! in-memory identity in a tempdir, and a real quinn endpoint. The Client
//! side is NOT a hand-written decoder: the Android `ClientMachine` signs the
//! AuthProof, the Android `FrameDecoder` decodes every control frame, and the
//! Android `MediaSession` receives every QUIC DATAGRAM and produces the §9
//! diagnostics the Server must observe.
//!
//! No key is read from an old tmp/worktree path: both the Client identity and
//! the Server identity are freshly generated per test run.

use std::sync::Arc;
use std::time::Duration;

use audio_share_client_v2_native::auth::sec1_compressed;
use audio_share_client_v2_native::control::{ClientMachine, ClientState, Event, Verdict};
use audio_share_client_v2_native::frame::{encode_frame as android_encode, FrameDecoder};
use audio_share_client_v2_native::media_session::{DatagramOutcome, MediaSession, PlaybackReport};
use audio_share_client_v2_native::proto::frame::Body as AndroidBody;
use audio_share_client_v2_native::proto::{Configuration, Frame as AndroidFrame, PcmParameters};
use audio_share_client_v2_native::ring::MediaRing;
use audio_share_core::capture::{CaptureConfig, SyntheticCapture};
use audio_share_core::protocol::{AudioFormat, Encoding};
use audio_share_core::server::{Server, ServerConfig as CoreConfig, ServerHandle};
use p256::ecdsa::SigningKey;
use quinn::{Connection, RecvStream, SendStream};
use v2_transport::auth_store::{AuthorizationStore, ClientState as StoreState, StoredClient};
use v2_transport::codes::ALPN_PROTOCOL;
use v2_transport::driver::{SessionServer, AUTH_EXPORTER_LABEL};
use v2_transport::identity::{IdentityConfig, ServerIdentity};
use v2_transport::limits::{CapacityGuard, Limits};
use v2_transport::transport::{Transport, TransportConfig};

const CLIENT_NAME: &str = "m4 interop client";
const FRAME_BYTES: usize = 8 * 64; // 64 F32LE stereo frames per capture callback

fn new_key() -> SigningKey {
    SigningKey::random(&mut rand_core::OsRng)
}

/// Read one `u32 BE length | Frame` message as raw bytes (framing validation
/// is the Android decoder's job, not ours).
async fn read_wire(recv: &mut RecvStream) -> Vec<u8> {
    let mut prefix = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut prefix))
        .await
        .expect("control prefix timeout")
        .expect("control prefix");
    let len = u32::from_be_bytes(prefix) as usize;
    assert!((1..=65536).contains(&len), "control length {len}");
    let mut body = vec![0u8; len];
    tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut body))
        .await
        .expect("control body timeout")
        .expect("control body");
    prefix.iter().copied().chain(body).collect()
}

struct RunningServer {
    core: ServerHandle,
    transport: Arc<Transport>,
    accept_task: tokio::task::JoinHandle<()>,
    sessions: Arc<SessionServer>,
}

impl RunningServer {
    async fn stop(self) {
        self.transport.stop();
        self.accept_task.abort();
        let _ = self.accept_task.await;
        self.core.stop().await.expect("Server Core stop");
    }
}

async fn start_server(key: &SigningKey) -> RunningServer {
    let format = AudioFormat::new(Encoding::PcmFloat, 2, 48_000);
    let core = Server::start(
        CoreConfig {
            host: "127.0.0.1".into(),
            port: 0,
            ..CoreConfig::default()
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
    .expect("fresh isolated identity");
    let store = Arc::new(AuthorizationStore::in_memory());
    store.insert_for_tests(StoredClient {
        client_id: "0000000000000001".to_owned(),
        public_key: key
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
        display_name: CLIENT_NAME.to_owned(),
        state: StoreState::Active,
        created_at: String::new(),
        last_seen_at: String::new(),
    });
    let mut sessions = SessionServer::new(&identity, store);
    sessions.capture_source = Some(as_cmd::v2_capture_source(
        observer.clone(),
        core.media_tap(),
    ));
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

/// The Android-side Client: real machine, real framing decoder, one real
/// media session, all on a real QUIC connection.
struct AndroidClient {
    connection: Connection,
    send: SendStream,
    recv: RecvStream,
    decoder: FrameDecoder,
    machine: ClientMachine,
}

impl AndroidClient {
    async fn connect(server: &RunningServer, key: SigningKey) -> Self {
        let connection = v2_transport::test_support::connect_with_alpn(
            server.transport.local_addr(),
            ALPN_PROTOCOL,
        )
        .expect("test endpoint")
        .await
        .expect("QUIC handshake");
        assert!(
            connection.max_datagram_size().is_some(),
            "DATAGRAM negotiated"
        );
        let (send, recv) = connection.open_bi().await.expect("control stream");
        let mut exporter = [0u8; 32];
        connection
            .export_keying_material(&mut exporter, AUTH_EXPORTER_LABEL, b"")
            .expect("TLS exporter");
        let mut machine = ClientMachine::new();
        machine
            .set_auth_context(
                server.sessions.server_public_key.clone(),
                exporter.to_vec(),
                std::sync::Arc::new(audio_share_client_v2_native::signer::SoftwareSigner::new(
                    key,
                )),
            )
            .expect("signer context");
        Self {
            connection,
            send,
            recv,
            decoder: FrameDecoder::new(),
            machine,
        }
    }

    /// Feed the machine's outgoing frames onto the real control stream.
    async fn send_verdict(&mut self, verdict: Verdict) {
        if let Verdict::Send(frame) = verdict {
            let wire = android_encode(&frame).expect("Android encode");
            self.send.write_all(&wire).await.expect("write control");
        }
    }

    /// Read one Server control frame with the ANDROID decoder.
    async fn recv_frame(&mut self) -> AndroidFrame {
        let wire = read_wire(&mut self.recv).await;
        let frames = self.decoder.push(&wire).expect("Android framing decoder");
        assert_eq!(frames.len(), 1, "one framed control message");
        frames.into_iter().next().unwrap()
    }

    /// Deliver one frame to the real Android machine and send what it emits.
    async fn step(&mut self, frame: AndroidFrame) -> Option<Event> {
        let (result, event) = self.machine.on_frame(&frame);
        let verdict = result.expect("no local close during the happy path");
        self.send_verdict(verdict).await;
        event
    }
}

/// Full-stack: the Android Client's real auth/negotiate/subscribe runs over a
/// live authenticated QUIC connection against the real Rust Server, receives
/// real media DATAGRAMs into its real `MediaSession`, and reports §9
/// diagnostics that the Server records as playback evidence.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn android_client_plays_real_server_media_over_live_quic() {
    let key = new_key();
    let server = start_server(&key).await;
    let mut client = AndroidClient::connect(&server, key.clone()).await;

    // ---- real auth exchange: Android signs, Server verifies ----------------
    let init = client.machine.auth_init(sec1_compressed(&key));
    let init_verdict = client
        .machine
        .enter_authenticating(init)
        .expect("enter authenticating");
    client.send_verdict(init_verdict).await;

    let challenge = client.recv_frame().await;
    let (result, _) = client.machine.on_frame(&challenge);
    let proof = result.expect("auth proof");
    client.send_verdict(proof).await;

    let auth_result = client.recv_frame().await;
    let (result, _) = client.machine.on_frame(&auth_result);
    let negotiate = result.expect("negotiate");
    client.send_verdict(negotiate).await;

    // READY entry: NegotiateResponse then ConfigurationPublished. Both are
    // decoded by the Android FrameDecoder and folded by the Android machine.
    let negotiate_response = client.recv_frame().await;
    client.step(negotiate_response).await;
    let publication = client.recv_frame().await;
    let config_event = client.step(publication).await;
    assert_eq!(config_event, Some(Event::ConfigurationPublished(1)));
    assert_eq!(client.machine.state, ClientState::Ready);

    // ---- Subscribe → StreamStarted(real anchor) ---------------------------
    let subscribe = client.machine.request_subscribe();
    client.send_verdict(subscribe).await;
    let subscribe_response = client.recv_frame().await;
    assert_eq!(
        client.step(subscribe_response).await,
        Some(Event::Subscribed)
    );
    let stream_started = client.recv_frame().await;
    let anchor = match client.step(stream_started).await {
        Some(Event::StreamStarted(anchor)) => anchor,
        other => panic!("expected StreamStarted anchor, got {other:?}"),
    };
    assert_eq!(client.machine.state, ClientState::Playing);
    assert_ne!(anchor.stream_instance_id, 0);
    assert_eq!(anchor.config_generation, 1);

    // The Android media session is built from the configuration the Android
    // machine stored (its own decode of the Server's publication).
    let configuration = client.machine.config.expect("configuration");
    let expected_geometry = Configuration {
        config_generation: 1,
        codec: 1,
        pcm: Some(PcmParameters {
            sample_rate: 48_000,
            channels: 2,
            sample_format: 1,
        }),
    };
    assert_eq!(configuration, expected_geometry);

    let mut backing = vec![0u8; 256 * 1024];
    let ring = unsafe { MediaRing::attach(backing.as_mut_ptr(), backing.len()) };
    let mut media = MediaSession::start(
        anchor.stream_instance_id,
        anchor.config_generation,
        anchor.initial_timestamp,
        &configuration,
        Some(ring),
    )
    .expect("Android accepts the published geometry");

    // ---- real QUIC DATAGRAMs → the Android receiver -----------------------
    // The M4 send path paces from the shared capture; observed inter-datagram
    // gaps on a loaded host reach ~1.8 s (the M5 pacing investigation item in
    // the roadmap). Four datagrams therefore need a generous ceiling: this
    // test asserts wire correctness and cross-end delivery, not M5's latency
    // budget (p95 ≤ 80 ms), which is explicitly NOT part of M4.
    let mut delivered = 0usize;
    let mut last: Option<(u32, u64)> = None;
    while delivered < 4 {
        let datagram =
            tokio::time::timeout(Duration::from_secs(20), client.connection.read_datagram())
                .await
                .expect("media datagram timeout")
                .expect("datagram");
        // Independent normative oracle: 24-byte BE header by hand.
        assert!(datagram.len() > 24);
        let instance = u32::from_be_bytes(datagram[0..4].try_into().unwrap());
        let generation = u32::from_be_bytes(datagram[4..8].try_into().unwrap());
        let sequence = u32::from_be_bytes(datagram[8..12].try_into().unwrap());
        let timestamp = u64::from_be_bytes(datagram[12..20].try_into().unwrap());
        let flags = u32::from_be_bytes(datagram[20..24].try_into().unwrap());
        assert_eq!(instance, anchor.stream_instance_id);
        assert_eq!(generation, anchor.config_generation);
        assert_eq!((datagram.len() - 24) % 8, 0, "frame-aligned F32LE stereo");
        if let Some((prev_seq, prev_ts)) = last {
            let frames = ((datagram.len() - 24) / 8) as u64;
            assert_eq!(sequence, prev_seq + 1, "per-instance sequence");
            assert_eq!(timestamp, prev_ts + frames, "§7.2 frame clock");
            assert_eq!(flags & 0x1, 0, "STREAM_START once");
        } else {
            assert!(timestamp >= anchor.initial_timestamp, "at/after the anchor");
            assert_eq!(
                flags & 0x1,
                0x1,
                "STREAM_START on the instance's first datagram"
            );
        }
        last = Some((sequence, timestamp));

        assert_eq!(
            media.on_datagram(0, &datagram),
            DatagramOutcome::Delivered,
            "Android delivered real Server media"
        );
        delivered += 1;
    }

    // ---- §9: the Android receiver's real report reaches the Server --------
    // Only what AudioTrack actually accepted counts as playback evidence.
    media.record_playback(PlaybackReport {
        submitted_bytes: 8 * 64 * 4, // four datagrams' worth of frames
        underruns: 0,
        buffer_current_ms: 20,
    });
    let diagnostics = media.diagnostics(0);
    assert_eq!(diagnostics.stream_instance_id, anchor.stream_instance_id);
    assert_eq!(diagnostics.config_generation, anchor.config_generation);
    assert_eq!(diagnostics.cumulative_received, 4);
    assert_eq!(
        diagnostics.submitted_position,
        anchor.initial_timestamp + 256,
        "submitted_position is anchor + platform-accepted frames"
    );
    let report = AndroidFrame {
        body: Some(AndroidBody::PlaybackDiagnostics(diagnostics)),
    };
    let wire = android_encode(&report).expect("Android encodes diagnostics");
    client
        .send
        .write_all(&wire)
        .await
        .expect("write diagnostics");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let session_id = 1; // first session on this endpoint
    loop {
        if let Some(recorded) = server.sessions.playback_diagnostics(session_id) {
            assert_eq!(recorded.stream_instance_id, anchor.stream_instance_id);
            assert_eq!(recorded.cumulative_received, 4);
            assert_eq!(recorded.submitted_position, anchor.initial_timestamp + 256);
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Server must observe the report"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // ---- explicit close: one-way, no echo ---------------------------------
    let close = client.machine.user_stop();
    client.send_verdict(close).await;
    assert_eq!(client.machine.state, ClientState::Closed);
    client
        .connection
        .close(quinn::VarInt::from_u32(701), b"client stopping");
    server.stop().await;
}
