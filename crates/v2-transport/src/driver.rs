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
use crate::limits::{outbound_limit_violation, AddressRateGuard, Limits, OutstandingRequests};
use crate::schema::{self, frame, Frame};
use crate::session::Termination;
use crate::state_machine::{Event, Outcome, ServerConfig, ServerState, StateMachine};
use crate::transport::{classify_close, CLOSING_GRACE};

/// The TLS-Exporter label for the auth context's TLV(6) (pairing.md §9.1;
/// the Android client derives the same value from the same TLS connection).
pub const AUTH_EXPORTER_LABEL: &[u8] = b"EXPORTER audio-share v2 auth";

/// The connection-scoped pairing exporter. quinn's export call is
/// synchronous, so the pure state machine receives this seam and never
/// depends on quinn (pairing.md §6.3).
#[derive(Clone)]
struct ConnectionExporter {
    connection: quinn::Connection,
}

impl std::fmt::Debug for ConnectionExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionExporter").finish_non_exhaustive()
    }
}

impl crate::pairing::PairingExporter for ConnectionExporter {
    fn export(&self, label: &[u8], context: &[u8], len: usize) -> Option<Vec<u8>> {
        let mut output = vec![0u8; len];
        self.connection
            .export_keying_material(output.as_mut_slice(), label, context)
            .ok()?;
        Some(output)
    }
}

/// Auth-exchange deadline: `AuthInit → AuthResult` outcome must complete
/// inside 10 s (spec §3); the deadline starts at transport establishment and
/// only applies while the machine is CONNECTING/AUTHENTICATING.
pub const AUTH_EXCHANGE_DEADLINE: Duration = Duration::from_secs(10);

/// Revocation polling interval: a revoked identity's live session must be
/// torn down within 5 s (pairing.md §9.2/PR-13). 2 s leaves margin for the
/// Error(204)+Close(204)+transport-close round.
pub const REVOCATION_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// How often the session layer polls the shared capture (§7.4 media tick).
const MEDIA_TICK: Duration = Duration::from_millis(10);
/// Cap on datagrams produced by one media tick, so one tick never monopolizes
/// the runtime. The §7.4 pacer and the 256-datagram queue bound the rest.
const MAX_DATAGRAMS_PER_TICK: usize = 64;

/// One session's media task: owns the pump, sends DATAGRAMs, and reports a
/// fatal transport limitation back to the control loop. Ends when dropped.
struct MediaTask {
    handle: tokio::task::JoinHandle<()>,
    generation: tokio::sync::watch::Sender<u32>,
}

impl MediaTask {
    fn spawn(
        connection: quinn::Connection,
        mut chunks: tokio::sync::mpsc::Receiver<crate::media_source::MediaChunk>,
        stream: crate::media_source::StreamState,
        events: mpsc::UnboundedSender<DriverEvent>,
        session_id: u64,
        stream_instance_id: u32,
    ) -> Self {
        let (generation, generation_rx) = tokio::sync::watch::channel(stream.config_generation);
        let handle = tokio::spawn(async move {
            let mut pump = crate::media_pump::MediaPump::new(stream);
            // M5-05: bounded-cadence log of server-side drops so support can
            // distinguish pre-packetize/queue drops from Client-side network
            // loss in `server.log` (session_log event → tracing_bridge).
            let mut drop_reporter = crate::media_pump::DropReporter::new(Duration::from_secs(5));
            // M5-08: a second, unbiased 5 s snapshot emitted even with zero
            // drops, so pending/pacer state is not only sampled at drop
            // instants.
            let mut stats_reporter =
                crate::media_pump::MediaStatsReporter::new(Duration::from_secs(5));
            // §7.4/§10: encoded media goes into a bounded 256-datagram,
            // non-waiting per-Client queue before `send_datagram`. A stalled
            // Client therefore drops the newest media, never blocks capture.
            let mut queue = crate::send_queue::MediaSendQueue::new();
            let mut generation_rx = generation_rx;
            // The receiver starts "changed"; mark the initial value seen so the
            // first tick does not try to re-apply the generation it already has.
            generation_rx.borrow_and_update();
            let mut tick = tokio::time::interval(MEDIA_TICK);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = tick.tick() => {}
                    received = chunks.recv() => {
                        match received {
                            // The capture stream ended; the control loop owns
                            // the StreamEnded push and tears the task down.
                            None => return,
                            // §7.4: a buffer that arrives while this task is
                            // parked on `select!` is media, not a wake-up
                            // signal. Dropping it here would starve the pump to
                            // the few buffers left in the channel backlog.
                            Some(chunk) => {
                                if pump.accept(chunk).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
                // Drain every buffered capture chunk (bounded by the pump).
                loop {
                    match chunks.try_recv() {
                        Ok(chunk) => {
                            if pump.accept(chunk).is_err() {
                                return;
                            }
                        }
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return,
                    }
                }
                if generation_rx.has_changed().unwrap_or(false) {
                    let latest = *generation_rx.borrow_and_update();
                    if pump.set_generation(latest).is_err() {
                        let _ = events.send(DriverEvent::MediaDatagramUnsupported);
                        return;
                    }
                }
                // §7.3/D2: the limit is queried at send time, never cached.
                let Some(max_datagram_size) = connection.max_datagram_size() else {
                    let _ = events.send(DriverEvent::MediaDatagramUnsupported);
                    return;
                };
                let now = std::time::Instant::now();
                let datagrams =
                    match pump.take_sendable(now, max_datagram_size, MAX_DATAGRAMS_PER_TICK) {
                        Ok(datagrams) => datagrams,
                        Err(crate::media_pump::MediaPumpError::DatagramUnsupported) => {
                            let _ = events.send(DriverEvent::MediaDatagramUnsupported);
                            return;
                        }
                        Err(crate::media_pump::MediaPumpError::MisalignedPcm) => return,
                    };
                queue.enqueue(datagrams);
                // M5-05: report changed server-side drop counters at the 5 s
                // diagnostics cadence (tracing → tracing_bridge → server.log).
                if let Some(report) = drop_reporter.poll(
                    now,
                    pump.dropped_chunks(),
                    queue.dropped_newest(),
                    pump.pending_frames(),
                ) {
                    crate::session_log::media_drops(
                        session_id,
                        stream_instance_id,
                        report.pump_dropped_chunks,
                        report.queue_dropped_newest,
                        report.pending_frames,
                    );
                }
                // M5-08: emit the periodic pump snapshot every 5 s even when
                // nothing was dropped. Reports pending frames, pacer-consumed
                // frames and the pacer call rate.
                if let Some(stats) = stats_reporter.poll(
                    now,
                    pump.pending_frames(),
                    pump.pacer_consumed_frames(),
                    pump.pacer_calls(),
                ) {
                    crate::session_log::media_stats(
                        session_id,
                        stream_instance_id,
                        stats.pending_frames,
                        stats.consumed_frames,
                        stats.pacer_calls,
                        stats.pacer_calls_per_sec,
                    );
                }
                // Non-waiting drain: one pop per send, no await, no blocking.
                // quinn consumes the Bytes on every outcome and does not return
                // it on error, so keep the Vec to requeue a TooLarge refusal.
                while let Some(datagram) = queue.pop_front() {
                    match connection.send_datagram(bytes::Bytes::from(datagram.clone())) {
                        Ok(()) => {}
                        Err(quinn::SendDatagramError::TooLarge) => {
                            // The path limit shrank after the send-time query:
                            // keep the datagram for the next (smaller) limit.
                            // §7.4: a send refusal is not a media drop.
                            queue.push_front(datagram);
                            break;
                        }
                        Err(quinn::SendDatagramError::UnsupportedByPeer)
                        | Err(quinn::SendDatagramError::Disabled) => {
                            queue.clear();
                            let _ = events.send(DriverEvent::MediaDatagramUnsupported);
                            return;
                        }
                        Err(quinn::SendDatagramError::ConnectionLost(_)) => {
                            queue.clear();
                            return;
                        }
                    }
                }
            }
        });
        Self { handle, generation }
    }

    fn update_generation(&mut self, generation: u32) {
        // Only a real change notifies the media task (§7.2: identical content
        // keeps the generation).
        self.generation.send_if_modified(|current| {
            if *current == generation {
                false
            } else {
                *current = generation;
                true
            }
        });
    }
}

impl Drop for MediaTask {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// The §5.2 pushes implied by a capture-status transition.
fn stream_events(
    previous: Option<&crate::media_source::MediaStatus>,
    current: &crate::media_source::MediaStatus,
) -> Vec<crate::state_machine::StreamLifecycle> {
    use crate::media_source::MediaStatus;
    use crate::state_machine::StreamLifecycle;
    match (previous, current) {
        (Some(MediaStatus::Active(old)), MediaStatus::Active(new))
            if old.stream_instance_id == new.stream_instance_id =>
        {
            if old.config_generation == new.config_generation {
                Vec::new()
            } else {
                vec![StreamLifecycle::ConfigChanged {
                    config_generation: new.config_generation,
                }]
            }
        }
        (Some(MediaStatus::Active(old)), MediaStatus::Active(new)) => {
            vec![StreamLifecycle::Restarted {
                old_stream_instance_id: old.stream_instance_id,
                stream_instance_id: new.stream_instance_id,
                config_generation: new.config_generation,
                initial_timestamp: new.next_frame,
            }]
        }
        (_, MediaStatus::Active(new)) => vec![StreamLifecycle::Started {
            stream_instance_id: new.stream_instance_id,
            config_generation: new.config_generation,
            initial_timestamp: new.next_frame,
        }],
        (Some(MediaStatus::Active(old)), MediaStatus::Ended { code }) => {
            vec![StreamLifecycle::Ended {
                stream_instance_id: old.stream_instance_id,
                code: if *code == ErrorCode::CAPTURE_FAILURE.code {
                    ErrorCode::CAPTURE_FAILURE
                } else {
                    ErrorCode::STREAM_ENDED
                },
            }]
        }
        _ => Vec::new(),
    }
}

#[derive(Clone, Debug)]
enum DriverEvent {
    /// The session registry admitted a newer session of the same identity:
    /// this (old) session emits Close(303) and transport-closes (TM-3).
    Superseded(ErrorCode),
    /// The Client opened a second stream: Error(402)+Close(402) (§5.2).
    SecondStream,
    /// The media task found the transport cannot carry datagrams (§7.3 → 602).
    MediaDatagramUnsupported,
    /// The local operator approved a verified pairing (from the Pairing Core
    /// decision callback; never a blocking UI call inside `handle`). The
    /// payload is the process-local `approval_id`, not the wire request id.
    PairingApproved(u64),
    /// The local operator declined a verified pairing.
    PairingDeclined(u64),
}

/// One live session in the per-Client registry.
struct SessionHandle {
    session_id: u64,
    events: mpsc::UnboundedSender<DriverEvent>,
}

// 为调用者保留既有 driver::CaptureSnapshot 导入路径。
pub use crate::state_machine::CaptureSnapshot;

/// 每个控制请求前读取的实时 capture 快照来源。
pub type CaptureStateSource = dyn Fn() -> CaptureSnapshot + Send + Sync;

/// The v2 Server session layer: shared configuration + the per-Client
/// registry + the source-address rate guard. One per endpoint; cheap to
/// share across the accept loop's spawned tasks via `Arc`.
pub struct SessionServer {
    /// The Server's raw Ed25519 identity public key (auth_context TLV(4)).
    pub server_public_key: Vec<u8>,
    /// Accepted protocol minor within major 2.
    pub protocol_minor: u32,
    /// 未设置实时来源时使用的静态 capture 观察值。
    pub capture: CaptureSnapshot,
    /// 可选实时 capture 来源；设置后在每个控制请求前覆盖 `capture`。
    pub capture_source: Option<Arc<CaptureStateSource>>,
    /// The shared capture's media (PCM chunks + stream lifecycle). When set,
    /// a subscribed session receives DATAGRAM media (§7).
    pub media_source: Option<Arc<dyn crate::media_source::MediaSource>>,
    /// The production Pairing Core shared with the CLI/GUI. `None` refuses
    /// PairingRequest (201) but auth and media are unaffected.
    pub pairing: Option<Arc<crate::pairing::PairingCore>>,
    /// Latest §9 playback diagnostics per session id — the only playback
    /// evidence the Server keeps (a QUIC ACK is never proof).
    diagnostics: Mutex<HashMap<u64, crate::schema::PlaybackDiagnostics>>,
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
            .field("capture_active", &self.capture.stream_active)
            .field("capture_healthy", &self.capture.healthy)
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
            capture: CaptureSnapshot::absent(),
            capture_source: None,
            media_source: None,
            pairing: None,
            diagnostics: Mutex::new(HashMap::new()),
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

    fn capture_snapshot(&self) -> CaptureSnapshot {
        self.capture_source
            .as_ref()
            .map_or_else(|| self.capture.clone(), |source| source())
    }

    /// The latest §9 playback diagnostics a session reported (§9: diagnostics
    /// are the only playback evidence; a QUIC ACK is not decode or playback).
    pub fn playback_diagnostics(
        &self,
        session_id: u64,
    ) -> Option<crate::schema::PlaybackDiagnostics> {
        self.diagnostics
            .lock()
            .expect("diagnostics lock")
            .get(&session_id)
            .cloned()
    }

    fn record_diagnostics(&self, session_id: u64, diagnostics: crate::schema::PlaybackDiagnostics) {
        let mut map = self.diagnostics.lock().expect("diagnostics lock");
        map.insert(session_id, diagnostics);
        // §10: bounded per-session state only; drop entries for sessions that
        // no longer exist so a long-lived Server cannot accumulate them.
        let registry = self.registry.lock().expect("session registry lock");
        let live: Vec<u64> = registry.values().map(|handle| handle.session_id).collect();
        drop(registry);
        map.retain(|id, _| live.contains(id) || *id == session_id);
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
            capture: self.capture_snapshot(),
            max_auth_failures_per_connection: self.limits.max_auth_failures_per_connection,
            #[cfg(feature = "test-support")]
            server_random_override: None,
            // The driver always supplies the real per-connection exporter;
            // fixture replay supplies the recorded constant.
            auth_exporter: Some(exporter.to_vec()),
            session_id_base,
            pairing: self.pairing.clone(),
            pairing_exporter: Some(Arc::new(ConnectionExporter {
                connection: connection.clone(),
            })),
            #[cfg(feature = "test-support")]
            pairing_server_random_override: None,
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
        // §5.2: an admitted pairing exchange replaces the 10 s auth deadline
        // with the 10 min pairing deadline.
        let mut pairing_deadline = Box::pin(tokio::time::sleep(Duration::from_secs(3600)));
        let mut pairing_deadline_armed = false;
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
        // §10: at most 8 outstanding requests per session (over → 302). The
        // current handlers reply synchronously, so this never exceeds one in
        // practice; the guard is the enforcement point for async handlers and
        // for duplicate in-flight ids.
        let mut outstanding = OutstandingRequests::new(self.limits.max_outstanding_requests);

        // Media (§7). The media task is created only while this session is
        // PLAYING and dropped the moment it is not; capture transitions become
        // ordered §5.2 pushes through the machine here.
        let mut media: Option<MediaTask> = None;
        let mut last_status = self.media_source.as_ref().map(|source| source.status());
        let mut media_tick = tokio::time::interval(MEDIA_TICK);
        media_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            // Arm/disarm the pairing deadline from the machine's state; this
            // runs after every handled event, so an approved pairing (which
            // leaves AUTHENTICATING) disarms it immediately.
            if machine.is_pairing_exchange() {
                if !pairing_deadline_armed {
                    pairing_deadline
                        .as_mut()
                        .reset(Instant::now() + crate::pairing::PAIRING_DEADLINE);
                    pairing_deadline_armed = true;
                }
            } else {
                pairing_deadline_armed = false;
            }
            tokio::select! {
                _ = media_tick.tick(), if self.media_source.is_some() => {
                    if let Some(source) = self.media_source.as_ref() {
                        let status = source.status();
                        if last_status.as_ref() != Some(&status) {
                            for event in stream_events(last_status.as_ref(), &status) {
                                let outcomes = machine.handle(Event::StreamLifecycle(event));
                                if let Some(termination) = self
                                    .flush(
                                        &connection,
                                        &mut send,
                                        outcomes,
                                        machine.identity().is_some(),
                                        &mut closing_code,
                                        &mut grace,
                                        &mut grace_armed,
                                    )
                                    .await
                                {
                                    return self.terminate(&watcher, &machine, registered, termination);
                                }
                            }
                            last_status = Some(status);
                        }
                        let playing =
                            machine.state() == ServerState::Playing && machine.subscribed();
                        if !playing {
                            media = None;
                        } else if media.is_none() {
                            if let crate::media_source::MediaStatus::Active(stream) = status {
                                let instance = stream.stream_instance_id;
                                media = Some(MediaTask::spawn(
                                    connection.clone(),
                                    source.subscribe(),
                                    stream,
                                    event_tx.clone(),
                                    machine.session_id().unwrap_or(0),
                                    instance,
                                ));
                            }
                        } else if let (Some(task), Some(stream)) = (
                            media.as_mut(),
                            match status {
                                crate::media_source::MediaStatus::Active(stream) => Some(stream),
                                _ => None,
                            },
                        ) {
                            task.update_generation(stream.config_generation);
                        }
                    }
                }
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
                                            machine.identity().is_some(),
                                            &mut closing_code,
                                            &mut grace,
                                            &mut grace_armed,
                                        )
                                        .await
                                    {
                                        return self.terminate(&watcher, &machine, registered, termination);
                                    }
                                    self.sync_registry(&machine, &event_tx, &mut registered);
                                    continue;
                                }
                            };
                            for inbound in frames {
                                let inbound_request_id = crate::state_machine::request_id_of(&inbound.frame);
                                if let Some(request_id) = inbound_request_id {
                                    if !outstanding.track(request_id) {
                                        tracing::info!(
                                            source = %masked,
                                            code = 302u32,
                                            "v2 session: outstanding-request limit reached"
                                        );
                                        let outcomes = vec![rate_limited_outcome()];
                                        if let Some(termination) = self
                                            .flush(
                                                &connection,
                                                &mut send,
                                                outcomes,
                                                machine.identity().is_some(),
                                                &mut closing_code,
                                                &mut grace,
                                                &mut grace_armed,
                                            )
                                            .await
                                        {
                                            return self.terminate(&watcher, &machine, registered, termination);
                                        }
                                        continue;
                                    }
                                }
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
                                            machine.identity().is_some(),
                                            &mut closing_code,
                                            &mut grace,
                                            &mut grace_armed,
                                        )
                                        .await
                                    {
                                        return self.terminate(&watcher, &machine, registered, termination);
                                    }
                                    continue;
                                }
                                // Pick up operator edits (revoke/delete/replace)
                                // before consulting authorization: new
                                // connections see a changed store immediately
                                // (pairing.md §9.2).
                                if matches!(
                                    inbound.frame.body,
                                    Some(frame::Body::AuthInit(_) | frame::Body::AuthProof(_))
                                ) {
                                    if let Err(error) = self.store.reload_if_changed() {
                                        tracing::error!(
                                            error = %error,
                                            "v2 session: authorization store reload failed; failing closed"
                                        );
                                    }
                                }
                                // READY 会话期间 capture 可能结束（ADR-0003）；
                                // Subscribe/配置/流信息不得继续使用连接启动时的健康值。
                                machine.update_capture(self.capture_snapshot());
                                let before = machine.auth_failures();
                                let mut outcomes = machine.handle(Event::Frame {
                                    frame: Box::new(inbound.frame),
                                    source: std::mem::take(&mut source_bytes),
                                });
                                // A verified pairing is handed to the local
                                // operator (GUI/CLI). The decision returns as
                                // a DriverEvent; the state machine never
                                // blocks on human input.
                                if let Some(mut request) = machine.take_pairing_approval_request() {
                                    if let Some(pairing) = self.pairing.as_ref() {
                                        request.address = peer.to_string();
                                        let approval_id = request.approval_id;
                                        let decisions = event_tx.clone();
                                        pairing.request_approval(
                                            request,
                                            Box::new(move |approved| {
                                                let event = if approved {
                                                    DriverEvent::PairingApproved(approval_id)
                                                } else {
                                                    DriverEvent::PairingDeclined(approval_id)
                                                };
                                                let _ = decisions.send(event);
                                            }),
                                        );
                                    }
                                }
                                if let Some(request_id) = inbound_request_id {
                                    outstanding.complete(request_id);
                                }
                                // §9: a Client's PlaybackDiagnostics is the
                                // only playback evidence; keep the latest per
                                // session for low-frequency observation.
                                if let (Some(session_id), Some(diagnostics)) = (
                                    machine.session_id(),
                                    machine.last_playback_diagnostics().cloned(),
                                ) {
                                    self.record_diagnostics(session_id, diagnostics);
                                }
                                let after = machine.auth_failures();
                                if let Some(code) = machine.take_auth_failure_code() {
                                    // Red line: log the registry code + a
                                    // masked source only — never the proof,
                                    // the key, or the identity. The code is
                                    // the actual failure (202/203/204/401),
                                    // never a blanket 203.
                                    tracing::info!(
                                        source = %masked,
                                        code = code.code,
                                        "v2 session: authentication failed"
                                    );
                                }
                                if let Some(code) = machine.take_pairing_failure_code() {
                                    // Pairing red line: code + masked source
                                    // only, never the nonce, SAS, binding, or
                                    // client_proof.
                                    tracing::info!(
                                        source = %masked,
                                        code = code.code,
                                        "v2 session: pairing failed"
                                    );
                                }
                                if after > before && self.note_auth_failure(source) {
                                    tracing::info!(source = %masked, code = 302u32, "v2 session: source auth-failure budget reached");
                                    outcomes = vec![rate_limited_outcome()];
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
                                        machine.identity().is_some(),
                                        &mut closing_code,
                                        &mut grace,
                                        &mut grace_armed,
                                    )
                                    .await
                                {
                                    return self.terminate(&watcher, &machine, registered, termination);
                                }
                                // The live pairing view is published only
                                // after PairingResponse was written (ticket
                                // 29). A failed write returned above, so no
                                // local surface sees a pairing the Client did
                                // not receive.
                                if let Some(mut presented) = machine.take_pairing_presented() {
                                    if let Some(pairing) = self.pairing.as_ref() {
                                        // Ticket 33: the operator-facing log
                                        // records the public client id and the
                                        // mode only — never the SAS.
                                        crate::session_log::pairing_presented(
                                            &presented.client_id,
                                            crate::session_log::pairing_mode(presented.mode),
                                        );
                                        presented.address = peer.to_string();
                                        pairing.publish_presented(presented);
                                    }
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
                        DriverEvent::MediaDatagramUnsupported => {
                            machine.handle(Event::Fatal(ErrorCode::DATAGRAM_UNSUPPORTED))
                        }
                        DriverEvent::PairingApproved(approval_id) => {
                            machine.handle(Event::PairingApproved { approval_id })
                        }
                        DriverEvent::PairingDeclined(approval_id) => {
                            machine.handle(Event::PairingDeclined { approval_id })
                        }
                    };
                    if let Some(termination) = self
                        .flush(
                            &connection,
                            &mut send,
                            outcomes,
                            machine.identity().is_some(),
                            &mut closing_code,
                            &mut grace,
                            &mut grace_armed,
                        )
                        .await
                    {
                        return self.terminate(&watcher, &machine, registered, termination);
                    }
                    self.sync_registry(&machine, &event_tx, &mut registered);
                }
                _ = &mut auth_deadline, if !auth_deadline_fired && !machine.is_pairing_exchange() => {
                    auth_deadline_fired = true;
                    let outcomes = machine.handle(Event::AuthDeadline);
                    if let Some(termination) = self
                        .flush(
                            &connection,
                            &mut send,
                            outcomes,
                            machine.identity().is_some(),
                            &mut closing_code,
                            &mut grace,
                            &mut grace_armed,
                        )
                        .await
                    {
                        return self.terminate(&watcher, &machine, registered, termination);
                    }
                }
                _ = &mut pairing_deadline, if pairing_deadline_armed => {
                    pairing_deadline_armed = false;
                    let outcomes = machine.handle(Event::PairingDeadline);
                    if let Some(code) = machine.take_pairing_failure_code() {
                        tracing::info!(
                            source = %masked,
                            code = code.code,
                            "v2 session: pairing failed"
                        );
                    }
                    if let Some(termination) = self
                        .flush(
                            &connection,
                            &mut send,
                            outcomes,
                            machine.identity().is_some(),
                            &mut closing_code,
                            &mut grace,
                            &mut grace_armed,
                        )
                        .await
                    {
                        return self.terminate(&watcher, &machine, registered, termination);
                    }
                    self.sync_registry(&machine, &event_tx, &mut registered);
                }
                _ = revocation.tick() => {
                    // Reload before the check so a deleted/edited record is
                    // visible within this ≤ 5 s window (pairing.md §9.2).
                    if let Err(error) = self.store.reload_if_changed() {
                        tracing::error!(
                            error = %error,
                            "v2 session: authorization store reload failed; failing closed"
                        );
                    }
                    let outcomes = machine.handle(Event::RevocationCheck);
                    if let Some(termination) = self
                        .flush(
                            &connection,
                            &mut send,
                            outcomes,
                            machine.identity().is_some(),
                            &mut closing_code,
                            &mut grace,
                            &mut grace_armed,
                        )
                        .await
                    {
                        return self.terminate(&watcher, &machine, registered, termination);
                    }
                }
                error = connection.closed() => {
                    closed_by_peer = Some(error);
                    break;
                }
                _ = &mut grace, if grace_armed => {
                    let code = closing_code.unwrap_or(ErrorCode::NETWORK_FAILURE);
                    connection.close(quinn::VarInt::from_u32(code.code), b"");
                    return self.terminate(&watcher, &machine, registered, Termination::ServerClosed(code));
                }
            }
        }

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
        self.terminate(&watcher, &machine, registered, termination)
    }

    /// Stop the second-stream watcher and release the registered session once
    /// on every terminal path, preserving the existing flush classification.
    fn terminate(
        &self,
        watcher: &tokio::task::JoinHandle<()>,
        machine: &StateMachine,
        registered: Option<u64>,
        termination: Termination,
    ) -> Termination {
        watcher.abort();
        if let (Some(pairing), Some(approval_id)) =
            (self.pairing.as_ref(), machine.pending_pairing_approval_id())
        {
            // The exchange ended without a decision: drop the local prompt so
            // an operator cannot approve a dead connection.
            pairing.cancel_approval(approval_id);
        }
        // Ticket 33: one structured line covers every terminal path (Client
        // stop 701, revocation 204, supersede 303, loss 601, …). Only an
        // established session has an identity and a session id.
        if let (Some(identity), Some(_)) = (machine.identity(), machine.session_id()) {
            crate::session_log::session_closed(
                &crate::auth::client_id_hex(identity),
                termination.code().code,
            );
        }
        self.release(machine, registered);
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
        authenticated: bool,
        closing_code: &mut Option<ErrorCode>,
        grace: &mut std::pin::Pin<Box<tokio::time::Sleep>>,
        grace_armed: &mut bool,
    ) -> Option<Termination> {
        for outcome in outcomes {
            let Outcome {
                frame,
                close_code,
                transport_close_code,
                paired_client,
            } = outcome;
            let has_frame = frame.is_some();
            if let Some(frame) = frame {
                // §10/TM-6/TM-7: never emit a frame over its size bound; an
                // oversized frame is a local bug, so fail closed (599).
                // §4.2 (spec:91): "senders MUST cap at 256 bytes; receivers
                // truncate for logging and MUST NOT reject a frame solely for
                // a longer detail." This is the sender-side MUST for our own
                // outbound Error, so an oversized one is a bug, not something
                // to truncate here; fail closed instead.
                if let Some(reason) = outbound_limit_violation(&frame, authenticated) {
                    tracing::error!(%reason, "v2 session: outbound frame exceeds its §10 bound");
                    connection.close(quinn::VarInt::from_u32(ErrorCode::INTERNAL_ERROR.code), b"");
                    return Some(Termination::ServerClosed(ErrorCode::INTERNAL_ERROR));
                }
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
            // The pairing success result (when present) has now been written.
            // Only here may a one-shot consumer learn about the pairing; a
            // failed/oversized/encoding-error write returned above without
            // broadcasting, so no local surface observes a pairing the Client
            // did not see.
            if let Some(client) = paired_client {
                if let Some(core) = &self.pairing {
                    core.notify_paired(client);
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
        crate::session_log::session_accepted(&crate::auth::client_id_hex(identity), session_id);
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
        paired_client: None,
    }
}

/// Log red line: authentication failures name the source only in masked form.
///
/// IPv4 keeps the first two octets, IPv6 the first two segments; the host /
/// interface part never reaches a log line (threat-model.md §TM-4 line 35).
pub fn masked_source(source: IpAddr) -> String {
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
