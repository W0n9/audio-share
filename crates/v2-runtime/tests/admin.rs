//! Pairing administration seam tests: a real Pairing Core over an in-memory
//! store, no QUIC, no GUI.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use p256::ecdsa::SigningKey;
use v2_runtime::admin::{
    ClientStateView, CorePairingAdmin, InvitationStateView, PairingAdmin, RequestedPairingMode,
};
use v2_transport::auth_store::{Authorization, AuthorizationStore};
use v2_transport::pairing::{
    self, ApprovalDecision, ApprovalRequest, PairingCore, PairingPresented, NONCE_TTL,
};
use v2_transport::schema::{PairingMode as WirePairingMode, PairingRequest};

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

/// A deterministic clock for the invitation lifetime.
#[derive(Debug)]
struct ManualClock(Mutex<Instant>);

impl ManualClock {
    fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(Instant::now())))
    }

    fn advance(&self, duration: Duration) {
        *self.0.lock().unwrap() += duration;
    }
}

impl pairing::Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

fn admin_with_clock() -> (Arc<CorePairingAdmin>, Arc<PairingCore>, Arc<ManualClock>) {
    let store = Arc::new(AuthorizationStore::in_memory());
    let clock = ManualClock::new();
    let core = Arc::new(PairingCore::with_clock(
        vec![7u8; 32],
        Arc::clone(&store),
        clock.clone(),
    ));
    let admin = Arc::new(CorePairingAdmin::new(
        Arc::clone(&core),
        "192.168.1.20:65531",
    ));
    (admin, core, clock)
}

#[test]
fn invitation_state_reports_active_consumed_and_expired() {
    let (admin, core, clock) = admin_with_clock();
    assert_eq!(admin.invitation_state(), None);

    let invitation = admin.begin(RequestedPairingMode::Qr).expect("begin QR");
    assert_eq!(admin.invitation_state(), Some(InvitationStateView::Active));

    let uri = invitation.qr_uri.expect("QR URI");
    let parsed = pairing::parse_qr_uri(&uri).expect("QR payload parses");
    core.admit_request(&PairingRequest {
        pairing_mode: WirePairingMode::Qr as i32,
        pairing_nonce: parsed.nonce.to_vec(),
        ..PairingRequest::default()
    })
    .expect("admit the first request");
    assert_eq!(
        admin.invitation_state(),
        Some(InvitationStateView::Consumed)
    );

    admin
        .begin(RequestedPairingMode::Qr)
        .expect("begin QR again");
    assert_eq!(admin.invitation_state(), Some(InvitationStateView::Active));
    clock.advance(NONCE_TTL + Duration::from_millis(1));
    assert_eq!(admin.invitation_state(), Some(InvitationStateView::Expired));
}

fn approval(
    request_id: u64,
    client_id: &str,
    restore_candidate: Option<&str>,
    core: &PairingCore,
    accepted: Arc<AtomicBool>,
) {
    core.request_approval(
        ApprovalRequest {
            approval_id: request_id,
            client_id: client_id.to_owned(),
            display_name: "Pixel 8".to_owned(),
            mode: WirePairingMode::Qr,
            address: "192.168.1.44:50000".to_owned(),
            sas: Some("123 456".to_owned()),
            restore_candidate: restore_candidate.map(str::to_owned),
        },
        Box::new(move |decision| {
            accepted.store(decision == ApprovalDecision::Accept, Ordering::SeqCst);
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
    approval(41, "0011223344556677", None, &core, Arc::clone(&accepted));

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
    approval(42, "0011223344556677", None, &core, Arc::clone(&accepted));

    assert!(admin.decline(42));
    assert!(!accepted.load(Ordering::SeqCst));
    assert!(admin.pending().is_empty());
}

#[test]
fn restore_routes_the_explicit_restore_decision_to_the_core() {
    let (admin, core, _store) = admin();
    let decision = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&decision);
    core.request_approval(
        ApprovalRequest {
            approval_id: 43,
            client_id: "0011223344556677".to_owned(),
            display_name: "Pixel 8".to_owned(),
            mode: WirePairingMode::Qr,
            address: "192.168.1.44:50000".to_owned(),
            sas: Some("123 456".to_owned()),
            restore_candidate: Some("Pixel 8".to_owned()),
        },
        Box::new(move |choice| {
            *sink.lock().expect("decision lock") = Some(choice);
        }),
    );
    assert_eq!(
        admin.pending()[0].restore_candidate.as_deref(),
        Some("Pixel 8")
    );

    assert!(admin.restore(43));
    assert_eq!(
        *decision.lock().expect("decision lock"),
        Some(ApprovalDecision::Restore),
        "Restore is carried as its own decision, never a plain Accept"
    );
    assert!(admin.pending().is_empty());
    assert!(!admin.restore(43), "an approval decision is single-use");
}

#[test]
fn clients_list_and_revoke_use_the_shared_store() {
    let (admin, core, store) = admin();
    assert!(admin.clients().is_empty());

    let public_key = client_key();
    let persisted = core
        .persist_client(&public_key, "Pixel 8", pairing::PersistMode::New)
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

#[test]
fn pending_and_presented_views_flag_a_restore_candidate() {
    let (admin, core, _store) = admin();
    let public_key = client_key();
    let record = core
        .persist_client(&public_key, "Pixel 8", pairing::PersistMode::New)
        .expect("persist pairing");
    core.revoke(&record.client_id).expect("revoke");

    let accepted = Arc::new(AtomicBool::new(false));
    approval(
        51,
        &record.client_id,
        Some("Pixel 8"),
        &core,
        Arc::clone(&accepted),
    );
    core.publish_presented(PairingPresented {
        approval_id: 51,
        mode: WirePairingMode::Qr,
        client_id: record.client_id.clone(),
        display_name: "Pixel 8".to_owned(),
        address: "192.168.1.44:50000".to_owned(),
        sas: Some("123 456".to_owned()),
        fingerprint: core.fingerprint(),
        expires_at: Instant::now() + Duration::from_secs(600),
        restore_candidate: Some("Pixel 8".to_owned()),
    });

    let pending = admin.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].restore_candidate.as_deref(),
        Some("Pixel 8"),
        "a revoked identity flags a restore before approval"
    );
    assert_eq!(pending[0].display_name, "Pixel 8");
    let presented = admin.presented();
    assert_eq!(presented.len(), 1);
    assert_eq!(
        presented[0].restore_candidate.as_deref(),
        Some("Pixel 8"),
        "the live card also flags a restore"
    );
    assert_eq!(presented[0].display_name, "Pixel 8");

    // A fresh identity (no stored record) is an ordinary pairing.
    approval(
        52,
        "0011223344556677",
        None,
        &core,
        Arc::new(AtomicBool::new(false)),
    );
    let fresh = admin
        .pending()
        .into_iter()
        .find(|request| request.approval_id == 52)
        .expect("fresh approval");
    assert!(fresh.restore_candidate.is_none());
}

#[test]
fn declining_a_restore_keeps_the_record_revoked_and_adds_nothing() {
    let (admin, core, store) = admin();
    let public_key = client_key();
    let record = core
        .persist_client(&public_key, "Pixel 8", pairing::PersistMode::New)
        .expect("persist pairing");
    core.revoke(&record.client_id).expect("revoke");

    let accepted = Arc::new(AtomicBool::new(true));
    approval(
        61,
        &record.client_id,
        Some("Pixel 8"),
        &core,
        Arc::clone(&accepted),
    );
    assert_eq!(
        admin.pending()[0].restore_candidate.as_deref(),
        Some("Pixel 8")
    );

    assert!(admin.decline(61));
    assert!(!accepted.load(Ordering::SeqCst));
    assert_eq!(admin.clients().len(), 1, "no record is added");
    assert_eq!(admin.clients()[0].client_id, record.client_id);
    assert_eq!(admin.clients()[0].state, ClientStateView::Revoked);
    assert!(matches!(
        store.authorize(&public_key),
        Authorization::Revoked(_)
    ));
}

#[test]
fn delete_refuses_active_records_and_removes_revoked_ones() {
    let (admin, core, store) = admin();
    let public_key = client_key();
    let record = core
        .persist_client(&public_key, "Pixel 8", pairing::PersistMode::New)
        .expect("persist pairing");

    let error = admin
        .delete(&record.client_id)
        .expect_err("an active record cannot be deleted");
    assert!(error.contains("revoke"), "error was {error}");
    assert_eq!(admin.clients().len(), 1, "the active record is unchanged");
    assert_eq!(admin.clients()[0].state, ClientStateView::Active);

    admin.revoke(&record.client_id).expect("revoke");
    let deleted = admin.delete(&record.client_id).expect("revoked deletes");
    assert_eq!(deleted.client_id, record.client_id);
    assert_eq!(deleted.state, ClientStateView::Revoked);
    assert!(admin.clients().is_empty());
    assert!(
        matches!(store.authorize(&public_key), Authorization::Unknown(_)),
        "a deleted identity is refused at authentication"
    );
}
