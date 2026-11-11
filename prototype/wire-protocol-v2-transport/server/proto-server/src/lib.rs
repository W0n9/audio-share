//! wire-protocol-v2 transport prototype — QUIC test Server core.
//!
//! Throwaway code answering the ticket-01 transport question only; it does not
//! freeze the v2 normative wire layout. Behavior contract lives in
//! `.scratch/wire-protocol-v2/prototype-brief.md`.
//!
//! Session state machine (prototype-simple):
//! `CONNECTING → AUTHENTICATING → READY → PLAYING → CLOSING/CLOSED`.
//! AUTH is required before anything else; SUBSCRIBE is only legal in READY;
//! violations produce an explicit `CLOSE(PROTOCOL_ERROR)` or
//! `AUTH_ERR` + close, never a silent ignore.

pub mod stats;
mod session;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use quinn::crypto::rustls::QuicServerConfig;
use tokio::sync::watch;

use crate::stats::{EndpointStatsSnapshot, SessionStats};

/// Default bind address — deliberately different from the v1 port 65530.
pub const DEFAULT_BIND: SocketAddr = SocketAddr::new(
    std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)),
    65531,
);

/// Default shared test credential (never a production secret).
pub const DEFAULT_CREDENTIAL: &str = "test-credential-do-not-use";

/// Server configuration for the prototype.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// UDP bind address. The prototype default is `127.0.0.1:65531`.
    pub bind: SocketAddr,
    /// The single accepted AUTH credential token.
    pub credential: String,
    /// Tokens treated as revoked: they get the exact bad-credential wire
    /// treatment (`AUTH_ERR(BAD_CREDENTIAL)` + `CLOSE(AUTH_FAILED)`) but are
    /// counted under the distinct `auth_err_revoked` stat.
    pub revoked_credentials: Vec<String>,
    /// Idle timeout applied while in CONNECTING/READY (no media flowing).
    /// `None` disables it.
    pub idle_timeout: Option<Duration>,
    /// Audio produced per media ticker tick, in milliseconds of audio.
    pub chunk_target_ms: u64,
    /// Bounded outbound media queue capacity (datagrams).
    pub queue_capacity: usize,
    /// Test-only hook: artificial delay in the media forwarder, used by
    /// integration tests to force the bounded queue to overflow
    /// deterministically. `None` (production shape) never delays.
    pub forwarder_delay: Option<Duration>,
    /// DPLPMTUD (path MTU discovery). Disabled by default in the prototype so
    /// `max_datagram_size()` stays at quinn's 1200-byte initial MTU and the
    /// cross-endpoint datagram-size invariant is deterministic. The reduced
    /// MTU impairment runs should enable this to watch payload resizing react
    /// to path-limit changes.
    pub mtud_discovery: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: DEFAULT_BIND,
            credential: DEFAULT_CREDENTIAL.to_string(),
            revoked_credentials: Vec::new(),
            idle_timeout: Some(Duration::from_secs(60)),
            chunk_target_ms: 5,
            queue_capacity: 256,
            forwarder_delay: None,
            mtud_discovery: false,
        }
    }
}

/// Shared server-side state.
struct Shared {
    config: ServerConfig,
    sessions: std::sync::Mutex<Vec<Arc<SessionStats>>>,
    next_session_id: AtomicU32,
    accepted: AtomicU64,
    handshake_failures: AtomicU64,
    shutdown_tx: watch::Sender<bool>,
}

impl Shared {
    fn live_sessions(&self) -> Vec<Arc<SessionStats>> {
        self.sessions
            .lock()
            .expect("session registry mutex")
            .iter()
            .filter(|s| !s.closed())
            .cloned()
            .collect()
    }
}

/// Handle to a running prototype server.
#[derive(Clone)]
pub struct ServerHandle {
    local_addr: SocketAddr,
    spki_sha256: [u8; 32],
    shared: Arc<Shared>,
    endpoint: quinn::Endpoint,
}

impl ServerHandle {
    /// Actual bound address (useful with port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// sha256 over the server certificate's SubjectPublicKeyInfo DER.
    pub fn spki_sha256(&self) -> [u8; 32] {
        self.spki_sha256
    }

    /// The pin value as lowercase hex, the format clients take on the CLI.
    pub fn spki_sha256_hex(&self) -> String {
        proto_core::spki::to_hex(&self.spki_sha256)
    }

    /// Snapshots of every session (live and finished) seen so far.
    pub fn session_stats(&self) -> Vec<stats::SessionStatsSnapshot> {
        self.shared
            .sessions
            .lock()
            .expect("session registry mutex")
            .iter()
            .map(|s| s.snapshot())
            .collect()
    }

    /// Endpoint-level connection counters.
    pub fn endpoint_stats(&self) -> EndpointStatsSnapshot {
        EndpointStatsSnapshot {
            accepted: self.shared.accepted.load(Ordering::Relaxed),
            handshake_failures: self.shared.handshake_failures.load(Ordering::Relaxed),
        }
    }

    /// Graceful stop: every live session is told `CLOSE(SERVER_STOP)`, waits
    /// briefly for peers to drain, then the endpoint is closed.
    pub async fn stop(&self) {
        let _ = self.shared.shutdown_tx.send(true);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline {
            if self.shared.live_sessions().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // Application error code on the QUIC transport close mirrors the
        // CLOSE-frame reason value (`CloseReason` is the single source).
        self.endpoint
            .close(quinn::VarInt::from_u32(proto_core::frame::CloseReason::ServerStop as u32), b"server-stop");
    }
}

/// Generates the ephemeral self-signed ECDSA P-256 identity and returns
/// `(cert_der, pkcs8_key_der, spki_sha256)`.
fn generate_identity() -> anyhow::Result<(rcgen::Certificate, Vec<u8>, [u8; 32])> {
    let key_pair =
        rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|e| anyhow::anyhow!("keygen: {e}"))?;
    let mut params = rcgen::CertificateParams::new(vec!["localhost".to_string()])
        .map_err(|e| anyhow::anyhow!("cert params: {e}"))?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "audio-share wire-protocol-v2 prototype");
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| anyhow::anyhow!("self-signed cert: {e}"))?;
    let spki = proto_core::spki::extract_spki_der(cert.der().as_ref())
        .map_err(|e| anyhow::anyhow!("spki extract: {e}"))?;
    let spki_hash = proto_core::spki::spki_sha256(&spki);
    let pk_der = key_pair.public_key_der();
    let spki_from_key: &[u8] = pk_der.as_ref();
    anyhow::ensure!(
        spki.as_slice() == spki_from_key,
        "SPKI extraction disagrees with key pair serialization"
    );
    let key_der = key_pair.serialize_der().to_vec();
    Ok((cert, key_der, spki_hash))
}

/// Builds the rustls server config: TLS 1.3 only, ALPN `audio-share/v2`,
/// 0-RTT disabled (`max_early_data_size == 0`).
///
/// 0-RTT evidence path: quinn 0.11 additionally requires an explicit
/// `ServerConfig::enable_0rtt()` plus `Incoming::accept_0rtt()` to admit early
/// data — this server never calls either (grep-verifiable), and the app only
/// reads the control stream after the handshake completes. No quinn runtime
/// API exposes the negotiated 0-RTT state, so configuration audit plus the
/// rustls assertion below is the strongest available evidence.
pub fn build_tls_server_config(
    cert: rustls::pki_types::CertificateDer<'static>,
    key: rustls::pki_types::PrivatePkcs8KeyDer<'static>,
) -> anyhow::Result<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| anyhow::anyhow!("tls versions: {e}"))?
        .with_no_client_auth()
        .with_single_cert(
            vec![cert],
            rustls::pki_types::PrivateKeyDer::Pkcs8(key),
        )
        .map_err(|e| anyhow::anyhow!("cert/key: {e}"))?;
    tls.alpn_protocols = vec![proto_core::frame::ALPN.to_vec()];
    // 0-RTT disabled: rustls only accepts early data when max_early_data_size
    // is non-zero. Zero is the default; set explicitly and asserted in tests.
    tls.max_early_data_size = 0;
    Ok(tls)
}

/// Spawns the prototype server. Must be called inside a tokio runtime.
pub fn spawn(config: ServerConfig) -> anyhow::Result<ServerHandle> {
    let (cert, key_der, spki_hash) = generate_identity()?;
    let cert_der = rustls::pki_types::CertificateDer::from(cert.der().as_ref().to_vec());
    let tls = build_tls_server_config(
        cert_der,
        rustls::pki_types::PrivatePkcs8KeyDer::from(key_der),
    )?;
    let quic_tls: QuicServerConfig = QuicServerConfig::try_from(tls)
        .map_err(|e| anyhow::anyhow!("quic tls config: {e}"))?;
    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_tls));
    let mut transport = quinn::TransportConfig::default();
    if !config.mtud_discovery {
        // Explicit prototype decision: quinn 0.11 enables DPLPMTUD by default,
        // which grows max_datagram_size() on unimpaired paths and makes
        // cross-endpoint size assertions racy. Keep the initial 1200-byte MTU.
        transport.mtu_discovery_config(None);
    }
    // 0-RTT is never enabled: quinn requires explicit opt-in via
    // ServerConfig::enable_0rtt() (not called) and per-connection
    // accept_0rtt() (not called).
    server_config.transport_config(Arc::new(transport));
    let endpoint = quinn::Endpoint::server(server_config, config.bind)?;
    let local_addr = endpoint.local_addr()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let shared = Arc::new(Shared {
        config,
        sessions: std::sync::Mutex::new(Vec::new()),
        next_session_id: AtomicU32::new(1),
        accepted: AtomicU64::new(0),
        handshake_failures: AtomicU64::new(0),
        shutdown_tx,
    });
    tokio::spawn(accept_loop(endpoint.clone(), shared.clone(), shutdown_rx));
    tracing::info!(
        addr = %local_addr,
        spki_sha256 = %proto_core::spki::to_hex(&spki_hash),
        "prototype server listening"
    );
    Ok(ServerHandle {
        local_addr,
        spki_sha256: spki_hash,
        shared,
        endpoint,
    })
}

async fn accept_loop(
    endpoint: quinn::Endpoint,
    shared: Arc<Shared>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        let incoming = tokio::select! {
            _ = shutdown.changed() => break,
            incoming = endpoint.accept() => match incoming {
                Some(incoming) => incoming,
                None => break,
            },
        };
        let shared = shared.clone();
        tokio::spawn(async move {
            match incoming.accept() {
                Ok(connecting) => match connecting.await {
                    Ok(conn) => {
                        shared.accepted.fetch_add(1, Ordering::Relaxed);
                        session::run(conn, shared).await;
                    }
                    Err(e) => {
                        shared.handshake_failures.fetch_add(1, Ordering::Relaxed);
                        tracing::debug!(error = %e, "handshake failed");
                    }
                },
                Err(e) => {
                    shared.handshake_failures.fetch_add(1, Ordering::Relaxed);
                    tracing::debug!(error = %e, "incoming rejected");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runtime assertion for the 0-RTT evidence path: the rustls server
    /// config this server actually runs must have 0-RTT (early data)
    /// disabled and the exact prototype ALPN set. quinn-level evidence
    /// (enable_0rtt/accept_0rtt never called) is a source audit documented
    /// in the README; no quinn runtime API exposes the negotiated 0-RTT
    /// state.
    #[test]
    fn server_tls_config_disables_0rtt_and_sets_alpn() {
        let (cert, key, spki) = generate_identity().expect("identity");
        let tls = build_tls_server_config(
            rustls::pki_types::CertificateDer::from(cert.der().as_ref().to_vec()),
            rustls::pki_types::PrivatePkcs8KeyDer::from(key),
        )
        .expect("tls config");
        assert_eq!(
            tls.max_early_data_size, 0,
            "0-RTT must stay disabled on the server"
        );
        assert_eq!(
            tls.alpn_protocols,
            vec![proto_core::frame::ALPN.to_vec()],
            "ALPN must be exactly audio-share/v2"
        );
        assert_eq!(proto_core::spki::to_hex(&spki).len(), 64);
    }
}
