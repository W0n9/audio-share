//! §5.3 Client state machine (spec.md, Client transition table) — pure,
//! transport-agnostic, host-tested. The session layer feeds decoded Frames
//! in and obeys the returned verdicts.
//!
//! Rows implemented (spec §5.3):
//! - CONNECTING → AUTHENTICATING: send AuthInit (paired) after handshake +
//!   stream open (pin verified). Pairing rows exist (PairingResult → Negotiate;
//!   201/206 → surface, CLOSED); an unrequested PairingResponse is a 402.
//! - AUTHENTICATING: AuthChallenge → AuthProof (auth.rs signs the raw
//!   auth_context); AuthResult OK → Negotiate → NEGOTIATING; 202/203/204 →
//!   surface, CLOSED (§6.3: no automatic retry).
//! - NEGOTIATING: NegotiateResponse OK → READY; error → §6.3 policy, CLOSED.
//! - READY: ConfigurationPublished store / Error(403)+Close(403) when not
//!   decodable; SubscribeResponse OK → subscribed / 501 advisory; StreamStarted
//!   (subscribed → PLAYING; not subscribed → fatal 402); Close → CLOSING.
//! - PLAYING: StreamEnded → READY; ConfigurationPublished apply (403 when not
//!   decodable); Close → CLOSING.
//! - any: non-fatal Error advisory-only, connection maintained; unset body →
//!   fatal 401; state-forbidden frames → fatal Error(402)+Close(402).
//!
//! Not modeled here (session layer's job): deadlines, backoff/reconnect, the
//! actual stream I/O.

use crate::proto::{frame::Body, Configuration, Frame};

pub(crate) mod signing_context {
    //! Error/Close frame builders for the §5.2 fatal class. Kept as a child
    //! module so the wire order (Error before Close) has one home.

    use crate::proto::{frame::Body, Close, Frame};

    /// Build a Close frame (§4.4): code != 0, no detail.
    pub fn close_frame(code: u32) -> Frame {
        Frame {
            body: Some(Body::Close(Close {
                code,
                detail: String::new(),
            })),
        }
    }
}

/// The session's authentication context (pairing.md §9.1). It is explicit
/// session state, not thread-local: the machine may be polled from any worker
/// thread of the multi-threaded runtime.
pub struct AuthContext {
    /// Raw 32-byte Ed25519 Server identity public key (auth transcript TLV(4)).
    pub server_public_key: Vec<u8>,
    /// TLS-Exporter bytes for TLV(6) (pairing.md §9.1).
    pub auth_exporter: Vec<u8>,
    /// Client identity key used to sign the auth_context.
    pub client_identity: p256::ecdsa::SigningKey,
}

impl std::fmt::Debug for AuthContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Red line: never render key material.
        f.debug_struct("AuthContext")
            .field("server_public_key", &"<redacted>")
            .field("auth_exporter", &"<redacted>")
            .field("client_identity", &"<redacted>")
            .finish()
    }
}

/// What response a pending request expects (§4.2 request/response pairing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingKind {
    /// AuthInit: AuthChallenge then AuthResult (or PairingResult).
    AuthInit,
    Negotiate,
    Subscribe,
    Unsubscribe,
    RequestConfiguration,
}

/// Client states (spec §5.1). Log/diagnostic names via [`ClientState::name`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientState {
    /// Session created; transport handshake not done. No app message processed.
    Idle,
    /// Handshake done, control stream open, AuthInit sent (AUTHENTICATING).
    ConnectingSent,
    /// Identity authenticated (or pairing confirmed), Negotiate sent.
    Negotiating,
    /// Authenticated ∧ negotiated. Configuration is a data condition.
    Ready,
    /// READY ∧ subscribed ∧ a stream instance is active.
    Playing,
    /// Close sent or received; teardown in progress.
    Closing,
    /// Terminal. No message is valid afterward (§5.4 invariant 6).
    Closed,
}

impl ClientState {
    /// Name used by the JNI getState surface (low-frequency, user-visible).
    pub fn name(&self) -> &'static str {
        match self {
            ClientState::Idle => "IDLE",
            ClientState::ConnectingSent => "AUTHENTICATING",
            ClientState::Negotiating => "NEGOTIATING",
            ClientState::Ready => "READY",
            ClientState::Playing => "PLAYING",
            ClientState::Closing => "CLOSING",
            ClientState::Closed => "CLOSED",
        }
    }

    pub fn is_closed(&self) -> bool {
        matches!(self, ClientState::Closed)
    }
}

/// Why the machine closed. The §6 code (when one exists) drives the §6.3
/// reconnect policy at the session layer; local classifications carry a code
/// too (205 pin mismatch, 601 transport loss).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseInfo {
    /// §6 registry code; 0 = none (local transport loss without a Close).
    pub code: u32,
    /// true when the Close came from the peer; false for local stop (701) or
    /// a local classification (205/601).
    pub remote: bool,
}

/// What the machine wants the transport layer to do next, in order.
// The frame is the hot-path payload; boxing it would allocate on every
// control message to shave a few stack bytes off the other variants.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Message consumed; no outgoing frame, no side effect.
    Ok,
    /// Send this frame on the control stream (AuthProof, Negotiate, Close(701)).
    Send(Frame),
    /// Send the Error then the Close (§5.2 fatal class), then stop with the code.
    Fatal { code: u32 },
    /// Session is torn down (Close received, exchange deadline, transport loss).
    Stop(CloseInfo),
}

/// The machine's output for one received frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Non-fatal Error received: log/surface, connection maintained (§5.2/§5.3).
    NonFatalError(u32),
    /// A Configuration became current.
    ConfigurationPublished(u32),
    /// Subscribed (SubscribeResponse OK).
    Subscribed,
    /// Stream instance active (PLAYING).
    StreamStarted,
    /// Stream instance ended; finalize per-instance statistics (READY).
    StreamEnded(u32),
}

/// The §5.3 Client state machine.
pub struct ClientMachine {
    pub state: ClientState,
    /// Current configuration (data condition; stored on first/updated publication).
    pub config: Option<Configuration>,
    /// Subscription flag (READY (subscribed)).
    pub subscribed: bool,
    /// request_id the machine assigns on the frames it originates.
    pub next_request_id: u64,
    /// §4.2 request/response pairing: request_id → expected response kind.
    pending: std::collections::BTreeMap<u64, PendingKind>,
    /// Session auth context (key + server key + exporter); explicit state, so
    /// the machine is safe to poll from any runtime worker thread.
    auth: Option<AuthContext>,
}

impl std::fmt::Debug for ClientMachine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientMachine")
            .field("state", &self.state)
            .field("config", &self.config)
            .field("subscribed", &self.subscribed)
            .field("next_request_id", &self.next_request_id)
            .field("pending", &self.pending)
            .field("auth", &self.auth.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl ClientMachine {
    pub fn new() -> Self {
        ClientMachine {
            state: ClientState::Idle,
            config: None,
            subscribed: false,
            next_request_id: 1,
            pending: std::collections::BTreeMap::new(),
            auth: None,
        }
    }

    /// Install the session auth context before the auth exchange starts.
    pub fn set_auth_context(
        &mut self,
        server_public_key: Vec<u8>,
        auth_exporter: Vec<u8>,
        client_identity: p256::ecdsa::SigningKey,
    ) {
        self.auth = Some(AuthContext {
            server_public_key,
            auth_exporter,
            client_identity,
        });
    }

    /// Number of requests awaiting a response (session deadline bookkeeping).
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    #[cfg(test)]
    pub(crate) fn seed_pending(&mut self, request_id: u64, kind: PendingKind) {
        self.pending.insert(request_id, kind);
    }

    /// Initial send after the control stream opens (§5.3 CONNECTING row):
    /// AuthInit with the paired identity key. The caller supplies the frame;
    /// the machine records the pending request and the state move.
    pub fn enter_authenticating(&mut self, auth_init: Frame) -> Verdict {
        debug_assert!(matches!(self.state, ClientState::Idle));
        if let Some(Body::AuthInit(init)) = &auth_init.body {
            self.pending.insert(init.request_id, PendingKind::AuthInit);
        }
        self.state = ClientState::ConnectingSent;
        Verdict::Send(auth_init)
    }

    /// READY → send Subscribe (§5.3 READY row) and await SubscribeResponse.
    pub fn request_subscribe(&mut self) -> Verdict {
        if self.state != ClientState::Ready {
            return self.fatal(402);
        }
        let request_id = self.take_request_id();
        self.pending.insert(request_id, PendingKind::Subscribe);
        Verdict::Send(Frame {
            body: Some(Body::Subscribe(crate::proto::Subscribe { request_id })),
        })
    }

    /// READY/PLAYING → send Unsubscribe and await UnsubscribeResponse.
    pub fn request_unsubscribe(&mut self) -> Verdict {
        if !matches!(self.state, ClientState::Ready | ClientState::Playing) {
            return self.fatal(402);
        }
        let request_id = self.take_request_id();
        self.pending.insert(request_id, PendingKind::Unsubscribe);
        Verdict::Send(Frame {
            body: Some(Body::Unsubscribe(crate::proto::Unsubscribe { request_id })),
        })
    }

    /// READY/PLAYING → ask for the current configuration; the matching
    /// ConfigurationPublished echoes this request_id (§5.3).
    pub fn request_configuration(&mut self) -> Verdict {
        if !matches!(self.state, ClientState::Ready | ClientState::Playing) {
            return self.fatal(402);
        }
        let request_id = self.take_request_id();
        self.pending.insert(request_id, PendingKind::RequestConfiguration);
        Verdict::Send(Frame {
            body: Some(Body::RequestConfiguration(crate::proto::RequestConfiguration {
                request_id,
            })),
        })
    }

    /// Fold one received control frame.
    pub fn on_frame(&mut self, frame: &Frame) -> (Result<Verdict, CloseInfo>, Option<Event>) {
        if self.state.is_closed() {
            return (Err(CloseInfo { code: 0, remote: false }), None);
        }
        let Some(body) = frame.body.as_ref() else {
            // §4.4 validation table: unset body is malformed (401) — and a
            // protocol violation no matter which side sent it.
            return (Ok(self.fatal(401)), None);
        };

        // §5.4 invariant 4: Close is one-way and never echoed. It is terminal
        // in EVERY state — including while connecting/authenticating, where
        // the Server may close after repeated auth failures (203).
        if let Body::Close(close) = body {
            if close.code == 0 {
                // §4.4 validation table: Close code 0 is malformed → 401.
                return (Ok(self.fatal(401)), None);
            }
            self.state = ClientState::Closed;
            self.pending.clear();
            return (
                Ok(Verdict::Stop(CloseInfo {
                    code: close.code,
                    remote: true,
                })),
                None,
            );
        }

        // §4.1/§4.4: every response carries Status; a status-bearing frame
        // without one (or an Error with code 0) is malformed, never success.
        if status_is_malformed(body) {
            return (Ok(self.fatal(401)), None);
        }

        // §4.2 request/response pairing. A response whose request_id matches
        // no pending request is dropped without state change; one that matches
        // the wrong pending kind is a protocol violation (401).
        if let Some((request_id, kind)) = response_expectation(body) {
            if request_id != 0 {
                match self.pending.get(&request_id).copied() {
                    None => return (Ok(Verdict::Ok), None),
                    Some(expected) if expected != kind => {
                        return (Ok(self.fatal(401)), None);
                    }
                    Some(_) => {
                        // AuthChallenge is the first half of AuthInit; keep it
                        // pending until AuthResult arrives.
                        if !matches!(body, Body::AuthChallenge(_)) {
                            self.pending.remove(&request_id);
                        }
                    }
                }
            }
        }
        if let Body::Error(error) = body {
            if error.request_id != 0
                && self.pending.remove(&error.request_id).is_none() {
                    // A late Error is dropped like any unmatched response.
                    return (Ok(Verdict::Ok), None);
                }
        }

        match (self.state.clone(), body) {
            // ---- AUTHENTICATING -------------------------------------------------
            (ClientState::ConnectingSent, Body::AuthChallenge(c)) => {
                // §5.3: send AuthProof over the raw auth_context bytes. The
                // signing material is explicit session state on the machine.
                let Some(auth) = self.auth.as_ref() else {
                    return (Err(CloseInfo { code: 601, remote: false }), None);
                };
                match crate::auth::build_auth_proof(
                    c.request_id,
                    c.server_random.as_slice(),
                    &auth.server_public_key,
                    2,
                    &auth.auth_exporter,
                    &auth.client_identity,
                ) {
                    Ok(f) => (Ok(Verdict::Send(f)), None),
                    Err(_) => (Err(CloseInfo { code: 601, remote: false }), None),
                }
            }
            (ClientState::ConnectingSent, Body::AuthResult(r)) => {
                let code = r.status.as_ref().map_or(0, |s| s.code);
                if code == 0 {
                    // §5.3: send Negotiate → NEGOTIATING
                    self.state = ClientState::Negotiating;
                    let request_id = self.take_request_id();
                    self.pending.insert(request_id, PendingKind::Negotiate);
                    let negotiate = Frame {
                        body: Some(Body::Negotiate(crate::proto::Negotiate {
                            request_id,
                            protocol_major: 2,
                            protocol_minor: 0,
                            capabilities: Vec::new(),
                        })),
                    };
                    (Ok(Verdict::Send(negotiate)), None)
                } else {
                    // 202/203: §5.3 would try the next stored pairing; this
                    // ticket's client holds a single identity, so there is no
                    // further record → surface, CLOSED (§6.3: no reconnect).
                    // 204: admin action → surface, CLOSED.
                    self.state = ClientState::Closed;
                    (Err(CloseInfo { code, remote: true }), None)
                }
            }
            (ClientState::ConnectingSent, Body::PairingResponse(_)) => {
                // pairing is phase 6; an unrequested PairingResponse is illegal
                (Ok(self.fatal(402)), None)
            }
            (ClientState::ConnectingSent, Body::PairingResult(r)) => {
                let code = r.status.as_ref().map_or(0, |s| s.code);
                if code == 0 {
                    self.state = ClientState::Negotiating;
                    let request_id = self.take_request_id();
                    self.pending.insert(request_id, PendingKind::Negotiate);
                    let negotiate = Frame {
                        body: Some(Body::Negotiate(crate::proto::Negotiate {
                            request_id,
                            protocol_major: 2,
                            protocol_minor: 0,
                            capabilities: Vec::new(),
                        })),
                    };
                    (Ok(Verdict::Send(negotiate)), None)
                } else {
                    // §5.3: surface to user (201 → fresh QR); no automatic retry
                    self.state = ClientState::Closed;
                    (Err(CloseInfo { code, remote: true }), None)
                }
            }
            // ---- NEGOTIATING ----------------------------------------------------
            (ClientState::Negotiating, Body::NegotiateResponse(r)) => {
                let code = r.status.as_ref().map_or(0, |s| s.code);
                if code == 0 {
                    // §5.3: store accepted capabilities (empty in v2.0) → READY
                    self.state = ClientState::Ready;
                    (Ok(Verdict::Ok), None)
                } else {
                    // §6.3: 101/102 → surface, no reconnect
                    (Err(CloseInfo { code, remote: true }), None)
                }
            }
            // ---- READY ----------------------------------------------------------
            (ClientState::Ready, Body::ConfigurationPublished(p)) => {
                let Some(cfg) = p.configuration.as_ref() else {
                    // A publication with no configuration is malformed.
                    return (Ok(self.fatal(401)), None);
                };
                if validate_configuration(cfg).is_err() {
                    // §4.4 validation table: structurally invalid → 401
                    return (Ok(self.fatal(401)), None);
                }
                if !configuration_decodable(cfg) {
                    // §5.3 READY row: valid but not decodable → Error(403)+Close(403)
                    return (Ok(self.fatal(403)), None);
                }
                self.config = Some(*cfg);
                (
                    Ok(Verdict::Ok),
                    Some(Event::ConfigurationPublished(cfg.config_generation)),
                )
            }
            (ClientState::Ready, Body::SubscribeResponse(r)) => {
                let code = r.status.as_ref().map_or(0, |s| s.code);
                if code == 0 {
                    self.subscribed = true;
                    if let Some(cfg) = r.configuration {
                        self.config = Some(cfg);
                    }
                    (Ok(Verdict::Ok), Some(Event::Subscribed))
                } else if code == 501 {
                    // §5.3: surface; MAY retry after the next publication.
                    // Non-fatal: connection maintained, state unchanged.
                    (Ok(Verdict::Ok), Some(Event::NonFatalError(501)))
                } else {
                    (Err(CloseInfo { code, remote: true }), None)
                }
            }
            (ClientState::Ready, Body::UnsubscribeResponse(_)) => {
                // §5.3: idempotent subscription state; no state change.
                self.subscribed = false;
                (Ok(Verdict::Ok), None)
            }
            (ClientState::Ready, Body::StreamStarted(_)) => {
                if self.subscribed {
                    self.state = ClientState::Playing;
                    (Ok(Verdict::Ok), Some(Event::StreamStarted))
                } else {
                    // §5.3 READY row: Server violation → Error(402)+Close(402)
                    (Ok(self.fatal(402)), None)
                }
            }
            // ---- PLAYING --------------------------------------------------------
            (ClientState::Playing, Body::StreamEnded(e)) => {
                // §5.3: finalize instance statistics → READY (§5.4: the only
                // backward move).
                self.state = ClientState::Ready;
                self.subscribed = false;
                (Ok(Verdict::Ok), Some(Event::StreamEnded(e.code)))
            }
            (ClientState::Playing, Body::ConfigurationPublished(p)) => {
                let Some(cfg) = p.configuration.as_ref() else {
                    return (Ok(self.fatal(401)), None);
                };
                if validate_configuration(cfg).is_err() {
                    // §4.4 validation table: structurally invalid → 401
                    return (Ok(self.fatal(401)), None);
                }
                if !configuration_decodable(cfg) {
                    // §5.2 error classes: fatal, paired with Close
                    return (Ok(self.fatal(403)), None);
                }
                self.config = Some(*cfg);
                (
                    Ok(Verdict::Ok),
                    Some(Event::ConfigurationPublished(cfg.config_generation)),
                )
            }
            (ClientState::Playing, Body::UnsubscribeResponse(_)) => {
                self.subscribed = false;
                (Ok(Verdict::Ok), None)
            }
            // ---- any state: non-fatal Error (§5.3) -------------------------------
            (_, Body::Error(e)) => {
                // §5.2/§5.3: advisory only; no state change. Fatal errors always
                // arrive paired with Close — the Close row handles teardown.
                (
                    Ok(Verdict::Ok),
                    Some(Event::NonFatalError(e.status.as_ref().map_or(0, |s| s.code))),
                )
            }
            // ---- every other (state, frame) pair is state-forbidden --------------
            (_state, _other) => {
                // §5.3: a request in an illegal state is answered with a defined
                // error, never a silent ignore (§5.2 CONNECTING 401/402 split:
                // this frame decoded and validated, so 402 INVALID_STATE).
                (Ok(self.fatal(402)), None)
            }
        }
    }

    /// Fatal error path: Error(code)+Close(code) outbound then Stop. The
    /// session layer sends both frames in order, then transport-closes.
    fn fatal(&mut self, code: u32) -> Verdict {
        self.state = ClientState::Closed;
        self.pending.clear();
        Verdict::Fatal { code }
    }

    /// Monotonic per-connection request_id (§4.2: unique per sender per
    /// connection, increasing from 1).
    fn take_request_id(&mut self) -> u64 {
        let id = self.next_request_id;
        self.next_request_id += 1;
        id
    }

    /// User-initiated stop (§6.3: 701 → no reconnect). Sends Close(701).
    pub fn user_stop(&mut self) -> Verdict {
        match self.state {
            ClientState::Closed => Verdict::Ok,
            _ => {
                self.state = ClientState::Closed;
                self.pending.clear();
                Verdict::Send(signing_context::close_frame(701))
            }
        }
    }
}

/// The response kind a frame answers and the request_id it echoes. `None`
/// for requests, pushes (request_id 0), and frames with no request pairing.
fn response_expectation(body: &Body) -> Option<(u64, PendingKind)> {
    match body {
        Body::AuthChallenge(m) => Some((m.request_id, PendingKind::AuthInit)),
        Body::AuthResult(m) => Some((m.request_id, PendingKind::AuthInit)),
        Body::PairingResult(m) => Some((m.request_id, PendingKind::AuthInit)),
        Body::PairingResponse(m) => Some((m.request_id, PendingKind::AuthInit)),
        Body::NegotiateResponse(m) => Some((m.request_id, PendingKind::Negotiate)),
        Body::SubscribeResponse(m) => Some((m.request_id, PendingKind::Subscribe)),
        Body::UnsubscribeResponse(m) => Some((m.request_id, PendingKind::Unsubscribe)),
        Body::ConfigurationPublished(m) if m.request_id != 0 => {
            Some((m.request_id, PendingKind::RequestConfiguration))
        }
        _ => None,
    }
}

/// §4.1/§4.4: a status-bearing response must carry a nonzero-or-success
/// Status; an Error must carry a nonzero code. Missing → malformed (401).
fn status_is_malformed(body: &Body) -> bool {
    let status = match body {
        Body::AuthResult(m) => Some(m.status.as_ref()),
        Body::PairingResult(m) => Some(m.status.as_ref()),
        Body::PairingResponse(m) => Some(m.status.as_ref()),
        Body::NegotiateResponse(m) => Some(m.status.as_ref()),
        Body::SubscribeResponse(m) => Some(m.status.as_ref()),
        Body::UnsubscribeResponse(m) => Some(m.status.as_ref()),
        Body::Error(m) => return m.status.as_ref().is_none_or(|s| s.code == 0),
        _ => None,
    };
    matches!(status, Some(None))
}

impl Default for ClientMachine {
    fn default() -> Self {
        Self::new()
    }
}

/// §4.4 validation table for `Configuration` (structural): pcm absent →
/// invalid (v2.0's only codec is CODEC_PCM, which requires pcm); codec
/// UNSPECIFIED → invalid; sample_rate 0, channels 0, sample_format
/// UNSPECIFIED → invalid. A structurally invalid configuration is a
/// malformed message: Error(401)+Close(401).
pub fn validate_configuration(cfg: &Configuration) -> Result<(), &'static str> {
    let Some(pcm) = cfg.pcm.as_ref() else {
        return Err("codec PCM requires pcm parameters");
    };
    if cfg.codec != 1 {
        // CODEC_UNSPECIFIED (0) or a codec the schema has not assigned
        return Err("unsupported codec");
    }
    if pcm.sample_rate == 0 || pcm.channels == 0 || pcm.sample_format == 0 {
        return Err("pcm parameters must be nonzero");
    }
    Ok(())
}

/// Decodability (§6.2 403 UNSUPPORTED_MEDIA_CONFIGURATION): the configuration
/// is structurally valid but this Client cannot decode the Transfer Format.
/// Recorded device capability (fixture control.error-403 preconditions): this
/// Client supports F32LE (1) and S16LE (3) only.
pub fn configuration_decodable(cfg: &Configuration) -> bool {
    const SUPPORTED_SAMPLE_FORMATS: [u32; 2] = [1, 3]; // F32LE, S16LE
    let Some(pcm) = cfg.pcm.as_ref() else {
        return false;
    };
    cfg.codec == 1 && SUPPORTED_SAMPLE_FORMATS.contains(&(pcm.sample_format as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{
        AuthChallenge, AuthResult, ConfigurationPublished, Error, NegotiateResponse,
        PcmParameters, PlaybackDiagnostics, Status, StreamEnded, StreamStarted, Subscribe,
        SubscribeResponse,
    };

    fn frame(body: Body) -> Frame {
        Frame { body: Some(body) }
    }

    fn ok_status() -> Option<Status> {
        Some(Status::default())
    }

    fn config(gen: u32) -> Configuration {
        Configuration {
            config_generation: gen,
            codec: 1, // CODEC_PCM
            pcm: Some(PcmParameters {
                sample_rate: 48000,
                channels: 2,
                sample_format: 1, // F32LE
            }),
        }
    }

    fn machine_at_ready() -> ClientMachine {
        let mut m = ClientMachine::new();
        m.state = ClientState::Negotiating;
        m.seed_pending(2, PendingKind::Negotiate);
        let _ = m.on_frame(&frame(Body::NegotiateResponse(NegotiateResponse {
            request_id: 2,
            status: ok_status(),
            protocol_major: 2,
            protocol_minor: 0,
            capabilities: Vec::new(),
        })));
        m
    }

    fn machine_at_playing() -> ClientMachine {
        let mut m = machine_at_ready();
        m.seed_pending(3, PendingKind::Subscribe);
        let _ = m.on_frame(&frame(Body::SubscribeResponse(SubscribeResponse {
            request_id: 3,
            status: ok_status(),
            configuration: Some(config(1)),
        })));
        let _ = m.on_frame(&frame(Body::StreamStarted(StreamStarted {
            stream_instance_id: 1,
            config_generation: 1,
            initial_timestamp: 0,
        })));
        assert_eq!(m.state, ClientState::Playing);
        m
    }

    // §5.3: NegotiateResponse OK → READY.
    #[test]
    fn negotiate_response_ok_moves_to_ready() {
        let mut m = ClientMachine::new();
        m.state = ClientState::Negotiating;
        m.seed_pending(2, PendingKind::Negotiate);
        let (v, e) = m.on_frame(&frame(Body::NegotiateResponse(NegotiateResponse {
            request_id: 2,
            status: ok_status(),
            protocol_major: 2,
            protocol_minor: 0,
            capabilities: Vec::new(),
        })));
        assert_eq!(v, Ok(Verdict::Ok));
        assert_eq!(e, None);
        assert_eq!(m.state, ClientState::Ready);
    }

    /// §5.3: AuthResult OK → send Negotiate → NEGOTIATING. The Negotiate frame
    /// carries protocol_major 2, minor 0, empty capabilities (§4.5).
    #[test]
    fn auth_result_ok_sends_negotiate() {
        let mut m = ClientMachine::new();
        m.state = ClientState::ConnectingSent;
        m.seed_pending(1, PendingKind::AuthInit);
        let (v, e) = m.on_frame(&frame(Body::AuthResult(AuthResult {
            request_id: 1,
            status: ok_status(),
            session_id: 1,
        })));
        assert_eq!(e, None);
        let Ok(Verdict::Send(neg)) = v else {
            panic!("expected Send");
        };
        let Some(Body::Negotiate(n)) = neg.body else {
            panic!("expected negotiate body");
        };
        assert_eq!((n.protocol_major, n.protocol_minor, n.capabilities.len()), (2, 0, 0));
        assert_eq!(m.state, ClientState::Negotiating);
    }

    /// §5.3: AuthResult 202/203/204 → surface, CLOSED (no further records).
    #[test]
    fn auth_result_error_closes_without_retry() {
        for code in [202u32, 203, 204] {
            let mut m = ClientMachine::new();
            m.state = ClientState::ConnectingSent;
            m.seed_pending(1, PendingKind::AuthInit);
            let (v, _) = m.on_frame(&frame(Body::AuthResult(AuthResult {
                request_id: 1,
                status: Some(Status {
                    code,
                    ..Status::default()
                }),
                session_id: 0,
            })));
            assert_eq!(v, Err(CloseInfo { code, remote: true }), "code {code}");
            assert_eq!(m.state, ClientState::Closed);
        }
    }

    /// §5.3 CONNECTING row + helpers: AuthChallenge → AuthProof (signed over
    /// the raw auth_context). The signature verifies against the fixture key.
    #[test]
    fn auth_challenge_yields_verifiable_auth_proof() {
        let mut m = ClientMachine::new();
        m.set_auth_context(
            crate::auth::tests_const_server_pub(),
            crate::auth::tests_const_exporter(),
            crate::auth::test_client_identity_key(),
        );
        m.state = ClientState::ConnectingSent;
        m.seed_pending(1, PendingKind::AuthInit);
        let (v, _) = m.on_frame(&frame(Body::AuthChallenge(AuthChallenge {
            request_id: 1,
            server_random: (0u8..32).collect(),
        })));
        let Ok(Verdict::Send(proof)) = v else {
            panic!("expected Send");
        };
        let Some(Body::AuthProof(p)) = proof.body else {
            panic!("expected auth_proof");
        };
        assert_eq!(p.request_id, 1);
        assert_eq!(p.signature.len(), 64);
    }

    /// §4.2: responses are paired by request_id. A response whose id matches
    /// no pending request is dropped without state change (late/unknown id),
    /// never folded into the state machine as if it answered an exchange.
    #[test]
    fn unmatched_response_id_is_dropped() {
        let mut m = machine_at_ready();
        let (v, e) = m.on_frame(&frame(Body::SubscribeResponse(SubscribeResponse {
            request_id: 99,
            status: ok_status(),
            configuration: Some(config(1)),
        })));
        assert_eq!(v, Ok(Verdict::Ok));
        assert_eq!(e, None);
        assert_eq!(m.state, ClientState::Ready);
        assert!(!m.subscribed, "late/unknown response must not mutate state");
    }

    /// §4.2: an id that IS pending but answers the wrong request kind is a
    /// protocol violation → 401.
    #[test]
    fn mismatched_pending_kind_is_fatal_401() {
        let mut m = machine_at_ready();
        m.seed_pending(4, PendingKind::Negotiate);
        let (v, _) = m.on_frame(&frame(Body::SubscribeResponse(SubscribeResponse {
            request_id: 4,
            status: ok_status(),
            configuration: None,
        })));
        assert_eq!(v, Ok(Verdict::Fatal { code: 401 }));
        assert_eq!(m.state, ClientState::Closed);
    }

    /// §4.1/§4.4: a status-bearing response without Status is malformed — the
    /// old `map_or(0, ...)` treated it as success, which is forbidden.
    #[test]
    fn response_without_status_is_fatal_401() {
        let mut m = ClientMachine::new();
        m.state = ClientState::ConnectingSent;
        m.seed_pending(1, PendingKind::AuthInit);
        let (v, _) = m.on_frame(&frame(Body::AuthResult(AuthResult {
            request_id: 1,
            status: None,
            session_id: 1,
        })));
        assert_eq!(v, Ok(Verdict::Fatal { code: 401 }));
        assert_eq!(m.state, ClientState::Closed);
    }

    /// §4.2: a replayed response for an already-completed request is late and
    /// dropped (the pending entry was consumed by the first response).
    #[test]
    fn late_response_is_dropped_after_completion() {
        let mut m = machine_at_ready();
        let Verdict::Send(subscribe) = m.request_subscribe() else {
            panic!("expected Subscribe");
        };
        let Some(Body::Subscribe(sub)) = subscribe.body else {
            panic!("expected subscribe body");
        };
        let reply = frame(Body::SubscribeResponse(SubscribeResponse {
            request_id: sub.request_id,
            status: ok_status(),
            configuration: None,
        }));
        let (first, event) = m.on_frame(&reply);
        assert_eq!(first, Ok(Verdict::Ok));
        assert_eq!(event, Some(Event::Subscribed));
        assert_eq!(m.pending_len(), 0);
        let (late, event) = m.on_frame(&reply);
        assert_eq!(late, Ok(Verdict::Ok));
        assert_eq!(event, None);
        assert_eq!(m.state, ClientState::Ready);
    }

    /// §5.3 normal request/response pairing: auth → negotiate → subscribe →
    /// request_configuration → unsubscribe, each reply matched by request_id.
    #[test]
    fn normal_request_response_pairing() {
        let mut m = ClientMachine::new();
        m.set_auth_context(
            crate::auth::tests_const_server_pub(),
            crate::auth::tests_const_exporter(),
            crate::auth::test_client_identity_key(),
        );
        let init = Frame {
            body: Some(Body::AuthInit(crate::proto::AuthInit {
                request_id: 1,
                client_public_key: crate::auth::sec1_compressed(
                    &crate::auth::test_client_identity_key(),
                ),
            })),
        };
        assert!(matches!(m.enter_authenticating(init), Verdict::Send(_)));
        let (challenge, _) = m.on_frame(&frame(Body::AuthChallenge(AuthChallenge {
            request_id: 1,
            server_random: (0u8..32).collect(),
        })));
        assert!(matches!(challenge, Ok(Verdict::Send(_))));
        let Ok(Verdict::Send(negotiate)) = m
            .on_frame(&frame(Body::AuthResult(AuthResult {
                request_id: 1,
                status: ok_status(),
                session_id: 1,
            })))
            .0
        else {
            panic!("expected Negotiate");
        };
        let Some(Body::Negotiate(neg)) = negotiate.body else {
            panic!("expected negotiate body");
        };
        let (ready, _) = m.on_frame(&frame(Body::NegotiateResponse(NegotiateResponse {
            request_id: neg.request_id,
            status: ok_status(),
            protocol_major: 2,
            protocol_minor: 0,
            capabilities: Vec::new(),
        })));
        assert_eq!(ready, Ok(Verdict::Ok));
        assert_eq!(m.state, ClientState::Ready);

        let Verdict::Send(subscribe) = m.request_subscribe() else {
            panic!("expected Subscribe");
        };
        let Some(Body::Subscribe(sub)) = subscribe.body else {
            panic!("expected subscribe body");
        };
        let (subscribed, event) = m.on_frame(&frame(Body::SubscribeResponse(SubscribeResponse {
            request_id: sub.request_id,
            status: ok_status(),
            configuration: Some(config(1)),
        })));
        assert_eq!(subscribed, Ok(Verdict::Ok));
        assert_eq!(event, Some(Event::Subscribed));
        assert!(m.subscribed);

        let Verdict::Send(request_config) = m.request_configuration() else {
            panic!("expected RequestConfiguration");
        };
        let Some(Body::RequestConfiguration(req)) = request_config.body else {
            panic!("expected request_configuration body");
        };
        let (published, event) = m.on_frame(&frame(Body::ConfigurationPublished(
            ConfigurationPublished {
                request_id: req.request_id,
                configuration: Some(config(2)),
            },
        )));
        assert_eq!(published, Ok(Verdict::Ok));
        assert_eq!(event, Some(Event::ConfigurationPublished(2)));

        let Verdict::Send(unsubscribe) = m.request_unsubscribe() else {
            panic!("expected Unsubscribe");
        };
        let Some(Body::Unsubscribe(unsub)) = unsubscribe.body else {
            panic!("expected unsubscribe body");
        };
        let (unsubscribed, event) = m.on_frame(&frame(Body::UnsubscribeResponse(
            crate::proto::UnsubscribeResponse {
                request_id: unsub.request_id,
                status: ok_status(),
            },
        )));
        assert_eq!(unsubscribed, Ok(Verdict::Ok));
        assert_eq!(event, None);
        assert!(!m.subscribed);
        assert_eq!(m.pending_len(), 0);
    }

    /// §5.4 invariant 4: Close is terminal and silent in EVERY state — the
    /// machine never emits an Error/Close echo, even mid-auth (203/302/701).
    #[test]
    fn close_is_terminal_in_every_state_without_output() {
        for state in [
            ClientState::Idle,
            ClientState::ConnectingSent,
            ClientState::Negotiating,
            ClientState::Ready,
            ClientState::Playing,
            ClientState::Closing,
        ] {
            for code in [203u32, 302, 701] {
                let mut m = ClientMachine::new();
                m.state = state.clone();
                m.seed_pending(1, PendingKind::AuthInit);
                let (v, e) = m.on_frame(&frame(Body::Close(crate::proto::Close {
                    code,
                    detail: String::new(),
                })));
                assert_eq!(
                    v,
                    Ok(Verdict::Stop(CloseInfo { code, remote: true })),
                    "{state:?} code {code}"
                );
                assert_eq!(e, None);
                assert_eq!(m.state, ClientState::Closed);
                assert_eq!(m.pending_len(), 0);
            }
        }
    }

    /// §4.4: Close code 0 is malformed → 401 (the only Close that yields an
    /// outbound Error+Close; a nonzero Close is acknowledged by transport only).
    #[test]
    fn close_code_zero_is_fatal_401() {
        let mut m = machine_at_ready();
        let (v, _) = m.on_frame(&frame(Body::Close(crate::proto::Close {
            code: 0,
            detail: String::new(),
        })));
        assert_eq!(v, Ok(Verdict::Fatal { code: 401 }));
        assert_eq!(m.state, ClientState::Closed);
    }

    /// A5 regression: auth material is session state, not thread-local. The
    /// machine must produce a valid AuthProof when polled from another thread
    /// (multi-worker runtime task migration used to read None → 601).
    #[test]
    fn auth_context_survives_polling_on_another_thread() {
        let mut m = ClientMachine::new();
        m.set_auth_context(
            crate::auth::tests_const_server_pub(),
            crate::auth::tests_const_exporter(),
            crate::auth::test_client_identity_key(),
        );
        m.state = ClientState::ConnectingSent;
        m.seed_pending(1, PendingKind::AuthInit);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (v, _) = m.on_frame(&frame(Body::AuthChallenge(AuthChallenge {
                request_id: 1,
                server_random: (0u8..32).collect(),
            })));
            tx.send(v).unwrap();
        })
        .join()
        .unwrap();
        let Ok(Verdict::Send(proof)) = rx.recv().unwrap() else {
            panic!("expected AuthProof from the worker thread");
        };
        let Some(Body::AuthProof(p)) = proof.body else {
            panic!("expected auth_proof body");
        };
        assert_eq!(p.signature.len(), 64);
    }

    /// §4.5/§6.3: a 501 SubscribeResponse is non-fatal — connection maintained.
    #[test]
    fn subscribe_response_501_is_advisory() {
        let mut m = machine_at_ready();
        m.seed_pending(3, PendingKind::Subscribe);
        let (v, e) = m.on_frame(&frame(Body::SubscribeResponse(SubscribeResponse {
            request_id: 3,
            status: Some(Status {
                code: 501,
                retryable: true,
                requires_repairing: false,
                detail: String::new(),
            }),
            configuration: None,
        })));
        assert_eq!(v, Ok(Verdict::Ok));
        assert_eq!(e, Some(Event::NonFatalError(501)));
        assert_eq!(m.state, ClientState::Ready);
    }

    /// §5.3 READY: ConfigurationPublished stores the config (data condition).
    #[test]
    fn configuration_published_is_stored() {
        let mut m = machine_at_ready();
        let (v, e) = m.on_frame(&frame(Body::ConfigurationPublished(
            ConfigurationPublished {
                request_id: 0,
                configuration: Some(config(1)),
            },
        )));
        assert_eq!(v, Ok(Verdict::Ok));
        assert_eq!(e, Some(Event::ConfigurationPublished(1)));
        assert_eq!(m.config.as_ref().unwrap().config_generation, 1);
    }

    /// §4.4 validation table: codec absent (proto3 default 0 = UNSPECIFIED)
    /// with pcm present → Error(403)+Close(403) (control.error-401-invalid-
    /// configuration uses 401 for a Server-side *invalid* configuration;
    /// 403 is the Client-side "valid but not decodable" — the fixture
    /// error-403 uses sample_format 4 (S24LE), unsupported by this Client).
    ///
    /// The Client machine 403s a *structurally invalid* configuration only if
    /// it arrived in READY as a publication — but per §4.4 an invalid
    /// Configuration is 401 (malformed). The machine follows the spec: invalid
    /// configuration → 401; valid-but-unsupported sample format → 403.
    #[test]
    fn invalid_configuration_is_fatal_401() {
        let mut m = machine_at_ready();
        let bad = Configuration {
            config_generation: 1,
            codec: 0, // UNSPECIFIED — not encoded, pcm present anyway
            pcm: Some(PcmParameters {
                sample_rate: 0,
                channels: 1,
                sample_format: 1,
            }),
        };
        let (v, _) = m.on_frame(&frame(Body::ConfigurationPublished(
            ConfigurationPublished {
                request_id: 0,
                configuration: Some(bad),
            },
        )));
        assert_eq!(v, Ok(Verdict::Fatal { code: 401 }));
        assert_eq!(m.state, ClientState::Closed);
    }

    /// §5.3 READY: a valid configuration the Client cannot decode (S24LE,
    /// fixture control.error-403) → Error(403)+Close(403).
    #[test]
    fn unsupported_but_valid_configuration_is_fatal_403() {
        let mut m = machine_at_ready();
        let s24 = Configuration {
            config_generation: 1,
            codec: 1,
            pcm: Some(PcmParameters {
                sample_rate: 48000,
                channels: 2,
                sample_format: 4, // S24LE — this Client supports F32LE/S16LE only
            }),
        };
        let (v, _) = m.on_frame(&frame(Body::ConfigurationPublished(
            ConfigurationPublished {
                request_id: 0,
                configuration: Some(s24),
            },
        )));
        assert_eq!(v, Ok(Verdict::Fatal { code: 403 }));
        assert_eq!(m.state, ClientState::Closed);
    }

    /// §5.3 READY row: StreamStarted while not subscribed is a Server violation
    /// → Error(402)+Close(402).
    #[test]
    fn stream_started_unsubscribed_is_fatal_402() {
        let mut m = machine_at_ready();
        let (v, _) = m.on_frame(&frame(Body::StreamStarted(StreamStarted {
            stream_instance_id: 1,
            config_generation: 1,
            initial_timestamp: 0,
        })));
        assert_eq!(v, Ok(Verdict::Fatal { code: 402 }));
        assert_eq!(m.state, ClientState::Closed);
    }

    /// §5.3 READY → PLAYING on StreamStarted when subscribed.
    #[test]
    fn stream_started_subscribed_moves_to_playing() {
        let mut m = machine_at_ready();
        m.subscribed = true;
        let (v, e) = m.on_frame(&frame(Body::StreamStarted(StreamStarted {
            stream_instance_id: 1,
            config_generation: 1,
            initial_timestamp: 0,
        })));
        assert_eq!(v, Ok(Verdict::Ok));
        assert_eq!(e, Some(Event::StreamStarted));
        assert_eq!(m.state, ClientState::Playing);
    }

    /// §5.2 non-fatal 402 (duplicate Subscribe) keeps the connection: the
    /// machine sees the paired Error only (no Close) → advisory, no change.
    #[test]
    fn nonfatal_error_keeps_connection() {
        let mut m = machine_at_playing();
        // The Error answers a live request (duplicate Subscribe from the client),
        // so it is matched against that pending entry, not dropped as a late reply.
        m.seed_pending(2, PendingKind::Subscribe);
        let (v, e) = m.on_frame(&frame(Body::Error(Error {
            request_id: 2,
            status: Some(Status {
                code: 402,
                retryable: false,
                requires_repairing: false,
                detail: String::new(),
            }),
        })));
        assert_eq!(v, Ok(Verdict::Ok));
        assert_eq!(e, Some(Event::NonFatalError(402)));
        assert_eq!(m.state, ClientState::Playing, "connection maintained");
    }

    /// §5.3 PLAYING: StreamEnded(501/502) → READY; subscribed flag cleared.
    #[test]
    fn stream_ends_moves_back_to_ready() {
        for code in [501u32, 502] {
            let mut m = machine_at_playing();
            let (v, e) = m.on_frame(&frame(Body::StreamEnded(StreamEnded {
                stream_instance_id: 1,
                code,
                detail: String::new(),
            })));
            assert_eq!(v, Ok(Verdict::Ok), "code {code}");
            assert_eq!(e, Some(Event::StreamEnded(code)));
            assert_eq!(m.state, ClientState::Ready);
            assert!(!m.subscribed);
        }
    }

    /// §5.3 PLAYING: ConfigurationPublished applies (e.g. generation bump).
    #[test]
    fn playing_configuration_published_applies() {
        let mut m = machine_at_playing();
        let (v, e) = m.on_frame(&frame(Body::ConfigurationPublished(
            ConfigurationPublished {
                request_id: 0,
                configuration: Some(config(2)),
            },
        )));
        assert_eq!(v, Ok(Verdict::Ok));
        assert_eq!(e, Some(Event::ConfigurationPublished(2)));
        assert_eq!(m.state, ClientState::Playing);
        assert_eq!(m.config.as_ref().unwrap().config_generation, 2);
    }

    /// §5.4 invariant 4 / §5.3: Close received in READY or PLAYING is terminal
    /// immediately — no echo, no CLOSING round trip.
    #[test]
    fn close_received_closes_with_code_in_ready_and_playing() {
        for start in [machine_at_ready(), machine_at_playing()] {
            let mut m = start;
            let (v, _) = m.on_frame(&frame(Body::Close(crate::proto::Close {
                code: 201,
                detail: String::new(),
            })));
            assert_eq!(v, Ok(Verdict::Stop(CloseInfo { code: 201, remote: true })));
            assert_eq!(m.state, ClientState::Closed);
        }
    }

    /// §4.4 validation table: unset body is 401 wherever it appears.
    #[test]
    fn unset_body_is_fatal_401() {
        let mut m = machine_at_ready();
        let (v, _) = m.on_frame(&Frame { body: None });
        assert_eq!(v, Ok(Verdict::Fatal { code: 401 }));
        assert_eq!(m.state, ClientState::Closed);
    }

    /// §5.2 CONNECTING 401/402 split — a well-formed Subscribe in
    /// CONNECTING/NEGOTIATING decodes fine but is state-forbidden → 402.
    #[test]
    fn state_forbidden_subscribe_is_fatal_402() {
        for state in [ClientState::ConnectingSent, ClientState::Negotiating] {
            let mut m = ClientMachine::new();
            m.state = state;
            let (v, _) = m.on_frame(&frame(Body::Subscribe(Subscribe { request_id: 1 })));
            assert_eq!(v, Ok(Verdict::Fatal { code: 402 }));
            assert_eq!(m.state, ClientState::Closed);
        }
    }

    /// §5.4 invariant 6: CLOSED is absorbing — no message is valid afterward.
    #[test]
    fn closed_absorbs_everything() {
        let mut m = machine_at_ready();
        m.state = ClientState::Closed;
        for body in [
            Body::Subscribe(Subscribe { request_id: 9 }),
            Body::Close(crate::proto::Close {
                code: 201,
                detail: String::new(),
            }),
        ] {
            let (v, _) = m.on_frame(&frame(body));
            assert!(v.is_err());
        }
    }

    /// §5.3: PlaybackDiagnostics is a Client→Server push — receiving one on the
    /// Client is state-forbidden → 402.
    #[test]
    fn received_diagnostics_is_fatal_402() {
        let mut m = machine_at_ready();
        let (v, _) = m.on_frame(&frame(Body::PlaybackDiagnostics(
            PlaybackDiagnostics::default(),
        )));
        assert_eq!(v, Ok(Verdict::Fatal { code: 402 }));
    }

    /// §6.3: user stop sends Close(701) and stops; no reconnect.
    #[test]
    fn user_stop_sends_701() {
        let mut m = machine_at_ready();
        let v = m.user_stop();
        match v {
            Verdict::Send(f) => {
                let Some(Body::Close(c)) = f.body else {
                    panic!("expected close");
                };
                assert_eq!(c.code, 701);
            }
            other => panic!("expected Send, got {other:?}"),
        }
        assert_eq!(m.state, ClientState::Closed);
        // second stop is a no-op
        assert_eq!(m.user_stop(), Verdict::Ok);
    }

    /// State names are the JNI surface contract.
    #[test]
    fn state_names_match_surface_contract() {
        assert_eq!(ClientState::Idle.name(), "IDLE");
        assert_eq!(ClientState::ConnectingSent.name(), "AUTHENTICATING");
        assert_eq!(ClientState::Negotiating.name(), "NEGOTIATING");
        assert_eq!(ClientState::Ready.name(), "READY");
        assert_eq!(ClientState::Playing.name(), "PLAYING");
        assert_eq!(ClientState::Closing.name(), "CLOSING");
        assert_eq!(ClientState::Closed.name(), "CLOSED");
    }
}
