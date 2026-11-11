//! Single-connection session: control-frame state machine and the media pump.
//!
//! Media pump shape: a producer task paces synthetic PCM at real-time rate
//! (ticker, ~5 ms of audio per batch) and pushes datagrams into a **bounded
//! queue** with non-waiting `try_send` (overflow drops the newest and logs);
//! a forwarder task drains the queue into non-waiting `send_datagram`. A slow
//! client can therefore never block the producer, and per-connection queues
//! isolate clients from each other.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use proto_core::frame::{self, CloseReason, Frame};
use proto_core::media::{
    chunk_payload_len, encode_datagram, frame_bytes, MediaHeader, DEFAULT_CHANNELS,
    DEFAULT_SAMPLE_RATE, SAMPLE_FORMAT_F32LE,
};
use quinn::{Connection, RecvStream, SendStream};
use tokio::sync::mpsc;

use crate::stats::{self, SessionStats, STATE_CLOSING, STATE_CLOSED, STATE_PLAYING, STATE_READY};
use crate::Shared;

/// Bumped when a capture restart would occur; the prototype starts a new
/// media instance on every (re)SUBSCRIBE.
const DEFAULT_CONFIG_GENERATION: u32 = 1;

pub(super) async fn run(conn: Connection, shared: Arc<Shared>) {
    let stats = Arc::new(SessionStats::new());
    *stats.remote_addr.lock().expect("remote addr mutex") = conn.remote_address().to_string();
    shared
        .sessions
        .lock()
        .expect("session registry mutex")
        .push(stats.clone());
    stats.set_state(stats::STATE_CONNECTING);

    let (mut tx, mut rx) = match conn.accept_bi().await {
        Ok(streams) => streams,
        Err(e) => {
            tracing::debug!(error = %e, "control stream never opened");
            stats.set_state(stats::STATE_CLOSED);
            return;
        }
    };
    tracing::debug!("control stream accepted");

    let mut shutdown_rx = shared.shutdown_tx.subscribe();
    let mut shutdown_seen = false;
    let mut media: Option<MediaPump> = None;

    loop {
        let playing = media.is_some();
        let event = if playing {
            // Media is flowing: the connection is alive without control frames.
            tokio::select! {
                biased;
                _ = shutdown_rx.wait_for(|v| *v) => SessionEvent::Shutdown,
                res = read_frame(&mut rx) => frame_event(res),
                res = conn.accept_bi() => match res {
                    Ok(_) => SessionEvent::ExtraStream,
                    Err(_) => SessionEvent::TransportDead,
                },
            }
        } else {
            let idle = shared.config.idle_timeout;
            tokio::select! {
                biased;
                _ = shutdown_rx.wait_for(|v| *v) => SessionEvent::Shutdown,
                res = async {
                    match idle {
                        Some(idle) => match tokio::time::timeout(idle, read_frame(&mut rx)).await {
                            Ok(res) => frame_event(res),
                            Err(_) => SessionEvent::IdleTimeout,
                        },
                        None => frame_event(read_frame(&mut rx).await),
                    }
                } => res,
                res = conn.accept_bi() => match res {
                    Ok(_) => SessionEvent::ExtraStream,
                    Err(_) => SessionEvent::TransportDead,
                },
            }
        };

        match event {
            SessionEvent::Frame(frame) => {
                match handle_frame(&frame, &shared, &stats, &conn, &mut tx, &mut media).await {
                    Flow::Continue => {}
                    Flow::CloseWith(reason, prelude) => {
                        break_close(
                            &conn, &mut tx, &mut rx, &stats, reason, prelude, &mut shutdown_seen,
                        )
                        .await;
                        break;
                    }
                }
            }
            SessionEvent::PeerEof => {
                tracing::debug!("client half-closed the control stream without CLOSE");
                break_close(
                    &conn,
                    &mut tx,
                    &mut rx,
                    &stats,
                    CloseReason::Normal,
                    None,
                    &mut shutdown_seen,
                )
                .await;
                break;
            }
            SessionEvent::IdleTimeout => {
                break_close(
                    &conn,
                    &mut tx,
                    &mut rx,
                    &stats,
                    CloseReason::IdleTimeout,
                    None,
                    &mut shutdown_seen,
                )
                .await;
                break;
            }
            SessionEvent::Shutdown => {
                shutdown_seen = true;
                break_close(
                    &conn,
                    &mut tx,
                    &mut rx,
                    &stats,
                    CloseReason::ServerStop,
                    None,
                    &mut shutdown_seen,
                )
                .await;
                break;
            }
            SessionEvent::ExtraStream => {
                tracing::debug!("unexpected second bidirectional stream");
                break_close(
                    &conn, &mut tx, &mut rx, &stats, CloseReason::ProtocolError, None,
                    &mut shutdown_seen,
                )
                .await;
                break;
            }
            SessionEvent::ProtocolViolation(err) => {
                tracing::debug!(violation = %violation_text(&err), "protocol violation");
                let (reason, prelude) = match err {
                    // A non-utf-8 AUTH payload is an auth-stage malformation:
                    // answer AUTH_ERR(MALFORMED) then CLOSE(AUTH_FAILED).
                    ReadFrameError::Parse(frame::FrameParseError::BadUtf8) => {
                        stats.auth_err_malformed.fetch_add(1, Ordering::Relaxed);
                        (
                            CloseReason::AuthFailed,
                            Some(Frame::AuthErr(frame::AuthErrCode::Malformed)),
                        )
                    }
                    _ => (CloseReason::ProtocolError, None),
                };
                break_close(
                    &conn, &mut tx, &mut rx, &stats, reason, prelude, &mut shutdown_seen,
                )
                .await;
                break;
            }
            SessionEvent::TransportDead => {
                tracing::debug!("transport closed by peer");
                stats.set_state(stats::STATE_CLOSED);
                stats.closed.store(true, Ordering::Relaxed);
                break;
            }
        }
    }

    if let Some(pump) = media.take() {
        pump.stop().await;
    }
    if shutdown_seen {
        return; // server stopping; the endpoint is being closed by stop()
    }
}

enum SessionEvent {
    Frame(Frame),
    PeerEof,
    IdleTimeout,
    Shutdown,
    /// A second bidirectional stream was opened: the prototype allows exactly
    /// one control stream.
    ExtraStream,
    ProtocolViolation(ReadFrameError),
    TransportDead,
}

fn frame_event(res: Result<Frame, ReadFrameError>) -> SessionEvent {
    match res {
        Ok(frame) => SessionEvent::Frame(frame),
        Err(ReadFrameError::Eof) => SessionEvent::PeerEof,
        Err(ReadFrameError::Transport(_)) => SessionEvent::TransportDead,
        Err(e) => SessionEvent::ProtocolViolation(e),
    }
}

fn violation_text(e: &ReadFrameError) -> &'static str {
    match e {
        ReadFrameError::Eof => "unexpected EOF",
        ReadFrameError::EmptyFrame => "zero-length frame",
        ReadFrameError::FrameTooLarge(_) => "frame length exceeds 64 KiB cap",
        ReadFrameError::Parse(frame::FrameParseError::UnknownType(_)) => "unknown msg_type",
        ReadFrameError::Parse(frame::FrameParseError::BadPayloadLen { .. }) => "bad payload length",
        ReadFrameError::Parse(frame::FrameParseError::UnknownAuthErrCode(_)) => {
            "unknown auth error code"
        }
        ReadFrameError::Parse(frame::FrameParseError::UnknownCloseReason(_)) => {
            "unknown close reason"
        }
        ReadFrameError::Parse(frame::FrameParseError::BadUtf8) => "non-utf8 AUTH payload",
        ReadFrameError::Transport(_) => "transport error",
    }
}

enum Flow {
    Continue,
    /// Close with a reason, optionally sending a prelude frame first
    /// (e.g. AUTH_ERR before CLOSE(AUTH_FAILED)).
    CloseWith(CloseReason, Option<Frame>),
}

async fn handle_frame(
    frame: &Frame,
    shared: &Arc<Shared>,
    stats: &Arc<SessionStats>,
    conn: &Connection,
    tx: &mut SendStream,
    media: &mut Option<MediaPump>,
) -> Flow {
    match frame {
        Frame::Auth { token } => {
            stats.frames_auth_in.fetch_add(1, Ordering::Relaxed);
            match stats.state.load(Ordering::Relaxed) {
                stats::STATE_CONNECTING => {
                    stats.set_state(stats::STATE_AUTHENTICATING);
                    // Revoked tokens are denied exactly like bad credentials
                    // (same wire frames), counted under their own stat.
                    if shared.config.revoked_credentials.iter().any(|t| t == token) {
                        stats.auth_err_revoked.fetch_add(1, Ordering::Relaxed);
                        return Flow::CloseWith(
                            CloseReason::AuthFailed,
                            Some(Frame::AuthErr(frame::AuthErrCode::BadCredential)),
                        );
                    }
                    if constant_time_eq(token, &shared.config.credential) {
                        let session_id = shared
                            .next_session_id
                            .fetch_add(1, Ordering::Relaxed);
                        stats
                            .session_id
                            .store(session_id as u64, Ordering::Relaxed);
                        if let Err(e) =
                            send_frame(tx, &Frame::AuthOk { session_id }).await
                        {
                            tracing::debug!(error = %e, "AUTH_OK send failed");
                            return Flow::CloseWith(CloseReason::InternalError, None);
                        }
                        stats.auth_ok_out.fetch_add(1, Ordering::Relaxed);
                        stats.set_state(stats::STATE_READY);
                        tracing::info!(session_id, "client authenticated");
                        Flow::Continue
                    } else {
                        stats
                            .auth_err_bad_credential
                            .fetch_add(1, Ordering::Relaxed);
                        return Flow::CloseWith(
                            CloseReason::AuthFailed,
                            Some(Frame::AuthErr(frame::AuthErrCode::BadCredential)),
                        );
                    }
                }
                _ => {
                    // AUTH outside CONNECTING is a state violation.
                    Flow::CloseWith(CloseReason::ProtocolError, None)
                }
            }
        }
        Frame::Subscribe => {
            stats.frames_subscribe_in.fetch_add(1, Ordering::Relaxed);
            if stats.state.load(Ordering::Relaxed) != stats::STATE_READY {
                // SUBSCRIBE is only legal in READY. Pre-auth subscriptions get
                // an explicit AUTH_ERR(NOT_AUTHENTICATED) in addition to the
                // PROTOCOL_ERROR close.
                if stats.session_id.load(Ordering::Relaxed) == 0 {
                    stats
                        .auth_err_not_authenticated
                        .fetch_add(1, Ordering::Relaxed);
                    return Flow::CloseWith(
                        CloseReason::ProtocolError,
                        Some(Frame::AuthErr(frame::AuthErrCode::NotAuthenticated)),
                    );
                }
                return Flow::CloseWith(CloseReason::ProtocolError, None);
            }
            let instance =
                stats.media_instances.fetch_add(1, Ordering::Relaxed).saturating_add(1) as u32;
            let payload = frame::MediaStartPayload {
                stream_instance_id: instance,
                config_generation: DEFAULT_CONFIG_GENERATION,
                sample_rate: DEFAULT_SAMPLE_RATE,
                channels: DEFAULT_CHANNELS,
                sample_format: SAMPLE_FORMAT_F32LE,
            };
            if let Err(e) = send_frame(tx, &Frame::MediaStart(payload)).await {
                tracing::debug!(error = %e, "MEDIA_START send failed");
                return Flow::CloseWith(CloseReason::InternalError, None);
            }
            stats.media_start_out.fetch_add(1, Ordering::Relaxed);
            let pump = MediaPump::start(
                conn.clone(),
                stats.clone(),
                shared.config.chunk_target_ms,
                shared.config.queue_capacity,
                shared.config.forwarder_delay,
                instance,
            );
            stats.set_state(STATE_PLAYING);
            *media = Some(pump);
            tracing::info!(instance, "media started");
            Flow::Continue
        }
        Frame::Unsubscribe => {
            stats.frames_unsubscribe_in.fetch_add(1, Ordering::Relaxed);
            if stats.state.load(Ordering::Relaxed) != STATE_PLAYING {
                return Flow::CloseWith(CloseReason::ProtocolError, None);
            }
            if let Some(pump) = media.take() {
                pump.stop().await;
            }
            stats.media_unsubscribes.fetch_add(1, Ordering::Relaxed);
            stats.set_state(STATE_READY);
            tracing::info!("media stopped");
            Flow::Continue
        }
        Frame::Close(reason) => {
            stats.frames_close_in.fetch_add(1, Ordering::Relaxed);
            stats.mark_close_received(*reason as u8);
            match reason {
                CloseReason::Normal => Flow::CloseWith(CloseReason::Normal, None),
                _ => {
                    // Only the server decides SERVER_STOP/PROTOCOL_ERROR/...;
                    // a client claiming them is a protocol violation.
                    Flow::CloseWith(CloseReason::ProtocolError, None)
                }
            }
        }
        Frame::AuthErr(_) | Frame::AuthOk { .. } | Frame::MediaStart(_) => {
            // Server-to-client frame types received from the client.
            Flow::CloseWith(CloseReason::ProtocolError, None)
        }
    }
}

/// Credential comparison. The prototype uses a throwaway LAN token; a plain
/// comparison is sufficient here and deliberately NOT presented as a hardened
/// constant-time primitive.
fn constant_time_eq(a: &str, b: &str) -> bool {
    a.as_bytes() == b.as_bytes()
}

async fn send_frame(tx: &mut SendStream, frame: &Frame) -> std::io::Result<()> {
    let bytes = frame::encode(frame)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large"))?;
    tx.write_all(&bytes).await.map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::ConnectionAborted, e.to_string())
    })
}

#[derive(Debug)]
#[allow(dead_code)] // payload fields are diagnostics; violation_text summarizes
enum ReadFrameError {
    Eof,
    EmptyFrame,
    FrameTooLarge(u32),
    Parse(frame::FrameParseError),
    Transport(String),
}

impl From<quinn::ReadExactError> for ReadFrameError {
    fn from(e: quinn::ReadExactError) -> Self {
        match e {
            quinn::ReadExactError::FinishedEarly(_) => ReadFrameError::Eof,
            other => ReadFrameError::Transport(other.to_string()),
        }
    }
}

async fn read_frame(rx: &mut RecvStream) -> Result<Frame, ReadFrameError> {
    let mut len_buf = [0u8; 4];
    rx.read_exact(&mut len_buf).await?;
    let frame_len = u32::from_be_bytes(len_buf);
    if frame_len == 0 {
        return Err(ReadFrameError::EmptyFrame);
    }
    if frame_len > frame::MAX_FRAME_LEN {
        return Err(ReadFrameError::FrameTooLarge(frame_len));
    }
    let mut ty_buf = [0u8; 1];
    rx.read_exact(&mut ty_buf).await?;
    let mut payload = vec![0u8; (frame_len - 1) as usize];
    rx.read_exact(&mut payload).await?;
    frame::parse(ty_buf[0], &payload).map_err(ReadFrameError::Parse)
}

/// Full graceful close: send any prelude frame (e.g. AUTH_ERR), the CLOSE
/// frame, half-close, wait briefly for the peer to finish reading, then close
/// the transport with a matching application error code.
async fn break_close(
    conn: &Connection,
    tx: &mut SendStream,
    rx: &mut RecvStream,
    stats: &Arc<SessionStats>,
    reason: CloseReason,
    prelude: Option<Frame>,
    shutdown_seen: &mut bool,
) {
    stats.set_state(STATE_CLOSING);
    if matches!(reason, CloseReason::ServerStop) {
        *shutdown_seen = true;
    }
    if let Some(prelude) = prelude {
        if matches!(prelude, Frame::AuthErr(_)) {
            stats.auth_err_out.fetch_add(1, Ordering::Relaxed);
        }
        let _ = send_frame(tx, &prelude).await;
    }
    if matches!(reason, CloseReason::ProtocolError) {
        stats.protocol_errors_out.fetch_add(1, Ordering::Relaxed);
    }
    let close_frame = Frame::Close(reason);
    if send_frame(tx, &close_frame).await.is_ok() {
        stats.close_out.fetch_add(1, Ordering::Relaxed);
    }
    stats.mark_close_sent(reason as u8);
    let _ = tx.finish();
    // Give the peer a moment to drain the stream before the transport close.
    let _ = tokio::time::timeout(Duration::from_millis(500), read_frame(rx)).await;
    conn.close(
        quinn::VarInt::from_u32(reason as u32),
        close_reason_ascii(reason),
    );
    stats.set_state(STATE_CLOSED);
    stats.closed.store(true, Ordering::Relaxed);
}

fn close_reason_ascii(reason: CloseReason) -> &'static [u8] {
    match reason {
        CloseReason::Normal => b"normal",
        CloseReason::ServerStop => b"server-stop",
        CloseReason::ProtocolError => b"protocol-error",
        CloseReason::AuthFailed => b"auth-failed",
        CloseReason::IdleTimeout => b"idle-timeout",
        CloseReason::InternalError => b"internal-error",
    }
}

/// Owns the media producer + forwarder task pair for one subscription.
struct MediaPump {
    producer: tokio::task::JoinHandle<()>,
    forwarder: tokio::task::JoinHandle<()>,
}

impl MediaPump {
    fn start(
        conn: Connection,
        stats: Arc<SessionStats>,
        chunk_target_ms: u64,
        queue_capacity: usize,
        forwarder_delay: Option<Duration>,
        stream_instance_id: u32,
    ) -> Self {
        let (queue_tx, queue_rx) = mpsc::channel(queue_capacity);
        let producer = tokio::spawn(produce(
            conn.clone(),
            stats.clone(),
            chunk_target_ms,
            queue_tx,
            stream_instance_id,
        ));
        let forwarder = tokio::spawn(forward(conn, stats.clone(), queue_rx, forwarder_delay));
        Self { producer, forwarder }
    }

    /// Stops the producer immediately; the forwarder is left to self-terminate
    /// when the queue drains (bounded, ≤ queue_capacity) or the connection
    /// dies. It must never be awaited from the control path — a slow drain
    /// would stall UNSUBSCRIBE/CLOSE handling.
    async fn stop(self) {
        self.producer.abort();
        let _ = self.producer.await;
        // Detach the forwarder: bounded queue drain, no control-path latency.
        drop(self.forwarder);
    }
}

async fn produce(
    conn: Connection,
    stats: Arc<SessionStats>,
    chunk_target_ms: u64,
    queue_tx: mpsc::Sender<bytes::Bytes>,
    stream_instance_id: u32,
) {
    let channels = DEFAULT_CHANNELS;
    let fb = frame_bytes(channels);
    let frames_per_tick = (DEFAULT_SAMPLE_RATE as u128 * chunk_target_ms as u128 / 1000) as usize;
    if frames_per_tick == 0 {
        tracing::error!("chunk_target_ms too small; no media produced");
        return;
    }
    let mut ticker = tokio::time::interval(Duration::from_millis(chunk_target_ms.max(1)));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut pos: u64 = 0;
    let mut sequence: u32 = 0;
    loop {
        ticker.tick().await;
        // The datagram limit is re-queried every tick; it must never be
        // assumed (no fixed 1464 arithmetic).
        let Some(max_datagram_size) = conn.max_datagram_size() else {
            tracing::debug!("peer does not support datagrams; stopping media");
            return;
        };
        let desired = frames_per_tick * fb;
        let payload_len = chunk_payload_len(max_datagram_size, fb, desired);
        if payload_len == 0 {
            tracing::error!(
                max_datagram_size,
                "datagram limit cannot carry one sample frame; stopping media"
            );
            return;
        }
        let frames_per_dg = payload_len / fb;
        stats.media_batches.fetch_add(1, Ordering::Relaxed);
        let mut remaining = frames_per_tick;
        while remaining > 0 {
            let frames = remaining.min(frames_per_dg);
            let header = MediaHeader {
                stream_instance_id,
                config_generation: DEFAULT_CONFIG_GENERATION,
                sequence,
                timestamp: pos,
            };
            let datagram = encode_datagram(&header, pos, frames, channels);
            debug_assert!(datagram.len() <= max_datagram_size);
            sequence = sequence.wrapping_add(1);
            pos += frames as u64;
            remaining -= frames;
            stats.media_produced.fetch_add(1, Ordering::Relaxed);
            match queue_tx.try_send(datagram) {
                Ok(()) => {
                    stats.media_queued.fetch_add(1, Ordering::Relaxed);
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // Bounded queue overflow: drop the NEWEST datagram, never
                    // wait behind stale media.
                    stats.media_queue_dropped.fetch_add(1, Ordering::Relaxed);
                    tracing::debug!(
                        stream_instance_id,
                        "media queue full; dropped newest datagram"
                    );
                }
                Err(mpsc::error::TrySendError::Closed(_)) => return,
            }
        }
    }
}

async fn forward(
    conn: Connection,
    stats: Arc<SessionStats>,
    mut queue_rx: mpsc::Receiver<bytes::Bytes>,
    forwarder_delay: Option<Duration>,
) {
    while let Some(datagram) = queue_rx.recv().await {
        if let Some(delay) = forwarder_delay {
            tokio::time::sleep(delay).await;
        }
        match conn.send_datagram(datagram.clone()) {
            Ok(()) => {
                stats.media_sent.fetch_add(1, Ordering::Relaxed);
                stats
                    .media_bytes_sent
                    .fetch_add(datagram.len() as u64, Ordering::Relaxed);
            }
            // quinn 0.11.12 never reports a full send buffer from
            // send_datagram(): its internal datagram queue silently evicts
            // oldest-first, so counted drops only happen in our own bounded
            // queue above. Everything arriving here is fatal for media.
            Err(e) => {
                tracing::debug!(error = %e, "send_datagram failed; stopping forwarder");
                return;
            }
        }
    }
}
