//! The §5.2 Server session state machine — pure, synchronous, decidable.
//!
//! This module owns **state and semantics only**: given one [`Event`] it
//! returns the ordered [`Outcome`]s (control frames to write, fatal close
//! codes, transport-close codes) and the new [`ServerState`]. I/O lives in the
//! connection driver (a later wiring slice); tests exercise the table row by
//! row exactly as `docs/wire-protocol-v2/spec.md` §5.2 lists it.
//!
//! Rows implemented here (media rows — capture-driven PLAYING transitions —
//! are ticket 10+, and pairing-crypto rows route through the pairing gate):
//!
//! - CONNECTING: AuthInit → challenge · PairingRequest → admission ·
//!   capacity (transport close 301, enforced by the accept loop) ·
//!   malformed → 401 · valid-but-forbidden → 402 · second-stream → 402.
//! - CONNECTING/AUTHENTICATING: auth deadline (10 s) → 601.
//! - AUTHENTICATING: proof verify → OK/202/203/204 · 5th failure → Close(203)
//!   · AuthInit retry → fresh challenge · address rate → 302 · supersede.
//! - NEGOTIATING: Negotiate → OK/101/401 · anything else → 402.
//! - READY: Subscribe/Unsubscribe/RequestConfiguration rows incl. the
//!   non-fatal duplicate-Subscribe 402 and the idempotent Unsubscribe.
//! - any: advisory Error · fatal error pair · transport lost · idle timeout.
//! - CLOSING: absorbs; no new requests.

use std::sync::Arc;
use std::time::Instant;

use crate::auth;
use crate::auth_store::{Authorization, AuthorizationStore, StoredClient};
use crate::codes::ErrorCode;
use crate::pairing::{
    self, ApprovalRequest, PairingCore, PairingError, PairingExporter, PairingPermit,
    PairingPresented, PAIRING_EXPORTER_LABEL, SAS_EXPORTER_LABEL,
};
use crate::schema::{frame, Frame, PairingMode, Status};

/// The §5.1/§5.2 session states this machine drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerState {
    /// Transport handshake done; no app message processed yet.
    Connecting,
    /// An authentication or pairing exchange is in progress.
    Authenticating,
    /// Authenticated; awaiting/validating version + capabilities.
    Negotiating,
    /// Authenticated ∧ negotiated (subscribedness is a data flag).
    Ready,
    /// READY ∧ subscribed ∧ a stream instance is active (spec §5.1).
    Playing,
    /// `Close` sent or received; teardown in progress; no new requests.
    Closing,
    /// Terminal — all session state released (§5.4).
    Closed,
}

/// A read-only capture/stream observation shared by the Server and its
/// session machines (§5.2). It carries state, not media or recovery actions.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureSnapshot {
    /// The current Transfer Format, or `None` when capture is absent.
    pub configuration: Option<crate::schema::Configuration>,
    /// Whether the capture backend is healthy.
    pub healthy: bool,
    /// Whether a stream instance is active (Subscribe pushes StreamStarted).
    pub stream_active: bool,
    /// The active stream instance id (meaningful iff `stream_active`).
    pub stream_instance_id: u32,
    /// Capture-clock position of the next media frame (iff `stream_active`).
    pub next_frame_timestamp: u64,
}

impl CaptureSnapshot {
    /// No capture injected: Subscribe is refused with 501.
    pub fn absent() -> Self {
        Self {
            configuration: None,
            healthy: true,
            stream_active: false,
            stream_instance_id: 1,
            next_frame_timestamp: 0,
        }
    }
}

/// Server identity/session policy plus a replaceable capture observation.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// The raw 32-byte Ed25519 identity public key (PairingResponse /
    /// auth_context TLV(4)).
    pub server_public_key: Vec<u8>,
    /// The Server's minor within major 2 (NegotiateResponse).
    pub protocol_minor: u32,
    /// The latest capture observation used for configuration publication,
    /// Subscribe gating and StreamStarted (§5.2).
    pub capture: CaptureSnapshot,
    /// Auth failures per connection (TM-4; default 5, `Limits`).
    pub max_auth_failures_per_connection: u32,
    /// Test-support only: fixed challenge randomness (spec §4.4 says 32
    /// CSPRNG bytes; the golden vectors pin constants, and pairing.md §11.3
    /// requires such bypasses behind a compile-time feature). Production
    /// builds draw fresh randomness on every `AuthInit`.
    #[cfg(feature = "test-support")]
    pub server_random_override: Option<Vec<u8>>,
    /// The TLS-Exporter auth component (pairing.md §9.1 TLV(6)): a
    /// connection-scoped secret both peers derive from the same TLS session.
    /// The driver always supplies the real per-connection output; fixture
    /// replay supplies the recorded constant.
    pub auth_exporter: Option<Vec<u8>>,
    /// The first session id this machine assigns (§3: the Server assigns
    /// ids from a registry-wide counter; the driver seeds each machine with
    /// its position). Defaults to 1 for a standalone machine; the golden-
    /// vector replay uses it to reproduce the registry's numbering.
    pub session_id_base: u64,
    /// The shared production Pairing Core. `None` disables production pairing
    /// (a PairingRequest is then refused 201, the pre-ticket-19 behavior).
    pub pairing: Option<Arc<PairingCore>>,
    /// The connection-scoped TLS exporter used to derive `binding` and
    /// `sas_seed` (pairing.md §6.3). The driver supplies the real quinn
    /// exporter; tests supply the golden-vector stand-ins.
    pub pairing_exporter: Option<Arc<dyn PairingExporter>>,
    /// Test-support only: fixed pairing `server_random` (the golden vector
    /// pins `4041…`). Distinct from the auth challenge override.
    #[cfg(feature = "test-support")]
    pub pairing_server_random_override: Option<Vec<u8>>,
}

impl ServerConfig {
    /// Test-support builds may pin the challenge randomness (golden-vector
    /// replay); production always returns `None` here (compile-time gated,
    /// pairing.md §11.3).
    fn fixed_server_random(&self) -> Option<Vec<u8>> {
        #[cfg(feature = "test-support")]
        {
            self.server_random_override.clone()
        }
        #[cfg(not(feature = "test-support"))]
        {
            None
        }
    }

    /// Test-support builds may pin the pairing `server_random` independently
    /// of the auth challenge randomness (the golden vectors differ).
    fn fixed_pairing_server_random(&self) -> Option<Vec<u8>> {
        #[cfg(feature = "test-support")]
        {
            self.pairing_server_random_override.clone()
        }
        #[cfg(not(feature = "test-support"))]
        {
            None
        }
    }
}

/// One inbound occurrence the machine reacts to.
#[derive(Clone, Debug)]
pub enum Event {
    /// A decoded control frame from the Client (framing already validated).
    /// The frame is boxed: variants stay size-balanced (clippy
    /// large_enum_variant) — frames are rare, heap cost is negligible.
    Frame {
        /// The decoded frame.
        frame: Box<Frame>,
        /// The connection's source address (rate-limit key, TM-4).
        source: Vec<u8>,
    },
    /// The 10 s auth-exchange deadline fired (spec §3).
    AuthDeadline,
    /// The 30 s idle timeout fired (§5.2 "any" row → 601).
    IdleTimeout,
    /// The transport died without a Close message (§5.2 "any" row).
    TransportLost,
    /// The Client sent a raw Close (§5.2 READY/PLAYING/any row: no echo).
    CloseReceived,
    /// A fatal condition arose server-side (§5.2 "any": matching Error+Close).
    Fatal(ErrorCode),
    /// Periodic revocation check (pairing.md §9.2: ≤ 5 s termination).
    RevocationCheck,
    /// The 10-minute pairing deadline fired (§5.2 → Error(206)+Close(206)).
    PairingDeadline,
    /// The local operator approved a verified pairing (§5.2 "approval
    /// granted" row). Fed by the driver from the Pairing Core decision.
    PairingApproved {
        /// The process-local approval handle (never the wire request id).
        approval_id: u64,
    },
    /// The local operator declined a verified pairing (§5.2 "approval
    /// declined" row): 201 + Close(201), nothing persisted.
    PairingDeclined {
        /// The process-local approval handle (never the wire request id).
        approval_id: u64,
    },
    /// The session registry decided this session is superseded by a newer
    /// session of the same identity (TM-3; the driver owns the registry and
    /// translates its decision into this event on the OLD machine). The
    /// machine emits Close carrying the code and closes the transport with
    /// it (§5.2: Close(303) on the superseded session).
    Superseded {
        /// The close code carrying the supersede reason (303).
        code: ErrorCode,
    },
    /// The shared capture's stream lifecycle changed (§5.2 capture rows). The
    /// driver derives these from the media source; the machine turns them
    /// into the ordered pushes a subscribed Client must receive.
    StreamLifecycle(StreamLifecycle),
}

/// A capture-driven stream transition (§5.2 "capture started/ended/restart/
/// config change" rows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamLifecycle {
    /// A stream instance became active.
    Started {
        /// The new instance id.
        stream_instance_id: u32,
        /// Generation of the media that follows.
        config_generation: u32,
        /// Capture-clock position of the first frame the Client will get.
        initial_timestamp: u64,
    },
    /// The active instance ended. `code` distinguishes 502 STREAM_ENDED from
    /// 501 CAPTURE_FAILURE.
    Ended {
        /// The instance that ended.
        stream_instance_id: u32,
        /// §6 code carried by StreamEnded.
        code: ErrorCode,
    },
    /// A capture restart: the old instance ends normally and a new one starts.
    Restarted {
        /// The instance that ended.
        old_stream_instance_id: u32,
        /// The new instance id.
        stream_instance_id: u32,
        /// Generation of the new instance's media.
        config_generation: u32,
        /// Capture-clock origin of the new instance.
        initial_timestamp: u64,
    },
    /// A decode-affecting configuration change within the active instance.
    ConfigChanged {
        /// The new generation (published as ConfigurationPublished).
        config_generation: u32,
    },
}

/// One machine output, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// The control frame to send (None for transport-close-only outcomes).
    pub frame: Option<Frame>,
    /// When set: a fatal pairing — send this Error frame's code as `Close`
    /// right after the frame, then transport-close with the same code
    /// (§5.2 fatal class: Error + Close + transport close).
    pub close_code: Option<ErrorCode>,
    /// When set (and `frame` is None): close the transport directly with this
    /// code, no control frame (§5.2 rows that name only a transport close).
    pub transport_close_code: Option<ErrorCode>,
    /// A persisted pairing to announce after this outcome's frame (the
    /// `PairingResult(OK)`) has been written successfully. The driver fires
    /// the broadcast only then, so a one-shot consumer cannot stop the Server
    /// before the success result reaches the Client.
    pub paired_client: Option<StoredClient>,
}

impl Outcome {
    fn frame(frame: Frame) -> Self {
        Self {
            frame: Some(frame),
            close_code: None,
            transport_close_code: None,
            paired_client: None,
        }
    }

    /// The fatal pair: Error + Close with the same code, then transport close.
    fn fatal(code: ErrorCode, request_id: u64) -> Vec<Outcome> {
        vec![
            Outcome {
                frame: Some(Frame {
                    body: Some(frame::Body::Error(crate::schema::Error {
                        request_id,
                        status: Status::registry(code.code),
                    })),
                }),
                close_code: None,
                transport_close_code: None,
                paired_client: None,
            },
            Outcome {
                frame: Some(Frame {
                    body: Some(frame::Body::Close(crate::schema::Close {
                        code: code.code,
                        detail: String::new(),
                    })),
                }),
                close_code: Some(code),
                transport_close_code: None,
                paired_client: None,
            },
        ]
    }

    /// A bare transport close (301/601-style rows).
    fn transport_close(code: ErrorCode) -> Self {
        Self {
            frame: None,
            close_code: None,
            transport_close_code: Some(code),
            paired_client: None,
        }
    }
}

/// The result of a supersede decision: which older session must be closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupersedeRequest {
    /// The superseded session's id.
    pub old_session_id: u64,
    /// The code the old session is closed with: 303.
    pub code: ErrorCode,
}

/// A pairing exchange admitted by the admission gate. It stays pending until
/// the Client's proof verifies **and** the local operator approves (§5.2).
struct PendingPairing {
    request_id: u64,
    /// Process-local approval handle, assigned when the exchange is surfaced
    /// to the local operator.
    approval_id: Option<u64>,
    mode: PairingMode,
    client_public_key: Vec<u8>,
    display_name: String,
    binding: [u8; pairing::BINDING_LEN],
    sas: Option<String>,
    /// `binding_echo` + `client_proof` verified; persistence still waits for
    /// local approval.
    verified: bool,
    /// The driver has already handed this request to the local operator.
    approval_requested: bool,
    /// The live view published to the operator before `PairingConfirm`
    /// (delivered once; ticket 29).
    presented: Option<PairingPresented>,
    /// The TM-5 concurrency slot for this exchange; dropped with the pending
    /// exchange on approval, decline, deadline, or connection loss.
    _permit: PairingPermit,
}

/// One Server-side session machine (§5.2). Not `Sync`-shared: one per
/// connection; the session registry (identity → live session) is shared.
pub struct StateMachine {
    config: ServerConfig,
    store: Arc<AuthorizationStore>,
    state: ServerState,
    /// The next session id (Server counter from 1, spec §4.4).
    next_session_id: u64,
    /// This session's id, set at auth success.
    session_id: Option<u64>,
    /// The authenticated identity (33-byte compressed key), set at success.
    identity: Option<Vec<u8>>,
    /// Client's authorized subscription flag (§5.2 READY rows).
    subscribed: bool,
    /// TM-4 per-connection failure counter.
    auth_failures: u32,
    /// Code of the latest authentication failure, drained by the driver for
    /// its redacted failure log (§6 registry).
    last_auth_failure_code: Option<ErrorCode>,
    /// The pending challenge's server_random (verification context).
    pending_challenge: Option<Vec<u8>>,
    /// The request_id of the pending auth exchange.
    pending_request_id: Option<u64>,
    /// The claimed client key from the pending AuthInit (proof verification
    /// context; pairing.md §9.1 TLV(3)).
    pending_client_key: Option<Vec<u8>>,
    /// Supersede decision for the driver (an older session of this identity).
    supersede: Option<SupersedeRequest>,
    /// The latest §9 PlaybackDiagnostics this Client reported (the only
    /// playback evidence the Server accepts; never a transport ACK).
    last_diagnostics: Option<crate::schema::PlaybackDiagnostics>,
    /// The admitted pairing exchange awaiting proof + local approval.
    pending_pairing: Option<PendingPairing>,
    /// The §6 code of the most recent pairing failure, drained by the driver
    /// for its redacted failure log (never a nonce, SAS, or proof).
    last_pairing_failure_code: Option<ErrorCode>,
}

impl StateMachine {
    /// A fresh machine at CONNECTING.
    pub fn new(config: ServerConfig, store: &Arc<AuthorizationStore>) -> Self {
        let session_id_base = config.session_id_base.max(1);
        Self {
            config,
            store: Arc::clone(store),
            state: ServerState::Connecting,
            next_session_id: session_id_base,
            session_id: None,
            identity: None,
            subscribed: false,
            auth_failures: 0,
            last_auth_failure_code: None,
            pending_challenge: None,
            pending_request_id: None,
            pending_client_key: None,
            supersede: None,
            last_diagnostics: None,
            pending_pairing: None,
            last_pairing_failure_code: None,
        }
    }

    /// The current state.
    pub const fn state(&self) -> ServerState {
        self.state
    }

    /// The session id, once assigned (auth success).
    pub const fn session_id(&self) -> Option<u64> {
        self.session_id
    }

    /// The authenticated identity's public key, once known.
    pub fn identity(&self) -> Option<&[u8]> {
        self.identity.as_deref()
    }

    /// The subscription flag (§5.2 READY rows).
    pub const fn subscribed(&self) -> bool {
        self.subscribed
    }

    /// Auth failures recorded on this connection (TM-4 diagnostics).
    pub const fn auth_failures(&self) -> u32 {
        self.auth_failures
    }

    /// A recorded supersede decision, if any (the driver closes the old
    /// session with Close(303)).
    pub const fn supersede_requested(&self) -> Option<ErrorCode> {
        match self.supersede {
            Some(_) => Some(ErrorCode::SESSION_SUPERSEDED),
            None => None,
        }
    }

    /// The current configuration (for constructing derived machines in tests).
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// The latest playback diagnostics this Client reported (§9).
    pub const fn last_playback_diagnostics(&self) -> Option<&crate::schema::PlaybackDiagnostics> {
        self.last_diagnostics.as_ref()
    }

    /// Refresh capture observations before a control request. This does not
    /// change session/subscription state or start/recover capture.
    pub fn update_capture(&mut self, capture: CaptureSnapshot) {
        self.config.capture = capture;
    }

    /// Process one event; returns the ordered outcomes.
    pub fn handle(&mut self, event: Event) -> Vec<Outcome> {
        // CLOSED absorbs everything (§5.4).
        if self.state == ServerState::Closed {
            return Vec::new();
        }
        // CLOSING emits no further frames (§5.2): repeated local events
        // (revocation polls, supersede notices, fatals) must not duplicate the
        // close pair the machine already emitted.
        if self.state == ServerState::Closing {
            if matches!(event, Event::TransportLost) {
                self.close(ServerState::Closed);
            }
            return Vec::new();
        }
        match event {
            Event::Frame { frame, source } => self.handle_frame(frame, source),
            Event::AuthDeadline => {
                // §5.2 CONNECTING/AUTHENTICATING: auth deadline (10 s) →
                // transport close carrying 601. A pairing exchange uses the
                // 10-minute pairing deadline instead (§5.2 AUTHENTICATING).
                if self.is_pairing_exchange() {
                    Vec::new()
                } else if matches!(
                    self.state,
                    ServerState::Connecting | ServerState::Authenticating
                ) {
                    self.close(ServerState::Closed);
                    vec![Outcome::transport_close(ErrorCode::NETWORK_FAILURE)]
                } else {
                    Vec::new()
                }
            }
            Event::PairingDeadline => self.handle_pairing_deadline(),
            Event::PairingApproved { approval_id } => self.handle_pairing_approved(approval_id),
            Event::PairingDeclined { approval_id } => self.handle_pairing_declined(approval_id),
            Event::IdleTimeout => {
                // §5.2 "any": idle timeout → transport close carrying 601.
                self.close(ServerState::Closed);
                vec![Outcome::transport_close(ErrorCode::NETWORK_FAILURE)]
            }
            Event::TransportLost => {
                // §5.2 "any": transport lost → classified 601 locally, CLOSED.
                self.close(ServerState::Closed);
                Vec::new()
            }
            Event::CloseReceived => {
                // §5.2 READY/PLAYING/any: Close received → no echo, CLOSING.
                self.close(ServerState::Closing);
                Vec::new()
            }
            Event::Fatal(code) => {
                // §5.2 "any": fatal error → matching Error + Close; stop
                // processing requests.
                let outcomes = Outcome::fatal(code, 0);
                self.close(ServerState::Closing);
                outcomes
            }
            Event::RevocationCheck => self.handle_revocation_check(),
            Event::StreamLifecycle(lifecycle) => self.handle_stream_lifecycle(lifecycle),
            Event::Superseded { code } => {
                // §5.2: the superseded session emits Close(code) and the
                // transport closes carrying the code (TM-3 fixed replacement).
                let outcome = Outcome {
                    frame: Some(Frame {
                        body: Some(frame::Body::Close(crate::schema::Close {
                            code: code.code,
                            detail: String::new(),
                        })),
                    }),
                    close_code: Some(code),
                    transport_close_code: Some(code),
                    paired_client: None,
                };
                self.close(ServerState::Closed);
                vec![outcome]
            }
        }
    }

    fn close(&mut self, new_state: ServerState) {
        // CLOSING and CLOSED are absorbing except CLOSING → CLOSED (§5.4).
        if self.state.is_closing_state() && new_state == ServerState::Closing {
            return;
        }
        // Every terminal path (deadline, Close, transport loss, fatal,
        // supersede, idle timeout) drops the operator-facing card so a dead
        // exchange cannot linger in the GUI.
        self.abandon_pending_pairing();
        self.state = new_state;
    }

    /// Drop the pending pairing and remove any operator-facing approval entry.
    ///
    /// `request_id` is per-connection and not globally unique; the approval
    /// map is keyed by the process-local `approval_id`.
    fn abandon_pending_pairing(&mut self) {
        let Some(pending) = self.pending_pairing.take() else {
            return;
        };
        if let (Some(core), Some(approval_id)) = (self.config.pairing.as_ref(), pending.approval_id)
        {
            core.cancel_approval(approval_id);
        }
    }

    fn handle_frame(&mut self, frame: Box<Frame>, source: Vec<u8>) -> Vec<Outcome> {
        // CLOSING accepts no new requests (§5.2).
        if self.state == ServerState::Closing {
            return Vec::new();
        }
        let request_id = frame_request_id(&frame);
        // §4.2 (spec:89): request_id is sender-assigned, unique per sender per
        // connection, monotonically increasing from 1; the receiver echoes it
        // verbatim and never interprets it (opaque). The Server therefore does
        // not validate inbound monotonicity/uniqueness — it only echoes.
        match frame.body {
            Some(frame::Body::Close(close)) => {
                if close.code == 0 {
                    // §4.4 validation table: `Close` with code == 0 is
                    // malformed (spec:264/:419), not a one-way close → 401.
                    return self.malformed_fatal(0);
                }
                // Close is one-way and never echoed (§5.4 invariant 4).
                self.close(ServerState::Closing);
                Vec::new()
            }
            Some(frame::Body::Error(error)) if error.status.code == 0 => {
                // §4.4 (spec:264): an Error with status.code == 0 is malformed
                // in EVERY state (not a state error) → fatal 401.
                self.malformed_fatal(error.request_id)
            }
            Some(frame::Body::Error(_)) => {
                // §5.2 "any": non-fatal Error (no paired Close) is advisory.
                Vec::new()
            }
            other => self.handle_request(other, request_id, source),
        }
    }

    fn handle_request(
        &mut self,
        body: Option<frame::Body>,
        request_id: u64,
        source: Vec<u8>,
    ) -> Vec<Outcome> {
        match (&self.state, body) {
            // -----------------------------------------------------------------
            // CONNECTING rows.
            // -----------------------------------------------------------------
            (ServerState::Connecting, Some(frame::Body::AuthInit(init))) => {
                self.handle_auth_init(init.request_id, &init.client_public_key, source)
            }
            (ServerState::Connecting, Some(frame::Body::PairingRequest(request))) => {
                // §4.4 decidability first: a malformed capabilities list
                // (duplicates, non-ascending, or > 255 entries) is rejected
                // 401 BEFORE pairing admission semantics are consulted (the
                // fixture cases error-401-pairing-capabilities-over-255 /
                // error-401-pairing-duplicate-capabilities pin this order).
                // A malformed request never burns the nonce.
                if auth::validate_capability_list_bounded(&request.capabilities, u8::MAX as usize)
                    .is_err()
                {
                    self.last_pairing_failure_code = Some(ErrorCode::MALFORMED_MESSAGE);
                    return self.malformed_fatal(request.request_id);
                }
                self.handle_pairing_request(request, &source)
            }
            (ServerState::Connecting, Some(frame::Body::Negotiate(_))) => {
                // Valid but state-forbidden → 402 (§5.2 CONNECTING split).
                self.invalid_state_fatal(request_id)
            }
            (ServerState::Connecting, Some(frame::Body::Subscribe(_)))
            | (ServerState::Connecting, Some(frame::Body::Unsubscribe(_)))
            | (ServerState::Connecting, Some(frame::Body::RequestConfiguration(_)))
            | (ServerState::Connecting, Some(frame::Body::PlaybackDiagnostics(_))) => {
                self.invalid_state_fatal(request_id)
            }
            (ServerState::Connecting, Some(frame::Body::AuthChallenge(_)))
            | (ServerState::Connecting, Some(frame::Body::AuthResult(_)))
            | (ServerState::Connecting, Some(frame::Body::NegotiateResponse(_)))
            | (ServerState::Connecting, Some(frame::Body::SubscribeResponse(_)))
            | (ServerState::Connecting, Some(frame::Body::UnsubscribeResponse(_)))
            | (ServerState::Connecting, Some(frame::Body::ConfigurationPublished(_)))
            | (ServerState::Connecting, Some(frame::Body::StreamStarted(_)))
            | (ServerState::Connecting, Some(frame::Body::StreamEnded(_)))
            | (ServerState::Connecting, Some(frame::Body::PairingResponse(_)))
            | (ServerState::Connecting, Some(frame::Body::PairingConfirm(_)))
            | (ServerState::Connecting, Some(frame::Body::PairingResult(_)))
            | (ServerState::Connecting, None) => {
                // Server→Client messages arriving at the Server, or a frame
                // with no body (already rejected by framing, but stay total):
                // malformed → 401 (§5.2 CONNECTING generic row).
                self.malformed_fatal(request_id)
            }
            (ServerState::Connecting, Some(frame::Body::AuthProof(proof))) => {
                // §5.2: the auth exchange needs a pending challenge. A stray
                // AuthProof in CONNECTING is valid-but-forbidden → 402.
                self.invalid_state_fatal(proof.request_id)
            }

            // -----------------------------------------------------------------
            // AUTHENTICATING rows.
            // -----------------------------------------------------------------
            (ServerState::Authenticating, Some(frame::Body::AuthProof(proof))) => {
                self.handle_auth_proof(proof.request_id, &proof.signature)
            }
            (ServerState::Authenticating, Some(frame::Body::AuthInit(init))) => {
                // §5.2 AUTHENTICATING AuthInit (retry): fresh challenge.
                self.handle_auth_init(init.request_id, &init.client_public_key, source)
            }
            (ServerState::Authenticating, Some(frame::Body::PairingConfirm(confirm))) => {
                self.handle_pairing_confirm(confirm, &source)
            }
            (ServerState::Authenticating, Some(frame::Body::PairingRequest(_))) => {
                // A second PairingRequest in one connection is not the pairing
                // exchange's confirm step; keep the pre-ticket-19 201+Close(201)
                // refusal (no second nonce is admitted on this connection).
                self.pairing_rejected(request_id)
            }
            (ServerState::Authenticating, Some(_)) => {
                // Valid but state-forbidden in AUTHENTICATING → 402.
                self.invalid_state_fatal(request_id)
            }
            (ServerState::Authenticating, None) => self.malformed_fatal(0),

            // -----------------------------------------------------------------
            // NEGOTIATING rows.
            // -----------------------------------------------------------------
            (ServerState::Negotiating, Some(frame::Body::Negotiate(negotiate))) => {
                self.handle_negotiate(negotiate)
            }
            (ServerState::Negotiating, Some(_)) => {
                // §5.2 NEGOTIATING "any other Frame" → 402 + Close(402).
                self.invalid_state_fatal(request_id)
            }
            (ServerState::Negotiating, None) => self.malformed_fatal(0),

            // -----------------------------------------------------------------
            // READY rows.
            // -----------------------------------------------------------------
            (ServerState::Ready, Some(frame::Body::Subscribe(subscribe))) => {
                self.handle_subscribe(subscribe.request_id)
            }
            (ServerState::Ready, Some(frame::Body::Unsubscribe(unsubscribe))) => {
                // §5.2: Unsubscribe → OK, idempotent (duplicate/late included),
                // and READY returns to the unsubscribed state so a later
                // Subscribe is accepted rather than refused as a duplicate.
                self.subscribed = false;
                let response = crate::schema::UnsubscribeResponse {
                    request_id: unsubscribe.request_id,
                    status: Status::ok(),
                };
                vec![Outcome::frame(Frame {
                    body: Some(frame::Body::UnsubscribeResponse(response)),
                })]
            }
            (ServerState::Ready, Some(frame::Body::RequestConfiguration(request))) => {
                // §5.2: ConfigurationPublished with the request echoed.
                let push = crate::schema::ConfigurationPublished {
                    request_id: request.request_id,
                    configuration: self.config.capture.configuration.clone(),
                };
                vec![Outcome::frame(Frame {
                    body: Some(frame::Body::ConfigurationPublished(push)),
                })]
            }
            (ServerState::Ready, Some(frame::Body::PlaybackDiagnostics(diagnostics))) => {
                // §9: diagnostics ride the control stream; they are the only
                // playback evidence. Accepted silently (no reply).
                self.last_diagnostics = Some(diagnostics.clone());
                Vec::new()
            }
            (ServerState::Ready, Some(_)) => {
                // §5.2 READY: the enumerated rows are Subscribe/Unsubscribe/
                // RequestConfiguration/Close/diagnostics; anything else
                // (AuthInit, Negotiate, pairing) is state-forbidden → 402.
                self.invalid_state_fatal(request_id)
            }
            (ServerState::Ready, None) => self.malformed_fatal(0),

            // -----------------------------------------------------------------
            // PLAYING rows (§5.2). Only reachable while a stream instance is
            // active for a subscribed session.
            // -----------------------------------------------------------------
            (ServerState::Playing, Some(frame::Body::Subscribe(subscribe))) => {
                // §5.2 PLAYING: duplicate Subscribe → non-fatal 402, kept.
                vec![Outcome::frame(Frame {
                    body: Some(frame::Body::Error(crate::schema::Error {
                        request_id: subscribe.request_id,
                        status: Status::registry(ErrorCode::INVALID_STATE.code),
                    })),
                })]
            }
            (ServerState::Playing, Some(frame::Body::Unsubscribe(unsubscribe))) => {
                // §5.2 PLAYING: push StreamEnded(502, "unsubscribed") FIRST,
                // then UnsubscribeResponse(OK). The instance id is the one
                // this session is playing.
                self.subscribed = false;
                self.state = ServerState::Ready;
                let instance = self.config.capture.stream_instance_id;
                vec![
                    Outcome::frame(Frame {
                        body: Some(frame::Body::StreamEnded(crate::schema::StreamEnded {
                            stream_instance_id: instance,
                            code: ErrorCode::STREAM_ENDED.code,
                            detail: "unsubscribed".to_owned(),
                        })),
                    }),
                    Outcome::frame(Frame {
                        body: Some(frame::Body::UnsubscribeResponse(
                            crate::schema::UnsubscribeResponse {
                                request_id: unsubscribe.request_id,
                                status: Status::ok(),
                            },
                        )),
                    }),
                ]
            }
            (ServerState::Playing, Some(frame::Body::RequestConfiguration(request))) => {
                let push = crate::schema::ConfigurationPublished {
                    request_id: request.request_id,
                    configuration: self.config.capture.configuration.clone(),
                };
                vec![Outcome::frame(Frame {
                    body: Some(frame::Body::ConfigurationPublished(push)),
                })]
            }
            (ServerState::Playing, Some(frame::Body::PlaybackDiagnostics(diagnostics))) => {
                // §9: recorded as playback evidence; no reply, never fatal.
                self.last_diagnostics = Some(diagnostics.clone());
                Vec::new()
            }
            (ServerState::Playing, Some(_)) => self.invalid_state_fatal(request_id),
            (ServerState::Playing, None) => self.malformed_fatal(0),

            // -----------------------------------------------------------------
            // CLOSING / CLOSED: unreachable through handle_frame (it returns
            // early for CLOSING and CLOSE/ERROR bodies never reach here), but
            // the match must stay total.
            // -----------------------------------------------------------------
            (_, Some(frame::Body::Close(_))) | (_, Some(frame::Body::Error(_))) => {
                // §4.4 validation table: `Error` with status.code == 0 or
                // `Close` with code == 0 is malformed → 401. Non-zero codes
                // never reach here (handle_frame handled them).
                self.malformed_fatal(request_id)
            }
            (ServerState::Closing, _) | (ServerState::Closed, _) => Vec::new(),
        }
    }

    // -------------------------------------------------------------------------
    // Authentication (§5.2 AUTHENTICATING band; pairing.md §9.1).
    // -------------------------------------------------------------------------

    fn handle_auth_init(
        &mut self,
        request_id: u64,
        client_public_key: &[u8],
        _source: Vec<u8>,
    ) -> Vec<Outcome> {
        // §4.4 validation: the public key must be a 33-byte compressed P-256
        // point; anything else is a generic validation failure → 401.
        if auth::parse_client_public_key(client_public_key).is_err() {
            self.last_auth_failure_code = Some(ErrorCode::MALFORMED_MESSAGE);
            return self.malformed_fatal(request_id);
        }
        self.pending_client_key = Some(client_public_key.to_vec());
        // TM-4: an address already over its sliding window is closed 302
        // before another challenge is minted.
        // (The per-connection failure counter is the first line; the source
        // guard is owned by the accept loop, which consults the same
        // AddressRateGuard; the machine records this connection's failures.)
        let server_random = self.config.fixed_server_random().unwrap_or_else(|| {
            let mut bytes = vec![0u8; 32];
            getrandom(&mut bytes);
            bytes
        });
        self.pending_challenge = Some(server_random.clone());
        self.pending_request_id = Some(request_id);
        self.state = ServerState::Authenticating;
        vec![Outcome::frame(Frame {
            body: Some(frame::Body::AuthChallenge(crate::schema::AuthChallenge {
                request_id,
                server_random,
            })),
        })]
    }

    fn handle_auth_proof(&mut self, request_id: u64, signature: &[u8]) -> Vec<Outcome> {
        let Some(server_random) = self.pending_challenge.clone() else {
            return self.invalid_state_fatal(request_id);
        };
        let Some(client_public_key) = self.pending_client_key.clone() else {
            return self.invalid_state_fatal(request_id);
        };
        // The exchange echoes one request_id; a mismatched proof id is
        // answered with the illegal-request code.
        let expected_id = self.pending_request_id.unwrap_or(0);
        if request_id != expected_id {
            return self.invalid_state_fatal(request_id);
        }

        // §9.1: auth_context = TLV(1,"auth") || TLV(2,server_random) ||
        // TLV(3,client_pubkey) || TLV(4,server_pubkey) || TLV(5,major) ||
        // TLV(6,exporter). The exporter component is connection-scoped TLS
        // material — the driver injects it (test harnesses inject the fixture
        // constant); until wired, this machine consults the configured
        // stand-in so the TLV structure is identical in test and production.
        let context = auth::build_auth_context(
            &server_random,
            &client_public_key,
            &self.config.server_public_key,
            self.config.auth_exporter.as_deref().unwrap_or(&[]),
        );

        // PR-9: one ECDSA-P256-SHA256 over the RAW context bytes.
        let signature_ok = auth::verify_auth_proof(&client_public_key, &context, signature).is_ok();

        if !signature_ok {
            // §5.2: signature invalid → AuthResult(203); count failure; the
            // 5th closes with Close(203).
            return self.record_auth_failure(ErrorCode::AUTHENTICATION_FAILED, request_id);
        }

        // Signature valid: consult the authorization state (§9.2 — at every
        // authentication, never a cached answer).
        match self.store.authorize(&client_public_key) {
            Authorization::Authorized(_) => {
                // §8.1 last_seen_at, best effort: an auth success never fails
                // because an auxiliary timestamp write failed.
                let _ = self
                    .store
                    .touch_last_seen(&client_public_key, &pairing::rfc3339_now());
                self.pending_challenge = None;
                self.pending_request_id = None;
                let (session_id, supersede) = self.establish_session(&client_public_key);
                let _ = supersede;
                vec![Outcome::frame(Frame {
                    body: Some(frame::Body::AuthResult(crate::schema::AuthResult {
                        request_id,
                        status: Status::ok(),
                        session_id,
                    })),
                })]
            }
            Authorization::Unknown(_) => {
                // §5.2: AuthResult(202); count failure; the Client may
                // re-AuthInit with another stored pairing.
                self.record_auth_failure(ErrorCode::AUTHORIZATION_UNKNOWN, request_id)
            }
            Authorization::Revoked(_) | Authorization::Unavailable => {
                // §5.2: AuthResult(204); count failure. `Unavailable` is a
                // corrupt store: fail closed rather than keep serving the
                // last known-good table (pairing.md §10.3).
                self.record_auth_failure(ErrorCode::AUTHORIZATION_DENIED, request_id)
            }
        }
    }

    /// The §6 code of the most recent authentication failure, for the
    /// driver's failure log (202 unknown, 203 bad proof, 204 revoked, 401
    /// malformed). Consumed once so one failure produces one log line.
    pub fn take_auth_failure_code(&mut self) -> Option<ErrorCode> {
        self.last_auth_failure_code.take()
    }

    /// The §6 code of the most recent pairing failure, for the driver's
    /// redacted log (201 rejection, 302 rate limit, 401 malformed). One
    /// failure produces one log line.
    pub fn take_pairing_failure_code(&mut self) -> Option<ErrorCode> {
        self.last_pairing_failure_code.take()
    }

    /// Whether this session is inside a pairing exchange (CONNECTING or
    /// AUTHENTICATING with an admitted pairing). The driver uses this to
    /// replace the 10 s auth deadline with the 10 min pairing deadline.
    pub fn is_pairing_exchange(&self) -> bool {
        self.pending_pairing.is_some()
            && matches!(
                self.state,
                ServerState::Connecting | ServerState::Authenticating
            )
    }

    /// The pending pairing exchange's process-local approval handle, when the
    /// exchange has been surfaced to the local operator.
    pub fn pending_pairing_approval_id(&self) -> Option<u64> {
        self.pending_pairing
            .as_ref()
            .and_then(|pending| pending.approval_id)
    }

    /// Hand the live pairing (mode, SAS, identity, address, expiry) to the
    /// driver exactly once, before the Client's `PairingConfirm` (ticket 29).
    ///
    /// The driver publishes it only after `PairingResponse` has been written;
    /// this view changes no wire or approval semantics.
    pub fn take_pairing_presented(&mut self) -> Option<PairingPresented> {
        self.pending_pairing
            .as_mut()
            .and_then(|pending| pending.presented.take())
    }

    /// Hand a verified-but-unapproved pairing to the local operator exactly
    /// once. The driver registers an approval callback and never blocks the
    /// state machine on human input.
    pub fn take_pairing_approval_request(&mut self) -> Option<ApprovalRequest> {
        let core = self.config.pairing.as_ref()?;
        let pending = self.pending_pairing.as_mut()?;
        if !pending.verified || pending.approval_requested {
            return None;
        }
        pending.approval_requested = true;
        let approval_id = pending
            .approval_id
            .unwrap_or_else(|| core.next_approval_id());
        pending.approval_id = Some(approval_id);
        let client_id: String = auth::client_id(&pending.client_public_key)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Some(ApprovalRequest {
            approval_id,
            client_id,
            display_name: pending.display_name.clone(),
            mode: pending.mode,
            address: String::new(),
            sas: pending.sas.clone(),
        })
    }

    fn record_auth_failure(&mut self, code: ErrorCode, request_id: u64) -> Vec<Outcome> {
        self.auth_failures += 1;
        self.last_auth_failure_code = Some(code);
        self.pending_challenge = None;
        self.pending_request_id = None;
        self.pending_client_key = None;
        if self.auth_failures >= self.config.max_auth_failures_per_connection {
            // §5.2/TM-4: the 5th failure closes with Close(203). The row names
            // Close alone (the 202/203/204 AuthResults already reported).
            self.close(ServerState::Closing);
            vec![Outcome {
                frame: Some(Frame {
                    body: Some(frame::Body::Close(crate::schema::Close {
                        code: ErrorCode::AUTHENTICATION_FAILED.code,
                        detail: String::new(),
                    })),
                }),
                close_code: Some(ErrorCode::AUTHENTICATION_FAILED),
                transport_close_code: None,
                paired_client: None,
            }]
        } else {
            // Non-fatal: the failure is reported in the exchange's response
            // (AuthResult with the code); no Close rides along (§5.2: the
            // Client may re-AuthInit with another stored pairing).
            vec![Outcome::frame(Frame {
                body: Some(frame::Body::AuthResult(crate::schema::AuthResult {
                    request_id,
                    status: Status::registry(code.code),
                    session_id: 0,
                })),
            })]
        }
    }

    // -------------------------------------------------------------------------
    // Pairing exchange (spec §5.2 pairing rows; pairing.md §6–§7).
    // -------------------------------------------------------------------------

    /// Enter NEGOTIATING exactly like a successful `AuthProof` (§5.2), so the
    /// driver's registry, revocation polling, and 303 supersede all apply to
    /// a freshly paired session.
    fn establish_session(&mut self, client_public_key: &[u8]) -> (u64, SupersedeRequest) {
        // Any in-flight pairing exchange is resolved by this session
        // establishment (a paired session reaches here through approval; an
        // auth retry that succeeded abandons an unverified pairing).
        self.abandon_pending_pairing();
        self.identity = Some(client_public_key.to_vec());
        self.state = ServerState::Negotiating;
        // TM-3: supersede an older session of the same identity.
        let session_id = self.next_session_id;
        self.next_session_id += 1;
        self.session_id = Some(session_id);
        let supersede = SupersedeRequest {
            old_session_id: session_id.saturating_sub(1),
            code: ErrorCode::SESSION_SUPERSEDED,
        };
        self.supersede = Some(supersede.clone());
        (session_id, supersede)
    }

    fn handle_pairing_request(
        &mut self,
        request: crate::schema::PairingRequest,
        source: &[u8],
    ) -> Vec<Outcome> {
        let Some(core) = self.config.pairing.clone() else {
            // No production Pairing Core wired: preserve the pre-ticket-19
            // refusal (201 + Close(201), no detail).
            self.last_pairing_failure_code = Some(ErrorCode::PAIRING_REJECTED);
            return self.pairing_rejected(request.request_id);
        };
        let address = parse_source_address(source);
        if address.is_some_and(|address| core.is_limited(address)) {
            self.last_pairing_failure_code = Some(ErrorCode::RATE_LIMITED);
            return self.pairing_rate_limited();
        }

        // Structural validation: these are malformed requests (401) and must
        // not burn the nonce (the capability-list check already ran in the
        // CONNECTING arm).
        if auth::parse_client_public_key(&request.client_public_key).is_err()
            || request.client_random.len() != 32
            || request.client_name.len() > 63
        {
            self.last_pairing_failure_code = Some(ErrorCode::MALFORMED_MESSAGE);
            return self.malformed_fatal(request.request_id);
        }
        let mode = match PairingMode::try_from(request.pairing_mode) {
            Ok(PairingMode::Qr) => {
                if request.pairing_nonce.len() != pairing::NONCE_LEN {
                    self.last_pairing_failure_code = Some(ErrorCode::MALFORMED_MESSAGE);
                    return self.malformed_fatal(request.request_id);
                }
                PairingMode::Qr
            }
            Ok(PairingMode::Sas) => {
                if !request.pairing_nonce.is_empty() {
                    self.last_pairing_failure_code = Some(ErrorCode::MALFORMED_MESSAGE);
                    return self.malformed_fatal(request.request_id);
                }
                PairingMode::Sas
            }
            Ok(PairingMode::Unspecified) | Err(_) => {
                return self.record_pairing_rejection(request.request_id, &core, address);
            }
        };
        // Admission gate: a valid QR nonce is consumed here, before the major
        // check and any crypto, regardless of the eventual outcome (§5). The
        // permit holds one of the eight TM-5 concurrent-pairing slots.
        let permit = match core.admit_request(&request) {
            Ok(permit) => permit,
            Err(PairingError::ConcurrentLimit) => {
                // TM-5 pairing bound: over → 302, no PairingResult.
                self.last_pairing_failure_code = Some(ErrorCode::RATE_LIMITED);
                return self.pairing_rate_limited();
            }
            Err(_) => return self.record_pairing_rejection(request.request_id, &core, address),
        };
        if request.protocol_major != crate::codes::PROTOCOL_MAJOR {
            return self.record_pairing_rejection(request.request_id, &core, address);
        }

        let server_random = self
            .config
            .fixed_pairing_server_random()
            .filter(|random| random.len() == 32)
            .unwrap_or_else(random_32_bytes);
        let context = match pairing::binding_context(
            mode,
            &request.pairing_nonce,
            &request.client_random,
            &server_random,
            &request.client_public_key,
            &self.config.server_public_key,
            &request.capabilities,
            &pairing::normalize_display_name(&request.client_name),
        ) {
            Ok(context) => context,
            Err(_) => {
                self.last_pairing_failure_code = Some(ErrorCode::MALFORMED_MESSAGE);
                return self.malformed_fatal(request.request_id);
            }
        };
        let Some(exporter) = self.config.pairing_exporter.clone() else {
            return self.internal_error(request.request_id);
        };
        let Some(binding_bytes) =
            exporter.export(PAIRING_EXPORTER_LABEL, &context, pairing::BINDING_LEN)
        else {
            return self.internal_error(request.request_id);
        };
        let Ok(binding) = <[u8; pairing::BINDING_LEN]>::try_from(binding_bytes.as_slice()) else {
            return self.internal_error(request.request_id);
        };
        let sas = exporter
            .export(SAS_EXPORTER_LABEL, &context, pairing::SAS_SEED_LEN)
            .and_then(|seed| pairing::derive_sas_code(&seed))
            .map(pairing::format_sas);

        let display_name = pairing::normalize_display_name(&request.client_name);
        let client_id = auth::client_id(&request.client_public_key).to_vec();
        // The live operator view is handed to the driver now, but the driver
        // publishes it only after PairingResponse is written (ticket 29). It
        // shares the approval handle used later by the post-confirm card.
        let approval_id = core.next_approval_id();
        let presented = PairingPresented {
            approval_id,
            mode,
            client_id: client_id.iter().map(|byte| format!("{byte:02x}")).collect(),
            display_name: display_name.clone(),
            address: String::from_utf8_lossy(source).into_owned(),
            sas: sas.clone(),
            fingerprint: core.fingerprint(),
            expires_at: Instant::now() + pairing::PAIRING_DEADLINE,
        };
        self.pending_pairing = Some(PendingPairing {
            request_id: request.request_id,
            approval_id: Some(approval_id),
            mode,
            client_public_key: request.client_public_key.clone(),
            display_name: display_name.clone(),
            binding,
            sas,
            verified: false,
            approval_requested: false,
            presented: Some(presented),
            _permit: permit,
        });
        self.state = ServerState::Authenticating;
        vec![Outcome::frame(Frame {
            body: Some(frame::Body::PairingResponse(
                crate::schema::PairingResponse {
                    request_id: request.request_id,
                    status: Status::ok(),
                    pairing_mode: mode as i32,
                    pairing_nonce: request.pairing_nonce.clone(),
                    client_random: request.client_random.clone(),
                    server_random,
                    server_public_key: self.config.server_public_key.clone(),
                    protocol_major: crate::codes::PROTOCOL_MAJOR,
                    capabilities: request.capabilities.clone(),
                    binding: binding.to_vec(),
                    client_id,
                    display_name,
                },
            )),
        })]
    }

    fn handle_pairing_confirm(
        &mut self,
        confirm: crate::schema::PairingConfirm,
        source: &[u8],
    ) -> Vec<Outcome> {
        let Some(core) = self.config.pairing.clone() else {
            return self.pairing_rejected(confirm.request_id);
        };
        let address = parse_source_address(source);
        let Some(pending) = self.pending_pairing.as_ref() else {
            return self.record_pairing_rejection(confirm.request_id, &core, address);
        };
        if confirm.request_id != pending.request_id {
            return self.record_pairing_rejection(confirm.request_id, &core, address);
        }
        let client_public_key = pending.client_public_key.clone();
        let binding = pending.binding;
        // The binding echo must equal this connection's exporter derivation;
        // the proof must prove possession of the *claimed identity key*
        // (pairing.md §6.3.1). Both are checked before persistence.
        let binding_ok =
            confirm.binding_echo.len() == binding.len() && confirm.binding_echo == binding;
        let proof_ok =
            auth::verify_pairing_proof(&client_public_key, &binding, &confirm.client_proof).is_ok();
        if !binding_ok || !proof_ok {
            return self.record_pairing_rejection(confirm.request_id, &core, address);
        }
        if let Some(pending) = self.pending_pairing.as_mut() {
            pending.verified = true;
        }
        // Persistence and PairingResult wait for Event::PairingApproved.
        Vec::new()
    }

    fn handle_pairing_approved(&mut self, approval_id: u64) -> Vec<Outcome> {
        let Some(pending) = self.pending_pairing.as_ref() else {
            return Vec::new();
        };
        if !pending.verified || pending.approval_id != Some(approval_id) {
            return Vec::new();
        }
        let request_id = pending.request_id;
        let client_public_key = pending.client_public_key.clone();
        let display_name = pending.display_name.clone();
        let Some(core) = self.config.pairing.clone() else {
            return self.internal_error(request_id);
        };
        let client = match core.persist_client(&client_public_key, &display_name) {
            Ok(client) => client,
            Err(error) => {
                // §10.3: a store write failure fails closed, never a silent
                // success. The detail is local only; the wire Error carries none.
                tracing::error!(error = %error, "v2 pairing: persist failed; failing closed");
                return self.internal_error(request_id);
            }
        };
        self.abandon_pending_pairing();
        self.establish_session(&client_public_key);
        let mut outcome = Outcome::frame(Frame {
            body: Some(frame::Body::PairingResult(crate::schema::PairingResult {
                request_id,
                status: Status::ok(),
            })),
        });
        // The driver broadcasts this only after the frame above is written.
        outcome.paired_client = Some(client);
        vec![outcome]
    }

    fn handle_pairing_declined(&mut self, approval_id: u64) -> Vec<Outcome> {
        let Some(pending) = self.pending_pairing.as_ref() else {
            return Vec::new();
        };
        if !pending.verified || pending.approval_id != Some(approval_id) {
            return Vec::new();
        }
        let request_id = pending.request_id;
        // §5.2: verified proofs + declined approval → 201 + Close(201), no
        // detail, nothing persisted.
        self.abandon_pending_pairing();
        self.pairing_rejected(request_id)
    }

    fn handle_pairing_deadline(&mut self) -> Vec<Outcome> {
        if self.state != ServerState::Authenticating {
            return Vec::new();
        }
        let Some(pending) = self.pending_pairing.take() else {
            return Vec::new();
        };
        if let (Some(core), Some(approval_id)) = (self.config.pairing.as_ref(), pending.approval_id)
        {
            core.cancel_approval(approval_id);
        }
        // §5.2: 10 min without a pairing result → Error(206)+Close(206).
        self.last_pairing_failure_code = Some(ErrorCode::PAIRING_PENDING_TIMEOUT);
        let outcomes = Outcome::fatal(ErrorCode::PAIRING_PENDING_TIMEOUT, pending.request_id);
        self.close(ServerState::Closing);
        outcomes
    }

    /// Count one pairing failure against the connection's source; a source
    /// over its TM-5 budget is answered Close(302) instead of 201.
    fn record_pairing_rejection(
        &mut self,
        request_id: u64,
        core: &PairingCore,
        address: Option<std::net::IpAddr>,
    ) -> Vec<Outcome> {
        self.last_pairing_failure_code = Some(ErrorCode::PAIRING_REJECTED);
        if address.is_some_and(|address| core.note_failure(address)) {
            self.last_pairing_failure_code = Some(ErrorCode::RATE_LIMITED);
            return self.pairing_rate_limited();
        }
        self.pairing_rejected(request_id)
    }

    fn pairing_rate_limited(&mut self) -> Vec<Outcome> {
        self.close(ServerState::Closing);
        vec![Outcome {
            frame: Some(Frame {
                body: Some(frame::Body::Close(crate::schema::Close {
                    code: ErrorCode::RATE_LIMITED.code,
                    detail: String::new(),
                })),
            }),
            close_code: Some(ErrorCode::RATE_LIMITED),
            transport_close_code: None,
            paired_client: None,
        }]
    }

    fn internal_error(&mut self, request_id: u64) -> Vec<Outcome> {
        let outcomes = Outcome::fatal(ErrorCode::INTERNAL_ERROR, request_id);
        self.close(ServerState::Closing);
        outcomes
    }

    // -------------------------------------------------------------------------
    // Capture-driven stream lifecycle (§5.2 READY/PLAYING capture rows).
    // -------------------------------------------------------------------------

    /// Turn one capture transition into the ordered pushes a subscribed
    /// Client receives. Media rows only apply after negotiation; before READY
    /// the machine has no Client to notify and returns nothing.
    fn handle_stream_lifecycle(&mut self, lifecycle: StreamLifecycle) -> Vec<Outcome> {
        if !matches!(self.state, ServerState::Ready | ServerState::Playing) {
            return Vec::new();
        }
        match lifecycle {
            StreamLifecycle::Started {
                stream_instance_id,
                config_generation,
                initial_timestamp,
            } => {
                // §5.2 READY "capture started | subscribed → push StreamStarted
                // | PLAYING"; "not subscribed → no push | READY".
                if !self.subscribed {
                    return Vec::new();
                }
                self.state = ServerState::Playing;
                vec![Outcome::frame(Frame {
                    body: Some(frame::Body::StreamStarted(crate::schema::StreamStarted {
                        stream_instance_id,
                        config_generation,
                        initial_timestamp,
                    })),
                })]
            }
            StreamLifecycle::Ended {
                stream_instance_id,
                code,
            } => {
                // §5.2 PLAYING: StreamEnded(502 normal / 501 failure) → READY.
                // The subscription survives a capture end; only an explicit
                // Unsubscribe clears it.
                if self.state != ServerState::Playing {
                    return Vec::new();
                }
                self.state = ServerState::Ready;
                vec![Outcome::frame(Frame {
                    body: Some(frame::Body::StreamEnded(crate::schema::StreamEnded {
                        stream_instance_id,
                        code: code.code,
                        detail: String::new(),
                    })),
                })]
            }
            StreamLifecycle::Restarted {
                old_stream_instance_id,
                stream_instance_id,
                config_generation,
                initial_timestamp,
            } => {
                // §5.2 PLAYING "capture restart (new instance)": StreamEnded(502)
                // then StreamStarted(new) — in that order.
                if self.state != ServerState::Playing {
                    return Vec::new();
                }
                vec![
                    Outcome::frame(Frame {
                        body: Some(frame::Body::StreamEnded(crate::schema::StreamEnded {
                            stream_instance_id: old_stream_instance_id,
                            code: ErrorCode::STREAM_ENDED.code,
                            detail: String::new(),
                        })),
                    }),
                    Outcome::frame(Frame {
                        body: Some(frame::Body::StreamStarted(crate::schema::StreamStarted {
                            stream_instance_id,
                            config_generation,
                            initial_timestamp,
                        })),
                    }),
                ]
            }
            StreamLifecycle::ConfigChanged { config_generation } => {
                // §5.2 PLAYING "decode-affecting config change" →
                // ConfigurationPublished(0). The driver captures the change
                // because the DATA plane must switch generation too (§7.6).
                if self.state != ServerState::Playing {
                    return Vec::new();
                }
                let Some(mut configuration) = self.config.capture.configuration.clone() else {
                    return Vec::new();
                };
                configuration.config_generation = config_generation;
                vec![Outcome::frame(Frame {
                    body: Some(frame::Body::ConfigurationPublished(
                        crate::schema::ConfigurationPublished {
                            request_id: 0,
                            configuration: Some(configuration),
                        },
                    )),
                })]
            }
        }
    }

    fn handle_revocation_check(&mut self) -> Vec<Outcome> {
        // pairing.md §9.2 / TM-9: on an authorization-state change, every
        // session authenticated by an affected identity closes within ≤ 5 s
        // with the fatal pair Error(204)+Close(204).
        let Some(identity) = self.identity.clone() else {
            return Vec::new();
        };
        // A record that disappeared (Unknown) is a revocation too, and a
        // corrupt reload (Unavailable) fails closed: both tear the session
        // down with the authorization-denied pair (pairing.md §9.2/§10.3).
        if matches!(
            self.store.authorize(&identity),
            Authorization::Revoked(_) | Authorization::Unknown(_) | Authorization::Unavailable
        ) {
            let outcomes = Outcome::fatal(ErrorCode::AUTHORIZATION_DENIED, 0);
            self.close(ServerState::Closing);
            outcomes
        } else {
            Vec::new()
        }
    }

    // -------------------------------------------------------------------------
    // Negotiation (§5.2 NEGOTIATING band; §4.5 capability rules).
    // -------------------------------------------------------------------------

    fn handle_negotiate(&mut self, negotiate: crate::schema::Negotiate) -> Vec<Outcome> {
        // §4.4 validation table: major ≠ 2 or minor > Server's → 101.
        if negotiate.protocol_major != crate::codes::PROTOCOL_MAJOR
            || negotiate.protocol_minor > self.config.protocol_minor
        {
            let outcomes = Outcome::fatal(ErrorCode::VERSION_MISMATCH, negotiate.request_id);
            self.close(ServerState::Closing);
            return outcomes;
        }
        // §4.4 validation: duplicated or not ascending → 401.
        if auth::validate_capability_list(&negotiate.capabilities).is_err() {
            let outcomes = Outcome::fatal(ErrorCode::MALFORMED_MESSAGE, negotiate.request_id);
            self.close(ServerState::Closing);
            return outcomes;
        }
        // §4.5: unknown ids are ignored; the honored intersection is empty in
        // v2.0 (no optional capabilities exist). NegotiateResponse is the only
        // session-authoritative set.
        let accepted: Vec<u32> = Vec::new();
        self.state = ServerState::Ready;
        let mut outcomes = vec![Outcome::frame(Frame {
            body: Some(frame::Body::NegotiateResponse(
                crate::schema::NegotiateResponse {
                    request_id: negotiate.request_id,
                    status: Status::ok(),
                    protocol_major: crate::codes::PROTOCOL_MAJOR,
                    protocol_minor: self.config.protocol_minor,
                    capabilities: accepted,
                },
            )),
        })];
        // §5.1 READY entry: push ConfigurationPublished if a Transfer Format
        // is current.
        if let Some(configuration) = self.config.capture.configuration.clone() {
            let push = crate::schema::ConfigurationPublished {
                request_id: 0,
                configuration: Some(configuration),
            };
            outcomes.push(Outcome::frame(Frame {
                body: Some(frame::Body::ConfigurationPublished(push)),
            }));
        }
        outcomes
    }

    // -------------------------------------------------------------------------
    // READY subscription rows.
    // -------------------------------------------------------------------------

    fn handle_subscribe(&mut self, request_id: u64) -> Vec<Outcome> {
        if self.subscribed {
            // §5.2 READY: duplicate Subscribe → non-fatal Error(402,
            // retryable=false); connection maintained; no state change.
            return vec![Outcome::frame(Frame {
                body: Some(frame::Body::Error(crate::schema::Error {
                    request_id,
                    status: Status::registry(ErrorCode::INVALID_STATE.code),
                })),
            })];
        }
        // Absent = no Transfer Format is current (capture never injected).
        if !self.config.capture.healthy || self.config.capture.configuration.is_none() {
            // §5.2: capture unhealthy or absent → Error(501, retryable=true);
            // no state change.
            return vec![Outcome::frame(Frame {
                body: Some(frame::Body::Error(crate::schema::Error {
                    request_id,
                    status: Status::registry(ErrorCode::CAPTURE_FAILURE.code),
                })),
            })];
        }
        self.subscribed = true;
        let mut outcomes = vec![Outcome::frame(Frame {
            body: Some(frame::Body::SubscribeResponse(
                crate::schema::SubscribeResponse {
                    request_id,
                    status: Status::ok(),
                    configuration: self.config.capture.configuration.clone(),
                },
            )),
        })];
        // §5.2 READY Subscribe: "if a stream is active, push StreamStarted
        // (active instance, current generation, next-frame timestamp)" and
        // the session enters PLAYING; otherwise it stays READY (subscribed)
        // and a later capture start pushes StreamStarted.
        if self.config.capture.stream_active {
            if let Some(configuration) = &self.config.capture.configuration {
                outcomes.push(Outcome::frame(Frame {
                    body: Some(frame::Body::StreamStarted(crate::schema::StreamStarted {
                        stream_instance_id: self.config.capture.stream_instance_id,
                        config_generation: configuration.config_generation,
                        initial_timestamp: self.config.capture.next_frame_timestamp,
                    })),
                }));
                self.state = ServerState::Playing;
            }
        }
        outcomes
    }

    // -------------------------------------------------------------------------
    // Shared refusal helpers.
    // -------------------------------------------------------------------------

    fn invalid_state_fatal(&mut self, request_id: u64) -> Vec<Outcome> {
        let outcomes = Outcome::fatal(ErrorCode::INVALID_STATE, request_id);
        self.close(ServerState::Closing);
        outcomes
    }

    fn malformed_fatal(&mut self, request_id: u64) -> Vec<Outcome> {
        let outcomes = Outcome::fatal(ErrorCode::MALFORMED_MESSAGE, request_id);
        self.close(ServerState::Closing);
        outcomes
    }

    fn pairing_rejected(&mut self, request_id: u64) -> Vec<Outcome> {
        // §5.2: PairingResult(201)+Close(201), no detail, nothing persisted,
        // then the transport close carries 201 after the grace (§5.1).
        let outcomes = vec![
            Outcome::frame(Frame {
                body: Some(frame::Body::PairingResult(crate::schema::PairingResult {
                    request_id,
                    status: Status::registry(ErrorCode::PAIRING_REJECTED.code),
                })),
            }),
            Outcome {
                frame: Some(Frame {
                    body: Some(frame::Body::Close(crate::schema::Close {
                        code: ErrorCode::PAIRING_REJECTED.code,
                        detail: String::new(),
                    })),
                }),
                close_code: Some(ErrorCode::PAIRING_REJECTED),
                transport_close_code: None,
                paired_client: None,
            },
        ];
        self.abandon_pending_pairing();
        self.close(ServerState::Closing);
        outcomes
    }
}

/// Whether CLOSING is in progress on a state value.
impl ServerState {
    /// CLOSING or later (CLOSING accepts no new requests, §5.2).
    pub const fn is_closing_state(self) -> bool {
        matches!(self, Self::Closing | Self::Closed)
    }
}

/// The request id a Client request body carries (0 = none/push). Used by the
/// driver's outstanding-request accounting (spec §10).
pub fn request_id_of(frame: &Frame) -> Option<u64> {
    let id = frame_request_id(frame);
    (id != 0).then_some(id)
}

fn frame_request_id(frame: &Frame) -> u64 {
    match &frame.body {
        Some(frame::Body::AuthInit(m)) => m.request_id,
        Some(frame::Body::AuthProof(m)) => m.request_id,
        Some(frame::Body::Negotiate(m)) => m.request_id,
        Some(frame::Body::Subscribe(m)) => m.request_id,
        Some(frame::Body::Unsubscribe(m)) => m.request_id,
        Some(frame::Body::RequestConfiguration(m)) => m.request_id,
        Some(frame::Body::PairingRequest(m)) => m.request_id,
        Some(frame::Body::PairingConfirm(m)) => m.request_id,
        _ => 0,
    }
}

/// Parse the driver's source-address bytes (it passes the IP's text form) for
/// the TM-5 per-address pairing-failure guard.
fn parse_source_address(source: &[u8]) -> Option<std::net::IpAddr> {
    std::str::from_utf8(source).ok()?.parse().ok()
}

/// A fresh 32-byte CSPRNG value (pairing `server_random`).
fn random_32_bytes() -> Vec<u8> {
    let mut bytes = vec![0u8; 32];
    getrandom(&mut bytes);
    bytes
}

/// Fill bytes from the OS CSPRNG.
fn getrandom(buffer: &mut [u8]) {
    use ring::rand::{SecureRandom, SystemRandom};
    SystemRandom::new()
        .fill(buffer)
        .expect("OS CSPRNG is available");
}
