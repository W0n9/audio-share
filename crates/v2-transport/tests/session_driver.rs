//! Live-QUIC integration tests for the ticket-07 session driver
//! (`v2_transport::driver`): the §5.2 machine end to end over a real quinn
//! connection, including the PR-9 auth exchange with the real TLS-Exporter,
//! the auth deadline (601), malformed-message refusal (401), the per-Client
//! supersede path (303), the source-address rate limit (302), and revocation
//! (204 ≤ 5 s).
//!
//! The client half is deliberately minimal and test-only: it accepts any
//! certificate (production SPKI pinning is ticket 08's deliverable) and signs
//! with the fixture's TEST-ONLY scalar
//! (`fixtures/wire-protocol-v2/test-keys.json`, pairing.md §11.3).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};

use v2_transport::auth::build_auth_context;
use v2_transport::auth_store::{AuthorizationStore, ClientState, StoredClient};
use v2_transport::codes::{ErrorCode, ALPN_PROTOCOL};
use v2_transport::driver::{SessionServer, AUTH_EXPORTER_LABEL};
use v2_transport::framing::{check_length_prefix, decode_body, encode_frame, LengthCheck};
use v2_transport::identity::{IdentityConfig, ServerIdentity};
use v2_transport::limits::{CapacityGuard, Limits};
use v2_transport::schema::{
    frame, AuthInit, AuthProof, AuthResult, Frame, Negotiate, Status, Subscribe,
};
use v2_transport::transport::{Transport, TransportConfig};

/// TEST-ONLY client identity scalar (`fixtures/wire-protocol-v2/test-keys.json`).
const CLIENT_SCALAR: &str = "87fb33cdf19dc9ec6324570648e8f354d26d5262e4930182382220bec2fdfac7";

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn client_key() -> SigningKey {
    SigningKey::from_slice(&unhex(CLIENT_SCALAR)).expect("fixture scalar")
}

fn client_public_key(key: &SigningKey) -> Vec<u8> {
    key.verifying_key()
        .to_encoded_point(true)
        .as_bytes()
        .to_vec()
}

fn store_with_client() -> AuthorizationStore {
    let store = AuthorizationStore::in_memory();
    store.insert_for_tests(StoredClient {
        client_id: "cf7940b474310246".to_owned(),
        public_key: client_public_key(&client_key()),
        display_name: "Test Phone".to_owned(),
        state: ClientState::Active,
    });
    store
}

fn loopback() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

/// A running v2 endpoint: transport accept loop + one `SessionServer`.
struct TestServer {
    transport: Arc<Transport>,
    identity: Arc<ServerIdentity>,
    address: SocketAddr,
    store: Arc<AuthorizationStore>,
    _dir: tempfile::TempDir,
}

impl TestServer {
    async fn start(
        store: AuthorizationStore,
        limits: Limits,
        tweak: impl FnOnce(&mut SessionServer),
    ) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let identity = Arc::new(
            ServerIdentity::open(&IdentityConfig {
                data_dir: dir.path().to_path_buf(),
                allow_loose_permissions: false,
            })
            .expect("identity"),
        );
        let transport = Arc::new(
            Transport::start(
                TransportConfig {
                    bind: loopback(),
                    limits: limits.clone(),
                },
                &identity,
            )
            .expect("start"),
        );
        let address = transport.local_addr();
        let store = Arc::new(store);
        let mut server = SessionServer::new(&identity, Arc::clone(&store));
        server.limits = limits.clone();
        // Tests shorten the §5.1 grace and the ≤5 s revocation poll; the
        // production defaults are asserted separately in `driver.rs`.
        server.closing_grace = Duration::from_millis(150);
        server.revocation_poll = Duration::from_millis(100);
        tweak(&mut server);
        let server = Arc::new(server);

        let accept_transport = Arc::clone(&transport);
        let accept_server = Arc::clone(&server);
        tokio::spawn(async move {
            let mut guard = CapacityGuard::new(limits);
            loop {
                let accepted = accept_transport.accept_one(&mut guard).await;
                let Some(connection) = accepted.connection else {
                    continue;
                };
                let mut session_guard = guard.clone();
                let server = Arc::clone(&accept_server);
                let remote = accepted.remote_address;
                tokio::spawn(async move {
                    let _ = server.serve_connection(connection, remote).await;
                    session_guard.session_ended();
                });
            }
        });

        Self {
            transport,
            identity,
            address,
            store,
            _dir: dir,
        }
    }
}

async fn connect(address: SocketAddr) -> quinn::Connection {
    connect_with_alpn(address, ALPN_PROTOCOL)
        .expect("client endpoint")
        .await
        .expect("handshake")
}

fn connect_with_alpn(address: SocketAddr, alpn: &[u8]) -> Result<quinn::Connecting, String> {
    let provider = rustls::crypto::ring::default_provider();
    let mut crypto = rustls::ClientConfig::builder_with_provider(provider.into())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("TLS 1.3 available")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAny))
        .with_no_client_auth();
    crypto.alpn_protocols = vec![alpn.to_vec()];
    let endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap())
        .map_err(|error| error.to_string())?;
    let config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
            .map_err(|error| error.to_string())?,
    ));
    endpoint
        .connect_with(config, address, "audio-share-server.local")
        .map_err(|error| error.to_string())
}

/// Test-only verifier: accepts any certificate (pairing.md §11.3).
#[derive(Debug)]
struct AcceptAny;

impl rustls::client::danger::ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls_pki_types::CertificateDer<'_>,
        _intermediates: &[rustls_pki_types::CertificateDer<'_>],
        _server_name: &rustls_pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _certificate: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _certificate: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![rustls::SignatureScheme::ED25519]
    }
}

async fn read_frame(recv: &mut quinn::RecvStream) -> Frame {
    let mut prefix = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut prefix))
        .await
        .expect("frame prefix timeout")
        .expect("frame prefix");
    let body_len = match check_length_prefix(prefix) {
        LengthCheck::Accept { body_len } => body_len,
        other => panic!("invalid length prefix: {other:?}"),
    };
    let mut body = vec![0u8; body_len];
    tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut body))
        .await
        .expect("frame body timeout")
        .expect("frame body");
    decode_body(&body).expect("decodable frame")
}

async fn send_frame(send: &mut quinn::SendStream, frame: &Frame) {
    send.write_all(&encode_frame(frame).expect("encode"))
        .await
        .expect("write frame");
}

fn auth_init(request_id: u64, public_key: &[u8]) -> Frame {
    Frame {
        body: Some(frame::Body::AuthInit(AuthInit {
            request_id,
            client_public_key: public_key.to_vec(),
        })),
    }
}

fn auth_proof(request_id: u64, signature: Vec<u8>) -> Frame {
    Frame {
        body: Some(frame::Body::AuthProof(AuthProof {
            request_id,
            signature,
        })),
    }
}

fn subscribe(request_id: u64) -> Frame {
    Frame {
        body: Some(frame::Body::Subscribe(Subscribe { request_id })),
    }
}

fn negotiate(request_id: u64, major: u32, capabilities: &[u32]) -> Frame {
    Frame {
        body: Some(frame::Body::Negotiate(Negotiate {
            request_id,
            protocol_major: major,
            protocol_minor: 0,
            capabilities: capabilities.to_vec(),
        })),
    }
}

/// PR-9: single ECDSA-P256-SHA256 over the RAW auth_context, signed with the
/// Test Client's fixture identity key.
async fn authenticate(
    connection: &quinn::Connection,
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    server_public_key: &[u8],
    request_id: u64,
) -> AuthResult {
    let key = client_key();
    let public_key = client_public_key(&key);
    send_frame(send, &auth_init(request_id, &public_key)).await;
    let server_random = match read_frame(recv).await.body {
        Some(frame::Body::AuthChallenge(challenge)) => {
            assert_eq!(challenge.request_id, request_id);
            assert_eq!(
                challenge.server_random.len(),
                32,
                "32-byte CSPRNG challenge"
            );
            challenge.server_random
        }
        other => panic!("expected AuthChallenge, got {other:?}"),
    };
    let mut exporter = [0u8; 32];
    connection
        .export_keying_material(&mut exporter, AUTH_EXPORTER_LABEL, b"")
        .expect("TLS exporter");
    let context = build_auth_context(&server_random, &public_key, server_public_key, &exporter);
    let signature: Signature = key.sign(&context);
    send_frame(send, &auth_proof(request_id, signature.to_vec())).await;
    match read_frame(recv).await.body {
        Some(frame::Body::AuthResult(result)) => result,
        other => panic!("expected AuthResult, got {other:?}"),
    }
}

fn close_code(frame: &Frame) -> Option<u32> {
    match &frame.body {
        Some(frame::Body::Close(close)) => Some(close.code),
        _ => None,
    }
}

fn error_code(frame: &Frame) -> Option<u32> {
    match &frame.body {
        Some(frame::Body::Error(error)) => Some(error.status.code),
        _ => None,
    }
}

fn status_code(status: &Status) -> u32 {
    status.code
}

async fn await_close(connection: &quinn::Connection) -> u32 {
    let error = tokio::time::timeout(Duration::from_secs(5), connection.closed())
        .await
        .expect("connection close timeout");
    match error {
        quinn::ConnectionError::ApplicationClosed(close) => {
            u32::try_from(close.error_code.into_inner()).unwrap()
        }
        other => panic!("expected application close, got {other:?}"),
    }
}

#[tokio::test]
async fn auth_happy_path_reaches_ready_over_quic() {
    let server = TestServer::start(store_with_client(), Limits::defaults(), |_| {}).await;
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");

    let result = authenticate(
        &connection,
        &mut send,
        &mut recv,
        server.identity.public_key_raw(),
        1,
    )
    .await;
    assert_eq!(result.request_id, 1);
    assert_eq!(status_code(&result.status), 0, "auth succeeds");
    assert_eq!(result.session_id, 1, "first Server-assigned session id");

    send_frame(&mut send, &negotiate(2, 2, &[])).await;
    match read_frame(&mut recv).await.body {
        Some(frame::Body::NegotiateResponse(response)) => {
            assert_eq!(response.request_id, 2);
            assert_eq!(status_code(&response.status), 0);
            assert_eq!(response.protocol_major, 2);
        }
        other => panic!("expected NegotiateResponse, got {other:?}"),
    }

    // §5.2 READY: Subscribe is served with a SubscribeResponse (no capture
    // is wired yet, so no StreamStarted push is expected).
    send_frame(&mut send, &subscribe(3)).await;
    match read_frame(&mut recv).await.body {
        Some(frame::Body::SubscribeResponse(response)) => {
            assert_eq!(response.request_id, 3);
            assert_eq!(status_code(&response.status), 0);
        }
        other => panic!("expected SubscribeResponse, got {other:?}"),
    }

    connection.close(quinn::VarInt::from_u32(701), b"");
    server.transport.stop();
}

#[tokio::test]
async fn second_control_stream_is_invalid_state_402() {
    let server = TestServer::start(store_with_client(), Limits::defaults(), |_| {}).await;
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    // QUIC streams only become visible to the peer once bytes are sent, so
    // open the control stream with a real AuthInit first.
    send_frame(&mut send, &auth_init(1, &client_public_key(&client_key()))).await;
    assert!(matches!(
        read_frame(&mut recv).await.body,
        Some(frame::Body::AuthChallenge(_))
    ));
    // A second Client-initiated stream while the control stream is live is the
    // §3/§5.2 "second stream" row: Error(402)+Close(402).
    let (mut second_send, _second_recv) = connection.open_bi().await.expect("second stream");
    second_send
        .write_all(b"\0")
        .await
        .expect("second stream byte");

    assert_eq!(
        error_code(&read_frame(&mut recv).await),
        Some(ErrorCode::INVALID_STATE.code)
    );
    assert_eq!(
        close_code(&read_frame(&mut recv).await),
        Some(ErrorCode::INVALID_STATE.code)
    );
    assert_eq!(
        await_close(&connection).await,
        ErrorCode::INVALID_STATE.code
    );
    server.transport.stop();
}

#[tokio::test]
async fn auth_deadline_closes_with_601() {
    let server = TestServer::start(store_with_client(), Limits::defaults(), |server| {
        server.auth_deadline = Duration::from_millis(150);
    })
    .await;
    let connection = connect(server.address).await;
    // Never open the control stream: the 10 s auth-exchange budget (§3)
    // expires and the Server transport-closes 601.
    assert_eq!(
        await_close(&connection).await,
        ErrorCode::NETWORK_FAILURE.code
    );
    server.transport.stop();
}

#[tokio::test]
async fn empty_length_prefix_is_malformed_401() {
    let server = TestServer::start(store_with_client(), Limits::defaults(), |_| {}).await;
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    // A zero-length prefix is §4.4 malformed: Error(401)+Close(401), then the
    // transport close carries 401 after the grace.
    send.write_all(&0u32.to_be_bytes())
        .await
        .expect("empty frame");

    assert_eq!(
        error_code(&read_frame(&mut recv).await),
        Some(ErrorCode::MALFORMED_MESSAGE.code)
    );
    assert_eq!(
        close_code(&read_frame(&mut recv).await),
        Some(ErrorCode::MALFORMED_MESSAGE.code)
    );
    assert_eq!(
        await_close(&connection).await,
        ErrorCode::MALFORMED_MESSAGE.code
    );
    server.transport.stop();
}

#[tokio::test]
async fn revoked_identity_is_torn_down_with_204() {
    let server = TestServer::start(store_with_client(), Limits::defaults(), |_| {}).await;
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    let result = authenticate(
        &connection,
        &mut send,
        &mut recv,
        server.identity.public_key_raw(),
        1,
    )
    .await;
    assert_eq!(status_code(&result.status), 0);

    // Operator revokes the identity; the driver polls and must tear the live
    // session down with 204 within 5 s (pairing.md §9.2 / PR-13).
    server
        .store
        .revoke(&client_public_key(&client_key()))
        .expect("revoke");

    assert_eq!(
        error_code(&read_frame(&mut recv).await),
        Some(ErrorCode::AUTHORIZATION_DENIED.code)
    );
    assert_eq!(
        close_code(&read_frame(&mut recv).await),
        Some(ErrorCode::AUTHORIZATION_DENIED.code)
    );
    assert_eq!(
        await_close(&connection).await,
        ErrorCode::AUTHORIZATION_DENIED.code
    );
    server.transport.stop();
}

#[tokio::test]
async fn version_mismatch_is_101() {
    let server = TestServer::start(store_with_client(), Limits::defaults(), |_| {}).await;
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    authenticate(
        &connection,
        &mut send,
        &mut recv,
        server.identity.public_key_raw(),
        1,
    )
    .await;

    // §5.2 NEGOTIATING: major ≠ 2 → Error(101)+Close(101) → CLOSING.
    send_frame(&mut send, &negotiate(2, 1, &[])).await;
    let error = read_frame(&mut recv).await;
    assert_eq!(error_code(&error), Some(ErrorCode::VERSION_MISMATCH.code));
    match &error.body {
        Some(frame::Body::Error(error)) => assert_eq!(error.request_id, 2, "echoes the request id"),
        other => panic!("expected Error, got {other:?}"),
    }
    assert_eq!(
        close_code(&read_frame(&mut recv).await),
        Some(ErrorCode::VERSION_MISMATCH.code)
    );
    assert_eq!(
        await_close(&connection).await,
        ErrorCode::VERSION_MISMATCH.code
    );
    server.transport.stop();
}

/// Five failed proofs on one connection: AuthResult(203) for attempts 1–4,
/// then the 5th outcome (Close(203) by default, or the address budget's
/// Close(302) when the source limit is already reached).
async fn five_failed_proofs(send: &mut quinn::SendStream, recv: &mut quinn::RecvStream) -> u32 {
    let public_key = client_public_key(&client_key());
    for attempt in 1..=4u64 {
        send_frame(send, &auth_init(attempt, &public_key)).await;
        match read_frame(recv).await.body {
            Some(frame::Body::AuthChallenge(_)) => {}
            other => panic!("expected AuthChallenge, got {other:?}"),
        }
        // Structurally valid, cryptographically invalid: r||s from zeroes.
        send_frame(send, &auth_proof(attempt, vec![0u8; 64])).await;
        match read_frame(recv).await.body {
            Some(frame::Body::AuthResult(result)) => {
                assert_eq!(
                    status_code(&result.status),
                    ErrorCode::AUTHENTICATION_FAILED.code
                );
            }
            other => panic!("expected AuthResult(203), got {other:?}"),
        }
    }
    send_frame(send, &auth_init(5, &public_key)).await;
    match read_frame(recv).await.body {
        Some(frame::Body::AuthChallenge(_)) => {}
        other => panic!("expected AuthChallenge, got {other:?}"),
    }
    send_frame(send, &auth_proof(5, vec![0u8; 64])).await;
    let last = read_frame(recv).await;
    close_code(&last).unwrap_or_else(|| panic!("expected Close, got {:?}", last.body))
}

#[tokio::test]
async fn per_connection_and_per_source_auth_failure_limits() {
    let limits = Limits {
        auth_failures_per_window: 20,
        auth_failure_window: Duration::from_secs(60),
        ..Limits::defaults()
    };
    let server = TestServer::start(store_with_client(), limits, |_| {}).await;

    // Three connections burn 15 of the 20-per-60 s source budget; each still
    // ends with the per-connection Close(203) (TM-4/TM-5).
    for _ in 0..3 {
        let connection = connect(server.address).await;
        let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
        assert_eq!(
            five_failed_proofs(&mut send, &mut recv).await,
            ErrorCode::AUTHENTICATION_FAILED.code
        );
        connection.close(
            quinn::VarInt::from_u32(ErrorCode::AUTHENTICATION_FAILED.code),
            b"",
        );
    }

    // The fourth connection's fifth failure reaches the address budget: the
    // source-limit verdict wins and the close carries 302 (fixtures/error-302).
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    assert_eq!(
        five_failed_proofs(&mut send, &mut recv).await,
        ErrorCode::RATE_LIMITED.code
    );
    connection.close(quinn::VarInt::from_u32(ErrorCode::RATE_LIMITED.code), b"");

    // A fifth connection is refused at AuthInit: the source is over budget.
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    send_frame(&mut send, &auth_init(1, &client_public_key(&client_key()))).await;
    assert_eq!(
        close_code(&read_frame(&mut recv).await),
        Some(ErrorCode::RATE_LIMITED.code)
    );
    connection.close(quinn::VarInt::from_u32(ErrorCode::RATE_LIMITED.code), b"");

    server.transport.stop();
}

#[tokio::test]
async fn new_session_supersedes_the_old_with_303() {
    let server = TestServer::start(store_with_client(), Limits::defaults(), |_| {}).await;

    let first = connect(server.address).await;
    let (mut first_send, mut first_recv) = first.open_bi().await.expect("first control stream");
    let first_result = authenticate(
        &first,
        &mut first_send,
        &mut first_recv,
        server.identity.public_key_raw(),
        1,
    )
    .await;
    assert_eq!(status_code(&first_result.status), 0);
    assert_eq!(first_result.session_id, 1);

    let second = connect(server.address).await;
    let (mut second_send, mut second_recv) = second.open_bi().await.expect("second control stream");
    let second_result = authenticate(
        &second,
        &mut second_send,
        &mut second_recv,
        server.identity.public_key_raw(),
        1,
    )
    .await;
    assert_eq!(status_code(&second_result.status), 0);
    assert_eq!(second_result.session_id, 2, "registry-wide numbering");

    // TM-3: the old session is closed with Close(303) and transport 303.
    assert_eq!(
        close_code(&read_frame(&mut first_recv).await),
        Some(ErrorCode::SESSION_SUPERSEDED.code)
    );
    assert_eq!(
        await_close(&first).await,
        ErrorCode::SESSION_SUPERSEDED.code
    );

    // The replacement keeps its session open.
    send_frame(&mut second_send, &negotiate(2, 2, &[])).await;
    match read_frame(&mut second_recv).await.body {
        Some(frame::Body::NegotiateResponse(response)) => {
            assert_eq!(status_code(&response.status), 0)
        }
        other => panic!("expected NegotiateResponse, got {other:?}"),
    }

    second.close(quinn::VarInt::from_u32(701), b"");
    server.transport.stop();
}
