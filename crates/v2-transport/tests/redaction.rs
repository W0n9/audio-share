//! Log redaction (threat-model.md §TM-4 line 35): connection logs may name the
//! source network, never the full address.

use v2_transport::driver::masked_source;

#[test]
fn ipv4_source_keeps_only_the_first_two_octets() {
    let source = "203.0.113.9".parse().expect("IPv4");
    assert_eq!(masked_source(source), "203.0.x.x");
}

#[test]
fn ipv6_source_keeps_only_the_first_two_segments() {
    let source = "2001:db8:1234:5678:9abc:def0:1234:5678".parse().expect("IPv6");
    assert_eq!(masked_source(source), "2001:db8::");
}
