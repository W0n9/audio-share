//! Pairing administration seam tests: a real Pairing Core over an in-memory
//! store, no QUIC, no GUI.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use p256::ecdsa::SigningKey;
use v2_runtime::admin::{ClientStateView, CorePairingAdmin, PairingAdmin, RequestedPairingMode};
use v2_transport::auth_store::{Authorization, AuthorizationStore};
use v2_transport::pairing::{self, ApprovalRequest, PairingCore};
use v2_transport::schema::PairingMode as WirePairingMode;

fn admin() -> (
    Arc<CorePairingAdmin>,
    Arc<PairingCore>,
    Arc<AuthorizationStore>,
) {
    let store = Arc::new(AuthorizationStore::in_memory());
    let core = Arc::new(PairingCore::new(vec![7u8; 32], Arc::clone(&store)));
    let admin = Arc::new(CorePairingAdmin::new(
        Arc::clone(&core),
        "192.168.1.20:65531",
    ));
    (admin, core, store)
}

fn client_key() -> Vec<u8> {
    let key = SigningKey::random(&mut rand_core::OsRng);
    key.verifying_key()
        .to_encoded_point(true)
        .as_bytes()
        .to_vec()
}

fn approval(request_id: u64, core: &PairingCore, accepted: Arc<AtomicBool>) {
    core.request_approval(
        ApprovalRequest {
            approval_id: request_id,
            client_id: "0011223344556677".to_owned(),
            display_name: "Pixel 8".to_owned(),
            mode: WirePairingMode::Qr,
            address: "192.168.1.44:50000".to_owned(),
            sas: Some("123 456".to_owned()),
        },
        Box::new(move |ok| {
            accepted.store(ok, Ordering::SeqCst);
        }),
    );
}

#[test]
fn qr_begin_returns_a_parseable_payload_and_an_svg() {
    let (admin, core, _store) = admin();
    let invitation = admin.begin(RequestedPairingMode::Qr).expect("begin QR");

    assert_eq!(invitation.mode, RequestedPairingMode::Qr);
    assert_eq!(invitation.fingerprint, core.fingerprint());
    assert!(invitation.expires_in_secs > 0);
    let uri = invitation.qr_uri.as_deref().expect("QR URI");
    let parsed = pairing::parse_qr_uri(uri).expect("QR payload parses");
    assert_eq!(parsed.server_public_key, *core.server_public_key());
    assert_eq!(parsed.addr_hint, "192.168.1.20:65531");
    let svg = invitation.qr_svg.as_deref().expect("QR SVG");
    assert!(svg.starts_with("<svg"), "unexpected SVG: {svg}");
    assert!(svg.contains("<path"), "QR SVG has no modules");

    // QR mode also opens the camera-less SAS fallback window.
    assert!(core.sas_window_open());
}

#[test]
fn sas_begin_opens_the_window_without_a_qr() {
    let (admin, core, _store) = admin();
    let invitation = admin.begin(RequestedPairingMode::Sas).expect("begin SAS");
    assert_eq!(invitation.mode, RequestedPairingMode::Sas);
    assert!(invitation.qr_uri.is_none());
    assert!(invitation.qr_svg.is_none());
    assert!(core.sas_window_open());

    admin.cancel();
    assert!(!core.sas_window_open());
}

#[test]
fn pending_approvals_are_listed_and_approve_routes_to_the_core() {
    let (admin, core, _store) = admin();
    let accepted = Arc::new(AtomicBool::new(false));
    approval(41, &core, Arc::clone(&accepted));

    let pending = admin.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].display_name, "Pixel 8");
    assert_eq!(pending[0].sas.as_deref(), Some("123 456"));
    assert_eq!(pending[0].mode, RequestedPairingMode::Qr);

    assert!(admin.approve(41));
    assert!(accepted.load(Ordering::SeqCst));
    assert!(admin.pending().is_empty());
    assert!(!admin.approve(41), "an approval decision is single-use");
}

#[test]
fn decline_is_explicit_and_does_not_authorize() {
    let (admin, core, _store) = admin();
    let accepted = Arc::new(AtomicBool::new(true));
    approval(42, &core, Arc::clone(&accepted));

    assert!(admin.decline(42));
    assert!(!accepted.load(Ordering::SeqCst));
    assert!(admin.pending().is_empty());
}

#[test]
fn clients_list_and_revoke_use_the_shared_store() {
    let (admin, core, store) = admin();
    assert!(admin.clients().is_empty());

    let public_key = client_key();
    let persisted = core
        .persist_client(&public_key, "Pixel 8")
        .expect("persist pairing");
    assert_eq!(persisted.display_name, "Pixel 8");
    assert_eq!(core.clients().len(), 1);

    let clients = admin.clients();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].client_id, persisted.client_id);
    assert_eq!(clients[0].state, ClientStateView::Active);

    let revoked = admin.revoke(&persisted.client_id).expect("revoke");
    assert_eq!(revoked.state, ClientStateView::Revoked);
    assert!(matches!(
        store.authorize(&public_key),
        Authorization::Revoked(_)
    ));
    // Revoke keeps the audit record.
    assert_eq!(admin.clients().len(), 1);
    assert_eq!(admin.clients()[0].state, ClientStateView::Revoked);
}

#[test]
fn revoke_of_an_unknown_client_is_an_error() {
    let (admin, _core, _store) = admin();
    let error = admin.revoke("deadbeefdeadbeef").expect_err("unknown id");
    assert!(
        matches!(
            error.to_string().as_str(),
            text if text.contains("unknown") || text.contains("Unknown")
        ),
        "error was {error}"
    );
}
