#![cfg(unix)]
//! Ticket 20: cross-end pairing interop — the real Android Client machine and
//! signing seam against the real production Server Pairing Core, over live
//! QUIC.
//!
//! These tests do not replay one end's encoder against itself:
//! - the Server is `v2_transport`'s production `PairingCore` + `SessionServer`;
//! - the Client is the Android native crate's `ClientMachine`, `FrameDecoder`,
//!   `IdentitySigner` and binding/SAS derivation;
//! - the SAS/fingerprint displayed by both ends is compared;
//! - the Server persists the Client only after verifying the Android
//!   `client_proof`.
//!
//! QR transcript, SAS transcript, wrong fingerprint, unknown identity,
//! revoked identity, identity change (pin mismatch), and a live-QUIC pairing
//! that reaches READY are covered.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use audio_share_client_v2_native::control::{ClientMachine, ClientState, Event, Verdict};
use audio_share_client_v2_native::frame::{encode_frame as android_encode, FrameDecoder};
use audio_share_client_v2_native::pairing::{ClientPairingMode, PairingExporter, PairingPhase};
use audio_share_client_v2_native::proto::frame::Body as AndroidBody;
use audio_share_client_v2_native::proto::Frame as AndroidFrame;
use audio_share_client_v2_native::session::{Session, SessionConfig, SessionIdentity};
use audio_share_client_v2_native::signer::SoftwareSigner;
use p256::ecdsa::SigningKey;
use quinn::{Connection, RecvStream, SendStream};
use v2_transport::auth_store::AuthorizationStore;
use v2_transport::codes::{ErrorCode, ALPN_PROTOCOL};
use v2_transport::driver::{SessionServer, AUTH_EXPORTER_LABEL};
use v2_transport::identity::{IdentityConfig, ServerIdentity};
use v2_transport::limits::{CapacityGuard, Limits};
use v2_transport::pairing::{ApprovalRequest, PairingCore};
use v2_transport::schema::PairingMode;
use v2_transport::transport::{Transport, TransportConfig};

fn new_key() -> SigningKey {
    SigningKey::random(&mut rand_core::OsRng)
}

fn public_key(key: &SigningKey) -> Vec<u8> {
    key.verifying_key()
        .to_encoded_point(true)
        .as_bytes()
        .to_vec()
}

/// Real TLS-Exporter adapter for the Android pairing derivation.
struct QuicExporter {
    conn: Connection,
}

impl std::fmt::Debug for QuicExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuicExporter").finish()
    }
}

impl PairingExporter for QuicExporter {
    fn export(&self, label: &[u8], context: &[u8], len: usize) -> Result<Vec<u8>, String> {
        let mut out = vec![0u8; len];
        self.conn
            .export_keying_material(&mut out, label, context)
            .map_err(|_| "TLS exporter unavailable".to_string())?;
        Ok(out)
    }
}

struct PairingServer {
    identity: Arc<ServerIdentity>,
    store: Arc<AuthorizationStore>,
    core: Arc<PairingCore>,
    address: SocketAddr,
    _transport: Arc<Transport>,
    _dir: tempfile::TempDir,
}

impl PairingServer {
    async fn start() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let identity = Arc::new(
            ServerIdentity::open(&IdentityConfig {
                data_dir: dir.path().to_path_buf(),
                allow_loose_permissions: false,
            })
            .expect("identity"),
        );
        let store = Arc::new(AuthorizationStore::in_memory());
        let core = Arc::new(PairingCore::from_identity(&identity, Arc::clone(&store)));
        let limits = Limits::defaults();
        let transport = Arc::new(
            Transport::start(
                TransportConfig {
                    bind: "127.0.0.1:0".parse().unwrap(),
                    limits: limits.clone(),
                },
                &identity,
            )
            .expect("transport"),
        );
        let address = transport.local_addr();
        let mut server = SessionServer::new(&identity, Arc::clone(&store));
        server.pairing = Some(Arc::clone(&core));
        server.auth_deadline = Duration::from_secs(10);
        server.revocation_poll = Duration::from_millis(50);
        server.closing_grace = Duration::from_millis(100);
        let server = Arc::new(server);
        let accepting = Arc::clone(&transport);
        tokio::spawn(async move {
            let mut guard = CapacityGuard::new(limits);
            loop {
                let accepted = accepting.accept_one(&mut guard).await;
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
            identity,
            store,
            core,
            address,
            _transport: transport,
            _dir: dir,
        }
    }

    fn server_key(&self) -> Vec<u8> {
        self.identity.public_key_raw().to_vec()
    }
}

async fn connect(address: SocketAddr) -> Connection {
    v2_transport::test_support::connect_with_alpn(address, ALPN_PROTOCOL)
        .expect("client endpoint")
        .await
        .expect("QUIC handshake")
}

async fn send_android(send: &mut SendStream, frame: &AndroidFrame) {
    let wire = android_encode(frame).expect("Android encode");
    send.write_all(&wire).await.expect("write control");
}

async fn recv_android(recv: &mut RecvStream, decoder: &mut FrameDecoder) -> AndroidFrame {
    let mut prefix = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(10), recv.read_exact(&mut prefix))
        .await
        .expect("frame prefix timeout")
        .expect("frame prefix");
    let len = u32::from_be_bytes(prefix) as usize;
    assert!((1..=65536).contains(&len), "control length {len}");
    let mut wire = prefix.to_vec();
    let mut body = vec![0u8; len];
    tokio::time::timeout(Duration::from_secs(10), recv.read_exact(&mut body))
        .await
        .expect("frame body timeout")
        .expect("frame body");
    wire.extend_from_slice(&body);
    let frames = decoder.push(&wire).expect("Android framing decoder");
    assert_eq!(frames.len(), 1, "one framed control message");
    frames.into_iter().next().unwrap()
}

/// Build the Android pairing machine and return the exact `PairingRequest`
/// bytes its encoder produced.
fn pairing_machine(
    mode: ClientPairingMode,
    nonce: Vec<u8>,
    key: &SigningKey,
    exporter: Arc<dyn PairingExporter>,
    expected_server_key: Option<Vec<u8>>,
) -> (ClientMachine, AndroidFrame) {
    let mut random = [0u8; 32];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut random);
    let mut machine = ClientMachine::new();
    let verdict = machine
        .begin_pairing(
            mode,
            nonce,
            random,
            "Interop Phone".into(),
            Arc::new(SoftwareSigner::new(key.clone())),
            exporter,
            expected_server_key,
        )
        .expect("begin pairing");
    let Verdict::Send(frame) = verdict else {
        panic!("expected PairingRequest");
    };
    (machine, frame)
}

async fn wait_for_approval(core: &PairingCore) -> ApprovalRequest {
    for _ in 0..500 {
        let pending = core.pending_approvals();
        if !pending.is_empty() {
            return pending.into_iter().next().unwrap();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the Server never surfaced an approval request");
}

async fn authenticate_with_android_machine(server: &PairingServer, key: &SigningKey) -> u32 {
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    let mut auth_exporter = [0u8; 32];
    connection
        .export_keying_material(&mut auth_exporter, AUTH_EXPORTER_LABEL, b"")
        .expect("auth exporter");
    let mut machine = ClientMachine::new();
    machine
        .set_auth_context(
            server.server_key(),
            auth_exporter.to_vec(),
            Arc::new(SoftwareSigner::new(key.clone())),
        )
        .expect("auth context");
    let init = machine.auth_init(public_key(key));
    let Verdict::Send(init) = machine.enter_authenticating(init).expect("enter auth") else {
        panic!("expected AuthInit");
    };
    send_android(&mut send, &init).await;

    let mut decoder = FrameDecoder::new();
    let challenge = recv_android(&mut recv, &mut decoder).await;
    let (verdict, _) = machine.on_frame(&challenge);
    let Verdict::Send(proof) = verdict.expect("auth proof") else {
        panic!("expected AuthProof");
    };
    send_android(&mut send, &proof).await;
    let result = recv_android(&mut recv, &mut decoder).await;
    match result.body {
        Some(AndroidBody::AuthResult(result)) => result.status.map(|s| s.code).unwrap_or(0),
        other => panic!("expected AuthResult, got {other:?}"),
    }
}

/// Complete a pairing and return the machine (now NEGOTIATING) and the
/// Android-derived credential. Also asserts the cross-end SAS match.
async fn pair(
    server: &PairingServer,
    mode: ClientPairingMode,
) -> (
    ClientMachine,
    audio_share_client_v2_native::pairing::PairedCredential,
    Connection,
) {
    let key = new_key();
    let invitation = match mode {
        ClientPairingMode::Qr => server
            .core
            .begin_qr(&format!("127.0.0.1:{}", server.address.port())),
        ClientPairingMode::Sas => server.core.begin_sas(),
    };
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    let exporter: Arc<dyn PairingExporter> = Arc::new(QuicExporter {
        conn: connection.clone(),
    });
    let expected = (mode == ClientPairingMode::Qr).then(|| server.server_key());
    let (mut machine, request) =
        pairing_machine(mode, invitation.nonce.clone(), &key, exporter, expected);
    send_android(&mut send, &request).await;

    let mut decoder = FrameDecoder::new();
    let response = recv_android(&mut recv, &mut decoder).await;
    let (result, event) = machine.on_frame(&response);
    assert_eq!(result.expect("verified response"), Verdict::Ok);
    let Event::PairingReady(info) = event.expect("PairingReady") else {
        panic!("expected PairingReady");
    };
    assert_eq!(info.phase, PairingPhase::AwaitingConfirmation);

    // Send the proof first: the Server registers the operator approval only
    // after verifying `client_proof` (pairing.md §6.3.1), and then holds it
    // until the operator decides.
    let Verdict::Send(confirm) = machine.confirm_pairing().expect("confirm") else {
        panic!("expected PairingConfirm");
    };
    send_android(&mut send, &confirm).await;

    // Cross-end SAS/fingerprint: the Server's derivation must equal the
    // Android derivation for the same connection.
    let approval = wait_for_approval(&server.core).await;
    assert_eq!(approval.mode, expected_mode(mode));
    assert_eq!(approval.sas.as_deref(), info.sas.as_deref());
    assert!(approval.sas.as_deref().unwrap().contains(' '));
    assert!(server.core.approve(approval.request_id));

    let result_frame = recv_android(&mut recv, &mut decoder).await;
    let (result, event) = machine.on_frame(&result_frame);
    let Verdict::Send(negotiate) = result.expect("PairingResult OK") else {
        panic!("expected Negotiate after PairingResult(OK)");
    };
    let Event::Paired(credential) = event.expect("Paired") else {
        panic!("expected Paired");
    };
    assert_eq!(
        server.store.clients()[0].public_key,
        public_key(&key),
        "the Server persisted exactly the Android identity"
    );
    send_android(&mut send, &negotiate).await;
    let negotiation = recv_android(&mut recv, &mut decoder).await;
    let (result, _) = machine.on_frame(&negotiation);
    result.expect("NegotiateResponse");
    assert_eq!(machine.state, ClientState::Ready);
    (machine, credential, connection)
}

fn expected_mode(mode: ClientPairingMode) -> PairingMode {
    match mode {
        ClientPairingMode::Qr => PairingMode::Qr,
        ClientPairingMode::Sas => PairingMode::Sas,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qr_pairing_completes_over_live_quic_with_cross_end_sas_and_proof() {
    let server = PairingServer::start().await;
    let (_machine, credential, _connection) = pair(&server, ClientPairingMode::Qr).await;
    assert_eq!(credential.client_id.len(), 16, "8-byte client id as hex");
    assert_eq!(
        credential.server_public_key_hex,
        server
            .server_key()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    assert_eq!(
        credential.server_spki_sha256_hex,
        server
            .identity
            .spki_sha256()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    assert_eq!(server.store.clients().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sas_pairing_completes_over_live_quic_and_reaches_ready() {
    let server = PairingServer::start().await;
    let (_machine, credential, _connection) = pair(&server, ClientPairingMode::Sas).await;
    assert_eq!(server.store.clients().len(), 1);
    assert!(!credential.display_name.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_qr_server_key_aborts_with_205_and_persists_nothing() {
    let server = PairingServer::start().await;
    let key = new_key();
    let invitation = server
        .core
        .begin_qr(&format!("127.0.0.1:{}", server.address.port()));
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    let exporter: Arc<dyn PairingExporter> = Arc::new(QuicExporter {
        conn: connection.clone(),
    });
    let (mut machine, request) = pairing_machine(
        ClientPairingMode::Qr,
        invitation.nonce,
        &key,
        exporter,
        Some(vec![0x42; 32]),
    );
    send_android(&mut send, &request).await;
    let mut decoder = FrameDecoder::new();
    let response = recv_android(&mut recv, &mut decoder).await;
    let (result, _) = machine.on_frame(&response);
    assert_eq!(
        result,
        Err(audio_share_client_v2_native::control::CloseInfo {
            code: 205,
            remote: false
        })
    );
    // No confirm was sent, so no approval and no persistence.
    assert!(server.store.clients().is_empty());
    assert!(server.core.pending_approvals().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_identity_is_202_and_revoked_identity_is_204() {
    let server = PairingServer::start().await;

    // Pair a working identity first.
    let key = new_key();
    let invitation = server.core.begin_sas();
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    let exporter: Arc<dyn PairingExporter> = Arc::new(QuicExporter {
        conn: connection.clone(),
    });
    let (mut machine, request) = pairing_machine(
        ClientPairingMode::Sas,
        invitation.nonce,
        &key,
        exporter,
        None,
    );
    send_android(&mut send, &request).await;
    let mut decoder = FrameDecoder::new();
    let response = recv_android(&mut recv, &mut decoder).await;
    let (_, event) = machine.on_frame(&response);
    assert!(matches!(event, Some(Event::PairingReady(_))));
    let Verdict::Send(confirm) = machine.confirm_pairing().expect("confirm") else {
        panic!("expected confirm");
    };
    send_android(&mut send, &confirm).await;
    let approval = wait_for_approval(&server.core).await;
    assert!(server.core.approve(approval.request_id));
    let result = recv_android(&mut recv, &mut decoder).await;
    let (_, event) = machine.on_frame(&result);
    assert!(matches!(event, Some(Event::Paired(_))));

    // Never-paired identity → 202; revoked paired identity → 204.
    assert_eq!(
        authenticate_with_android_machine(&server, &new_key()).await,
        ErrorCode::AUTHORIZATION_UNKNOWN.code
    );
    server.store.revoke(&public_key(&key)).expect("revoke");
    assert_eq!(
        authenticate_with_android_machine(&server, &key).await,
        ErrorCode::AUTHORIZATION_DENIED.code
    );
}

/// Full Android `Session` path: the JNI-facing session sends PairingRequest,
/// exposes SAS/fingerprint, waits for the confirm channel, sends the proof,
/// and reaches READY using only the freshly paired credential.
///
/// Synchronous for the same runtime-drop reason as the identity-change test.
#[test]
fn android_session_pairing_and_confirm_channel_reach_ready() {
    let rt = tokio::runtime::Runtime::new().expect("server runtime");
    let server = rt.block_on(PairingServer::start());
    let invitation = server
        .core
        .begin_qr(&format!("127.0.0.1:{}", server.address.port()));
    let pin =
        audio_share_client_v2_native::pairing::server_pin_hex(&server.server_key()).expect("pin");

    let session = Arc::new(
        Session::new(SessionConfig {
            host: "127.0.0.1".into(),
            port: server.address.port(),
            client_name: "interop-phone".into(),
            identity: SessionIdentity::PairQr {
                pinned_spki_hex: pin,
                server_public_key: server.server_key(),
                nonce: invitation.nonce,
                display_name: "Interop Phone".into(),
                signer: Arc::new(SoftwareSigner::new(new_key())),
            },
        })
        .expect("session"),
    );
    session.connect(None).expect("connect");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !session.pairing_json().contains("AWAITING_CONFIRMATION") {
        assert!(
            std::time::Instant::now() < deadline,
            "session never surfaced the pairing verification: {}",
            session.pairing_json()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let pairing = session.pairing_json();
    assert!(pairing.contains("\"sas\":\""));
    assert!(pairing.contains("\"fingerprint\":\""));
    assert!(!session.credential_json().contains("server_spki_sha256"));

    session.confirm_pairing().expect("confirm channel");
    let approval = rt.block_on(wait_for_approval(&server.core));
    assert!(server.core.approve(approval.request_id));

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !session.credential_json().contains("server_spki_sha256") {
        assert!(
            std::time::Instant::now() < deadline,
            "session never received PairingResult(OK): {} / {}",
            session.credential_json(),
            session.stats_json()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(session.state_name(), "READY");
    let credential = session.credential_json();
    assert!(credential.contains(&format!("\"client_id\":\"{}\"", approval.client_id)));
    assert_eq!(server.store.clients().len(), 1);
    assert_eq!(server.store.clients()[0].client_id, approval.client_id);
    drop(session);
    drop(rt);
}

/// Server identity change: a stored pin from Server A must reject Server B
/// with the local 205 classification (never a silent re-pair).
///
/// This is a synchronous test: `Session` owns its own tokio runtime, and a
/// runtime must not be dropped inside another async context.
#[test]
fn server_identity_change_is_a_local_205_not_a_silent_repair() {
    let rt = tokio::runtime::Runtime::new().expect("server runtime");
    let (server_a, server_b) =
        rt.block_on(async { (PairingServer::start().await, PairingServer::start().await) });
    let pin_a_hex: String = server_a
        .identity
        .spki_sha256()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let session = Arc::new(
        Session::new(SessionConfig {
            host: "127.0.0.1".into(),
            port: server_b.address.port(),
            client_name: "interop-client".into(),
            identity: SessionIdentity::Auth {
                pinned_spki_hex: pin_a_hex,
                server_public_key: server_a.server_key(),
                signer: Arc::new(SoftwareSigner::new(new_key())),
            },
        })
        .expect("session"),
    );
    session.connect(None).expect("connect");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let stats = session.stats_json();
        if stats.contains("\"close_code\":205") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "expected local 205 for the changed Server identity, got {stats}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(server_b.store.clients().is_empty());
    drop(session);
    drop(rt);
}
