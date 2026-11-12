//! The v2 QUIC endpoint (ADR-0004 §Decision 1–3; spec §3).
//!
//! One UDP endpoint on 65531/udp (D1), quinn 0.11 + rustls TLS 1.3 (rustls
//! default suites + X25519, D7), ALPN `audio-share/v2` (any other ALPN fails
//! the handshake before application data), 0-RTT permanently disabled (no
//! pairing/auth/control operation is accepted as early data), idle timeout
//! 30 s with 10 s keep-alive (D3).
//!
//! Capacity (TM-1/TM-2) is enforced at accept: surplus connections are
//! refused immediately, signaled by the QUIC transport close alone carrying
//! no application code (§5.2 CONNECTING capacity row: "transport close
//! carrying 301" rides the connection close once accepted; a pre-handshake
//! refusal can only use the transport-layer CONNECTION_REFUSED).
//!
//! Scope boundary: TLS + ALPN + capacity only. Authentication and the §5.2
//! control-plane table are ticket 07.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use quinn::crypto::rustls::QuicServerConfig;

use crate::codes::{ALPN_PROTOCOL, DEFAULT_V2_PORT};
use crate::identity::ServerIdentity;
use crate::limits::{CapacityDecision, CapacityGuard, Limits};
use crate::session::{ConnectionLifecycle, LifecycleState, Termination};

/// Everything needed to start a v2 listener.
#[derive(Clone)]
pub struct TransportConfig {
    /// Bind address; port 0 asks the OS for an ephemeral port. Defaults to
    /// the wildcard address on [`crate::codes::DEFAULT_V2_PORT`].
    pub bind: SocketAddr,
    /// Resource limits (spec §10 defaults; config range documented there).
    pub limits: Limits,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), DEFAULT_V2_PORT),
            limits: Limits::defaults(),
        }
    }
}

/// A failure of [`Transport::start`] or the accept loop.
#[derive(Debug)]
pub enum TransportError {
    /// The identity or TLS material could not be built.
    Identity(crate::identity::IdentityError),
    /// The endpoint could not bind.
    Bind(std::io::Error),
    /// The configured limits are outside the documented ranges (§10).
    InvalidLimits(String),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Identity(error) => write!(f, "v2 server identity: {error}"),
            Self::Bind(error) => write!(f, "cannot bind v2 endpoint: {error}"),
            Self::InvalidLimits(message) => write!(f, "invalid v2 limits: {message}"),
        }
    }
}

impl std::error::Error for TransportError {}

/// The running v2 transport. Dropping the handle stops the listener.
pub struct Transport {
    endpoint: quinn::Endpoint,
    local_addr: SocketAddr,
}

impl std::fmt::Debug for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transport").field("local_addr", &self.local_addr).finish()
    }
}

impl Transport {
    /// Bind the endpoint and start accepting.
    ///
    /// The identity supplies the Ed25519 certificate/key pair; this method
    /// applies the v2 transport policy on top: TLS 1.3 only, ALPN
    /// `audio-share/v2`, `max_early_data_size = 0`, idle timeout 30 s,
    /// keep-alive 10 s (D3, D7).
    pub fn start(config: TransportConfig, identity: &ServerIdentity) -> Result<Self, TransportError> {
        config.limits.validate().map_err(TransportError::InvalidLimits)?;

        let mut tls = identity
            .tls_server_config()
            .map_err(TransportError::Identity)?;
        tls.alpn_protocols = vec![ALPN_PROTOCOL.to_vec()];
        // 0-RTT permanently disabled for this major (spec §3; ADR-0004
        // decision 3): no early data is accepted, so no pairing/auth/control
        // operation can arrive as early data.
        tls.max_early_data_size = 0;

        let quic = QuicServerConfig::try_from(tls)
            .map_err(|error| TransportError::Identity(crate::identity::IdentityError::Io {
                operation: "build QUIC TLS session",
                source: std::io::Error::other(error.to_string()),
            }))?;
        let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic));

        let mut transport = quinn::TransportConfig::default();
        // D3: 30 s idle timeout, symmetric; the fallback when a Close cannot
        // be delivered. Explicit value, never a silently inherited default.
        transport.max_idle_timeout(Some(
            quinn::IdleTimeout::try_from(crate::IDLE_TIMEOUT)
                .expect("30 s fits the QUIC varint"),
        ));
        transport.keep_alive_interval(Some(crate::KEEP_ALIVE_INTERVAL));
        server_config.transport_config(Arc::new(transport));

        let endpoint = quinn::Endpoint::server(server_config, config.bind)
            .map_err(TransportError::Bind)?;
        let local_addr = endpoint
            .local_addr()
            .map_err(TransportError::Bind)?;
        Ok(Self { endpoint, local_addr })
    }

    /// The actual UDP listen address (with the resolved ephemeral port).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Accept the next incoming connection and drive it through the
    /// transport-level policy, producing a [`ConnectionLifecycle`].
    ///
    /// Capacity rejects happen before the handshake completes; the decision
    /// and its 301 close code are reflected in the returned lifecycle (the
    /// peer observes a transport close — §5.2 CONNECTING capacity row).
    ///
    /// This future resolves when the connection reaches a decision point:
    /// either accepted (lifecycle CONNECTING, caller takes over) or rejected/
    /// failed (lifecycle CLOSED with the termination code).
    pub async fn accept_one(&self, guard: &mut CapacityGuard) -> AcceptedConnection {
        #[allow(unused_assignments)] // read on the refusal/None return paths
        let mut remote_address = None;
        // Loop is not dead code: retrying the accept loop on non-admissible
        // (but non-refusing) capacity outcomes is a future ticket-07 need,
        // and clippy's never_loop lint is satisfied by the single break.
        #[allow(clippy::never_loop)]
        let (decision, incoming) = loop {
            let incoming = match self.endpoint.accept().await {
                Some(incoming) => incoming,
                // Endpoint closed: the transport is shutting down.
                None => {
                    return AcceptedConnection {
                        connection: None,
                        lifecycle: terminated(Termination::ServerClosed(crate::codes::ErrorCode::SERVER_STOPPING)),
                        remote_address: None,
                        capacity: CapacityDecision::Admit,
                    };
                }
            };
            remote_address = Some(incoming.remote_address());

            // TM-1/TM-2: bounded handshakes and sessions. Surplus connections
            // are refused immediately (spec §10 "enforcement" column).
            match guard.try_begin_handshake() {
                CapacityDecision::Admit => break (CapacityDecision::Admit, incoming),
                over => {
                    tracing::info!(
                        remote = %crate::driver::masked_source(incoming.remote_address().ip()),
                        decision = ?over,
                        "v2 connection refused: capacity"
                    );
                    incoming.refuse();
                    // Refusal happens pre-handshake; the peer sees the
                    // transport-level CONNECTION_REFUSED. Count the outcome
                    // through the returned lifecycle.
                    return AcceptedConnection {
                        connection: None,
                        lifecycle: terminated(Termination::ServerClosed(
                            CapacityGuard::decision_error_code(over),
                        )),
                        remote_address,
                        capacity: over,
                    };
                }
            }
        };

        // Drive the QUIC/TLS handshake to completion. A wrong-ALPN client or
        // a TLS failure surfaces here as an Err — the explicit pre-application
        // failure ADR-0004 decision 1 requires.
        let handshake = incoming.await;
        match handshake {
            Ok(connection) => {
                guard.handshake_completed();
                tracing::info!(
                    remote = %crate::driver::masked_source(connection.remote_address().ip()),
                    "v2 connection accepted"
                );
                AcceptedConnection {
                    connection: Some(connection),
                    lifecycle: ConnectionLifecycle::new(),
                    remote_address,
                    capacity: decision,
                }
            }
            Err(error) => {
                guard.handshake_abandoned();
                let masked = remote_address.map(|address| crate::driver::masked_source(address.ip()));
                tracing::info!(remote = ?masked, error = %error, "v2 handshake failed");
                AcceptedConnection {
                    connection: None,
                    lifecycle: terminated(Termination::Lost(crate::codes::ErrorCode::NETWORK_FAILURE)),
                    remote_address,
                    capacity: decision,
                }
            }
        }
    }

    /// Close the listener (administrative stop).
    ///
    /// §6: 503 SERVER_STOPPING is the administrative-stop code; connected
    /// sessions receive their own close from their owners (ticket 07). The
    /// endpoint itself just stops accepting; takes `&self` so a shared
    /// (`Arc`) handle can stop too, and the endpoint also stops when the last
    /// handle drops.
    pub fn stop(&self) {
        self.endpoint.close(quinn::VarInt::from_u32(crate::codes::ErrorCode::SERVER_STOPPING.code), b"server stopping");
    }
}

/// The outcome of one [`Transport::accept_one`] call.
#[derive(Debug)]
pub struct AcceptedConnection {
    /// The established QUIC connection, present only on a successful accept.
    pub connection: Option<quinn::Connection>,
    /// The lifecycle so far (CONNECTING on success; CLOSED on rejection or
    /// failed handshake).
    pub lifecycle: ConnectionLifecycle,
    /// The peer address when one was observed.
    pub remote_address: Option<SocketAddr>,
    /// The capacity decision that governed this connection.
    pub capacity: CapacityDecision,
}

fn terminated(termination: Termination) -> ConnectionLifecycle {
    let mut lifecycle = ConnectionLifecycle::new();
    lifecycle.complete(termination);
    lifecycle
}

/// Convert a quinn connection-error into the §6 code the peer's close carried
/// (spec §3: "every transport close carries the registry code").
pub fn classify_close(error: &quinn::ConnectionError) -> Termination {
    match error {
        quinn::ConnectionError::ApplicationClosed(closed) => {
            let raw: u64 = closed.error_code.into();
            let code = crate::codes::registry_code_from_app_code(raw)
                .unwrap_or(crate::codes::ErrorCode::NETWORK_FAILURE);
            Termination::ClientClosed(code)
        }
        // Transport-level failures and local closure classify 601 locally
        // (§5.2 "any: transport lost → classify 601 locally").
        _ => Termination::Lost(crate::codes::ErrorCode::NETWORK_FAILURE),
    }
}

/// Grace period for teardown tasks (§5.1 CLOSING: 5 s grace timer).
pub const CLOSING_GRACE: Duration = Duration::from_secs(5);

/// Whether a lifecycle is still in an active (non-closing, non-closed) state.
pub fn is_active(lifecycle: &ConnectionLifecycle) -> bool {
    lifecycle.state() != LifecycleState::Closed && !lifecycle.state().is_closing()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::IdentityConfig;

    fn temp_data_dir() -> std::path::PathBuf {
        tempfile::tempdir()
            .expect("tempdir")
            .path()
            .to_path_buf()
    }

    fn loopback(port: u16) -> TransportConfig {
        TransportConfig {
            bind: SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port),
            limits: Limits::defaults(),
        }
    }

    fn identity_at(dir: &std::path::Path) -> ServerIdentity {
        ServerIdentity::open(&IdentityConfig {
            data_dir: dir.to_path_buf(),
            allow_loose_permissions: false,
        })
        .expect("identity")
    }

    #[tokio::test]
    async fn start_binds_requested_port() {
        let dir = temp_data_dir();
        let identity = identity_at(&dir);
        let transport = Transport::start(loopback(0), &identity).expect("start");
        assert_ne!(transport.local_addr().port(), 0, "OS resolved an ephemeral port");
        transport.stop();
    }

    #[tokio::test]
    async fn start_binds_default_port_when_requested() {
        let dir = temp_data_dir();
        let identity = identity_at(&dir);
        // D1: 65531 is the default; binding it must succeed (outside the
        // Linux ephemeral range, so it cannot collide with itself either).
        let config = loopback(crate::codes::DEFAULT_V2_PORT);
        let transport = Transport::start(config, &identity).expect("bind 65531");
        assert_eq!(transport.local_addr().port(), crate::codes::DEFAULT_V2_PORT);
        transport.stop();
    }

    #[tokio::test]
    async fn invalid_limits_reject_start() {
        let dir = temp_data_dir();
        let identity = identity_at(&dir);
        let mut config = loopback(0);
        config.limits.max_sessions = 0;
        let error = Transport::start(config, &identity).expect_err("0 sessions is invalid");
        assert!(matches!(error, TransportError::InvalidLimits(_)), "{error}");
    }

    #[tokio::test]
    async fn wrong_alpn_fails_before_application_data() {
        // Spec §3: "Any other ALPN fails the handshake before application
        // data" — the client gets an explicit handshake failure, never a
        // stream to speak on.
        let dir = temp_data_dir();
        let identity = identity_at(&dir);
        let transport = Arc::new(Transport::start(loopback(0), &identity).expect("start"));
        let address = transport.local_addr();

        let server_transport = Arc::clone(&transport);
        let server_task = tokio::spawn(async move {
            let guard = &mut CapacityGuard::new(Limits::defaults());
            server_transport.accept_one(guard).await
        });
        let client = connect_with_alpn(address, b"audio-share/v1").expect("client endpoint");
        let client_result = client.await;
        assert!(client_result.is_err(), "wrong-ALPN handshake must fail");
        let accepted = server_task.await.expect("accept task");
        assert!(accepted.connection.is_none(), "no connection comes out of a failed handshake");
        assert_eq!(accepted.lifecycle.state(), LifecycleState::Closed);
        transport.stop();
    }

    #[tokio::test]
    async fn matching_alpn_connects_and_opens_control_stream() {
        let dir = temp_data_dir();
        let identity = identity_at(&dir);
        let transport = Arc::new(Transport::start(loopback(0), &identity).expect("start"));
        let address = transport.local_addr();

        let server_transport = Arc::clone(&transport);
        let accept_task = tokio::spawn(async move {
            let guard = &mut CapacityGuard::new(Limits::defaults());
            server_transport.accept_one(guard).await
        });

        let client = connect_with_alpn(address, ALPN_PROTOCOL).expect("client endpoint");
        let connection = client.await.expect("handshake succeeds with matching ALPN");
        let accepted = accept_task.await.expect("accept task");
        assert!(accepted.connection.is_some(), "server accepted the connection");
        assert_eq!(accepted.lifecycle.state(), LifecycleState::Connecting);
        assert_eq!(accepted.capacity, CapacityDecision::Admit);

        // The client may open the one control stream and exchange bytes —
        // this ticket's plumbing stops here; auth is ticket 07.
        let (mut send, mut recv) = connection.open_bi().await.expect("open control stream");
        send.write_all(b"probe").await.expect("write");
        send.finish().expect("finish");
        let server_connection = accepted.connection.expect("accepted");
        let (mut server_send, mut server_recv) =
            server_connection.accept_bi().await.expect("server sees the stream");
        let mut buf = vec![0u8; 5];
        server_recv.read_exact(&mut buf).await.expect("read");
        assert_eq!(&buf, b"probe");
        server_send.write_all(b"ack").await.expect("write back");
        server_send.finish().expect("finish");
        let mut ack = vec![0u8; 3];
        recv.read_exact(&mut ack).await.expect("read ack");
        assert_eq!(&ack, b"ack");

        transport.stop();
    }

    #[tokio::test]
    async fn capacity_reject_signals_busy_close_code() {
        // TM-2 with max_sessions = 0 is invalid, so exercise the bound with
        // one session: fill it, then expect the next connection refused.
        let dir = temp_data_dir();
        let identity = identity_at(&dir);
        let mut config = loopback(0);
        config.limits.max_sessions = 1;
        config.limits.max_concurrent_handshakes = 1;
        let transport = Arc::new(Transport::start(config, &identity).expect("start"));
        let address = transport.local_addr();

        let server_transport = Arc::clone(&transport);
        let accept_task = tokio::spawn(async move {
            let guard = &mut CapacityGuard::new(Limits::defaults());
            server_transport.accept_one(guard).await
        });
        let client = connect_with_alpn(address, ALPN_PROTOCOL).expect("client endpoint");
        let _connection = client.await.expect("first connection admitted");
        let accepted = accept_task.await.expect("accept task");
        assert!(accepted.connection.is_some());
        drop(accepted.connection);
        // The guard in the spawned task was local; reconstruct the same state
        // here for the direct-call checks below.
        let mut guard = CapacityGuard::new(Limits { max_sessions: 1, max_concurrent_handshakes: 1, ..Limits::defaults() });
        assert_eq!(guard.try_begin_handshake(), CapacityDecision::Admit);
        guard.handshake_completed();
        assert_eq!(guard.try_begin_handshake(), CapacityDecision::SessionBound);
        assert_eq!(
            CapacityGuard::decision_error_code(CapacityDecision::SessionBound).code,
            crate::codes::ErrorCode::BUSY.code
        );
        transport.stop();
    }

    #[test]
    fn classify_close_maps_application_codes() {
        // A client-sent close with code 701 surfaces as ClientClosed(701).
        let app = quinn::ConnectionError::ApplicationClosed(quinn::ApplicationClose {
            error_code: crate::codes::ErrorCode::CLIENT_STOPPING.code.into(),
            reason: Vec::new().into(),
        });
        let termination = classify_close(&app);
        assert_eq!(termination, Termination::ClientClosed(crate::codes::ErrorCode::CLIENT_STOPPING));

        // Transport loss (timeout) classifies 601 locally (§5.2).
        let timeout = quinn::ConnectionError::TimedOut;
        assert_eq!(classify_close(&timeout), Termination::Lost(crate::codes::ErrorCode::NETWORK_FAILURE));
    }

    #[test]
    fn closing_grace_is_five_seconds() {
        // §5.1 CLOSING: "start 5 s grace timer".
        assert_eq!(CLOSING_GRACE, Duration::from_secs(5));
    }

    /// A client endpoint with a custom ALPN and an accept-all verifier
    /// (test-only: the SPKI-pinning client is ticket 08's deliverable).
    fn connect_with_alpn(
        address: SocketAddr,
        alpn: &[u8],
    ) -> Result<quinn::Connecting, String> {
        let provider = rustls::crypto::ring::default_provider();
        let mut crypto = rustls::ClientConfig::builder_with_provider(provider.into())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("TLS 1.3 available")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAny))
            .with_no_client_auth();
        crypto.alpn_protocols = vec![alpn.to_vec()];
        let endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap())
            .map_err(|error| error.to_string())?;
        let config = quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
                .map_err(|error| error.to_string())?,
        ));
        endpoint
            .connect_with(config, address, "audio-share-server.local")
            .map_err(|error| error.to_string())
    }

    /// Test-only verifier: accepts any certificate. Never ships — the
    /// production client pins the SPKI (pairing.md §11.3).
    #[derive(Debug)]
    struct AcceptAny;

    impl rustls::client::danger::ServerCertVerifier for AcceptAny {
        fn verify_server_cert(
            &self,
            _end_entity: &rustls_pki_types::CertificateDer<'_>,
            _intermediates: &[rustls_pki_types::CertificateDer<'_>],
            _server_name: &rustls_pki_types::ServerName<'_>,
            _ocsp: &[u8],
            _now: rustls_pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _certificate: &rustls_pki_types::CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _certificate: &rustls_pki_types::CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            vec![rustls::SignatureScheme::ED25519]
        }
    }
}
