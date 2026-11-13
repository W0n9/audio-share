//! Log redaction (threat-model.md §TM-4 line 35): connection logs may name the
//! source network, never the full address.

use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use v2_transport::codes::ALPN_PROTOCOL;
use v2_transport::driver::masked_source;
use v2_transport::identity::{IdentityConfig, ServerIdentity};
use v2_transport::limits::{CapacityDecision, CapacityGuard, Limits};
use v2_transport::transport::{Transport, TransportConfig};

#[test]
fn ipv4_source_keeps_only_the_first_two_octets() {
    let source = "203.0.113.9".parse().expect("IPv4");
    assert_eq!(masked_source(source), "203.0.x.x");
}

#[test]
fn ipv6_source_keeps_only_the_first_two_segments() {
    let source = "2001:db8:1234:5678:9abc:def0:1234:5678"
        .parse()
        .expect("IPv6");
    assert_eq!(masked_source(source), "2001:db8::");
}

/// A `tracing` writer that keeps every formatted log line in memory.
#[derive(Clone, Default)]
struct Capture {
    buffer: Arc<Mutex<Vec<u8>>>,
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

fn capture_logs() -> (Capture, tracing::subscriber::DefaultGuard) {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(capture.clone())
        .finish();
    (capture, tracing::subscriber::set_default(subscriber))
}

fn line_containing<'a>(logs: &'a str, needle: &str) -> &'a str {
    logs.lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no {needle:?} log line in:\n{logs}"))
}

fn loopback() -> TransportConfig {
    TransportConfig {
        bind: "127.0.0.1:0".parse().expect("bind address"),
        limits: Limits::defaults(),
    }
}

fn identity() -> (tempfile::TempDir, ServerIdentity) {
    let dir = tempfile::tempdir().expect("tempdir");
    let identity = ServerIdentity::open(&IdentityConfig {
        data_dir: dir.path().to_path_buf(),
        allow_loose_permissions: false,
    })
    .expect("identity");
    (dir, identity)
}

use v2_transport::test_support::connect_with_alpn;

#[tokio::test]
async fn capacity_refusal_log_masks_the_remote_address() {
    let (_logs, _guard) = capture_logs();
    let (_dir, identity) = identity();
    let mut limits = Limits::defaults();
    limits.max_sessions = 1;
    let mut guard = CapacityGuard::new(limits);
    assert_eq!(guard.try_begin_handshake(), CapacityDecision::Admit);
    guard.handshake_completed();

    let transport = Arc::new(Transport::start(loopback(), &identity).expect("start"));
    let address = transport.local_addr();
    let client = connect_with_alpn(address, ALPN_PROTOCOL).expect("client endpoint");
    let (accepted, client_result) = tokio::join!(transport.accept_one(&mut guard), client);
    let connection = client_result.expect("coded refusal completes its bounded handshake");
    assert!(matches!(
        connection.closed().await,
        quinn::ConnectionError::ApplicationClosed(close) if u64::from(close.error_code) == 301
    ));
    assert!(accepted.connection.is_none(), "no session is admitted");
    let raw = accepted.remote_address.expect("peer address").to_string();

    let logs = _logs.text();
    let line = line_containing(&logs, "v2 connection refused: capacity");
    assert!(line.contains("127.0.x.x"), "masked source missing: {line}");
    assert!(!line.contains(&raw), "full remote address leaked: {line}");
    transport.stop();
}

#[tokio::test]
async fn accepted_connection_log_masks_the_remote_address() {
    let (_logs, _guard) = capture_logs();
    let (_dir, identity) = identity();
    let transport = Arc::new(Transport::start(loopback(), &identity).expect("start"));
    let address = transport.local_addr();
    let mut guard = CapacityGuard::new(Limits::defaults());
    let client = connect_with_alpn(address, ALPN_PROTOCOL).expect("client endpoint");
    let (accepted, client_result) = tokio::join!(transport.accept_one(&mut guard), client);
    let connection = client_result.expect("handshake");
    assert!(accepted.connection.is_some(), "session admitted");
    let raw = accepted.remote_address.expect("peer address").to_string();

    let logs = _logs.text();
    let line = line_containing(&logs, "v2 connection accepted");
    assert!(line.contains("127.0.x.x"), "masked source missing: {line}");
    assert!(!line.contains(&raw), "full remote address leaked: {line}");
    drop(accepted.connection);
    drop(connection);
    transport.stop();
}

#[tokio::test]
async fn failed_handshake_log_masks_the_remote_address() {
    let (_logs, _guard) = capture_logs();
    let (_dir, identity) = identity();
    let transport = Arc::new(Transport::start(loopback(), &identity).expect("start"));
    let address = transport.local_addr();
    let mut guard = CapacityGuard::new(Limits::defaults());
    let client = connect_with_alpn(address, b"audio-share/v1").expect("client endpoint");
    let (accepted, client_result) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(transport.accept_one(&mut guard), client)
    })
    .await
    .expect("handshake failure timeout");
    assert!(client_result.is_err(), "wrong ALPN must fail the handshake");
    assert!(accepted.connection.is_none());
    let raw = accepted.remote_address.expect("peer address").to_string();

    let logs = _logs.text();
    let line = line_containing(&logs, "v2 handshake failed");
    assert!(line.contains("127.0.x.x"), "masked source missing: {line}");
    assert!(!line.contains(&raw), "full remote address leaked: {line}");
    transport.stop();
}
