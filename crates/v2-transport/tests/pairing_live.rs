//! Live-QUIC pairing integration (ticket 19): a real quinn connection drives
//! the production `PairingCore` through QR and SAS pairing, the explicit
//! local-approval state, persistence, and post-pairing authorization.
//!
//! The Client half derives the real TLS exporters and signs with a fresh
//! P-256 key. No fixture private material is used for the live exchange.

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
    frame, AuthInit, Frame, Negotiate, PairingConfirm, PairingMode, PairingRequest,
    PairingResponse, Status,
};
use v2_transport::transport::{Transport, TransportConfig};

fn loopback() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

fn public_key(key: &SigningKey) -> Vec<u8> {
    key.verifying_key()
        .to_encoded_point(true)
        .as_bytes()
        .to_vec()
}

struct PairingServer {
    _transport: Arc<Transport>,
    identity: Arc<ServerIdentity>,
    store: Arc<AuthorizationStore>,
    core: Arc<PairingCore>,
    address: SocketAddr,
    _dir: tempfile::TempDir,
}

impl PairingServer {
    async fn start(auth_deadline: Duration) -> Self {
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
                    bind: loopback(),
                    limits: limits.clone(),
                },
                &identity,
            )
            .expect("start"),
        );
        let address = transport.local_addr();
        let mut server = SessionServer::new(&identity, Arc::clone(&store));
        server.pairing = Some(Arc::clone(&core));
        server.auth_deadline = auth_deadline;
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
            store,
            core,
            address,
            _dir: dir,
        }
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

fn pairing_request(mode: PairingMode, nonce: Vec<u8>, key: &SigningKey) -> PairingRequest {
    PairingRequest {
        request_id: 1,
        pairing_mode: mode as i32,
        pairing_nonce: nonce,
        client_random: (0..32).collect(),
        client_public_key: public_key(key),
        protocol_major: 2,
        capabilities: Vec::new(),
        client_name: "Live Test Phone".to_owned(),
    }
}

/// Complete the Client half of the pairing proof against the Server's
/// response: derive `binding` from the real TLS exporter and sign the §6.3.1
/// proof input.
async fn pairing_confirm(
    connection: &quinn::Connection,
    response: &PairingResponse,
    key: &SigningKey,
) -> PairingConfirm {
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
    PairingConfirm {
        request_id: response.request_id,
        binding_echo: binding.to_vec(),
        client_proof: proof.to_vec(),
    }
}

async fn negotiate(send: &mut quinn::SendStream, recv: &mut quinn::RecvStream) {
    send_frame(
        send,
        frame::Body::Negotiate(Negotiate {
            request_id: 2,
            protocol_major: 2,
            protocol_minor: 0,
            capabilities: Vec::new(),
        }),
    )
    .await;
    match read_frame(recv).await.body {
        Some(frame::Body::NegotiateResponse(response)) => {
            assert_eq!(response.status.code, 0);
        }
        other => panic!("expected NegotiateResponse, got {other:?}"),
    }
}

async fn await_auth_result(
    connection: quinn::Connection,
    key: &SigningKey,
    server_public_key: &[u8],
) -> u32 {
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    send_frame(
        &mut send,
        frame::Body::AuthInit(AuthInit {
            request_id: 1,
            client_public_key: public_key(key),
        }),
    )
    .await;
    let server_random = match read_frame(&mut recv).await.body {
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
        &mut send,
        frame::Body::AuthProof(v2_transport::schema::AuthProof {
            request_id: 1,
            signature: signature.to_vec(),
        }),
    )
    .await;
    match read_frame(&mut recv).await.body {
        Some(frame::Body::AuthResult(result)) => result.status.code,
        other => panic!("expected AuthResult, got {other:?}"),
    }
}

async fn wait_for_pending(core: &PairingCore) -> Vec<pairing::ApprovalRequest> {
    for _ in 0..200 {
        let pending = core.pending_approvals();
        if !pending.is_empty() {
            return pending;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the verified pairing never surfaced an approval request");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_concurrent_pairings_with_the_same_request_id_approve_independently() {
    let server = PairingServer::start(Duration::from_secs(10)).await;
    let key_a = SigningKey::random(&mut rand_core::OsRng);
    let key_b = SigningKey::random(&mut rand_core::OsRng);
    let client_a = client_id_hex(&key_a);
    let client_b = client_id_hex(&key_b);
    let invitation_a = server
        .core
        .begin_qr(&format!("127.0.0.1:{}", server.address.port()));
    let invitation_b = server
        .core
        .begin_qr(&format!("127.0.0.1:{}", server.address.port()));

    // Both connections use the same wire request_id (1); the pairing core must
    // still keep two independent local approvals.
    let (conn_a, mut send_a, mut recv_a) =
        verified_pairing(&server, invitation_a.nonce, &key_a, "Phone A").await;
    let (conn_b, mut send_b, mut recv_b) =
        verified_pairing(&server, invitation_b.nonce, &key_b, "Phone B").await;

    let pending = loop {
        let pending = server.core.pending_approvals();
        if pending.len() == 2 {
            break pending;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let request_a = pending
        .iter()
        .find(|request| request.client_id == client_a)
        .expect("Phone A pending");
    let request_b = pending
        .iter()
        .find(|request| request.client_id == client_b)
        .expect("Phone B pending");
    assert_ne!(request_a.approval_id, request_b.approval_id);
    assert_ne!(
        request_a.sas, request_b.sas,
        "the two connections must not share a SAS"
    );
    assert_eq!(request_a.display_name, "Phone A");
    assert_eq!(request_b.display_name, "Phone B");

    // Approving A stores only A; B stays pending with its own SAS/identity.
    assert!(server.core.approve(request_a.approval_id));
    match read_frame(&mut recv_a).await.body {
        Some(frame::Body::PairingResult(result)) => assert_eq!(result.status.code, 0),
        other => panic!("expected PairingResult(OK) for A, got {other:?}"),
    }
    let stored = server.store.clients();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].client_id, client_a);
    assert_eq!(stored[0].display_name, "Phone A");
    let still_pending = server.core.pending_approvals();
    assert_eq!(still_pending.len(), 1);
    assert_eq!(still_pending[0].client_id, client_b);
    assert_eq!(still_pending[0].approval_id, request_b.approval_id);

    assert!(server.core.approve(request_b.approval_id));
    match read_frame(&mut recv_b).await.body {
        Some(frame::Body::PairingResult(result)) => assert_eq!(result.status.code, 0),
        other => panic!("expected PairingResult(OK) for B, got {other:?}"),
    }
    assert_eq!(server.store.clients().len(), 2);
    negotiate(&mut send_b, &mut recv_b).await;
    negotiate(&mut send_a, &mut recv_a).await;
    let _ = (conn_a, conn_b);
}

/// Complete PairingRequest → PairingResponse → PairingConfirm and return the
/// still-open control stream (the exchange is verified but unapproved).
async fn verified_pairing(
    server: &PairingServer,
    nonce: Vec<u8>,
    key: &SigningKey,
    name: &str,
) -> (quinn::Connection, quinn::SendStream, quinn::RecvStream) {
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    let mut request = pairing_request(PairingMode::Qr, nonce, key);
    request.client_name = name.to_owned();
    send_frame(&mut send, frame::Body::PairingRequest(request)).await;
    let response = match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResponse(response)) => response,
        other => panic!("expected PairingResponse, got {other:?}"),
    };
    let confirm = pairing_confirm(&connection, &response, key).await;
    send_frame(&mut send, frame::Body::PairingConfirm(confirm)).await;
    (connection, send, recv)
}

fn client_id_hex(key: &SigningKey) -> String {
    auth::client_id(&public_key(key))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_cancel_rejects_a_verified_pairing_with_201_promptly() {
    let server = PairingServer::start(Duration::from_secs(10)).await;
    let key = SigningKey::random(&mut rand_core::OsRng);
    let invitation = server
        .core
        .begin_qr(&format!("127.0.0.1:{}", server.address.port()));
    let (_connection, _send, mut recv) =
        verified_pairing(&server, invitation.nonce, &key, "Cancelled Phone").await;
    let pending = wait_for_pending(&server.core).await;
    assert_eq!(pending.len(), 1);
    assert_eq!(server.store.clients().len(), 0);

    // Operator closes the pairing dialog: the waiting Client must get
    // PairingResult(201)+Close(201) now, not after the 10-minute deadline.
    server.core.cancel();
    match tokio::time::timeout(Duration::from_secs(2), read_frame(&mut recv))
        .await
        .expect("cancel must notify the waiting Client promptly")
        .body
    {
        Some(frame::Body::PairingResult(result)) => {
            assert_eq!(result.status.code, ErrorCode::PAIRING_REJECTED.code);
            assert!(result.status.detail.is_empty());
        }
        other => panic!("expected PairingResult(201), got {other:?}"),
    }
    match read_frame(&mut recv).await.body {
        Some(frame::Body::Close(close)) => {
            assert_eq!(close.code, ErrorCode::PAIRING_REJECTED.code)
        }
        other => panic!("expected Close(201), got {other:?}"),
    }
    assert!(server.core.pending_approvals().is_empty());
    assert!(server.store.clients().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qr_pairing_survives_the_auth_deadline_and_persists_after_approval() {
    // The 10 s auth deadline must not kill a pairing exchange: override it to
    // 100 ms and let the exchange outlive it before the proof arrives.
    let server = PairingServer::start(Duration::from_millis(100)).await;
    let key = SigningKey::random(&mut rand_core::OsRng);
    let invitation = server
        .core
        .begin_qr(&format!("127.0.0.1:{}", server.address.port()));
    let mut approvals = server.core.subscribe_approvals();
    let core = Arc::clone(&server.core);
    let approver = tokio::spawn(async move {
        if let Ok(request) = approvals.recv().await {
            assert_eq!(request.display_name, "Live Test Phone");
            assert_eq!(request.mode, PairingMode::Qr);
            assert!(request.sas.is_some(), "the operator sees the SAS");
            core.approve(request.approval_id);
        }
    });

    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    send_frame(
        &mut send,
        frame::Body::PairingRequest(pairing_request(
            PairingMode::Qr,
            invitation.nonce.clone(),
            &key,
        )),
    )
    .await;
    let response = match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResponse(response)) => response,
        other => panic!("expected PairingResponse, got {other:?}"),
    };
    assert_eq!(response.status.code, 0);
    assert_eq!(
        &response.server_public_key,
        server.identity.public_key_raw()
    );

    // Outlive the (shortened) auth deadline; the pairing deadline owns this
    // exchange now.
    tokio::time::sleep(Duration::from_millis(250)).await;

    let confirm = pairing_confirm(&connection, &response, &key).await;
    send_frame(&mut send, frame::Body::PairingConfirm(confirm)).await;
    let result = read_frame(&mut recv).await;
    match result.body {
        Some(frame::Body::PairingResult(result)) => assert_eq!(result.status.code, 0),
        other => panic!("expected PairingResult(OK), got {other:?}"),
    }
    approver.await.expect("approver task");

    // The freshly paired session is registered and can proceed to READY.
    negotiate(&mut send, &mut recv).await;
    let clients = server.store.clients();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].display_name, "Live Test Phone");
    assert!(!clients[0].created_at.is_empty());

    // Revocation terminates the active session promptly (≤ 5 s; poll 100 ms).
    server
        .store
        .revoke(&public_key(&key))
        .expect("revoke paired client");
    let close = loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut recv))
            .await
            .expect("revocation close within 5 s");
        if let Some(frame::Body::Close(close)) = frame.body {
            break close;
        }
    };
    assert_eq!(close.code, ErrorCode::AUTHORIZATION_DENIED.code);
    let error = tokio::time::timeout(Duration::from_secs(5), connection.closed())
        .await
        .expect("transport close");
    match error {
        quinn::ConnectionError::ApplicationClosed(close) => {
            assert_eq!(
                u32::try_from(close.error_code.into_inner()).unwrap(),
                ErrorCode::AUTHORIZATION_DENIED.code
            );
        }
        other => panic!("expected application close, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairing_holds_persistence_until_approval_then_session_is_usable() {
    let server = PairingServer::start(Duration::from_secs(10)).await;
    let key = SigningKey::random(&mut rand_core::OsRng);
    let invitation = server
        .core
        .begin_qr(&format!("127.0.0.1:{}", server.address.port()));
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    send_frame(
        &mut send,
        frame::Body::PairingRequest(pairing_request(PairingMode::Qr, invitation.nonce, &key)),
    )
    .await;
    let response = match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResponse(response)) => response,
        other => panic!("expected PairingResponse, got {other:?}"),
    };
    let confirm = pairing_confirm(&connection, &response, &key).await;
    send_frame(&mut send, frame::Body::PairingConfirm(confirm)).await;

    // Verified but unapproved: no result, nothing persisted.
    assert!(
        tokio::time::timeout(Duration::from_millis(300), read_frame(&mut recv))
            .await
            .is_err(),
        "no PairingResult before the operator decides"
    );
    assert!(server.store.clients().is_empty());
    let pending = wait_for_pending(&server.core).await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].display_name, "Live Test Phone");

    assert!(server.core.approve(pending[0].approval_id));
    match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResult(result)) => assert_eq!(result.status.code, 0),
        other => panic!("expected PairingResult(OK), got {other:?}"),
    }
    assert_eq!(server.store.clients().len(), 1);
    negotiate(&mut send, &mut recv).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declined_pairing_reports_201_and_persists_nothing() {
    let server = PairingServer::start(Duration::from_secs(10)).await;
    let key = SigningKey::random(&mut rand_core::OsRng);
    let invitation = server
        .core
        .begin_qr(&format!("127.0.0.1:{}", server.address.port()));
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    send_frame(
        &mut send,
        frame::Body::PairingRequest(pairing_request(PairingMode::Qr, invitation.nonce, &key)),
    )
    .await;
    let response = match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResponse(response)) => response,
        other => panic!("expected PairingResponse, got {other:?}"),
    };
    let confirm = pairing_confirm(&connection, &response, &key).await;
    send_frame(&mut send, frame::Body::PairingConfirm(confirm)).await;
    let pending = wait_for_pending(&server.core).await;
    assert_eq!(pending.len(), 1);
    assert!(server.core.decline(pending[0].approval_id));
    match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResult(result)) => {
            assert_eq!(result.status.code, ErrorCode::PAIRING_REJECTED.code);
            assert!(result.status.detail.is_empty());
        }
        other => panic!("expected PairingResult(201), got {other:?}"),
    }
    match read_frame(&mut recv).await.body {
        Some(frame::Body::Close(close)) => {
            assert_eq!(close.code, ErrorCode::PAIRING_REJECTED.code)
        }
        other => panic!("expected Close(201), got {other:?}"),
    }
    assert!(server.store.clients().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sas_pairing_uses_the_admin_window_and_real_exporter() {
    let server = PairingServer::start(Duration::from_secs(10)).await;
    let key = SigningKey::random(&mut rand_core::OsRng);
    let invitation = server.core.begin_sas();
    assert_eq!(invitation.mode, PairingMode::Sas);
    assert!(server.core.sas_window_open());
    let mut approvals = server.core.subscribe_approvals();
    let core = Arc::clone(&server.core);
    tokio::spawn(async move {
        if let Ok(request) = approvals.recv().await {
            assert_eq!(request.mode, PairingMode::Sas);
            core.approve(request.approval_id);
        }
    });

    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    send_frame(
        &mut send,
        frame::Body::PairingRequest(pairing_request(PairingMode::Sas, Vec::new(), &key)),
    )
    .await;
    let response = match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResponse(response)) => response,
        other => panic!("expected PairingResponse, got {other:?}"),
    };
    assert!(response.pairing_nonce.is_empty());
    let confirm = pairing_confirm(&connection, &response, &key).await;
    send_frame(&mut send, frame::Body::PairingConfirm(confirm)).await;
    match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResult(result)) => assert_eq!(result.status.code, 0),
        other => panic!("expected PairingResult(OK), got {other:?}"),
    }
    assert_eq!(server.store.clients().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_and_revoked_identities_are_refused_live() {
    let server = PairingServer::start(Duration::from_secs(10)).await;
    let paired = SigningKey::random(&mut rand_core::OsRng);
    let invitation = server
        .core
        .begin_qr(&format!("127.0.0.1:{}", server.address.port()));
    let mut approvals = server.core.subscribe_approvals();
    let core = Arc::clone(&server.core);
    tokio::spawn(async move {
        if let Ok(request) = approvals.recv().await {
            core.approve(request.approval_id);
        }
    });
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    send_frame(
        &mut send,
        frame::Body::PairingRequest(pairing_request(PairingMode::Qr, invitation.nonce, &paired)),
    )
    .await;
    let response = match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResponse(response)) => response,
        other => panic!("expected PairingResponse, got {other:?}"),
    };
    let confirm = pairing_confirm(&connection, &response, &paired).await;
    send_frame(&mut send, frame::Body::PairingConfirm(confirm)).await;
    match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResult(result)) => assert_eq!(result.status.code, 0),
        other => panic!("expected PairingResult(OK), got {other:?}"),
    }

    // A never-paired identity is AUTHORIZATION_UNKNOWN (202).
    let stranger = SigningKey::random(&mut rand_core::OsRng);
    assert_eq!(
        await_auth_result(
            connect(server.address).await,
            &stranger,
            server.identity.public_key_raw(),
        )
        .await,
        ErrorCode::AUTHORIZATION_UNKNOWN.code
    );

    // A revoked identity is AUTHORIZATION_DENIED (204) immediately.
    server.store.revoke(&public_key(&paired)).expect("revoke");
    assert_eq!(
        await_auth_result(
            connect(server.address).await,
            &paired,
            server.identity.public_key_raw(),
        )
        .await,
        ErrorCode::AUTHORIZATION_DENIED.code
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_pairing_capabilities_are_401_live() {
    let server = PairingServer::start(Duration::from_secs(10)).await;
    let key = SigningKey::random(&mut rand_core::OsRng);
    let connection = connect(server.address).await;
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    let mut request = pairing_request(PairingMode::Qr, vec![0xaa; 16], &key);
    request.capabilities = vec![2, 1];
    send_frame(&mut send, frame::Body::PairingRequest(request)).await;
    match read_frame(&mut recv).await.body {
        Some(frame::Body::Error(error)) => {
            assert_eq!(error.status.code, ErrorCode::MALFORMED_MESSAGE.code);
            assert!(error.status.detail.is_empty());
        }
        other => panic!("expected Error(401), got {other:?}"),
    }
    match read_frame(&mut recv).await.body {
        Some(frame::Body::Close(close)) => {
            assert_eq!(close.code, ErrorCode::MALFORMED_MESSAGE.code)
        }
        other => panic!("expected Close(401), got {other:?}"),
    }
    let error = tokio::time::timeout(Duration::from_secs(5), connection.closed())
        .await
        .expect("transport close");
    match error {
        quinn::ConnectionError::ApplicationClosed(close) => assert_eq!(
            u32::try_from(close.error_code.into_inner()).unwrap(),
            ErrorCode::MALFORMED_MESSAGE.code
        ),
        other => panic!("expected application close, got {other:?}"),
    }
}

// Keep the status helper used by the compile-time-facing assertions above.
#[allow(dead_code)]
fn status_code(status: &Status) -> u32 {
    status.code
}
