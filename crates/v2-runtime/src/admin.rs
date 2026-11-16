//! Protocol-neutral pairing administration for local surfaces (CLI and GUI).
//!
//! The production implementation wraps the shared [`v2_transport::pairing::PairingCore`];
//! a display-free controller can substitute a fake through this trait so its
//! tests need no QUIC endpoint or real identity.

use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use v2_transport::auth_store::ClientState;
use v2_transport::pairing::{ApprovalRequest, Invitation, PairingCore};

/// The pairing mode the operator asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairingMode {
    /// QR primary with the SAS admission window as a fallback (like `as-cmd pair`).
    Qr,
    /// SAS only (no nonce, no QR).
    Sas,
}

/// A fresh invitation as presented to the local operator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvitationView {
    /// The mode that opened this invitation.
    pub mode: PairingMode,
    /// The copyable QR payload URI (QR mode only).
    pub qr_uri: Option<String>,
    /// A self-contained SVG QR rendering (QR mode only).
    pub qr_svg: Option<String>,
    /// The §7.4 Server SPKI fingerprint.
    pub fingerprint: String,
    /// Seconds until the invitation expires, measured when it was created.
    pub expires_in_secs: u64,
}

/// One verified pairing awaiting the local operator's decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingRequestView {
    /// The pairing exchange's request id.
    pub request_id: u64,
    /// The canonical client id (hex).
    pub client_id: String,
    /// The accepted display name.
    pub display_name: String,
    /// QR or SAS.
    pub mode: PairingMode,
    /// The source address shown to the local operator only.
    pub address: String,
    /// The per-connection SAS (`123 456`), known only after the device connects.
    pub sas: Option<String>,
}

/// A persisted Client record for the device list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientView {
    /// Canonical client id (hex).
    pub client_id: String,
    /// Device display name.
    pub display_name: String,
    /// Active or revoked (revoked records stay visible for audit).
    pub state: ClientStateView,
    /// RFC 3339 UTC creation time, or empty for a legacy record.
    pub created_at: String,
    /// RFC 3339 UTC last-seen time, or empty for a legacy record.
    pub last_seen_at: String,
}

/// The authorization state exposed to the local operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientStateView {
    /// Authorized.
    Active,
    /// Revoked; the record remains for audit.
    Revoked,
}

/// Local pairing/administration operations.
///
/// Implementations must be cheap and synchronous: the GUI polls them from a
/// command thread, and the pairing core never blocks on human input.
pub trait PairingAdmin: Send + Sync + fmt::Debug {
    /// Open an invitation. QR mode also opens the SAS admission window.
    fn begin(&self, mode: PairingMode) -> Result<InvitationView, String>;
    /// Close the SAS window and drop undecided approvals.
    fn cancel(&self);
    /// Every verified-but-undecided pairing.
    fn pending(&self) -> Vec<PendingRequestView>;
    /// Approve one pending pairing; `false` when the id is not pending.
    fn approve(&self, request_id: u64) -> bool;
    /// Decline one pending pairing; `false` when the id is not pending.
    fn decline(&self, request_id: u64) -> bool;
    /// Every persisted Client record (active and revoked).
    fn clients(&self) -> Vec<ClientView>;
    /// Revoke one Client by canonical `client_id`; keeps the record for audit.
    fn revoke(&self, client_id: &str) -> Result<ClientView, String>;
    /// The §7.4 Server SPKI fingerprint.
    fn fingerprint(&self) -> String;
}

/// The production adapter over the shared Pairing Core.
#[derive(Debug)]
pub struct CorePairingAdmin {
    core: Arc<PairingCore>,
    addr_hint: String,
}

impl CorePairingAdmin {
    /// Wrap `core`; `addr_hint` is embedded in QR payloads.
    pub fn new(core: Arc<PairingCore>, addr_hint: impl Into<String>) -> Self {
        Self {
            core,
            addr_hint: addr_hint.into(),
        }
    }

    fn view(&self, invitation: &Invitation) -> InvitationView {
        let qr_uri = self.core.qr_uri(invitation, &self.addr_hint);
        let qr_svg = qr_uri.as_deref().and_then(crate::qr::render_svg);
        InvitationView {
            mode: match invitation.mode {
                v2_transport::schema::PairingMode::Sas => PairingMode::Sas,
                _ => PairingMode::Qr,
            },
            qr_uri,
            qr_svg,
            fingerprint: self.core.fingerprint(),
            expires_in_secs: invitation
                .expires_at
                .saturating_duration_since(Instant::now())
                .as_secs(),
        }
    }
}

impl PairingAdmin for CorePairingAdmin {
    fn begin(&self, mode: PairingMode) -> Result<InvitationView, String> {
        let invitation = match mode {
            PairingMode::Qr => {
                let invitation = self.core.begin_qr(&self.addr_hint);
                // The QR dialog is also the camera-less fallback: admit SAS
                // requests in the same operator-opened window (as-cmd pair).
                self.core.begin_sas();
                invitation
            }
            PairingMode::Sas => self.core.begin_sas(),
        };
        Ok(self.view(&invitation))
    }

    fn cancel(&self) {
        self.core.cancel();
    }

    fn pending(&self) -> Vec<PendingRequestView> {
        self.core
            .pending_approvals()
            .into_iter()
            .map(pending_view)
            .collect()
    }

    fn approve(&self, request_id: u64) -> bool {
        self.core.approve(request_id)
    }

    fn decline(&self, request_id: u64) -> bool {
        self.core.decline(request_id)
    }

    fn clients(&self) -> Vec<ClientView> {
        self.core.clients().into_iter().map(client_view).collect()
    }

    fn revoke(&self, client_id: &str) -> Result<ClientView, String> {
        self.core
            .revoke(client_id)
            .map(client_view)
            .map_err(|error| error.to_string())
    }

    fn fingerprint(&self) -> String {
        self.core.fingerprint()
    }
}

fn pending_view(request: ApprovalRequest) -> PendingRequestView {
    PendingRequestView {
        request_id: request.request_id,
        client_id: request.client_id,
        display_name: request.display_name,
        mode: match request.mode {
            v2_transport::schema::PairingMode::Sas => PairingMode::Sas,
            _ => PairingMode::Qr,
        },
        address: request.address,
        sas: request.sas,
    }
}

fn client_view(client: v2_transport::auth_store::StoredClient) -> ClientView {
    ClientView {
        client_id: client.client_id,
        display_name: client.display_name,
        state: match client.state {
            ClientState::Active => ClientStateView::Active,
            ClientState::Revoked => ClientStateView::Revoked,
        },
        created_at: client.created_at,
        last_seen_at: client.last_seen_at,
    }
}
