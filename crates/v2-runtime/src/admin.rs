//! Protocol-neutral pairing administration for local surfaces (CLI and GUI).
//!
//! The production implementation wraps the shared [`v2_transport::pairing::PairingCore`];
//! a display-free controller can substitute a fake through this trait so its
//! tests need no QUIC endpoint or real identity.

use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use v2_transport::auth_store::ClientState;
use v2_transport::pairing::{ApprovalRequest, Invitation, InvitationState, PairingCore};

/// The pairing mode the operator asks for.
///
/// Named distinctly from the wire [`v2_transport::schema::PairingMode`] and the
/// Android `ClientPairingMode` so adapter code cannot mix them up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestedPairingMode {
    /// QR primary with the SAS admission window as a fallback (like `as-cmd pair`).
    Qr,
    /// SAS only (no nonce, no QR).
    Sas,
}

/// A fresh invitation as presented to the local operator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvitationView {
    /// The mode that opened this invitation.
    pub mode: RequestedPairingMode,
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
    /// The process-local approval handle used by approve/decline.
    pub approval_id: u64,
    /// The canonical client id (hex).
    pub client_id: String,
    /// The accepted display name.
    pub display_name: String,
    /// QR or SAS.
    pub mode: RequestedPairingMode,
    /// The source address shown to the local operator only.
    pub address: String,
    /// The per-connection SAS (`123 456`), known only after the device connects.
    pub sas: Option<String>,
    /// `Some(saved_display_name)` when the identity's stored record is
    /// revoked: approving restores it in place (ADR-0006), and the saved name
    /// is the one that comes back. `None` for a new or active identity.
    pub restore_candidate: Option<String>,
}

/// A live pairing presented before the Client's confirmation (ticket 29).
///
/// The pairing code is derived from the live connection; the operator compares
/// it with the phone before the post-confirm approval card appears.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentedRequestView {
    /// The process-local handle the approval card will use.
    pub approval_id: u64,
    /// The canonical client id (hex).
    pub client_id: String,
    /// The accepted display name.
    pub display_name: String,
    /// QR or SAS.
    pub mode: RequestedPairingMode,
    /// The source address shown to the local operator only.
    pub address: String,
    /// The per-connection SAS (`123 456`).
    pub sas: Option<String>,
    /// The §7.4 Server SPKI fingerprint.
    pub fingerprint: String,
    /// Seconds until the pairing deadline, measured when the view was built.
    pub expires_in_secs: u64,
    /// `Some(saved_display_name)` when the identity's stored record is
    /// revoked: approving restores it in place (ADR-0006). `None` otherwise.
    pub restore_candidate: Option<String>,
}

/// Lifecycle of the most recently opened QR invitation, for local operator
/// surfaces. A consumed invitation stays consumed even after its expiry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvitationStateView {
    /// The QR nonce is outstanding and still usable.
    Active,
    /// The first well-formed pairing request consumed the nonce.
    Consumed,
    /// The invitation's lifetime elapsed without a request consuming it.
    Expired,
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
    fn begin(&self, mode: RequestedPairingMode) -> Result<InvitationView, String>;
    /// The lifecycle state of the most recently opened QR invitation, if one
    /// was opened. Local only; no wire message carries it.
    fn invitation_state(&self) -> Option<InvitationStateView>;
    /// Close the SAS window and decline every undecided approval.
    fn cancel(&self);
    /// Every verified-but-undecided pairing.
    fn pending(&self) -> Vec<PendingRequestView>;
    /// Every live pairing presented before the Client's confirmation.
    fn presented(&self) -> Vec<PresentedRequestView>;
    /// Approve one pending pairing by its process-local `approval_id`;
    /// `false` when the id is not pending. A plain Accept never restores a
    /// revoked identity (ADR-0006).
    fn approve(&self, approval_id: u64) -> bool;
    /// Approve one pending pairing with the explicit Restore decision;
    /// `false` when the id is not pending. Use only when the request carries
    /// a `restore_candidate` (ADR-0006).
    fn restore(&self, approval_id: u64) -> bool;
    /// Decline one pending pairing by its process-local `approval_id`;
    /// `false` when the id is not pending.
    fn decline(&self, approval_id: u64) -> bool;
    /// Every persisted Client record (active and revoked).
    fn clients(&self) -> Vec<ClientView>;
    /// Revoke one Client by canonical `client_id`; keeps the record for audit.
    fn revoke(&self, client_id: &str) -> Result<ClientView, String>;
    /// Delete one **revoked** Client record by canonical `client_id`
    /// (permanent). An active record is an error and is left unchanged: the
    /// operator must revoke first (ADR-0006).
    fn delete(&self, client_id: &str) -> Result<ClientView, String>;
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
            mode: requested_mode(invitation.mode),
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
    fn begin(&self, mode: RequestedPairingMode) -> Result<InvitationView, String> {
        let invitation = match mode {
            RequestedPairingMode::Qr => {
                let invitation = self.core.begin_qr(&self.addr_hint);
                // The QR dialog is also the camera-less fallback: admit SAS
                // requests in the same operator-opened window (as-cmd pair).
                self.core.begin_sas();
                invitation
            }
            RequestedPairingMode::Sas => self.core.begin_sas(),
        };
        Ok(self.view(&invitation))
    }

    fn cancel(&self) {
        self.core.cancel();
    }

    fn invitation_state(&self) -> Option<InvitationStateView> {
        self.core.invitation_state().map(|state| match state {
            InvitationState::Active => InvitationStateView::Active,
            InvitationState::Consumed => InvitationStateView::Consumed,
            InvitationState::Expired => InvitationStateView::Expired,
        })
    }

    fn pending(&self) -> Vec<PendingRequestView> {
        self.core
            .pending_approvals()
            .into_iter()
            .map(pending_view)
            .collect()
    }

    fn presented(&self) -> Vec<PresentedRequestView> {
        let now = Instant::now();
        self.core
            .presented()
            .into_iter()
            .map(|presented| PresentedRequestView {
                approval_id: presented.approval_id,
                client_id: presented.client_id,
                display_name: presented.display_name,
                mode: requested_mode(presented.mode),
                address: presented.address,
                sas: presented.sas,
                fingerprint: presented.fingerprint,
                expires_in_secs: presented
                    .expires_at
                    .saturating_duration_since(now)
                    .as_secs(),
                restore_candidate: presented.restore_candidate,
            })
            .collect()
    }

    fn approve(&self, approval_id: u64) -> bool {
        self.core.approve(approval_id)
    }

    fn restore(&self, approval_id: u64) -> bool {
        self.core.restore(approval_id)
    }

    fn decline(&self, approval_id: u64) -> bool {
        self.core.decline(approval_id)
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

    fn delete(&self, client_id: &str) -> Result<ClientView, String> {
        self.core
            .delete(client_id)
            .map(client_view)
            .map_err(|error| error.to_string())
    }

    fn fingerprint(&self) -> String {
        self.core.fingerprint()
    }
}

fn pending_view(request: ApprovalRequest) -> PendingRequestView {
    PendingRequestView {
        approval_id: request.approval_id,
        client_id: request.client_id,
        display_name: request.display_name,
        mode: requested_mode(request.mode),
        address: request.address,
        sas: request.sas,
        restore_candidate: request.restore_candidate,
    }
}

/// Single schema→admin mirror; an SAS invitation is SAS, everything else is
/// presented as the QR/mixed request the operator opened.
fn requested_mode(mode: v2_transport::schema::PairingMode) -> RequestedPairingMode {
    match mode {
        v2_transport::schema::PairingMode::Sas => RequestedPairingMode::Sas,
        _ => RequestedPairingMode::Qr,
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
