//! Pairing Core external seam (ticket 19): QR payload, transcript binding,
//! SAS, nonce lifecycle, SAS window, pairing-failure rate limit, and the
//! explicit local-approval decision.
//!
//! Expected values come from the accepted spec and the golden vectors in
//! `fixtures/wire-protocol-v2/control/pairing-*.json`; nothing here recomputes
//! an expectation the way the implementation does.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use v2_transport::auth_store::AuthorizationStore;
use v2_transport::pairing::{
    binding_context, derive_sas_code, encode_qr_uri, format_sas, parse_qr_uri, PairingCore,
    PairingError, PairingPresented, QrPayloadError, MAX_OUTSTANDING_NONCES, NONCE_TTL,
};
use v2_transport::schema::{PairingMode, PairingRequest};

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

const SERVER_PUB: &str = "11b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c";
const CLIENT_PUB: &str = "03d0a529de7475641ac050c792b42cff079107ca5310da7953d2595121477c0f6d";
const NONCE: &str = "808182838485868788898a8b8c8d8e8f";
const CLIENT_RANDOM: &str = "606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f";
const SERVER_RANDOM: &str = "404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f";

/// A deterministic clock for the monotonic nonce/window lifetime.
#[derive(Debug)]
struct ManualClock(std::sync::Mutex<Instant>);

impl ManualClock {
    fn new() -> Arc<Self> {
        Arc::new(Self(std::sync::Mutex::new(Instant::now())))
    }
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
    fn advance(&self, duration: Duration) {
        let mut now = self.0.lock().unwrap();
        *now += duration;
    }
}

impl v2_transport::pairing::Clock for ManualClock {
    fn now(&self) -> Instant {
        self.now()
    }
}

fn core_with_clock(clock: Arc<ManualClock>) -> PairingCore {
    PairingCore::with_clock(
        unhex(SERVER_PUB),
        Arc::new(AuthorizationStore::in_memory()),
        clock,
    )
}

fn qr_request(nonce: &[u8]) -> PairingRequest {
    PairingRequest {
        request_id: 1,
        pairing_mode: PairingMode::Qr as i32,
        pairing_nonce: nonce.to_vec(),
        client_random: unhex(CLIENT_RANDOM),
        client_public_key: unhex(CLIENT_PUB),
        protocol_major: 2,
        capabilities: Vec::new(),
        client_name: "Test Phone".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// QR payload (§4).
// ---------------------------------------------------------------------------

#[test]
fn qr_uri_round_trips_and_uses_the_fixed_generation_order() {
    let key: [u8; 32] = unhex(SERVER_PUB).try_into().unwrap();
    let nonce: [u8; 16] = unhex(NONCE).try_into().unwrap();
    let uri = encode_qr_uri("192.168.1.20:65531", &key, &nonce);
    assert_eq!(
        uri,
        "audioshare-pair://p1?pm=2&addr=192.168.1.20:65531&key=EbMH2b87hLeUHijTEoPgyBP7vtW0_9la5VBBIARa_Jw&nonce=gIGCg4SFhoeIiYqLjI2Ojw"
    );
    let parsed = parse_qr_uri(&uri).expect("generated URI parses");
    assert_eq!(parsed.addr_hint, "192.168.1.20:65531");
    assert_eq!(parsed.server_public_key, key);
    assert_eq!(parsed.nonce, nonce);
}

#[test]
fn qr_parser_accepts_any_field_order_and_rejects_unknown_fields() {
    let key = "EbMH2b87hLeUHijTEoPgyBP7vtW0_9la5VBBIARa_Jw";
    let nonce = "gIGCg4SFhoeIiYqLjI2Ojw";
    let reordered = format!("audioshare-pair://p1?nonce={nonce}&key={key}&addr=host.local:1&pm=2");
    assert!(parse_qr_uri(&reordered).is_ok());
    for bad in [
        "audioshare-pair://p1?pm=2&addr=host:1&key=AAAA&nonce=AAAA&extra=1",
        "audioshare-pair://p2?pm=2&addr=host:1&key=AAAA&nonce=AAAA",
        "https://host/p1?pm=2&addr=host:1&key=AAAA&nonce=AAAA",
        "audioshare-pair://p1?pm=3&addr=host:1&key=AAAA&nonce=AAAA",
        "audioshare-pair://p1?pm=2&addr=host&key=AAAA&nonce=AAAA",
        "audioshare-pair://p1?pm=2&addr=host:0&key=AAAA&nonce=AAAA",
        "audioshare-pair://p1?pm=2&addr=host:1&key=AAAA%3D&nonce=AAAA",
    ] {
        assert!(parse_qr_uri(bad).is_err(), "{bad} must be rejected");
    }
}

#[test]
fn qr_parser_reports_length_and_base64_failures_without_panicking() {
    let key = "EbMH2b87hLeUHijTEoPgyBP7vtW0_9la5VBBIARa_Jw";
    let nonce = "gIGCg4SFhoeIiYqLjI2Ojw";
    let short_key = format!("audioshare-pair://p1?pm=2&addr=h:1&key=AAAA&nonce={nonce}");
    let short_nonce = format!("audioshare-pair://p1?pm=2&addr=h:1&key={key}&nonce=AAAA");
    assert!(matches!(
        parse_qr_uri(&short_key),
        Err(QrPayloadError::KeyLength { .. })
    ));
    assert!(matches!(
        parse_qr_uri(&short_nonce),
        Err(QrPayloadError::NonceLength { .. })
    ));
    assert!(parse_qr_uri("not a uri").is_err());
}

// ---------------------------------------------------------------------------
// Transcript binding + SAS (§6.3, §7.1).
// ---------------------------------------------------------------------------

#[test]
fn qr_binding_context_matches_the_golden_vector() {
    let context = binding_context(
        PairingMode::Qr,
        &unhex(NONCE),
        &unhex(CLIENT_RANDOM),
        &unhex(SERVER_RANDOM),
        &unhex(CLIENT_PUB),
        &unhex(SERVER_PUB),
        &[],
        "Test Phone",
    )
    .expect("canonical inputs");
    assert_eq!(
        hex(&context),
        "9006ad9b963641483800cb15d74767762150fa3630ea17dfb4e600d16a32438b"
    );
}

#[test]
fn sas_binding_context_matches_the_golden_vector_and_derives_900067() {
    let context = binding_context(
        PairingMode::Sas,
        &[],
        &unhex(CLIENT_RANDOM),
        &unhex(SERVER_RANDOM),
        &unhex(CLIENT_PUB),
        &unhex(SERVER_PUB),
        &[],
        "Test Phone",
    )
    .expect("canonical inputs");
    assert_eq!(
        hex(&context),
        "c49ef431c6e6fd122f984f55db2cb06003cb08e07f68a38640ef483e3ad9fadd"
    );
    // The fixture's sas_seed is a fixed TLS-Exporter stand-in.
    assert_eq!(
        derive_sas_code(&unhex(
            "e0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff"
        )),
        Some(900_067)
    );
    assert_eq!(format_sas(900_067), "900 067");
    assert_eq!(format_sas(12), "000 012");
}

#[test]
fn binding_context_rejects_noncanonical_capabilities() {
    let err = binding_context(
        PairingMode::Qr,
        &unhex(NONCE),
        &unhex(CLIENT_RANDOM),
        &unhex(SERVER_RANDOM),
        &unhex(CLIENT_PUB),
        &unhex(SERVER_PUB),
        &[2, 1],
        "Test Phone",
    )
    .unwrap_err();
    assert!(matches!(err, PairingError::Malformed(_)), "{err:?}");
}

// ---------------------------------------------------------------------------
// Nonce lifecycle (§5).
// ---------------------------------------------------------------------------

#[test]
fn nonce_is_single_use_and_consumed_before_the_outcome() {
    let clock = ManualClock::new();
    let core = core_with_clock(Arc::clone(&clock));
    let invitation = core.begin_qr("host:1");
    assert_eq!(invitation.nonce.len(), 16);
    assert_eq!(invitation.expires_at, clock.now() + NONCE_TTL);

    assert!(core.admit_request(&qr_request(&invitation.nonce)).is_ok());
    // Even though the first exchange could still fail, the nonce is gone.
    assert!(matches!(
        core.admit_request(&qr_request(&invitation.nonce)),
        Err(PairingError::NonceInvalid)
    ));
}

#[test]
fn nonce_expires_after_ten_minutes_and_a_server_restart_has_no_nonces() {
    let clock = ManualClock::new();
    let core = core_with_clock(Arc::clone(&clock));
    let invitation = core.begin_qr("host:1");
    clock.advance(NONCE_TTL + Duration::from_millis(1));
    assert!(matches!(
        core.admit_request(&qr_request(&invitation.nonce)),
        Err(PairingError::NonceExpired)
    ));

    // A fresh core is a process restart: memory-only nonces are absent.
    let restarted = core_with_clock(Arc::clone(&clock));
    assert!(matches!(
        restarted.admit_request(&qr_request(&invitation.nonce)),
        Err(PairingError::NonceInvalid)
    ));
}

#[test]
fn at_most_eight_nonces_are_outstanding_and_the_ninth_evicts_the_oldest() {
    let clock = ManualClock::new();
    let core = core_with_clock(Arc::clone(&clock));
    let invitations: Vec<_> = (0..=MAX_OUTSTANDING_NONCES)
        .map(|_| core.begin_qr("host:1"))
        .collect();
    assert_eq!(invitations.len(), 9);
    assert!(matches!(
        core.admit_request(&qr_request(&invitations[0].nonce)),
        Err(PairingError::NonceInvalid)
    ));
    for invitation in &invitations[1..] {
        let mut request = qr_request(&invitation.nonce);
        request.request_id += 1;
        assert!(core.admit_request(&request).is_ok());
    }
}

// ---------------------------------------------------------------------------
// SAS admission window (§7.3) and pairing-failure rate limit (TM-5).
// ---------------------------------------------------------------------------

fn sas_request() -> PairingRequest {
    PairingRequest {
        request_id: 7,
        pairing_mode: PairingMode::Sas as i32,
        pairing_nonce: Vec::new(),
        client_random: unhex(CLIENT_RANDOM),
        client_public_key: unhex(CLIENT_PUB),
        protocol_major: 2,
        capabilities: Vec::new(),
        client_name: "Test Phone".to_owned(),
    }
}

#[test]
fn sas_requests_require_an_open_admin_window() {
    let clock = ManualClock::new();
    let core = core_with_clock(Arc::clone(&clock));
    assert!(matches!(
        core.admit_request(&sas_request()),
        Err(PairingError::SasWindowClosed)
    ));

    let window = core.begin_sas();
    assert_eq!(window.mode, PairingMode::Sas);
    assert!(window.nonce.is_empty());
    assert!(core.admit_request(&sas_request()).is_ok());

    // The window stays open for further SAS attempts until cancelled.
    assert!(core.admit_request(&sas_request()).is_ok());
    core.cancel();
    assert!(matches!(
        core.admit_request(&sas_request()),
        Err(PairingError::SasWindowClosed)
    ));
}

#[test]
fn sas_window_expires_after_ten_minutes() {
    let clock = ManualClock::new();
    let core = core_with_clock(Arc::clone(&clock));
    core.begin_sas();
    clock.advance(NONCE_TTL + Duration::from_millis(1));
    assert!(matches!(
        core.admit_request(&sas_request()),
        Err(PairingError::SasWindowClosed)
    ));
}

#[test]
fn five_pairing_failures_per_sliding_minute_rate_limit_the_address() {
    let core = core_with_clock(ManualClock::new());
    let address: IpAddr = "192.168.1.50".parse().unwrap();
    for _ in 0..4 {
        assert!(!core.note_failure(address));
    }
    assert!(core.note_failure(address), "the 5th failure trips TM-5");
    assert!(core.is_limited(address));
    assert!(!core.is_limited("192.168.1.51".parse().unwrap()));
}

#[test]
fn pairing_capacity_is_eight_and_a_released_permit_admits_again() {
    let core = core_with_clock(ManualClock::new());
    core.begin_sas();
    let mut permits = Vec::new();
    for slot in 0..8 {
        permits.push(
            core.admit_request(&sas_request())
                .unwrap_or_else(|error| panic!("admission {slot} should succeed: {error}")),
        );
    }
    assert!(
        matches!(
            core.admit_request(&sas_request()),
            Err(PairingError::ConcurrentLimit)
        ),
        "the 9th concurrent pairing must be rate-limited"
    );

    // The permit is RAII: releasing one frees exactly one slot.
    permits.pop();
    let admitted = core
        .admit_request(&sas_request())
        .expect("a released permit admits the next pairing");
    permits.push(admitted);
    assert!(matches!(
        core.admit_request(&sas_request()),
        Err(PairingError::ConcurrentLimit)
    ));
}

// ---------------------------------------------------------------------------
// Local approval (§5.2; pairing.md §6.3.1).
// ---------------------------------------------------------------------------

#[test]
fn approval_is_an_explicit_decision_and_never_a_side_effect() {
    let core = Arc::new(core_with_clock(ManualClock::new()));
    let request = v2_transport::pairing::ApprovalRequest {
        approval_id: 41,
        client_id: "cf7940b474310246".to_owned(),
        display_name: "Test Phone".to_owned(),
        mode: PairingMode::Qr,
        address: "192.168.1.20:65531".to_owned(),
        sas: Some("900 067".to_owned()),
    };
    let decisions = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&decisions);
    core.request_approval(
        request.clone(),
        Box::new(move |approved| {
            sink.lock().unwrap().push((request.approval_id, approved));
        }),
    );
    assert_eq!(core.pending_approvals().len(), 1);
    assert!(core.approve(41));
    assert!(core.pending_approvals().is_empty());
    assert_eq!(*decisions.lock().unwrap(), vec![(41, true)]);

    // A missing approval id is a no-op, not a panic.
    assert!(!core.approve(99));
}

#[test]
fn concurrent_approvals_with_the_same_wire_request_id_are_independent() {
    let core = Arc::new(core_with_clock(ManualClock::new()));
    let decisions = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut approval_ids = Vec::new();
    for (client_id, sas) in [
        ("0011223344556677", "111 111"),
        ("8899aabbccddeeff", "222 222"),
    ] {
        let sink = Arc::clone(&decisions);
        let approval_id = core.request_approval(
            v2_transport::pairing::ApprovalRequest {
                approval_id: 0,
                client_id: client_id.to_owned(),
                display_name: format!("Phone {client_id}"),
                mode: PairingMode::Qr,
                address: "192.168.1.20:50000".to_owned(),
                sas: Some(sas.to_owned()),
            },
            Box::new(move |approved| {
                sink.lock().unwrap().push((client_id.to_owned(), approved));
            }),
        );
        approval_ids.push(approval_id);
    }

    let pending = core.pending_approvals();
    assert_eq!(
        pending.len(),
        2,
        "two connections reusing request_id=1 must stay distinct: {pending:?}"
    );
    let ids: std::collections::HashSet<u64> =
        pending.iter().map(|request| request.approval_id).collect();
    assert_eq!(
        ids.len(),
        2,
        "each pending approval needs a unique local id"
    );
    assert_ne!(
        pending[0].sas, pending[1].sas,
        "SAS values must not be crossed"
    );

    // Approving the second leaves the first pending and notifies only its own
    // connection callback.
    assert!(core.approve(approval_ids[1]));
    let remaining = core.pending_approvals();
    assert_eq!(remaining.len(), 1, "{remaining:?}");
    assert_eq!(remaining[0].approval_id, approval_ids[0]);
    assert_eq!(
        remaining[0].client_id, "0011223344556677",
        "the surviving approval is the other connection"
    );
    assert_eq!(
        *decisions.lock().unwrap(),
        vec![("8899aabbccddeeff".to_owned(), true)]
    );
}

fn presented(approval_id: u64, sas: &str) -> PairingPresented {
    PairingPresented {
        approval_id,
        mode: PairingMode::Qr,
        client_id: "cf7940b474310246".to_owned(),
        display_name: "Test Phone".to_owned(),
        address: "192.168.1.20:65531".to_owned(),
        sas: Some(sas.to_owned()),
        fingerprint: "FP".to_owned(),
        expires_at: Instant::now() + Duration::from_secs(600),
    }
}

#[test]
fn presented_pairings_are_published_and_cleared_by_local_decisions() {
    let core = Arc::new(core_with_clock(ManualClock::new()));

    core.publish_presented(presented(41, "123 456"));
    core.publish_presented(presented(42, "654 321"));
    let live = core.presented();
    assert_eq!(live.len(), 2);
    assert!(live.iter().any(|request| request.approval_id == 41));
    assert!(live.iter().any(|request| request.approval_id == 42));

    // Clearing one id (deadline, connection loss, decline) leaves the other.
    core.cancel_approval(41);
    let live = core.presented();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].approval_id, 42);

    // A local approval/decline of the linked approval id clears the live view.
    core.request_approval(
        v2_transport::pairing::ApprovalRequest {
            approval_id: 42,
            client_id: "cf7940b474310246".to_owned(),
            display_name: "Test Phone".to_owned(),
            mode: PairingMode::Qr,
            address: "192.168.1.20:65531".to_owned(),
            sas: Some("654 321".to_owned()),
        },
        Box::new(|_| {}),
    );
    assert!(core.approve(42));
    assert!(core.presented().is_empty());

    // Closing the dialog clears every live view.
    core.publish_presented(presented(43, "111 222"));
    core.cancel();
    assert!(core.presented().is_empty());
}

#[test]
fn cancel_declines_every_pending_approval_and_closes_the_sas_window() {
    let core = Arc::new(core_with_clock(ManualClock::new()));
    core.begin_sas();
    let decisions = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut approval_ids = Vec::new();
    for client_id in ["0011223344556677", "8899aabbccddeeff"] {
        let sink = Arc::clone(&decisions);
        approval_ids.push(core.request_approval(
            v2_transport::pairing::ApprovalRequest {
                approval_id: 0,
                client_id: client_id.to_owned(),
                display_name: "Test Phone".to_owned(),
                mode: PairingMode::Sas,
                address: "192.168.1.20:50000".to_owned(),
                sas: Some("123 456".to_owned()),
            },
            Box::new(move |approved| {
                sink.lock().unwrap().push((client_id.to_owned(), approved));
            }),
        ));
    }

    core.cancel();
    assert!(!core.sas_window_open(), "cancel closes the SAS window");
    assert!(core.pending_approvals().is_empty());
    let mut decisions = decisions.lock().unwrap().clone();
    decisions.sort();
    assert_eq!(
        decisions,
        vec![
            ("0011223344556677".to_owned(), false),
            ("8899aabbccddeeff".to_owned(), false),
        ],
        "every pending connection must be told the pairing was declined"
    );
}
