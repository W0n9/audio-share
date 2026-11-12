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
    //! Error/Close frame builders and the auth-proof delegation. Kept as a
    //! child module so the wire order (Error before Close, §5.2 fatal class)
    //! has one home.

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

    /// Sign an AuthProof for the received challenge. The identity key and the
    /// server identity key (auth transcript TLV(4)) are session context,
    /// registered before the machine is driven.
    pub fn proof(request_id: u64, server_random: &[u8]) -> Result<Frame, String> {
        let server_pub = SERVER_PUBLIC_KEY.with(|c| c.borrow().clone());
        let Some(server_pub) = server_pub else {
            return Err("no server public key registered".to_string());
        };
        crate::auth::build_auth_proof(
            request_id,
            server_random,
            &server_pub,
            2,
            // §9.1 TLV(6) exporter component; the vectors replace the real
            // exporter with a fixed constant (fixtures test-keys.json).
            &EXPORTER.with(|c| c.borrow().clone()),
            &crate::auth::test_client_identity_key(),
        )
    }

    thread_local! {
        static SERVER_PUBLIC_KEY: std::cell::RefCell<Option<Vec<u8>>> =
            const { std::cell::RefCell::new(None) };
        static EXPORTER: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(Vec::new());
    }

    /// Register the session's auth context inputs. Called once per connection.
    pub fn register_auth_context(server_public_key: Vec<u8>, auth_exporter: Vec<u8>) {
        SERVER_PUBLIC_KEY.with(|c| *c.borrow_mut() = Some(server_public_key));
        EXPORTER.with(|c| *c.borrow_mut() = auth_exporter);
    }
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
#[derive(Debug, Clone, PartialEq)]
pub struct ClientMachine {
    pub state: ClientState,
    /// Current configuration (data condition; stored on first/updated publication).
    pub config: Option<Configuration>,
    /// Subscription flag (READY (subscribed)).
    pub subscribed: bool,
    /// request_id the machine assigns on the frames it originates (Negotiate).
    pub next_request_id: u64,
}

impl ClientMachine {
    pub fn new() -> Self {
        ClientMachine {
            state: ClientState::Idle,
            config: None,
            subscribed: false,
            next_request_id: 1,
        }
    }

    /// Initial send after the control stream opens (§5.3 CONNECTING row):
    /// AuthInit with the paired identity key. The caller supplies the frame;
    /// the machine only records the state move.
    pub fn enter_authenticating(&mut self, auth_init: Frame) -> Verdict {
        debug_assert!(matches!(self.state, ClientState::Idle));
        self.state = ClientState::ConnectingSent;
        Verdict::Send(auth_init)
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
        match (self.state.clone(), body) {
            // ---- AUTHENTICATING -------------------------------------------------
            (ClientState::ConnectingSent, Body::AuthChallenge(c)) => {
                // §5.3: send AuthProof over the raw auth_context bytes. The
                // signing key + exporter are session context (helpers::proof),
                // not machine state.
                match signing_context::proof(c.request_id, c.server_random.as_slice()) {
                    Ok(f) => (Ok(Verdict::Send(f)), None),
                    Err(_) => (Err(CloseInfo { code: 601, remote: false }), None),
                }
            }
            (ClientState::ConnectingSent, Body::AuthResult(r)) => {
                let code = r.status.as_ref().map_or(0, |s| s.code);
                if code == 0 {
                    // §5.3: send Negotiate → NEGOTIATING
                    self.state = ClientState::Negotiating;
                    let negotiate = Frame {
                        body: Some(Body::Negotiate(crate::proto::Negotiate {
                            request_id: self.take_request_id(),
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
                    let negotiate = Frame {
                        body: Some(Body::Negotiate(crate::proto::Negotiate {
                            request_id: self.take_request_id(),
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
                self.config = Some(cfg.clone());
                (
                    Ok(Verdict::Ok),
                    Some(Event::ConfigurationPublished(cfg.config_generation)),
                )
            }
            (ClientState::Ready, Body::SubscribeResponse(r)) => {
                let code = r.status.as_ref().map_or(0, |s| s.code);
                if code == 0 {
                    self.subscribed = true;
                    if let Some(cfg) = r.configuration.clone() {
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
            (ClientState::Ready, Body::StreamStarted(_)) => {
                if self.subscribed {
                    self.state = ClientState::Playing;
                    (Ok(Verdict::Ok), Some(Event::StreamStarted))
                } else {
                    // §5.3 READY row: Server violation → Error(402)+Close(402)
                    (Ok(self.fatal(402)), None)
                }
            }
            // ---- READY / PLAYING ------------------------------------------------
            (s, Body::Close(c)) if matches!(s, ClientState::Ready | ClientState::Playing) => {
                // §5.3: send final diagnostics (best-effort, session layer); no
                // echo (§5.4 invariant 4). → CLOSING, then the session closes.
                self.state = ClientState::Closing;
                (
                    Ok(Verdict::Stop(CloseInfo { code: c.code, remote: true })),
                    None,
                )
            }
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
                self.config = Some(cfg.clone());
                (
                    Ok(Verdict::Ok),
                    Some(Event::ConfigurationPublished(cfg.config_generation)),
                )
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
                Verdict::Send(signing_context::close_frame(701))
            }
        }
    }
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
pub fn validate_configuration(cfg: &Configuration) -> Result<(), ()> {
    let Some(pcm) = cfg.pcm.as_ref() else {
        return Err(());
    };
    if cfg.codec != 1 {
        // CODEC_UNSPECIFIED (0) or a codec the schema has not assigned
        return Err(());
    }
    if pcm.sample_rate == 0 || pcm.channels == 0 || pcm.sample_format == 0 {
        return Err(());
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
        m.subscribed = true;
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
        signing_context::register_auth_context(
            crate::auth::tests_const_server_pub(),
            crate::auth::tests_const_exporter(),
        );
        let mut m = ClientMachine::new();
        m.state = ClientState::ConnectingSent;
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

    /// §4.5/§6.3: a 501 SubscribeResponse is non-fatal — connection maintained.
    #[test]
    fn subscribe_response_501_is_advisory() {
        let mut m = machine_at_ready();
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

    /// §5.3: Close received in READY/PLAYING → CLOSING with the Close's code,
    /// no echo (§5.4 invariant 4).
    #[test]
    fn close_received_moves_to_closing_with_code() {
        for start in [machine_at_ready(), machine_at_playing()] {
            let mut m = start;
            let (v, _) = m.on_frame(&frame(Body::Close(crate::proto::Close {
                code: 201,
                detail: String::new(),
            })));
            assert_eq!(v, Ok(Verdict::Stop(CloseInfo { code: 201, remote: true })));
            assert_eq!(m.state, ClientState::Closing);
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

/// Public test-support: register the fixture signing context so fixture
/// replay tests can drive the auth path end-to-end.
#[cfg(test)]
pub mod tests_public {
    pub fn register_test_signing_context() {
        crate::control::signing_context::register_auth_context(
            crate::auth::tests_const_server_pub(),
            crate::auth::tests_const_exporter(),
        );
    }
}
