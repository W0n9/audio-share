//! Shared harness for the wire-protocol-v2 transport e2e tests.
//!
//! Seams under test (agreed by ticket 01):
//! - the control-frame protocol on the single bidirectional QUIC stream,
//! - transport properties: ALPN negotiation, SPKI pinning, datagram sizing,
//!   bounded non-waiting media sends, close reasons.
//! Server-side behavior is observed through the `proto-server` library's
//! per-connection stats snapshots; client-side through `proto-client`.

#![allow(dead_code)]

use proto_client::ClientConfig;
use proto_server::ServerHandle;
use std::time::Duration;

pub const TEST_CREDENTIAL: &str = "test-credential-do-not-use";

/// Starts a server on an ephemeral loopback port with the shared test credential.
pub async fn spawn_server() -> ServerHandle {
    spawn_server_with(|cfg| {
        cfg.credential = TEST_CREDENTIAL.to_string();
    })
    .await
}

/// Starts a server, applying `tweaks` to the default config before bind.
pub async fn spawn_server_with(
    tweaks: impl FnOnce(&mut proto_server::ServerConfig),
) -> ServerHandle {
    let mut cfg = proto_server::ServerConfig {
        bind: std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        ..Default::default()
    };
    tweaks(&mut cfg);
    proto_server::spawn(cfg).expect("server spawn on 127.0.0.1:0")
}

/// Client config pinned to the real server's SPKI.
pub fn client_config(server: &ServerHandle) -> ClientConfig {
    ClientConfig {
        server_addr: server.local_addr(),
        pinned_spki: server.spki_sha256(),
        alpn: String::from_utf8(proto_core::frame::ALPN.to_vec())
            .expect("ALPN is valid utf-8"),
        credential: TEST_CREDENTIAL.to_string(),
        connect_timeout: Duration::from_secs(5),
        mtud_discovery: false,
    }
}

/// SPKI sha256 of a *different* rcgen-generated self-signed certificate —
/// what an untrusted substitute server would present.
pub fn rogue_pin() -> [u8; 32] {
    let key_pair =
        rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("rogue keygen");
    let params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).expect("params");
    let cert = params.self_signed(&key_pair).expect("rogue self-signed");
    let spki = proto_core::spki::extract_spki_der(cert.der().as_ref()).expect("rogue spki");
    proto_core::spki::spki_sha256(&spki)
}

/// Initializes test logging from RUST_LOG (default: warn) once per process.
pub fn init_test_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_test_writer()
        .try_init();
}
