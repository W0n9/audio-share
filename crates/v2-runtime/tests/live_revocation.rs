//! Live revocation through the shared runtime's Pairing Core admin surface.
//!
//! The GUI revokes by `client_id`; that call must reach the same
//! `AuthorizationStore` the session driver polls. This test pairs a real
//! client over QUIC, revokes through `PairingAdmin`, and asserts the active
//! session closes with 204 well inside the 5 s bound (pairing.md §9.2).

use std::sync::Arc;
use std::time::Duration;

use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use v2_runtime::admin::{PairingAdmin, RequestedPairingMode};
use v2_runtime::{RuntimeConfig, V2Runtime};
use v2_transport::auth;
use v2_transport::codes::{ErrorCode, ALPN_PROTOCOL};
use v2_transport::framing::{check_length_prefix, decode_body, encode_frame, LengthCheck};
use v2_transport::pairing::{self, PAIRING_EXPORTER_LABEL};
use v2_transport::schema::{
    frame, Frame, PairingConfirm, PairingMode as WireMode, PairingRequest, PairingResponse,
};

fn public_key(key: &SigningKey) -> Vec<u8> {
    key.verifying_key()
        .to_encoded_point(true)
        .as_bytes()
        .to_vec()
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
    recv.read_exact(&mut body).await.expect("frame body");
    decode_body(&body).expect("decodable frame")
}

async fn send_frame(send: &mut quinn::SendStream, body: frame::Body) {
    send.write_all(&encode_frame(&Frame { body: Some(body) }).expect("encode"))
        .await
        .expect("write frame");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoking_through_the_admin_surface_closes_the_active_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = V2Runtime::start(
        RuntimeConfig {
            data_dir: dir.path().to_path_buf(),
            allow_loose_permissions: false,
            bind_host: "127.0.0.1".to_owned(),
            port: 0,
            addr_hint: None,
        },
        None,
    )
    .await
    .expect("runtime");
    let admin = runtime.admin();
    admin
        .begin(RequestedPairingMode::Sas)
        .expect("open SAS window");

    // Approve the verified pairing exactly as the operator would in the GUI.
    let mut approvals = runtime.pairing().subscribe_approvals();
    let approving: Arc<dyn PairingAdmin> = Arc::clone(&admin);
    let approver = tokio::spawn(async move {
        if let Ok(request) = approvals.recv().await {
            assert!(request.sas.is_some(), "the operator sees the SAS");
            assert!(approving.approve(request.approval_id));
        }
    });

    let key = SigningKey::random(&mut rand_core::OsRng);
    let connection =
        v2_transport::test_support::connect_with_alpn(runtime.local_addr(), ALPN_PROTOCOL)
            .expect("client endpoint")
            .await
            .expect("QUIC handshake");
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    send_frame(
        &mut send,
        frame::Body::PairingRequest(PairingRequest {
            request_id: 1,
            pairing_mode: WireMode::Sas as i32,
            pairing_nonce: Vec::new(),
            client_random: vec![0x42; 32],
            client_public_key: public_key(&key),
            protocol_major: 2,
            capabilities: Vec::new(),
            client_name: "GUI Revocation Phone".to_owned(),
        }),
    )
    .await;
    let response: PairingResponse = match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResponse(response)) => response,
        other => panic!("expected PairingResponse, got {other:?}"),
    };

    let context = pairing::binding_context(
        WireMode::Sas,
        &[],
        &response.client_random,
        &response.server_random,
        &public_key(&key),
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
    send_frame(
        &mut send,
        frame::Body::PairingConfirm(PairingConfirm {
            request_id: 1,
            binding_echo: binding.to_vec(),
            client_proof: proof.to_vec(),
        }),
    )
    .await;
    match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResult(result)) => assert_eq!(result.status.code, 0),
        other => panic!("expected PairingResult(OK), got {other:?}"),
    }
    approver.await.expect("approver task");

    let client = runtime
        .admin()
        .clients()
        .into_iter()
        .find(|client| client.display_name == "GUI Revocation Phone")
        .expect("the paired client is listed");
    assert_eq!(client.client_id, client_id(&key));

    // The GUI's revoke command path.
    let revoked = runtime
        .admin()
        .revoke(&client.client_id)
        .expect("revoke through admin");
    assert_eq!(revoked.state, v2_runtime::admin::ClientStateView::Revoked);

    // The driver's authorization poll closes the live session with 204.
    let close = loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut recv))
            .await
            .expect("revocation close within 5 s");
        if let Some(frame::Body::Close(close)) = frame.body {
            break close;
        }
    };
    assert_eq!(close.code, ErrorCode::AUTHORIZATION_DENIED.code);
    // The Close frame above is the spec's "session closed within 5 s". The
    // transport close follows the §5.1 five-second closing grace, so allow
    // that grace on top of the revocation bound here.
    let error = tokio::time::timeout(Duration::from_secs(8), connection.closed())
        .await
        .expect("transport close");
    match error {
        quinn::ConnectionError::ApplicationClosed(close) => assert_eq!(
            u32::try_from(close.error_code.into_inner()).unwrap(),
            ErrorCode::AUTHORIZATION_DENIED.code
        ),
        other => panic!("expected application close, got {other:?}"),
    }

    runtime.stop();
}

fn client_id(key: &SigningKey) -> String {
    auth::client_id(&public_key(key))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
