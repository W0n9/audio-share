//! QR `addr` hints derived from the configured Host.
//!
//! The GUI's Host applies only to the advertised address; v2 keeps binding the
//! dual-stack wildcard. These tests pin the formatting and fallback rules the
//! starter relies on, plus the contract that every emitted hint parses.

use v2_runtime::qr_addr_hint;
use v2_transport::pairing::{encode_qr_uri, parse_qr_uri};

const PORT: u16 = 65_531;

#[test]
fn a_concrete_ipv4_host_is_advertised_as_host_port() {
    assert_eq!(
        qr_addr_hint("192.168.1.20", PORT).as_deref(),
        Some("192.168.1.20:65531")
    );
}

#[test]
fn a_concrete_ipv6_host_is_advertised_in_brackets() {
    assert_eq!(
        qr_addr_hint("2001:db8::5", PORT).as_deref(),
        Some("[2001:db8::5]:65531")
    );
}

#[test]
fn an_already_bracketed_ipv6_host_is_not_double_bracketed() {
    assert_eq!(
        qr_addr_hint("[2001:db8::5]", PORT).as_deref(),
        Some("[2001:db8::5]:65531")
    );
}

#[test]
fn a_hostname_is_advertised_verbatim() {
    assert_eq!(
        qr_addr_hint("audio-share.local", PORT).as_deref(),
        Some("audio-share.local:65531")
    );
}

#[test]
fn surrounding_whitespace_is_trimmed() {
    assert_eq!(
        qr_addr_hint("  192.168.1.20  ", PORT).as_deref(),
        Some("192.168.1.20:65531")
    );
}

#[test]
fn an_empty_host_falls_back_to_the_default_address() {
    assert_eq!(qr_addr_hint("", PORT), None);
    assert_eq!(qr_addr_hint("   ", PORT), None);
}

#[test]
fn wildcard_hosts_fall_back_to_the_default_address() {
    assert_eq!(qr_addr_hint("0.0.0.0", PORT), None);
    assert_eq!(qr_addr_hint("::", PORT), None);
    assert_eq!(qr_addr_hint("[::]", PORT), None);
}

#[test]
fn a_scope_bearing_ipv6_host_falls_back_instead_of_emitting_an_illegal_hint() {
    // `%scope` cannot appear in a pairing payload; the runtime then falls back
    // to the machine's default non-loopback IPv4.
    assert_eq!(qr_addr_hint("fe80::1%eth0", PORT), None);
}

#[test]
fn a_host_that_cannot_be_a_payload_host_falls_back() {
    assert_eq!(qr_addr_hint("bad host", PORT), None);
    assert_eq!(qr_addr_hint("host_with_underscore", PORT), None);
    // The payload validator rejects port 0, so it is never emitted.
    assert_eq!(qr_addr_hint("192.168.1.20", 0), None);
}

#[test]
fn every_emitted_hint_round_trips_through_the_qr_parser() {
    let key = [7u8; 32];
    let nonce = [9u8; 16];
    for host in ["192.168.1.20", "2001:db8::5", "audio-share.local", "::1"] {
        let hint = qr_addr_hint(host, PORT).expect("a concrete host emits a hint");
        let uri = encode_qr_uri(&hint, &key, &nonce);
        let parsed = parse_qr_uri(&uri).expect("generated hint must parse");
        assert_eq!(parsed.addr_hint, hint);
    }
}
