//! QUIC session glue: tokio runtime, quinn client, control stream loop,
//! datagram receive loop, ring feeding, and the network-change rebind path.
//!
//! Mostly DEVICE/NETWORK ONLY: the connect/control/datagram loops are not
//! exercised by host tests (this crate's test scope is codec/state/verifier/
//! ring logic; the e2e server is a separate crate). Everything compiles under
//! `cargo test`; runtime behavior is explicitly UNTESTED until run against a
//! real server on a device. EXCEPTION: the rebind path (`rebind` /
//! `rebind_endpoint_from_raw_fd`) IS host-tested — it only needs a bare
//! endpoint, no live QUIC connection.

use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::codec::{encode_frame, Frame, FrameDecoder};
use crate::control::{ClientState, CloseInfo, CloseOrigin, CloseReason};
use crate::media::{MediaHeader, MediaTracker};
use crate::ring::MediaRing;
use crate::stats::{BufferStats, MediaCounters, StatsSnapshot};
use crate::verify::PinnedSpkiVerifier;

/// 4 MiB prototype ring: ~10.9 s of stereo f32 audio at 48 kHz.
pub const RING_CAPACITY_BYTES: usize = 4 * 1024 * 1024;
/// Prefetch target: 50 ms of stereo f32 at 48 kHz before playback starts.
pub const BUFFER_TARGET_BYTES: u64 = 9_600;

pub struct SessionConfig {
    pub host: String,
    pub port: u16,
    pub pinned_spki_hex: String,
    pub credential: String,
}

pub struct SharedState {
    pub state: Mutex<ClientState>,
    pub connected: AtomicBool,
    pub control_errors: AtomicU64,
    pub last_control_error: Mutex<String>,
    pub tracker: Mutex<Option<MediaTracker>>,
    pub ring: Mutex<Option<MediaRing>>,
    pub shutdown: tokio::sync::Notify,
    pub stopping: AtomicBool,
    /// Control SendStream after AUTH is sent; used by subscribe/unsubscribe/stop.
    /// tokio Mutex: the guard is held across `.await` in the control loop.
    pub control_stream: tokio::sync::Mutex<Option<quinn::SendStream>>,
    /// Connection handle for the datagram pump (set after handshake).
    pub conn: Mutex<Option<quinn::Connection>>,
    /// Endpoint kept alive for the connection lifetime (rebind scaffold).
    pub endpoint: Mutex<Option<quinn::Endpoint>>,
    /// MEDIA_START processed at least once.
    pub media_active: AtomicBool,
}

pub struct Session {
    pub config: SessionConfig,
    pub shared: Arc<SharedState>,
    runtime: tokio::runtime::Runtime,
}

impl Session {
    /// Build the session (no I/O yet). Called from the JNI thread.
    pub fn new(config: SessionConfig) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| format!("tokio runtime: {e}"))?;
        Ok(Session {
            config,
            runtime,
            shared: Arc::new(SharedState {
                state: Mutex::new(ClientState::Idle),
                connected: AtomicBool::new(false),
                control_errors: AtomicU64::new(0),
                last_control_error: Mutex::new(String::new()),
                tracker: Mutex::new(None),
                ring: Mutex::new(None),
                shutdown: tokio::sync::Notify::new(),
                stopping: AtomicBool::new(false),
                control_stream: tokio::sync::Mutex::new(None),
                conn: Mutex::new(None),
                endpoint: Mutex::new(None),
                media_active: AtomicBool::new(false),
            }),
        })
    }

    fn record_control_error(&self, msg: String) {
        *self.shared.last_control_error.lock().unwrap() = msg.clone();
        self.shared.control_errors.fetch_add(1, Ordering::Relaxed);
        log::warn!("control error: {msg}");
    }

    fn set_state(&self, s: ClientState) {
        *self.shared.state.lock().unwrap() = s;
    }

    /// Resolve the server address, optionally binding the UDP socket to the
    /// migration-hint local address (same family as the resolved server addr).
    fn make_client_socket(&self, bind: Option<std::net::IpAddr>) -> Result<UdpSocket, String> {
        let server = (self.config.host.as_str(), self.config.port)
            .to_socket_addrs_err()?
            .into_iter()
            .find(|a| match (a, bind) {
                (std::net::SocketAddr::V4(_), Some(std::net::IpAddr::V4(_)))
                | (std::net::SocketAddr::V6(_), Some(std::net::IpAddr::V6(_)))
                | (_, None) => true,
                _ => false,
            })
            .ok_or("no server address matching the bind hint family")?;
        let local: std::net::SocketAddr = match bind {
            Some(ip) => std::net::SocketAddr::new(ip, 0),
            None => {
                if server.is_ipv4() {
                    "0.0.0.0:0".parse().unwrap()
                } else {
                    "[::]:0".parse().unwrap()
                }
            }
        };
        UdpSocket::bind(local).map_err(|e| format!("bind {local}: {e}"))
    }

    /// Start the connect + control + media loops on the runtime. Non-blocking.
    /// `bind_local_address` is the migration scaffold hint (may be None).
    pub fn connect(self: &Arc<Self>, bind_local_address: Option<std::net::IpAddr>) -> Result<(), String> {
        if self.shared.stopping.load(Ordering::SeqCst) {
            return Err("session stopped".into());
        }
        let this = self.clone();
        self.runtime.spawn(async move {
            // run() consumes an Arc clone; `this` stays for error reporting
            let runner = this.clone();
            if let Err(e) = runner.run(bind_local_address).await {
                log::warn!("session ended: {e}");
                this.record_control_error(format!("session ended: {e}"));
                this.set_state(ClientState::Closed(CloseInfo {
                    reason: CloseReason::InternalError,
                    origin: CloseOrigin::Local,
                }));
                this.shared.connected.store(false, Ordering::Release);
            }
        });
        Ok(())
    }

    async fn run(self: Arc<Self>, bind_local_address: Option<std::net::IpAddr>) -> Result<(), String> {
        let socket = self.make_client_socket(bind_local_address)?;
        let verifier = Arc::new(
            PinnedSpkiVerifier::new(&self.config.pinned_spki_hex)
                .map_err(|e| format!("pin: {e}"))?,
        );
        let crypto = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
        // ALPN exactly `audio-share/v2`; TLS 1.3 only (rustls 0.23 default
        // version set); 0-RTT never configured — quinn/rustls reject early data
        // unless explicitly enabled, which this prototype never does.
        let mut crypto = crypto;
        crypto.alpn_protocols = vec![b"audio-share/v2".to_vec()];
        let client_config = quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
                .map_err(|e| format!("rustls->quic: {e}"))?,
        ));

        let mut endpoint = quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            None,
            socket,
            Arc::new(quinn::TokioRuntime),
        )
        .map_err(|e| format!("endpoint: {e}"))?;
        endpoint.set_default_client_config(client_config);

        self.set_state(ClientState::AwaitingAuth);
        log::info!(
            "connecting to {}:{} ALPN audio-share/v2",
            self.config.host,
            self.config.port
        );
        let connecting = endpoint
            .connect(
                (self.config.host.as_str(), self.config.port)
                    .to_socket_addrs_err()?
                    .into_iter()
                    .next()
                    .ok_or("no server addr")?,
                self.config.host.as_str(),
            )
            .map_err(|e| format!("connect: {e}"))?;
        let conn = tokio::select! {
            c = connecting => c.map_err(|e| format!("handshake: {e}"))?,
            _ = self.shared.shutdown.notified() => return Err("shutdown during handshake".into()),
        };
        *self.shared.endpoint.lock().unwrap() = Some(endpoint);
        *self.shared.conn.lock().unwrap() = Some(conn.clone());
        self.shared.connected.store(true, Ordering::Release);
        log::info!("QUIC established");

        // one reliable bidirectional control stream; AUTH before anything else
        // (never early data — this is a post-handshake 1-RTT write).
        let (mut tx, mut rx) = conn.open_bi().await.map_err(|e| format!("open_bi: {e}"))?;
        let auth = encode_frame(&Frame::Auth {
            token: self.config.credential.clone(),
        })
        .map_err(|e| format!("encode auth: {e}"))?;
        tx.write_all(&auth)
            .await
            .map_err(|e| format!("send auth: {e}"))?;
        *self.shared.control_stream.lock().await = Some(tx);

        let mut decoder = FrameDecoder::new();
        let mut rx_buf = vec![0u8; 8192];
        let result = 'control: loop {
            let n = tokio::select! {
                read = rx.read(&mut rx_buf) => {
                    match read {
                        Ok(Some(n)) => n,
                        Ok(None) => break 'control Err("control stream closed by server".into()),
                        Err(e) => break 'control Err(format!("control read: {e}")),
                    }
                }
                _ = self.shared.shutdown.notified() => break 'control Ok(()),
            };
            let msgs = match decoder.push(&rx_buf[..n]) {
                Ok(m) => m,
                Err(e) => break 'control Err(format!("frame decode: {e}")),
            };
            for msg in msgs {
                let state = self.shared.state.lock().unwrap().clone();
                let mut next = state;
                match next.on_control(&msg) {
                    Ok(_) => {}
                    Err(crate::control::ControlError::AuthRejected { code }) => {
                        self.set_state(ClientState::Closed(CloseInfo {
                            reason: CloseReason::AuthFailed,
                            origin: CloseOrigin::Remote,
                        }));
                        break 'control Err(format!("auth rejected (code {code:?})"));
                    }
                    Err(crate::control::ControlError::AfterClose) => {
                        break 'control Err("control after close".into());
                    }
                    Err(crate::control::ControlError::Protocol(m)) => {
                        self.record_control_error(m);
                        // brief: violations get CLOSE(PROTOCOL_ERROR), not silence
                        let mut tx_guard = self.shared.control_stream.lock().await;
                        if let (Some(tx), Ok(close_bytes)) = (
                            tx_guard.as_mut(),
                            encode_frame(&Frame::Close(CloseReason::ProtocolError)),
                        ) {
                            let _ = tx.write_all(&close_bytes).await;
                        }
                        drop(tx_guard);
                        self.set_state(ClientState::Closed(CloseInfo {
                            reason: CloseReason::ProtocolError,
                            origin: CloseOrigin::Local,
                        }));
                        if let Some(conn) = self.shared.conn.lock().unwrap().clone() {
                            conn.close(0u32.into(), b"protocol error");
                        }
                        break 'control Err("protocol error".into());
                    }
                }
                let playing = matches!(next, ClientState::Playing(_));
                self.set_state(next);
                if playing {
                    let cfg = if let ClientState::Playing(cfg) =
                        self.shared.state.lock().unwrap().clone()
                    {
                        cfg
                    } else {
                        unreachable!()
                    };
                    let first_media = !self.shared.media_active.swap(true, Ordering::SeqCst);
                    {
                        let mut tracker = self.shared.tracker.lock().unwrap();
                        match tracker.as_mut() {
                            Some(t) => t.reconfigure(cfg.clone()),
                            None => *tracker = Some(MediaTracker::new(cfg.clone())),
                        }
                    }
                    log::info!(
                        "media start: instance {} gen {} {} Hz {} ch",
                        cfg.stream_instance_id,
                        cfg.config_generation,
                        cfg.sample_rate,
                        cfg.channels
                    );
                    if first_media {
                        // Device-only datagram pump; runs until shutdown.
                        let shared = self.shared.clone();
                        tokio::spawn(media_pump(shared));
                    }
                }
                if self.shared.state.lock().unwrap().is_closed() {
                    break 'control Ok(());
                }
            }
        };

        self.shared.connected.store(false, Ordering::Release);
        if let Some(conn) = self.shared.conn.lock().unwrap().clone() {
            // 0 = application error code NORMAL_CLOSE; reason string empty in
            // the QUIC frame, protocol-level CLOSE already went on the stream
            conn.close(0u32.into(), b"");
        }
        result
    }

    pub fn subscribe(&self) -> Result<(), String> {
        let legal = self.shared.state.lock().unwrap().can_subscribe();
        if !legal {
            let msg = "subscribe rejected: not in READY (SUBSCRIBE is legal only in READY)"
                .to_string();
            self.record_control_error(msg.clone());
            return Err(msg);
        }
        self.runtime.block_on(async {
            let mut tx_guard = self.shared.control_stream.lock().await;
            match tx_guard.as_mut() {
                Some(tx) => {
                    let frame = encode_frame(&Frame::Subscribe)
                        .map_err(|e| format!("encode subscribe: {e}"))?;
                    tx.write_all(&frame)
                        .await
                        .map_err(|e| format!("send subscribe: {e}"))?;
                    log::info!("SUBSCRIBE sent");
                    Ok(())
                }
                None => Err("no control stream (not connected)".into()),
            }
        })
    }

    pub fn unsubscribe(&self) -> Result<(), String> {
        let legal = self.shared.state.lock().unwrap().can_unsubscribe();
        if !legal {
            let msg = "unsubscribe rejected: not in READY/PLAYING".to_string();
            self.record_control_error(msg.clone());
            return Err(msg);
        }
        self.runtime.block_on(async {
            let mut tx_guard = self.shared.control_stream.lock().await;
            match tx_guard.as_mut() {
                Some(tx) => {
                    let frame = encode_frame(&Frame::Unsubscribe)
                        .map_err(|e| format!("encode unsubscribe: {e}"))?;
                    tx.write_all(&frame)
                        .await
                        .map_err(|e| format!("send unsubscribe: {e}"))?;
                    log::info!("UNSUBSCRIBE sent");
                    Ok(())
                }
                None => Err("no control stream (not connected)".into()),
            }
        })
    }

    /// Clean stop: UNSUBSCRIBE if playing, local CLOSE, connection close.
    pub fn stop(&self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        let was_playing = self.shared.state.lock().unwrap().can_unsubscribe();
        if was_playing {
            let _ = self.unsubscribe();
        }
        self.runtime.block_on(async {
            let mut tx_guard = self.shared.control_stream.lock().await;
            if let (Some(tx), Ok(close_bytes)) =
                (tx_guard.as_mut(), encode_frame(&Frame::Close(CloseReason::Normal)))
            {
                let _ = tx.write_all(&close_bytes).await;
            }
        });
        self.shared.shutdown.notify_waiters();
        self.set_state(ClientState::Closed(CloseInfo {
            reason: CloseReason::Normal,
            origin: CloseOrigin::Local,
        }));
        self.shared.connected.store(false, Ordering::Release);
        log::info!("session stopped (LOCAL NORMAL)");
    }

    pub fn snapshot(&self) -> StatsSnapshot {
        let state = self.shared.state.lock().unwrap().clone();
        let tracker_guard = self.shared.tracker.lock().unwrap();
        let media: MediaCounters = tracker_guard.as_ref().map_or(MediaCounters::default(), |t| {
            MediaCounters {
                datagrams_observed: t.datagrams_observed,
                highest_seq: t.latest(),
                gaps: t.gaps,
                duplicates: t.duplicates,
                reordered: t.reordered,
                instance_changes: t.instance_changes,
                byte_errors: t.byte_errors,
                malformed_payloads: t.malformed_payloads,
                config_mismatches: t.config_mismatches,
                bytes: t.forwarded_bytes,
                frames: t.forwarded_frames,
            }
        });
        let media_config = tracker_guard.as_ref().map(|t| t.config.clone());
        drop(tracker_guard);
        let ring_guard = self.shared.ring.lock().unwrap();
        let buffer: BufferStats = ring_guard.as_ref().map_or(
            BufferStats {
                capacity_bytes: RING_CAPACITY_BYTES as u64,
                target_bytes: BUFFER_TARGET_BYTES,
                ..BufferStats::default()
            },
            |r| BufferStats {
                capacity_bytes: r.capacity as u64,
                write_pos: r.write_pos(),
                write_offset: r.write_offset(),
                target_bytes: BUFFER_TARGET_BYTES,
                dropped_bytes: r.dropped_bytes,
                drops: r.oversize_drops,
            },
        );
        drop(ring_guard);
        StatsSnapshot {
            state,
            connected: self.shared.connected.load(Ordering::Acquire),
            control_errors: self.shared.control_errors.load(Ordering::Relaxed),
            last_control_error: self.shared.last_control_error.lock().unwrap().clone(),
            media_config,
            media,
            buffer,
        }
    }

    /// Attach the Kotlin-owned direct ByteBuffer (once). `base`/`capacity` come
    /// from `get_direct_buffer_address`/`capacity` at the JNI layer.
    ///
    /// # Safety
    /// `base` must point to `capacity` bytes owned by the live ByteBuffer for
    /// the session's lifetime; the JNI layer guarantees Kotlin keeps the buffer
    /// referenced (see ProtoSession in the app).
    pub unsafe fn attach_buffer(&self, base: *mut u8, capacity: usize) -> Result<(), String> {
        let mut ring = self.shared.ring.lock().unwrap();
        if ring.is_some() {
            return Err("buffer already attached".into());
        }
        *ring = Some(MediaRing::attach(base, capacity));
        log::info!("media ring attached ({capacity} bytes)");
        Ok(())
    }

    /// Network-change migration: rebind the QUIC endpoint onto a UDP socket
    /// the JVM created and route-pinned to the active Android `Network`
    /// (`Network.bindSocket`), handed over as a raw fd. Low-frequency (once
    /// per network change). On failure the caller falls back to teardown +
    /// bounded-backoff reconnect (the fd is consumed either way, see the
    /// unsafe helper below).
    ///
    /// Host-tested (endpoint-level); end-to-end behavior against a live QUIC
    /// connection on a device is NOT RUN.
    pub fn rebind(&self, fd: i32) -> Result<(), String> {
        if self.shared.stopping.load(Ordering::SeqCst) {
            return Err("session stopped".into());
        }
        let endpoint = self
            .shared
            .endpoint
            .lock()
            .unwrap()
            .clone()
            .ok_or("no endpoint (not connected)")?;
        // quinn's rebind registers the socket with the runtime's IO driver,
        // which requires a runtime context — the JNI thread has none, so run
        // it with this session's runtime.
        self.runtime.block_on(async {
            // SAFETY: the JNI contract transfers fd ownership to native
            // (Kotlin must never close or reuse it afterwards); the socket
            // is dropped (fd closed) if the rebind fails.
            unsafe { rebind_endpoint_from_raw_fd(&endpoint, fd) }
        })
    }
}

/// Reconstruct a `std::net::UdpSocket` from a raw fd handed over by the JVM
/// side and rebind the endpoint onto it (quinn `Endpoint::rebind`): all
/// connections on the endpoint switch to the new socket live.
///
/// # Safety
/// `fd` must be a valid, open UDP socket file descriptor whose ownership is
/// transferred to this function: after the call the JVM side must never close
/// or reuse it. The reconstructed socket is consumed by `Endpoint::rebind` on
/// success and dropped (closing the fd) on failure.
unsafe fn rebind_endpoint_from_raw_fd(endpoint: &quinn::Endpoint, fd: i32) -> Result<(), String> {
    use std::os::fd::FromRawFd;
    let socket = UdpSocket::from_raw_fd(fd);
    endpoint
        .rebind(socket)
        .map_err(|e| format!("endpoint rebind: {e}"))
}

/// Device-only datagram loop: parse header, verify, feed the ring. UNTESTED
/// (needs a live QUIC server).
async fn media_pump(shared: Arc<SharedState>) {
    let conn = match shared.conn.lock().unwrap().clone() {
        Some(c) => c,
        None => return,
    };
    loop {
        tokio::select! {
            d = conn.read_datagram() => {
                match d {
                    Ok(datagram) => {
                        let (header, payload) = match MediaHeader::parse(&datagram) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        let forward = {
                            let mut tracker = shared.tracker.lock().unwrap();
                            match tracker.as_mut() {
                                Some(t) => t.observe(header, payload).forward,
                                None => false,
                            }
                        };
                        if forward {
                            let mut ring = shared.ring.lock().unwrap();
                            if let Some(r) = ring.as_mut() {
                                r.write(payload);
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("datagram read ended: {e}");
                        return;
                    }
                }
            }
            _ = shared.shutdown.notified() => return,
        }
    }
}

trait ToSocketAddrsErr {
    fn to_socket_addrs_err(&self) -> Result<Vec<std::net::SocketAddr>, String>;
}

impl ToSocketAddrsErr for (&str, u16) {
    fn to_socket_addrs_err(&self) -> Result<Vec<std::net::SocketAddr>, String> {
        use std::net::ToSocketAddrs;
        self.to_socket_addrs()
            .map(|i| i.collect())
            .map_err(|e| format!("resolve {}:{}: {e}", self.0, self.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::IntoRawFd;

    fn test_config() -> SessionConfig {
        SessionConfig {
            host: "127.0.0.1".to_string(),
            port: 65531,
            pinned_spki_hex: "00".repeat(32),
            credential: "test-credential-do-not-use".to_string(),
        }
    }

    /// Creates a bound UDP socket and hands its fd over raw (ownership moves
    /// away from the Rust wrapper via the proper `into_raw_fd` transfer),
    /// mirroring the JVM JNI fd contract.
    fn transfer_bound_socket() -> (i32, u16) {
        let sock = UdpSocket::bind("127.0.0.1:0").expect("bind");
        let port = sock.local_addr().expect("local addr").port();
        (sock.into_raw_fd(), port)
    }

    /// The migration path's core primitive: a bare client endpoint rebinds
    /// onto a raw fd exactly like the one handed over from the JVM side.
    #[tokio::test]
    async fn rebind_endpoint_swaps_the_underlying_socket() {
        let first = UdpSocket::bind("127.0.0.1:0").expect("bind first");
        let endpoint = quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            None,
            first,
            Arc::new(quinn::TokioRuntime),
        )
        .expect("endpoint");
        let old_port = endpoint.local_addr().expect("local addr").port();

        let (fd, new_port) = transfer_bound_socket();
        unsafe { rebind_endpoint_from_raw_fd(&endpoint, fd) }.expect("rebind");

        assert_ne!(
            endpoint.local_addr().expect("local addr after rebind").port(),
            old_port,
            "endpoint must now use the second socket"
        );
        assert_eq!(
            endpoint.local_addr().expect("local addr after rebind").port(),
            new_port,
        );
        endpoint.wait_idle().await; // drop cleanly, no live connections
    }

    /// A rebind is impossible before the handshake stored the endpoint; the
    /// fd must not be touched on that error path (caller falls back to
    /// teardown + backoff reconnect).
    #[test]
    fn rebind_without_stored_endpoint_fails_before_touching_the_fd() {
        let session = Session::new(test_config()).expect("session");
        let (fd, _port) = transfer_bound_socket();
        let err = session.rebind(fd).expect_err("no endpoint yet");
        assert!(err.contains("no endpoint"), "got: {err}");
    }

    /// A stopped session refuses to rebind.
    #[test]
    fn rebind_on_stopped_session_fails() {
        let session = Session::new(test_config()).expect("session");
        session.shared.stopping.store(true, Ordering::SeqCst);
        let (fd, _port) = transfer_bound_socket();
        assert!(session.rebind(fd).is_err());
    }
}
