//! The per-connection driver: wires the QUIC transport, the control-stream
//! framing, and the §5.2 state machine into a live session (ticket 07).
//!
//! Scope:
//! - one Client-initiated bidirectional stream is the control stream (§3);
//!   a second stream is refused with Error(402) + Close(402) (§5.2
//!   CONNECTING row);
//! - the 10 s auth-exchange deadline (§3) and the 5 s CLOSING grace (§5.1);
//! - TM-4/TM-5 authentication-failure rate limits: 5 per connection (inside
//!   the state machine) and 20 per sliding 60 s per source address (here) —
//!   the address budget answers with Close(302) (fixtures/error-302);
//! - the per-Client session registry: one live session per identity, the new
//!   one superseding the old with Close(303) (TM-3, fixtures/error-303);
//! - revocation polling: a revoked identity's live session is torn down with
//!   204 within 5 s (pairing.md §9.2/PR-13);
//! - log red lines: no credentials, signatures, keys, or media bytes are ever
//!   logged; authentication failures log only the §6 code and a masked
//!   source address.
//!
//! The driver owns all I/O time; [`StateMachine`] stays pure domain logic and
//! is fed one [`Event`] per occurrence.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::auth_store::AuthorizationStore;
use crate::codes::ErrorCode;
use crate::framing::{encode_frame, FrameReader};
use crate::identity::ServerIdentity;
use crate::limits::{AddressRateGuard, Limits};
use crate::schema::{self, frame, Frame};
use crate::session::Termination;
use crate::state_machine::{Event, Outcome, ServerConfig, ServerState, StateMachine};
use crate::transport::{classify_close, CLOSING_GRACE};

/// The TLS-Exporter label for the auth context's TLV(6) (pairing.md §9.1;
/// the Android client derives the same value from the same TLS connection).
pub const AUTH_EXPORTER_LABEL: &[u8] = b"EXPORTER audio-share v2 auth";

/// Auth-exchange deadline: `AuthInit → AuthResult` outcome must complete
/// inside 10 s (spec §3); the deadline starts at transport establishment and
/// only applies while the machine is CONNECTING/AUTHENTICATING.
pub const AUTH_EXCHANGE_DEADLINE: Duration = Duration::from_secs(10);

/// Revocation polling interval: a revoked identity's live session must be
/// torn down within 5 s (pairing.md §9.2/PR-13). 2 s leaves margin for the
/// Error(204)+Close(204)+transport-close round.
pub const REVOCATION_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Events the driver injects into a running session besides wire frames.
#[derive(Clone, Debug)]
enum DriverEvent {
    /// The session registry admitted a newer session of the same identity:
    /// this (old) session emits Close(303) and transport-closes (TM-3).
    Superseded(ErrorCode),
    /// The Client opened a second stream: Error(402)+Close(402) (§5.2).
    SecondStream,
}

/// One live session in the per-Client registry.
struct SessionHandle {
    session_id: u64,
    events: mpsc::UnboundedSender<DriverEvent>,
}

/// The v2 Server session layer: shared configuration + the per-Client
/// registry + the source-address rate guard. One per endpoint; cheap to
/// share across the accept loop's spawned tasks via `Arc`.
pub struct SessionServer {
    /// The Server's raw Ed25519 identity public key (auth_context TLV(4)).
    pub server_public_key: Vec<u8>,
    /// Accepted protocol minor within major 2.
    pub protocol_minor: u32,
    /// The current Transfer Format pushed after Negotiate/RequestConfiguration.
    pub current_configuration: Option<schema::Configuration>,
    /// Whether a capture instance is active (Subscribe → StreamStarted).
    pub capture_active: bool,
    /// Whether the capture backend is healthy (README Subscribe guard).
    pub capture_healthy: bool,
    /// Server-wide limits (spec §10); auth-failure fields feed TM-4/TM-5.
    pub limits: Limits,
    /// Auth-exchange deadline override (tests shorten it).
    pub auth_deadline: Duration,
    /// Revocation poll interval override (tests shorten it).
    pub revocation_poll: Duration,
    /// CLOSING grace after a Close was sent/received (§5.1; tests shorten it).
    pub closing_grace: Duration,
    store: Arc<AuthorizationStore>,
    registry: Mutex<HashMap<Vec<u8>, SessionHandle>>,
    next_session_id: AtomicU64,
    rate: Mutex<AddressRateGuard>,
}

impl std::fmt::Debug for SessionServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionServer")
            .field("protocol_minor", &self.protocol_minor)
            .field("capture_active", &self.capture_active)
            .field("capture_healthy", &self.capture_healthy)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl SessionServer {
    /// Build the session layer for a Server identity and authorization store.
    pub fn new(identity: &ServerIdentity, store: Arc<AuthorizationStore>) -> Self {
        let limits = Limits::defaults();
        Self {
            server_public_key: identity.public_key_raw().to_vec(),
            protocol_minor: 0,
            current_configuration: None,
            capture_active: false,
            capture_healthy: true,
            rate: Mutex::new(AddressRateGuard::new(
                limits.auth_failure_window,
                limits.auth_failures_per_window,
            )),
            limits,
            auth_deadline: AUTH_EXCHANGE_DEADLINE,
            revocation_poll: REVOCATION_POLL_INTERVAL,
            closing_grace: CLOSING_GRACE,
            store,
            registry: Mutex::new(HashMap::new()),
            next_session_id: AtomicU64::new(1),
        }
    }

    /// Drive one admitted QUIC connection through the §5.2 control plane
    /// until the session terminates. Returns how the transport ended.
    pub async fn serve_connection(
        &self,
        connection: quinn::Connection,
        remote_address: Option<SocketAddr>,
    ) -> Termination {
        let peer = remote_address.unwrap_or_else(|| connection.remote_address());
        let source = peer.ip();
        let masked = masked_source(source);
        tracing::debug!(source = %masked, "v2 session: transport established");

        // The auth context TLV(6) is connection-scoped: both peers derive it
        // from the same TLS session (pairing.md §9.1). An exporter failure is
        // a local TLS misconfiguration — fail the connection with 599 rather
        // than run an authentication that can never succeed.
        let mut exporter = [0u8; 32];
        if let Err(error) =
            connection.export_keying_material(&mut exporter, AUTH_EXPORTER_LABEL, b"")
        {
            tracing::error!(source = %masked, ?error, "v2 session: TLS exporter unavailable");
            connection.close(quinn::VarInt::from_u32(ErrorCode::INTERNAL_ERROR.code), b"");
            return Termination::ServerClosed(ErrorCode::INTERNAL_ERROR);
        }

        // The control stream is the first Client-initiated bidirectional
        // stream (§3). A Client that never opens one hits the auth-exchange
        // deadline (§3: exceed → Server closes, 601).
        let (mut send, mut recv) = match tokio::time::timeout(
            self.auth_deadline,
            connection.accept_bi(),
        )
        .await
        {
            Ok(Ok(streams)) => streams,
            Ok(Err(error)) => {
                tracing::debug!(source = %masked, error = %error, "v2 session: control stream unavailable");
                return classify_close(&connection.closed().await);
            }
            Err(_elapsed) => {
                tracing::info!(source = %masked, code = 601u32, "v2 session: no control stream within the auth deadline");
                connection.close(
                    quinn::VarInt::from_u32(ErrorCode::NETWORK_FAILURE.code),
                    b"",
                );
                return Termination::ServerClosed(ErrorCode::NETWORK_FAILURE);
            }
        };
        tracing::debug!(source = %masked, "v2 session: control stream open");

        // A second Client stream is a protocol violation (at most one Client
        // stream, §3) answered with Error(402)+Close(402) (§5.2).
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let watcher_connection = connection.clone();
        let watcher_tx = event_tx.clone();
        let watcher = tokio::spawn(async move {
            while let Ok(_second) = watcher_connection.accept_bi().await {
                if watcher_tx.send(DriverEvent::SecondStream).is_err() {
                    break;
                }
            }
        });

        let session_id_base = self.next_session_id.fetch_add(1, Ordering::SeqCst);
        let config = ServerConfig {
            server_public_key: self.server_public_key.clone(),
            protocol_minor: self.protocol_minor,
            current_configuration: self.current_configuration.clone(),
            capture_active: self.capture_active,
            capture_healthy: self.capture_healthy,
            max_auth_failures_per_connection: self.limits.max_auth_failures_per_connection,
            server_random_override: None,
            // The driver always supplies the real per-connection exporter;
            // fixture replay supplies the recorded constant.
            auth_exporter_override: Some(exporter.to_vec()),
            keep_old_on_supersede: crate::state_machine::SUPERSEDE_KEEP_OLD,
            session_id_base,
        };
        let mut machine = StateMachine::new(config, &self.store);
        let mut reader = FrameReader::default();
        let mut buffer = vec![0u8; 4096];
        let mut source_bytes = source.to_string().into_bytes();

        // Deadline/grace timers.
        let mut auth_deadline = Box::pin(tokio::time::sleep_until(
            Instant::now() + self.auth_deadline,
        ));
        let mut auth_deadline_fired = false;
        let mut revocation =
            tokio::time::interval_at(Instant::now() + self.revocation_poll, self.revocation_poll);
        revocation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut grace = Box::pin(tokio::time::sleep(Duration::from_secs(3600)));
        let mut grace_armed = false;
        // The Close code this driver sent/received; the transport close
        // carries it when the grace expires (§5.1).
        let mut closing_code: Option<ErrorCode> = None;
        let mut registered: Option<u64> = None;
        let mut closed_by_peer: Option<quinn::ConnectionError> = None;

        loop {
            tokio::select! {
                read = recv.read(&mut buffer) => {
                    match read {
                        Ok(Some(count)) => {
                            let frames = match reader.push(&buffer[..count]) {
                                Ok(frames) => frames,
                                Err(error) => {
                                    // Length-prefix / payload validation failed
                                    // before message dispatch (§4.4 → 401).
                                    tracing::info!(source = %masked, code = 401u32, "v2 session: malformed control framing");
                                    let outcomes = machine.handle(Event::Fatal(error.error_code()));
                                    if let Some(termination) = self
                                        .flush(
                                            &connection,
                                            &mut send,
                                            outcomes,
                                            &mut closing_code,
                                            &mut grace,
                                            &mut grace_armed,
                                        )
                                        .await
                                    {
                                        watcher.abort();
                                        self.release(&machine, registered);
                                        return termination;
                                    }
                                    self.sync_registry(&machine, &event_tx, &mut registered);
                                    continue;
                                }
                            };
                            for inbound in frames {
                                let request_is_close = matches!(
                                    inbound.frame.body,
                                    Some(frame::Body::Close(_))
                                );
                                let received_close_code = match &inbound.frame.body {
                                    Some(frame::Body::Close(close)) => {
                                        ErrorCode::from_code(close.code)
                                    }
                                    _ => None,
                                };
                                // TM-5: a source that exhausted its address
                                // budget is answered with Close(302) before
                                // any further authentication work (§5.2).
                                if matches!(
                                    machine.state(),
                                    ServerState::Connecting | ServerState::Authenticating
                                ) && self.source_is_limited(source)
                                {
                                    tracing::info!(source = %masked, code = 302u32, "v2 session: source auth-failure budget exhausted");
                                    let outcomes = vec![rate_limited_outcome()];
                                    if let Some(termination) = self
                                        .flush(
                                            &connection,
                                            &mut send,
                                            outcomes,
                                            &mut closing_code,
                                            &mut grace,
                                            &mut grace_armed,
                                        )
                                        .await
                                    {
                                        watcher.abort();
                                        self.release(&machine, registered);
                                        return termination;
                                    }
                                    continue;
                                }
                                let before = machine.auth_failures();
                                let mut outcomes = machine.handle(Event::Frame {
                                    frame: Box::new(inbound.frame),
                                    source: std::mem::take(&mut source_bytes),
                                });
                                let after = machine.auth_failures();
                                if after > before {
                                    // Red line: log the registry code + a
                                    // masked source only — never the proof,
                                    // the key, or the identity.
                                    tracing::info!(
                                        source = %masked,
                                        code = 203u32,
                                        "v2 session: authentication failed"
                                    );
                                    if self.note_auth_failure(source) {
                                        tracing::info!(source = %masked, code = 302u32, "v2 session: source auth-failure budget reached");
                                        outcomes = vec![rate_limited_outcome()];
                                    }
                                }
                                if request_is_close
                                    && machine.state() == ServerState::Closing
                                    && !grace_armed
                                {
                                    closing_code = received_close_code;
                                    grace.as_mut().reset(Instant::now() + self.closing_grace);
                                    grace_armed = true;
                                }
                                if let Some(termination) = self
                                    .flush(
                                        &connection,
                                        &mut send,
                                        outcomes,
                                        &mut closing_code,
                                        &mut grace,
                                        &mut grace_armed,
                                    )
                                    .await
                                {
                                    watcher.abort();
                                    self.release(&machine, registered);
                                    return termination;
                                }
                                self.sync_registry(&machine, &event_tx, &mut registered);
                            }
                        }
                        // Stream EOF/reset without a transport close: treat
                        // as transport loss (601) — the control stream lives
                        // for the whole session (§3).
                        Ok(None) | Err(_) => {
                            let error = connection.closed().await;
                            closed_by_peer = Some(error);
                            break;
                        }
                    }
                }
                event = event_rx.recv() => {
                    let Some(event) = event else { break };
                    let outcomes = match event {
                        DriverEvent::Superseded(code) => machine.handle(Event::Superseded { code }),
                        DriverEvent::SecondStream => machine.handle(Event::Fatal(ErrorCode::INVALID_STATE)),
                    };
                    if let Some(termination) = self
                        .flush(
                            &connection,
                            &mut send,
                            outcomes,
                            &mut closing_code,
                            &mut grace,
                            &mut grace_armed,
                        )
                        .await
                    {
                        watcher.abort();
                        self.release(&machine, registered);
                        return termination;
                    }
                    self.sync_registry(&machine, &event_tx, &mut registered);
                }
                _ = &mut auth_deadline, if !auth_deadline_fired => {
                    auth_deadline_fired = true;
                    let outcomes = machine.handle(Event::AuthDeadline);
                    if let Some(termination) = self
                        .flush(
                            &connection,
                            &mut send,
                            outcomes,
                            &mut closing_code,
                            &mut grace,
                            &mut grace_armed,
                        )
                        .await
                    {
                        watcher.abort();
                        self.release(&machine, registered);
                        return termination;
                    }
                }
                _ = revocation.tick() => {
                    let outcomes = machine.handle(Event::RevocationCheck);
                    if let Some(termination) = self
                        .flush(
                            &connection,
                            &mut send,
                            outcomes,
                            &mut closing_code,
                            &mut grace,
                            &mut grace_armed,
                        )
                        .await
                    {
                        watcher.abort();
                        self.release(&machine, registered);
                        return termination;
                    }
                }
                error = connection.closed() => {
                    closed_by_peer = Some(error);
                    break;
                }
                _ = &mut grace, if grace_armed => {                    let code = closing_code.unwrap_or(ErrorCode::NETWORK_FAILURE);
                    connection.close(quinn::VarInt::from_u32(code.code), b"");
                    watcher.abort();
                    self.release(&machine, registered);
                    return Termination::ServerClosed(code);
                }
            }
        }

        watcher.abort();
        self.release(&machine, registered);
        let termination = match closed_by_peer {
            Some(error) => {
                let classified = classify_close(&error);
                // If we authored the Close, report the server side of the
                // termination even when the peer's transport close arrived
                // first.
                match closing_code {
                    Some(code) if classified.code() == code => Termination::ServerClosed(code),
                    _ => classified,
                }
            }
            None => Termination::Lost(ErrorCode::NETWORK_FAILURE),
        };
        tracing::debug!(source = %masked, code = termination.code().code, "v2 session ended");
        termination
    }

    /// Write machine outcomes in order. Returns a termination when the
    /// transport was closed (immediately, or after a failed write).
    #[allow(clippy::too_many_arguments)]
    async fn flush(
        &self,
        connection: &quinn::Connection,
        send: &mut quinn::SendStream,
        outcomes: Vec<Outcome>,
        closing_code: &mut Option<ErrorCode>,
        grace: &mut std::pin::Pin<Box<tokio::time::Sleep>>,
        grace_armed: &mut bool,
    ) -> Option<Termination> {
        for outcome in outcomes {
            let Outcome {
                frame,
                close_code,
                transport_close_code,
            } = outcome;
            let has_frame = frame.is_some();
            if let Some(frame) = frame {
                let bytes = match encode_frame(&frame) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        // A frame we produced failed encoding — a local bug;
                        // fail the session with 599 rather than send silence.
                        tracing::error!(error = %error, "v2 session: encode failure");
                        connection
                            .close(quinn::VarInt::from_u32(ErrorCode::INTERNAL_ERROR.code), b"");
                        return Some(Termination::ServerClosed(ErrorCode::INTERNAL_ERROR));
                    }
                };
                if let Err(error) = send.write_all(&bytes).await {
                    tracing::debug!(error = %error, "v2 session: control write failed");
                    return Some(Termination::Lost(ErrorCode::NETWORK_FAILURE));
                }
            }
            if let Some(code) = transport_close_code {
                if has_frame {
                    // Frame + transport close: send the Close frame and let
                    // the peer transport-close first; the grace timer forces
                    // the same code otherwise (§5.1). This avoids dropping
                    // the Close frame by closing the transport in the same
                    // instant it was queued.
                    *closing_code = Some(code);
                    let _ = send.finish();
                    grace.as_mut().reset(Instant::now() + self.closing_grace);
                    *grace_armed = true;
                } else {
                    // Transport close only (301/601-style rows).
                    connection.close(quinn::VarInt::from_u32(code.code), b"");
                    return Some(Termination::ServerClosed(code));
                }
            } else if let Some(code) = close_code {
                // Fatal class: Error + Close frames were written; the
                // transport close carries the same code after the grace.
                *closing_code = Some(code);
                let _ = send.finish();
                grace.as_mut().reset(Instant::now() + self.closing_grace);
                *grace_armed = true;
            }
        }
        None
    }

    /// Register this session in the per-Client registry; a newer session of
    /// the same identity supersedes the old one (TM-3).
    fn sync_registry(
        &self,
        machine: &StateMachine,
        events: &mpsc::UnboundedSender<DriverEvent>,
        registered: &mut Option<u64>,
    ) {
        let (Some(identity), Some(session_id)) = (machine.identity(), machine.session_id()) else {
            return;
        };
        if *registered == Some(session_id) {
            return;
        }
        let mut registry = self.registry.lock().expect("session registry lock");
        let previous = registry.insert(
            identity.to_vec(),
            SessionHandle {
                session_id,
                events: events.clone(),
            },
        );
        if let Some(previous) = previous {
            if previous.session_id != session_id {
                // The old session's driver turns this into Close(303).
                let _ = previous
                    .events
                    .send(DriverEvent::Superseded(ErrorCode::SESSION_SUPERSEDED));
            }
        }
        *registered = Some(session_id);
    }

    /// Remove this session from the registry unless a newer session replaced
    /// it in the meantime.
    fn release(&self, machine: &StateMachine, registered: Option<u64>) {
        let (Some(identity), Some(session_id)) = (machine.identity(), machine.session_id()) else {
            return;
        };
        if registered != Some(session_id) {
            return;
        }
        let mut registry = self.registry.lock().expect("session registry lock");
        let matches = registry
            .get(identity)
            .is_some_and(|handle| handle.session_id == session_id);
        if matches {
            registry.remove(identity);
        }
    }

    fn source_is_limited(&self, source: IpAddr) -> bool {
        self.rate
            .lock()
            .expect("address rate guard lock")
            .is_limited(source, std::time::Instant::now())
    }

    /// Record one authentication failure against the source address; returns
    /// true when the source reached its budget (Close(302)).
    fn note_auth_failure(&self, source: IpAddr) -> bool {
        self.rate
            .lock()
            .expect("address rate guard lock")
            .record(source, std::time::Instant::now())
    }
}

/// The §5.2 rate-limited row: Close(302) as a control frame, transport close
/// carrying the same code, no paired Error (fixtures/error-302-rate-limited).
fn rate_limited_outcome() -> Outcome {
    Outcome {
        frame: Some(Frame {
            body: Some(frame::Body::Close(schema::Close {
                code: ErrorCode::RATE_LIMITED.code,
                detail: String::new(),
            })),
        }),
        close_code: Some(ErrorCode::RATE_LIMITED),
        transport_close_code: None,
    }
}

/// Log red line: authentication failures name the source only in masked form.
fn masked_source(source: IpAddr) -> String {
    match source {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            format!("{}.{}.x.x", octets[0], octets[1])
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            format!("{:x}:{:x}::", segments[0], segments[1])
        }
    }
}
