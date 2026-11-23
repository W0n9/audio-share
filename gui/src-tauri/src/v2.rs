//! The controller's v2 seam.
//!
//! The production implementation adapts the shared `v2-runtime` crate. Tests
//! inject a fake runtime and a fake Pairing Core, so the controller's pairing
//! and device-management behavior is exercised without QUIC or a real
//! identity. This is the same seam the Tauri commands call.

use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use audio_share_core::capture::media::MediaTap;
use audio_share_core::capture::CaptureObserver;
use serde::Deserialize;
use v2_runtime::{PairingAdmin, RequestedPairingMode};

/// The v2 QUIC default port, parallel to v1's 65530.
pub use v2_runtime::DEFAULT_V2_PORT;

/// Inputs the controller supplies when starting v2 after v1 is running.
pub struct V2StartInputs {
    /// The Server data directory (identity + `pairing/clients.json`).
    pub data_dir: PathBuf,
    /// POSIX loose-permission override (pairing.md §10.2).
    pub allow_loose_permissions: bool,
    /// The v2 QUIC port.
    pub port: u16,
    /// The Host configured in Network settings. It selects the QR `addr`
    /// advertisement only; v2 still binds the dual-stack wildcard.
    pub host: String,
    /// The live v1 capture observer (read-only format/health).
    pub capture: CaptureObserver,
    /// The live v1 media tap (PCM chunks for v2 subscribers).
    pub media_tap: MediaTap,
}

/// A started v2 runtime as the controller sees it.
pub trait V2Runtime: Send + Sync {
    /// The actual QUIC listen address.
    fn local_addr(&self) -> SocketAddr;
    /// The shared Pairing Core adapter.
    fn admin(&self) -> Arc<dyn PairingAdmin>;
    /// Stop accepting and close the endpoint. Idempotent.
    fn stop(&self);
}

impl V2Runtime for v2_runtime::V2Runtime {
    fn local_addr(&self) -> SocketAddr {
        v2_runtime::V2Runtime::local_addr(self)
    }

    fn admin(&self) -> Arc<dyn PairingAdmin> {
        v2_runtime::V2Runtime::admin(self)
    }

    fn stop(&self) {
        v2_runtime::V2Runtime::stop(self)
    }
}

/// A boxed async startup result.
pub type StartFuture = Pin<Box<dyn Future<Output = Result<Arc<dyn V2Runtime>, String>> + Send>>;

/// Starts (or refuses) v2 for the controller.
pub type V2Starter = Arc<dyn Fn(V2StartInputs) -> StartFuture + Send + Sync>;

/// The production starter: assemble the shared v2 runtime exactly as `as-cmd`.
pub fn production_starter() -> V2Starter {
    Arc::new(|inputs| {
        Box::pin(async move {
            // The Host chooses only the advertised QR address. Wildcards,
            // empty values, and scoped literals yield `None`, so the runtime
            // keeps its default non-loopback IPv4 fallback and the payload
            // stays parseable. The port is the v2 port the runtime binds.
            let addr_hint = v2_runtime::qr_addr_hint(&inputs.host, inputs.port);
            if addr_hint.is_none() && inputs.host.trim().contains('%') {
                tracing::warn!(
                    host = %inputs.host,
                    "v2 QR address falls back: a scoped IPv6 host cannot be advertised"
                );
            }
            let runtime = v2_runtime::V2Runtime::start(
                v2_runtime::RuntimeConfig {
                    data_dir: inputs.data_dir,
                    allow_loose_permissions: inputs.allow_loose_permissions,
                    bind_host: "::".to_owned(),
                    port: inputs.port,
                    addr_hint,
                },
                Some((inputs.capture, inputs.media_tap)),
            )
            .await
            .map_err(|error| error.to_string())?;
            Ok(Arc::new(runtime) as Arc<dyn V2Runtime>)
        })
    })
}

/// The frontend's pairing-mode selector.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PairingModeArg {
    /// QR primary with the SAS fallback window open.
    Qr,
    /// SAS only.
    Sas,
}

impl From<PairingModeArg> for RequestedPairingMode {
    fn from(mode: PairingModeArg) -> Self {
        match mode {
            PairingModeArg::Qr => RequestedPairingMode::Qr,
            PairingModeArg::Sas => RequestedPairingMode::Sas,
        }
    }
}
