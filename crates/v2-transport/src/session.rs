//! Connection lifecycle exposed through the domain API (spec §5.1 states,
//! §5.4 terminal behavior; as-cmd/GUI observe this shared v1/v2 lifecycle
//! state instead of parsing wire bytes — ticket 06 provides the plumbing only).
//!
//! The full §5.2 Server transition table (authentication, negotiation) is
//! ticket 07; this module carries the states and the transport-level
//! transitions this ticket implements: CONNECTING entry, capacity rejection,
//! transport loss, close codes.

use crate::codes::ErrorCode;

/// The §5.1 session states, as far as this ticket drives them. AUTHENTICATING
/// / NEGOTIATING / READY / PLAYING are entered by ticket 07's state machine;
/// they exist here so the domain API is stable from the start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleState {
    /// Transport handshake done (or in progress), no app message processed.
    Connecting,
    /// An authentication or pairing exchange is in progress (ticket 07).
    Authenticating,
    /// Identity authenticated, awaiting/validating version + capabilities
    /// (ticket 07).
    Negotiating,
    /// Authenticated and negotiated (ticket 07).
    Ready,
    /// Ready, subscribed, and a stream instance is active (ticket 07+).
    Playing,
    /// `Close` sent or received; teardown in progress (5 s grace, §5.1).
    Closing,
    /// Terminal: all session state released (§5.4).
    Closed,
}

impl LifecycleState {
    /// Whether the state is terminal (§5.4: CLOSED absorbs everything).
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Closed)
    }

    /// Whether CLOSING is in progress (teardown observable, §5.1).
    pub const fn is_closing(self) -> bool {
        matches!(self, Self::Closing)
    }
}

/// How a session's transport ended. Every variant carries the §6 registry
/// code that rode (or would have ridden) the QUIC application close —
/// "transport closes are diagnosable without application data" (spec §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Termination {
    /// The Server closed the transport with this §6 code (e.g. 301 busy).
    ServerClosed(ErrorCode),
    /// The Client closed the transport (e.g. 701 client stop, 303 observed).
    ClientClosed(ErrorCode),
    /// Transport lost without a Close message — classified 601 locally
    /// (spec §5.2/§5.3 "transport lost" rows).
    Lost(ErrorCode),
}

impl Termination {
    /// The registry code this termination carries.
    pub const fn code(self) -> ErrorCode {
        match self {
            Self::ServerClosed(code) | Self::ClientClosed(code) | Self::Lost(code) => code,
        }
    }
}

/// The observable lifecycle of one v2 Client connection.
///
/// The Server accept loop drives transitions; as-cmd/GUI read them through
/// the domain API. This is the "minimal visibility" the ticket requires —
/// state + termination reason, no wire-byte parsing downstream.
#[derive(Debug)]
pub struct ConnectionLifecycle {
    state: LifecycleState,
    termination: Option<Termination>,
}

impl ConnectionLifecycle {
    /// A fresh connection at CONNECTING (§5.1 entry).
    pub fn new() -> Self {
        Self { state: LifecycleState::Connecting, termination: None }
    }

    /// The current state.
    pub const fn state(&self) -> LifecycleState {
        self.state
    }

    /// The termination, once the connection has ended.
    pub const fn termination(&self) -> Option<Termination> {
        self.termination
    }

    /// Move to CLOSING (Close sent or received; teardown in progress).
    ///
    /// CLOSING accepts no new requests (§5.2) — the caller checks
    /// [`LifecycleState::is_closing`] before admitting work.
    pub fn begin_close(&mut self) {
        if !self.state.is_terminal() {
            self.state = LifecycleState::Closing;
        }
    }

    /// Complete the teardown: CLOSED is absorbing (§5.4). Records the
    /// termination for the domain API; further transitions are no-ops.
    pub fn complete(&mut self, termination: Termination) {
        if self.state.is_terminal() {
            return;
        }
        self.termination = Some(termination);
        self.state = LifecycleState::Closed;
    }

    /// Whether the session still accepts work (not CLOSING/CLOSED).
    pub const fn is_active(&self) -> bool {
        !matches!(self.state, LifecycleState::Closing | LifecycleState::Closed)
    }
}

impl Default for ConnectionLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_starts_connecting() {
        let lifecycle = ConnectionLifecycle::new();
        assert_eq!(lifecycle.state(), LifecycleState::Connecting);
        assert!(lifecycle.is_active());
        assert_eq!(lifecycle.termination(), None);
    }

    #[test]
    fn closing_then_closed_is_absorbing() {
        let mut lifecycle = ConnectionLifecycle::new();
        lifecycle.begin_close();
        assert!(lifecycle.state().is_closing());
        assert!(!lifecycle.is_active());
        lifecycle.complete(Termination::ServerClosed(ErrorCode::MALFORMED_MESSAGE));
        assert_eq!(lifecycle.state(), LifecycleState::Closed);
        assert!(lifecycle.state().is_terminal());
        // CLOSED absorbs further transitions (§5.4).
        lifecycle.complete(Termination::Lost(ErrorCode::NETWORK_FAILURE));
        lifecycle.begin_close();
        assert_eq!(lifecycle.state(), LifecycleState::Closed);
        assert_eq!(
            lifecycle.termination(),
            Some(Termination::ServerClosed(ErrorCode::MALFORMED_MESSAGE))
        );
    }

    #[test]
    fn termination_carries_registry_code() {
        assert_eq!(Termination::ServerClosed(ErrorCode::BUSY).code().code, 301);
        assert_eq!(Termination::ClientClosed(ErrorCode::CLIENT_STOPPING).code().code, 701);
        // Transport loss classifies 601 locally (§5.2 "any: transport lost").
        assert_eq!(Termination::Lost(ErrorCode::NETWORK_FAILURE).code().code, 601);
    }

    #[test]
    fn lost_without_close_classifies_601() {
        let mut lifecycle = ConnectionLifecycle::new();
        lifecycle.complete(Termination::Lost(ErrorCode::NETWORK_FAILURE));
        assert_eq!(lifecycle.termination().map(|t| t.code()), Some(ErrorCode::NETWORK_FAILURE));
    }
}
