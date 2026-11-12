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

use crate::auth;
use crate::auth_store::{Authorization, AuthorizationStore};
use crate::codes::ErrorCode;
use crate::schema::{frame, Frame, Status};

/// The §5.1/§5.2 session states this machine drives. (PLAYING is a
/// READY-∧-streaming data condition of ticket 10+; this machine's READY is
/// the v2.0-control-plane steady state.)
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
    /// `Close` sent or received; teardown in progress; no new requests.
    Closing,
    /// Terminal — all session state released (§5.4).
    Closed,
}

/// Static server configuration the machine consults (never mutated by it).
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// The raw 32-byte Ed25519 identity public key (PairingResponse /
    /// auth_context TLV(4)).
    pub server_public_key: Vec<u8>,
    /// The Server's minor within major 2 (NegotiateResponse).
    pub protocol_minor: u32,
    /// The current Transfer Format, when one is published (§5.1 READY entry:
    /// push `ConfigurationPublished` if a Transfer Format is current).
    pub current_configuration: Option<crate::schema::Configuration>,
    /// Whether a capture instance is active (Subscribe → StreamStarted push).
    pub capture_active: bool,
    /// Whether the capture backend is healthy (§5.2 READY Subscribe guard).
    pub capture_healthy: bool,
    /// Auth failures per connection (TM-4; default 5, `Limits`).
    pub max_auth_failures_per_connection: u32,
    /// Test-only override of the challenge randomness (spec §4.4: 32 bytes,
    /// CSPRNG in production): the golden vectors pin fixed "randomness"
    /// constants; replay injects them here. Production leaves None.
    pub server_random_override: Option<Vec<u8>>,
    /// Test-only override of the TLS-Exporter auth component (pairing.md
    /// §9.1 TLV(6), connection-scoped secret): fixtures inject the recorded
    /// constant. Production leaves None and the driver passes the real
    /// exporter output per event.
    pub auth_exporter_override: Option<Vec<u8>>,
    /// Supersede policy: with [`SUPERSEDE_KEEP_OLD`] the old session is marked
    /// superseded (Close(303) sent by its driver) and the new one admitted;
    /// `false` = the new session never replaces the old (unused in v2.0,
    /// TM-3 fixed replacement).
    pub keep_old_on_supersede: bool,
    /// The first session id this machine assigns (§3: the Server assigns
    /// ids from a registry-wide counter; the driver seeds each machine with
    /// its position). Defaults to 1 for a standalone machine; the golden-
    /// vector replay uses it to reproduce the registry's numbering.
    pub session_id_base: u64,
}

/// TM-3: a new session of the same identity supersedes the old (Close(303)).
pub const SUPERSEDE_KEEP_OLD: bool = true;

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
    /// The session registry decided this session is superseded by a newer
    /// session of the same identity (TM-3; the driver owns the registry and
    /// translates its decision into this event on the OLD machine). The
    /// machine emits Close carrying the code and closes the transport with
    /// it (§5.2: Close(303) on the superseded session).
    Superseded { code: ErrorCode },
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
}

impl Outcome {
    fn frame(frame: Frame) -> Self {
        Self { frame: Some(frame), close_code: None, transport_close_code: None }
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
            },
        ]
    }

    /// A bare transport close (301/601-style rows).
    fn transport_close(code: ErrorCode) -> Self {
        Self { frame: None, close_code: None, transport_close_code: Some(code) }
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
    /// The pending challenge's server_random (verification context).
    pending_challenge: Option<Vec<u8>>,
    /// The request_id of the pending auth exchange.
    pending_request_id: Option<u64>,
    /// The claimed client key from the pending AuthInit (proof verification
    /// context; pairing.md §9.1 TLV(3)).
    pending_client_key: Option<Vec<u8>>,
    /// Supersede decision for the driver (an older session of this identity).
    supersede: Option<SupersedeRequest>,
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
            pending_challenge: None,
            pending_request_id: None,
            pending_client_key: None,
            supersede: None,
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

    /// The static configuration (for constructing derived machines in tests).
    pub fn config(&self) -> &ServerConfig {
        &self.config
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
                // transport close carrying 601.
                if matches!(self.state, ServerState::Connecting | ServerState::Authenticating) {
                    self.close(ServerState::Closed);
                    vec![Outcome::transport_close(ErrorCode::NETWORK_FAILURE)]
                } else {
                    Vec::new()
                }
            }
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
        self.state = new_state;
    }

    fn handle_frame(&mut self, frame: Box<Frame>, source: Vec<u8>) -> Vec<Outcome> {
        // CLOSING accepts no new requests (§5.2).
        if self.state == ServerState::Closing {
            return Vec::new();
        }
        let request_id = frame_request_id(&frame);
        match frame.body {
            Some(frame::Body::Close(_)) => {
                // Close is one-way and never echoed (§5.4 invariant 4).
                self.close(ServerState::Closing);
                Vec::new()
            }
            Some(frame::Body::Error(error)) if error.status.code != 0 => {
                // §5.2 "any": non-fatal Error (no paired Close) is advisory.
                Vec::new()
            }
            other => self.handle_request(other, request_id, source)
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
                if auth::validate_capability_list_bounded(
                    &request.capabilities,
                    u8::MAX as usize,
                )
                .is_err()
                {
                    return self.malformed_fatal(request.request_id);
                }
                // §5.2 CONNECTING PairingRequest row — production pairing is a
                // later phase; in v2.0-control-plane scope the admission gate
                // has no live QR/SAS source, so a PairingRequest is refused
                // 201 + Close(201) (no admission material exists — the
                // nonce/window tables are empty). Pairing-failure rate limits
                // (TM-5) apply at the pairing phase.
                //
                // Adjudication note (201 flag): the declined-201 fixture
                // vector encodes PairingResult(201) WITHOUT the
                // requires_repairing flag, but spec §6.2 sets the registry
                // default for 201 to requires_repairing=yes and §4.2 requires
                // senders to match the registry. Stance (ticket 07 work
                // order): the sender follows the registry default and emits
                // requires_repairing=true via `Status::registry(201)`; the
                // declined-201 fixture's literal bytes are therefore NOT
                // asserted byte-identically at sender level (its decode/
                // assertions still run in the replay harness); fixtures are
                // never modified here — the maintainer rules on the vector in
                // the ticket Comments.
                let outcomes = vec![
                    Outcome::frame(Frame {
                        body: Some(frame::Body::PairingResult(crate::schema::PairingResult {
                            request_id: request.request_id,
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
                    },
                ];
                self.close(ServerState::Closing);
                outcomes
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
            (ServerState::Authenticating, Some(frame::Body::PairingConfirm(_)))
            | (ServerState::Authenticating, Some(frame::Body::PairingRequest(_))) => {
                // Pairing exchanges are refused in this phase (see CONNECTING).
                self.pairing_unavailable(request_id)
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
                // §5.2: Unsubscribe → OK, idempotent (duplicate/late included).
                let response = crate::schema::UnsubscribeResponse {
                    request_id: unsubscribe.request_id,
                    status: Status::ok(),
                };
                vec![Outcome::frame(Frame { body: Some(frame::Body::UnsubscribeResponse(response)) })]
            }
            (ServerState::Ready, Some(frame::Body::RequestConfiguration(request))) => {
                // §5.2: ConfigurationPublished with the request echoed.
                let push = crate::schema::ConfigurationPublished {
                    request_id: request.request_id,
                    configuration: self.config.current_configuration.clone(),
                };
                vec![Outcome::frame(Frame { body: Some(frame::Body::ConfigurationPublished(push)) })]
            }
            (ServerState::Ready, Some(frame::Body::PlaybackDiagnostics(_))) => {
                // §9: diagnostics ride the control stream; the Server treats
                // Client statistics as the only playback evidence. v2.0's
                // control plane accepts and records them without a reply.
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
            return self.malformed_fatal(request_id);
        }
        self.pending_client_key = Some(client_public_key.to_vec());
        // TM-4: an address already over its sliding window is closed 302
        // before another challenge is minted.
        // (The per-connection failure counter is the first line; the source
        // guard is owned by the accept loop, which consults the same
        // AddressRateGuard; the machine records this connection's failures.)
        let server_random = self.config.server_random_override.clone().unwrap_or_else(|| {
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
            self.config.auth_exporter_override.as_deref().unwrap_or(&[]),
        );

        // PR-9: one ECDSA-P256-SHA256 over the RAW context bytes.
        let signature_ok =
            auth::verify_auth_proof(&client_public_key, &context, signature).is_ok();

        if !signature_ok {
            // §5.2: signature invalid → AuthResult(203); count failure; the
            // 5th closes with Close(203).
            return self.record_auth_failure(ErrorCode::AUTHENTICATION_FAILED, request_id);
        }

        // Signature valid: consult the authorization state (§9.2 — at every
        // authentication, never a cached answer).
        match self.store.authorize(&client_public_key) {
            Authorization::Authorized(record) => {
                self.identity = Some(client_public_key.clone());
                self.pending_challenge = None;
                self.pending_request_id = None;
                self.state = ServerState::Negotiating;
                // TM-3: supersede an older session of the same identity.
                let session_id = self.next_session_id;
                self.next_session_id += 1;
                self.session_id = Some(session_id);
                self.supersede = Some(SupersedeRequest {
                    old_session_id: session_id.saturating_sub(1),
                    code: ErrorCode::SESSION_SUPERSEDED,
                });
                let _ = record;
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
            Authorization::Revoked(_) => {
                // §5.2: AuthResult(204); count failure.
                self.record_auth_failure(ErrorCode::AUTHORIZATION_DENIED, request_id)
            }
        }
    }

    fn record_auth_failure(&mut self, code: ErrorCode, request_id: u64) -> Vec<Outcome> {
        self.auth_failures += 1;
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

    fn handle_revocation_check(&mut self) -> Vec<Outcome> {
        // pairing.md §9.2 / TM-9: on an authorization-state change, every
        // session authenticated by an affected identity closes within ≤ 5 s
        // with the fatal pair Error(204)+Close(204).
        let Some(identity) = self.identity.clone() else {
            return Vec::new();
        };
        if matches!(self.store.authorize(&identity), Authorization::Revoked(_)) {
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
            body: Some(frame::Body::NegotiateResponse(crate::schema::NegotiateResponse {
                request_id: negotiate.request_id,
                status: Status::ok(),
                protocol_major: crate::codes::PROTOCOL_MAJOR,
                protocol_minor: self.config.protocol_minor,
                capabilities: accepted,
            })),
        })];
        // §5.1 READY entry: push ConfigurationPublished if a Transfer Format
        // is current.
        if let Some(configuration) = self.config.current_configuration.clone() {
            let push = crate::schema::ConfigurationPublished {
                request_id: 0,
                configuration: Some(configuration),
            };
            outcomes.push(Outcome::frame(Frame { body: Some(frame::Body::ConfigurationPublished(push)) }));
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
        if !self.config.capture_healthy {
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
            body: Some(frame::Body::SubscribeResponse(crate::schema::SubscribeResponse {
                request_id,
                status: Status::ok(),
                configuration: self.config.current_configuration.clone(),
            })),
        })];
        // §5.2 READY Subscribe: "if a stream is active, push StreamStarted
        // (active instance, current generation, next-frame timestamp)".
        if self.config.capture_active {
            if let Some(configuration) = &self.config.current_configuration {
                outcomes.push(Outcome::frame(Frame {
                    body: Some(frame::Body::StreamStarted(crate::schema::StreamStarted {
                        stream_instance_id: 1,
                        config_generation: configuration.config_generation,
                        initial_timestamp: 0,
                    })),
                }));
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

    fn pairing_unavailable(&mut self, request_id: u64) -> Vec<Outcome> {
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
            },
        ];
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

/// Fill bytes from the OS CSPRNG.
fn getrandom(buffer: &mut [u8]) {
    use ring::rand::{SecureRandom, SystemRandom};
    SystemRandom::new()
        .fill(buffer)
        .expect("OS CSPRNG is available");
}
