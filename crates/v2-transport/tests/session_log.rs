//! Server-side session log events (ticket 33).
//!
//! Support must be able to verify pairing, session establishment,
//! `ClientClosed(701)`, revocation, and identity reset from `server.log`.
//! These live tests drive the real QUIC driver and assert the emitted lines.
//!
//! Red line: a session log line may name the public `client_id`, the masked
//! source, the pairing mode, and the §6 code. It must never contain the SAS,
//! the nonce, the binding, the client proof, a private key, or raw audio.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};

use v2_transport::auth::{self, build_auth_context};
use v2_transport::auth_store::AuthorizationStore;
use v2_transport::codes::{ErrorCode, ALPN_PROTOCOL};
use v2_transport::driver::{SessionServer, AUTH_EXPORTER_LABEL};
use v2_transport::framing::{check_length_prefix, decode_body, encode_frame, LengthCheck};
use v2_transport::identity::{IdentityConfig, ServerIdentity};
use v2_transport::limits::{CapacityGuard, Limits};
use v2_transport::pairing::{self, PairingCore, PAIRING_EXPORTER_LABEL};
use v2_transport::schema::{
    frame, AuthInit, AuthProof, Frame, PairingConfirm, PairingMode, PairingRequest, PairingResponse,
};
use v2_transport::transport::{Transport, TransportConfig};

const CLIENT_SCALAR: &str = "87fb33cdf19dc9ec6324570648e8f354d26d5262e4930182382220bec2fdfac7";

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn fixture_key() -> SigningKey {
    SigningKey::from_slice(&unhex(CLIENT_SCALAR)).expect("fixture scalar")
}

fn public_key(key: &SigningKey) -> Vec<u8> {
    key.verifying_key()
        .to_encoded_point(true)
        .as_bytes()
        .to_vec()
}

fn client_id_hex(key: &SigningKey) -> String {
    auth::client_id(&public_key(key))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// ---------------------------------------------------------------------------
// Log capture (same seam as `tests/redaction.rs`)
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Capture {
    buffer: Arc<std::sync::Mutex<Vec<u8>>>,
}

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.buffer.lock().expect("capture lock")).into_owned()
    }
}

impl io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buffer
            .lock()
            .expect("capture lock")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Capture `tracing` events for the duration of the guard.
///
/// The tests are single-threaded runtimes: `set_default` is thread-local and
/// every spawned driver task runs on the same thread, so the capture sees
/// events from the whole server.
fn capture_logs() -> (Capture, tracing::subscriber::DefaultGuard) {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(capture.clone())
        .finish();
    (capture, tracing::subscriber::set_default(subscriber))
}

fn assert_logged(logs: &str, needle: &str) {
    assert!(
        logs.lines().any(|line| line.contains(needle)),
        "expected {needle:?} in:\n{logs}"
    );
}

// ---------------------------------------------------------------------------
// Server / client helpers
// ---------------------------------------------------------------------------

struct TestServer {
    _transport: Arc<Transport>,
    identity: Arc<ServerIdentity>,
    core: Arc<PairingCore>,
    address: SocketAddr,
    _dir: tempfile::TempDir,
}

impl TestServer {
    async fn start(store: AuthorizationStore) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let identity = Arc::new(
            ServerIdentity::open(&IdentityConfig {
                data_dir: dir.path().to_path_buf(),
                allow_loose_permissions: false,
            })
            .expect("identity"),
        );
        let store = Arc::new(store);
        let core = Arc::new(PairingCore::from_identity(&identity, Arc::clone(&store)));
        let limits = Limits::defaults();
        let transport = Arc::new(
            Transport::start(
                TransportConfig {
                    bind: "127.0.0.1:0".parse().expect("bind"),
                    limits: limits.clone(),
                },
                &identity,
            )
            .expect("start"),
        );
        let address = transport.local_addr();
        let mut server = SessionServer::new(&identity, Arc::clone(&store));
        server.pairing = Some(Arc::clone(&core));
        server.auth_deadline = Duration::from_secs(10);
        server.revocation_poll = Duration::from_millis(100);
        server.closing_grace = Duration::from_millis(150);
        let server = Arc::new(server);
        let accept_transport = Arc::clone(&transport);
        tokio::spawn(async move {
            let mut guard = CapacityGuard::new(limits);
            loop {
                let accepted = accept_transport.accept_one(&mut guard).await;
                let Some(connection) = accepted.connection else {
                    continue;
                };
                let mut session_guard = guard.clone();
                let server = Arc::clone(&server);
                let remote = accepted.remote_address;
                tokio::spawn(async move {
                    let _ = server.serve_connection(connection, remote).await;
                    session_guard.session_ended();
                });
            }
        });
        Self {
            _transport: transport,
            identity,
            core,
            address,
            _dir: dir,
        }
    }

    fn core(&self) -> Arc<PairingCore> {
        Arc::clone(&self.core)
    }
}

async fn connect(address: SocketAddr) -> quinn::Connection {
    v2_transport::test_support::connect_with_alpn(address, ALPN_PROTOCOL)
        .expect("client endpoint")
        .await
        .expect("QUIC handshake")
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
    let mut body = vec![0; body_len];
    tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut body))
        .await
        .expect("frame body timeout")
        .expect("frame body");
    decode_body(&body).expect("decodable frame")
}

async fn send_frame(send: &mut quinn::SendStream, body: frame::Body) {
    send.write_all(&encode_frame(&Frame { body: Some(body) }).expect("encode"))
        .await
        .expect("write frame");
}

async fn authenticate(
    connection: &quinn::Connection,
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    key: &SigningKey,
    server_public_key: &[u8],
) {
    send_frame(
        send,
        frame::Body::AuthInit(AuthInit {
            request_id: 1,
            client_public_key: public_key(key),
        }),
    )
    .await;
    let server_random = match read_frame(recv).await.body {
        Some(frame::Body::AuthChallenge(challenge)) => challenge.server_random,
        other => panic!("expected AuthChallenge, got {other:?}"),
    };
    let mut exporter = [0u8; 32];
    connection
        .export_keying_material(&mut exporter, AUTH_EXPORTER_LABEL, b"")
        .expect("TLS exporter");
    let context = build_auth_context(
        &server_random,
        &public_key(key),
        server_public_key,
        &exporter,
    );
    let signature: Signature = key.sign(&context);
    send_frame(
        send,
        frame::Body::AuthProof(AuthProof {
            request_id: 1,
            signature: signature.to_vec(),
        }),
    )
    .await;
    match read_frame(recv).await.body {
        Some(frame::Body::AuthResult(result)) => assert_eq!(result.status.code, 0),
        other => panic!("expected AuthResult, got {other:?}"),
    }
}

async fn authenticate_session(
    server: &TestServer,
    key: &SigningKey,
) -> (quinn::Connection, quinn::SendStream, quinn::RecvStream) {
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    authenticate(
        &connection,
        &mut send,
        &mut recv,
        key,
        server.identity.public_key_raw(),
    )
    .await;
    (connection, send, recv)
}

async fn await_app_close(connection: &quinn::Connection) -> u32 {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Session establishment and the Client's explicit stop are visible from the
/// Server side with the public client id and the §6 close code.
#[tokio::test]
async fn session_accept_and_client_stop_are_logged() {
    let (logs, _guard) = capture_logs();
    let key = fixture_key();
    let id = client_id_hex(&key);
    let store = AuthorizationStore::in_memory();
    store.insert_for_tests(v2_transport::auth_store::StoredClient {
        client_id: id.clone(),
        public_key: public_key(&key),
        display_name: "Test Phone".to_owned(),
        state: v2_transport::auth_store::ClientState::Active,
        created_at: String::new(),
        last_seen_at: String::new(),
    });
    let server = TestServer::start(store).await;
    let (connection, mut send, _recv) = authenticate_session(&server, &key).await;

    send_frame(
        &mut send,
        frame::Body::Close(v2_transport::schema::Close {
            code: ErrorCode::CLIENT_STOPPING.code,
            detail: String::new(),
        }),
    )
    .await;
    assert_eq!(
        await_app_close(&connection).await,
        ErrorCode::CLIENT_STOPPING.code
    );
    // Let the driver run its termination path before reading the capture.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let text = logs.text();
    assert_logged(&text, "event=session.accepted");
    assert_logged(&text, &format!("client_id={id}"));
    assert_logged(&text, "event=session.closed");
    assert_logged(&text, "code=701");
}

/// Revoking a paired Client is visible in the log, and the live session closes
/// with 204 (§9.2).
#[tokio::test]
async fn revocation_is_logged_and_closes_the_session() {
    let (logs, _guard) = capture_logs();
    let key = fixture_key();
    let id = client_id_hex(&key);
    let store = AuthorizationStore::in_memory();
    store.insert_for_tests(v2_transport::auth_store::StoredClient {
        client_id: id.clone(),
        public_key: public_key(&key),
        display_name: "Test Phone".to_owned(),
        state: v2_transport::auth_store::ClientState::Active,
        created_at: String::new(),
        last_seen_at: String::new(),
    });
    let server = TestServer::start(store).await;
    let (connection, _send, _recv) = authenticate_session(&server, &key).await;

    server.core().revoke(&id).expect("revoke");
    assert_eq!(
        await_app_close(&connection).await,
        ErrorCode::AUTHORIZATION_DENIED.code
    );
    tokio::time::sleep(Duration::from_millis(100)).await;

    let text = logs.text();
    assert_logged(&text, "event=client.revoked");
    assert_logged(&text, &format!("client_id={id}"));
    assert_logged(&text, "event=session.closed");
    assert_logged(&text, "code=204");
}

fn pairing_request(mode: PairingMode, nonce: Vec<u8>, key: &SigningKey) -> PairingRequest {
    PairingRequest {
        request_id: 1,
        pairing_mode: mode as i32,
        pairing_nonce: nonce,
        client_random: (0..32).collect(),
        client_public_key: public_key(key),
        protocol_major: 2,
        capabilities: Vec::new(),
        client_name: "Log Test Phone".to_owned(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Complete the Client half of the pairing proof and return the frame plus the
/// hex of the two secret values, so the redaction test can look for them.
async fn confirm_pairing(
    connection: &quinn::Connection,
    response: &PairingResponse,
    key: &SigningKey,
) -> (PairingConfirm, String, String) {
    let context = pairing::binding_context(
        PairingMode::try_from(response.pairing_mode).expect("mode"),
        &response.pairing_nonce,
        &response.client_random,
        &response.server_random,
        &public_key(key),
        &response.server_public_key,
        &response.capabilities,
        &response.display_name,
    )
    .expect("binding context");
    let mut binding = [0u8; 48];
    connection
        .export_keying_material(&mut binding, PAIRING_EXPORTER_LABEL, &context)
        .expect("TLS exporter");
    let proof: Signature = key.sign(&auth::build_pairing_proof_input(&binding));
    (
        PairingConfirm {
            request_id: response.request_id,
            binding_echo: binding.to_vec(),
            client_proof: proof.to_vec(),
        },
        hex(&binding),
        hex(&proof.to_vec()),
    )
}

/// Drive a QR pairing up to the operator decision. Returns the control stream,
/// the verified-but-undecided approval, and the secret values the log must
/// never contain.
async fn verified_pairing(
    server: &TestServer,
    nonce: Vec<u8>,
    key: &SigningKey,
) -> (
    quinn::Connection,
    quinn::SendStream,
    quinn::RecvStream,
    pairing::ApprovalRequest,
    String,
    String,
) {
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    send_frame(
        &mut send,
        frame::Body::PairingRequest(pairing_request(PairingMode::Qr, nonce, key)),
    )
    .await;
    let response = match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResponse(response)) => response,
        other => panic!("expected PairingResponse, got {other:?}"),
    };
    let (confirm, binding_hex, proof_hex) = confirm_pairing(&connection, &response, key).await;
    send_frame(&mut send, frame::Body::PairingConfirm(confirm)).await;
    let mut request = None;
    for _ in 0..200 {
        if let Some(pending) = server.core().pending_approvals().into_iter().next() {
            request = Some(pending);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let request = request.expect("the verified pairing never surfaced");
    (connection, send, recv, request, binding_hex, proof_hex)
}

/// Pairing presented, accepted, and declined are visible without the SAS or
/// any other pairing secret.
#[tokio::test]
async fn pairing_events_are_logged_without_pairing_secrets() {
    let (logs, _guard) = capture_logs();
    let key = SigningKey::random(&mut rand_core::OsRng);
    let id = client_id_hex(&key);
    let server = TestServer::start(AuthorizationStore::in_memory()).await;

    // Decline path.
    let invitation = server
        .core()
        .begin_qr(&format!("127.0.0.1:{}", server.address.port()));
    let nonce = invitation.nonce.clone();
    let (_connection, _send, mut recv, request, _binding, _proof) =
        verified_pairing(&server, nonce.clone(), &key).await;
    let sas = request.sas.clone().expect("the operator sees the SAS");
    server.core().decline(request.approval_id);
    match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResult(result)) => {
            assert_eq!(result.status.code, ErrorCode::PAIRING_REJECTED.code);
        }
        other => panic!("expected PairingResult(201), got {other:?}"),
    }

    // Approve path (a fresh invitation and a fresh connection).
    let invitation = server
        .core()
        .begin_qr(&format!("127.0.0.1:{}", server.address.port()));
    let (_connection, _send, mut recv, request, binding_hex, proof_hex) =
        verified_pairing(&server, invitation.nonce, &key).await;
    server.core().approve(request.approval_id);
    match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResult(result)) => assert_eq!(result.status.code, 0),
        other => panic!("expected PairingResult(OK), got {other:?}"),
    }
    tokio::time::sleep(Duration::from_millis(100)).await;

    let text = logs.text();
    assert_logged(&text, "event=pairing.presented");
    assert_logged(&text, &format!("client_id={id}"));
    assert_logged(&text, "event=pairing.declined");
    assert_logged(&text, "event=pairing.accepted");

    // Red line: SAS, nonce, binding, proof, and the private key must never
    // reach the log, and no binary/raw-audio bytes may appear.
    assert!(!text.contains(&sas), "the SAS leaked into the log:\n{text}");
    let nonce_hex = hex(&nonce);
    assert!(
        !text.contains(&nonce_hex),
        "the pairing nonce leaked into the log:\n{text}"
    );
    assert!(
        !text.contains(&binding_hex),
        "the binding leaked into the log:\n{text}"
    );
    assert!(
        !text.contains(&proof_hex),
        "the client proof leaked into the log:\n{text}"
    );
    assert!(
        !text.contains(CLIENT_SCALAR),
        "the private key scalar leaked into the log:\n{text}"
    );
    assert!(
        !text.contains("binding"),
        "a binding/proof reference reached the log:\n{text}"
    );
    assert!(
        !text.contains("client_proof"),
        "the client proof reached the log:\n{text}"
    );
    assert!(
        !text.contains('\0'),
        "binary/raw-audio bytes reached the log:\n{text}"
    );
}

/// The fail-closed identity reset is visible with the new public fingerprint.
#[tokio::test]
async fn identity_reset_is_logged_with_the_new_fingerprint() {
    let (logs, _guard) = capture_logs();
    let dir = tempfile::tempdir().expect("tempdir");
    let config = IdentityConfig {
        data_dir: dir.path().to_path_buf(),
        allow_loose_permissions: false,
    };
    let store = AuthorizationStore::in_memory();
    let identity = pairing::reset_identity(&config, &store).expect("reset");
    let text = logs.text();
    assert_logged(&text, "event=identity.reset");
    assert_logged(&text, &format!("fingerprint={}", identity.fingerprint()));
}
