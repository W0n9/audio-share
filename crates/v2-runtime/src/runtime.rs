//! The shared v2 Server runtime: identity, authorization store, Pairing Core,
//! QUIC listener, and the per-connection session driver.
//!
//! `as-cmd` and the Server GUI start exactly this runtime; neither re-implements
//! pairing or transport assembly. v1 remains independently started by the
//! caller so a v2 failure never disables v1.

use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use audio_share_core::capture::media::MediaTap;
use audio_share_core::capture::CaptureObserver;
use v2_transport::auth_store::AuthorizationStore;
use v2_transport::driver::SessionServer;
use v2_transport::identity::{IdentityConfig, ServerIdentity};
use v2_transport::limits::{CapacityGuard, Limits};
use v2_transport::pairing::PairingCore;
use v2_transport::transport::{Transport, TransportConfig};

use crate::admin::{CorePairingAdmin, PairingAdmin};
use crate::capture::{capture_media_source, v2_capture_source};

/// How to open the v2 runtime.
#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    /// The Server data directory (identity and `pairing/clients.json`).
    pub data_dir: PathBuf,
    /// POSIX override for loose identity-key permissions (pairing.md §10.2).
    pub allow_loose_permissions: bool,
    /// The bind host; empty or `::` is the dual-stack wildcard.
    pub bind_host: String,
    /// The v2 QUIC port; `0` binds an ephemeral port (tests).
    pub port: u16,
    /// The `host:port` advertised in the QR payload. `None` derives the
    /// machine's default private address and the actual bound port.
    pub addr_hint: Option<String>,
}

/// A v2 runtime start failure. Presented as an independent error so v1 can
/// keep running (pairing.md §10.2: "v1 and capture are unaffected").
#[derive(Debug)]
pub enum RuntimeError {
    /// Identity provisioning failed (permissions, corrupt key, I/O).
    Identity(String),
    /// The authorization store failed closed (corrupt JSON, I/O).
    Store(String),
    /// Bind-address resolution failed.
    Resolve(String),
    /// The QUIC endpoint could not be created.
    Transport(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identity(reason) => write!(f, "server identity: {reason}"),
            Self::Store(reason) => write!(f, "authorization store: {reason}"),
            Self::Resolve(reason) => write!(f, "resolve v2 bind address: {reason}"),
            Self::Transport(reason) => write!(f, "v2 transport: {reason}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

/// A running v2 endpoint and its shared Pairing Core.
pub struct V2Runtime {
    pairing: Arc<PairingCore>,
    admin: Arc<dyn PairingAdmin>,
    transport: Arc<Transport>,
    accept_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    local_addr: SocketAddr,
    addr_hint: String,
    fingerprint: String,
}

impl fmt::Debug for V2Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never expose nonces, SAS values, or key material.
        f.debug_struct("V2Runtime")
            .field("local_addr", &self.local_addr)
            .field("addr_hint", &self.addr_hint)
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

impl V2Runtime {
    /// Load the identity and store, open an invitation-capable session server,
    /// bind the QUIC endpoint, and start the accept loop.
    pub async fn start(
        config: RuntimeConfig,
        capture: Option<(CaptureObserver, MediaTap)>,
    ) -> Result<Self, RuntimeError> {
        let identity = ServerIdentity::open(&IdentityConfig {
            data_dir: config.data_dir.clone(),
            allow_loose_permissions: config.allow_loose_permissions,
        })
        .map_err(|error| RuntimeError::Identity(error.to_string()))?;
        let store = Arc::new(
            AuthorizationStore::open(&config.data_dir)
                .map_err(|error| RuntimeError::Store(error.to_string()))?,
        );
        let pairing = Arc::new(PairingCore::from_identity(&identity, Arc::clone(&store)));

        let mut sessions = SessionServer::new(&identity, Arc::clone(&store));
        if let Some((capture, media_tap)) = capture {
            sessions.capture_source = Some(v2_capture_source(capture.clone(), &media_tap));
            sessions.media_source = Some(capture_media_source(&media_tap, capture));
        }
        // The production admission path: PairingRequest reaches the shared
        // Pairing Core instead of the old unconditional refusal.
        sessions.pairing = Some(Arc::clone(&pairing));

        let limits = Limits::defaults();
        sessions.limits = limits.clone();
        let bind = resolve_bind_address(&config.bind_host, config.port).await?;
        let transport = Arc::new(
            Transport::start(
                TransportConfig {
                    bind,
                    limits: limits.clone(),
                },
                &identity,
            )
            .map_err(|error| RuntimeError::Transport(error.to_string()))?,
        );
        let local_addr = transport.local_addr();
        let addr_hint = config.addr_hint.unwrap_or_else(|| {
            let host = audio_share_core::server::default_host()
                .map(|address| address.to_string())
                .unwrap_or_else(|| "127.0.0.1".to_owned());
            format!("{host}:{}", local_addr.port())
        });
        let fingerprint = pairing.fingerprint();
        let admin: Arc<dyn PairingAdmin> = Arc::new(CorePairingAdmin::new(
            Arc::clone(&pairing),
            addr_hint.clone(),
        ));

        let accept_transport = Arc::clone(&transport);
        let sessions = Arc::new(sessions);
        let accept_task = tokio::spawn(async move {
            let mut guard = CapacityGuard::new(limits);
            loop {
                let accepted = accept_transport.accept_one(&mut guard).await;
                match accepted.connection {
                    Some(connection) => {
                        let remote = accepted.remote_address;
                        let server = Arc::clone(&sessions);
                        let mut session_guard = guard.clone();
                        tokio::spawn(async move {
                            let termination = server.serve_connection(connection, remote).await;
                            session_guard.session_ended();
                            let remote = remote
                                .map(|address| v2_transport::driver::masked_source(address.ip()))
                                .unwrap_or_else(|| "unknown".to_owned());
                            tracing::info!(
                                remote = %remote,
                                "v2 session ended: {:?}",
                                termination
                            );
                        });
                    }
                    None => {
                        let stopped = accepted.lifecycle.termination().is_some_and(|termination| {
                            termination.code().code
                                == v2_transport::codes::ErrorCode::SERVER_STOPPING.code
                        });
                        if stopped {
                            break;
                        }
                    }
                }
            }
        });

        Ok(Self {
            pairing,
            admin,
            transport,
            accept_task: Mutex::new(Some(accept_task)),
            local_addr,
            addr_hint,
            fingerprint,
        })
    }

    /// The shared Pairing Core (for CLI flows that drive invitations directly).
    pub fn admin(&self) -> Arc<dyn PairingAdmin> {
        Arc::clone(&self.admin)
    }

    /// The concrete shared Pairing Core, for event subscriptions.
    pub fn pairing(&self) -> &Arc<PairingCore> {
        &self.pairing
    }

    /// The actual QUIC listen address.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The `host:port` advertised in QR payloads.
    pub fn addr_hint(&self) -> &str {
        &self.addr_hint
    }

    /// The §7.4 Server identity fingerprint.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Stop accepting and close the endpoint. Idempotent; safe from the GUI
    /// controller before it awaits the v1 shutdown.
    pub fn stop(&self) {
        self.transport.stop();
        if let Some(task) = self
            .accept_task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            task.abort();
        }
    }
}

impl Drop for V2Runtime {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Resolve a bind host (name or IP) and port, preferring IPv6 so one listener
/// accepts IPv4-mapped and native IPv6 Clients.
pub async fn resolve_bind_address(host: &str, port: u16) -> Result<SocketAddr, RuntimeError> {
    let host = if host.is_empty() { "::" } else { host };
    tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| RuntimeError::Resolve(format!("{host}:{port}: {error}")))?
        .min_by_key(SocketAddr::is_ipv4)
        .ok_or_else(|| RuntimeError::Resolve(format!("{host}:{port} resolved to no addresses")))
}

/// The §9.2 revocation poll interval the driver uses. Revocation is effective
/// for new connections immediately and terminates an active session within
/// this interval, well inside the 5 s bound.
pub const REVOCATION_POLL_INTERVAL: Duration = v2_transport::driver::REVOCATION_POLL_INTERVAL;

/// The Server data directory implied by a config file path (`<config parent>`).
pub fn data_dir_for_config(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}
