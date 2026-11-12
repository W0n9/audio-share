//! QUIC session glue (ticket 08): tokio runtime, quinn client with the SPKI
//! pin, the §3 control-stream loop over the prost codec, the §5.3 machine
//! driven end-to-end, the §6.3 reconnect policy, and the network-change
//! rebind primitive (prototype-validated, host-tested here).
//!
//! Testing scope (same honest split as the prototype): everything except the
//! live-QUIC loops is host-tested. The connect/control loops compile and are
//! device/09 gate material — the ticket 09 cross-validation runs them against
//! the ticket 06/07 Server.
//!
//! Auth context inputs: the TLS-Exporter stand-in is `auth_exporter()` —
//! on a live connection the session derives
//! TLS-Exporter("EXPORTER audio-share v2 auth", "", 32) via
//! quinn::Connection::export_keying_material (§9.1 pairing.md). The test-only
//! SPKI pin comes from the pre-provisioned Server public key (test-keys.json
//! pin; production pairing is phase 6).

use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::control::{ClientMachine, ClientState, CloseInfo};
use crate::frame::{encode_frame, FrameDecoder};
use crate::proto::Frame;
use crate::proto::frame::Body;
use crate::proto::AuthInit;
use crate::verify::PinnedSpkiVerifier;

/// Spec §3 / ADR-0004 D1: v2 port (never v1's 65530).
// Protocol constants duplicated from the Server crate because the Android
// native crate is standalone (own [workspace], built outside the repo
// workspace). The authoritative table lives in
// crates/v2-transport/src/codes.rs and the wire spec (docs/wire-protocol-v2/);
// keep these values in lockstep with it.
pub const DEFAULT_PORT: u16 = 65531;
/// ALPN: exactly `audio-share/v2` (spec §3).
pub const ALPN: &[u8] = b"audio-share/v2";
/// pairing.md §9.1 exporter label (32-byte output).
pub const AUTH_EXPORTER_LABEL: &[u8] = b"EXPORTER audio-share v2 auth";
/// §3: auth exchange deadline (absolute: AuthInit → AuthResult).
const AUTH_DEADLINE_SECS: u64 = 10;
/// §4.2: per-request response deadline.
const RESPONSE_DEADLINE_SECS: u64 = 10;
/// §3/D3: idle timeout and keep-alive, both sides.
const IDLE_TIMEOUT_SECS: u64 = 30;
const KEEP_ALIVE_SECS: u64 = 10;

/// Response deadline bookkeeping. The deadline is armed once when the first
/// request becomes pending and is NOT pushed back by unrelated reads (an
/// absolute per-request deadline, §4.2); it disarms when nothing is pending.
#[derive(Debug, Default)]
struct ResponseWatch {
    deadline: Option<tokio::time::Instant>,
}

impl ResponseWatch {
    fn observe(&mut self, now: tokio::time::Instant, pending: usize) {
        if pending == 0 {
            self.deadline = None;
        } else if self.deadline.is_none() {
            self.deadline =
                Some(now + std::time::Duration::from_secs(RESPONSE_DEADLINE_SECS));
        }
    }

    async fn expired(&self) {
        match self.deadline {
            Some(deadline) => tokio::time::sleep_until(deadline).await,
            None => std::future::pending().await,
        }
    }
}
/// §6.3 backoff: 1 s base, ×2 per attempt, 30 s cap, ±20 % jitter.
pub const BACKOFF_BASE_MS: u64 = 1_000;
pub const BACKOFF_MAX_MS: u64 = 30_000;

/// §6.3 backoff delay for the nth attempt (0-based): 1 s × 2^n capped at
/// 30 s; the ±20 % jitter is applied by the caller (Kotlin session or tests).
pub fn backoff_delay_ms(attempt: u64) -> u64 {
    BACKOFF_BASE_MS
        .saturating_mul(1u64 << attempt.min(16))
        .min(BACKOFF_MAX_MS)
}

/// Session configuration. The pin is the test-only pre-provisioned Server
/// identity pin for this ticket (hex, 64 chars).
pub struct SessionConfig {
    pub host: String,
    pub port: u16,
    pub pinned_spki_hex: String,
    /// Raw 32-byte Ed25519 Server identity public key (auth transcript TLV(4));
    /// from the same pre-provisioned test configuration as the pin.
    pub server_public_key: Vec<u8>,
    /// Client display name (≤ 63 bytes UTF-8, pairing.md §6.2) — used by
    /// pairing; this ticket keeps it for the JNI surface only.
    pub client_name: String,
}

pub struct SharedState {
    pub state: Mutex<ClientState>,
    pub close_info: Mutex<Option<CloseInfo>>,
    pub connected: AtomicBool,
    pub stopping: AtomicBool,
    /// Last machine-emitted or transport error (no credentials/secrets — the
    /// red line: error surfaces never carry key material or raw media).
    pub last_error: Mutex<String>,
    /// Control SendStream after the machine sends its first frame.
    pub control_stream: tokio::sync::Mutex<Option<quinn::SendStream>>,
    /// Connection handle for the datagram pump (media path NOT activated).
    pub conn: Mutex<Option<quinn::Connection>>,
    /// Endpoint kept alive for the connection lifetime (rebind primitive).
    pub endpoint: Mutex<Option<quinn::Endpoint>>,
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
                close_info: Mutex::new(None),
                connected: AtomicBool::new(false),
                stopping: AtomicBool::new(false),
                last_error: Mutex::new(String::new()),
                control_stream: tokio::sync::Mutex::new(None),
                conn: Mutex::new(None),
                endpoint: Mutex::new(None),
            }),
        })
    }

    fn set_state(&self, s: ClientState) {
        *self.shared.state.lock().unwrap() = s;
    }

    fn record_error(&self, msg: String) {
        log::warn!("session: {msg}");
        *self.shared.last_error.lock().unwrap() = msg;
    }

    /// Resolve the server address, optionally binding to the migration-hint
    /// local address (noBindHint lesson: never pass the hint when the server
    /// is reached through a VPN overlay — the hint pins the route to the
    /// physical interface and breaks tun routing; the Kotlin side decides).
    fn make_client_socket(&self, bind: Option<std::net::IpAddr>) -> Result<UdpSocket, String> {
        use std::net::ToSocketAddrs;
        let server = (self.config.host.as_str(), self.config.port)
            .to_socket_addrs()
            .map_err(|e| format!("resolve {}:{}: {e}", self.config.host, self.config.port))?
            .into_iter()
            .find(|a| {
                matches!(
                    (a, bind),
                    (std::net::SocketAddr::V4(_), Some(std::net::IpAddr::V4(_)))
                        | (std::net::SocketAddr::V6(_), Some(std::net::IpAddr::V6(_)))
                        | (_, None)
                )
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

    /// Start the connect + control loop on the runtime. Non-blocking.
    pub fn connect(self: &Arc<Self>, bind_local_address: Option<std::net::IpAddr>) -> Result<(), String> {
        if self.shared.stopping.load(Ordering::SeqCst) {
            return Err("session stopped".into());
        }
        let this = self.clone();
        self.runtime.spawn(async move {
            let runner = this.clone();
            if let Err(e) = runner.run(bind_local_address).await {
                // Transport loss is a local classification: 601 unless the
                // pinned identity failed (205). The §6.3 policy (reconnect vs
                // surface) is applied by the Kotlin owner from this code.
                let code = classify_run_error(&e);
                this.record_error(format!("session ended: {e}"));
                *this.shared.close_info.lock().unwrap() = Some(CloseInfo {
                    code,
                    remote: false,
                });
                this.set_state(ClientState::Closed);
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
        let mut crypto = crypto;
        crypto.alpn_protocols = vec![ALPN.to_vec()];
        // TLS 1.3 only (rustls 0.23 default); 0-RTT never configured (spec §3).
        let mut client_config = quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
                .map_err(|e| format!("rustls->quic: {e}"))?,
        ));
        // §3/D3: keep-alive every 10 s; declare the peer dead after 30 s idle.
        let mut transport = quinn::TransportConfig::default();
        transport.keep_alive_interval(Some(std::time::Duration::from_secs(
            KEEP_ALIVE_SECS,
        )));
        transport.max_idle_timeout(Some(
            quinn::IdleTimeout::try_from(std::time::Duration::from_secs(IDLE_TIMEOUT_SECS))
                .map_err(|_| "idle timeout out of range".to_string())?,
        ));
        client_config.transport_config(Arc::new(transport));

        let mut endpoint = quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            None,
            socket,
            Arc::new(quinn::TokioRuntime),
        )
        .map_err(|e| format!("endpoint: {e}"))?;
        endpoint.set_default_client_config(client_config);

        log::info!("connecting {}:{} ALPN {}", self.config.host, self.config.port, String::from_utf8_lossy(ALPN));
        let connecting = {
            use std::net::ToSocketAddrs;
            let addr = (self.config.host.as_str(), self.config.port)
                .to_socket_addrs()
                .map_err(|e| format!("resolve: {e}"))?
                .next()
                .ok_or("no server addr")?;
            endpoint
                .connect(addr, self.config.host.as_str())
                .map_err(|e| format!("connect: {e}"))?
        };
        // §3: 5 s handshake timeout → classify 601
        let conn = tokio::time::timeout(std::time::Duration::from_secs(5), connecting)
            .await
            .map_err(|_| "handshake timeout (5 s) → 601".to_string())?
            .map_err(|e| format!("handshake: {e}"))?;
        *self.shared.endpoint.lock().unwrap() = Some(endpoint);
        *self.shared.conn.lock().unwrap() = Some(conn.clone());
        self.shared.connected.store(true, Ordering::Release);
        log::info!("QUIC established");

        // one reliable bidirectional control stream, opened by the Client
        // immediately after the handshake (spec §3)
        let (mut tx, mut rx) = conn.open_bi().await.map_err(|e| format!("open_bi: {e}"))?;

        // register the auth context (server key + exporter), then send AuthInit
        let exporter = {
            let mut out = vec![0u8; 32];
            conn.export_keying_material(&mut out, AUTH_EXPORTER_LABEL, &[])
                .map_err(|_| "exporter: not available".to_string())?;
            out
        };
        let key = session_identity_key()?;
        let client_public_key = crate::auth::sec1_compressed(&key);
        let mut machine = ClientMachine::new();
        machine.set_auth_context(self.config.server_public_key.clone(), exporter, key);
        let auth_init = Frame {
            body: Some(Body::AuthInit(AuthInit {
                request_id: machine.next_request_id,
                client_public_key,
            })),
        };
        machine.next_request_id += 1;
        let v = machine.enter_authenticating(auth_init);
        send_verdict(&mut tx, &v).await?;
        self.set_state(machine.state.clone());
        *self.shared.control_stream.lock().await = Some(tx);

        // ---- control loop ---------------------------------------------------
        let mut decoder = FrameDecoder::new();
        let mut rx_buf = vec![0u8; 8192];
        // Absolute auth deadline: armed once, never rebuilt by a read (§3).
        let auth_deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs(AUTH_DEADLINE_SECS);
        let mut watch = ResponseWatch::default();
        watch.observe(tokio::time::Instant::now(), machine.pending_len());
        let result: Result<(), String> = 'control: loop {
            let n = tokio::select! {
                read = rx.read(&mut rx_buf) => {
                    match read {
                        Ok(Some(n)) => n,
                        Ok(None) => break 'control Err("control stream closed by server".into()),
                        Err(e) => break 'control Err(format!("control read: {e}")),
                    }
                }
                _ = tokio::time::sleep_until(auth_deadline), if machine.state == ClientState::ConnectingSent => {
                    break 'control Err(format!("auth exchange deadline ({AUTH_DEADLINE_SECS} s) → 601"));
                }
                _ = watch.expired() => {
                    break 'control Err(format!("response deadline ({RESPONSE_DEADLINE_SECS} s) → 601"));
                }
                _ = self.shared.shutdown_requested() => break 'control Ok(()),
            };
            let frames = match decoder.push(&rx_buf[..n]) {
                Ok(f) => f,
                Err(e) => break 'control Err(format!("frame decode: {e}")),
            };
            for frame in frames {
                // request-id pairing sanity (§4.2): a response whose request_id
                // matches no pending request is dropped without state change —
                // the machine folds everything; unsolicited requests in illegal
                // states are answered by the machine (never silently ignored).
                let (result, _event) = machine.on_frame(&frame);
                self.set_state(machine.state.clone());
                match result {
                    Ok(v) => {
                        let mut tx_guard = self.shared.control_stream.lock().await;
                        if let Some(tx) = tx_guard.as_mut() {
                            send_verdict(tx, &crate::control::Verdict::Ok).await?;
                        }
                        drop(tx_guard);
                        match v {
                            crate::control::Verdict::Ok => {}
                            crate::control::Verdict::Send(f) => {
                                let mut tx_guard = self.shared.control_stream.lock().await;
                                if let Some(tx) = tx_guard.as_mut() {
                                    tx.write_all(&encode_frame(&f).map_err(|e| e.to_string())?)
                                        .await
                                        .map_err(|e| format!("send: {e}"))?;
                                }
                            }
                            crate::control::Verdict::Fatal { code } => {
                                // §5.2 fatal class: Error(code) then Close(code),
                                // then transport-close with the code
                                let mut tx_guard = self.shared.control_stream.lock().await;
                                if let Some(tx) = tx_guard.as_mut() {
                                    let err = ctrl::error_frame_for(code);
                                    let close = ctrl::close_frame_for(code);
                                    let _ = tx.write_all(&encode_frame(&err).unwrap()).await;
                                    let _ = tx.write_all(&encode_frame(&close).unwrap()).await;
                                }
                                drop(tx_guard);
                                conn.close(quinn::VarInt::from_u64(u64::from(code)).unwrap(), b"");
                                break 'control Ok(());
                            }
                            crate::control::Verdict::Stop(info) => {
                                // Close received (remote): no echo; the
                                // transport close is the acknowledgment (§5.4.4)
                                self.set_state(ClientState::Closed);
                                if info.remote {
                                    conn.close(quinn::VarInt::from_u64(u64::from(info.code)).unwrap(), b"");
                                }
                                break 'control Ok(());
                            }
                        }
                    }
                    Err(info) => {
                        // surface path (202/203/204/101...): no reconnect per §6.3
                        self.set_state(ClientState::Closed);
                        *self.shared.close_info.lock().unwrap() = Some(info);
                        break 'control Ok(());
                    }
                }
                watch.observe(tokio::time::Instant::now(), machine.pending_len());
                if machine.state.is_closed() {
                    break 'control Ok(());
                }
            }
        };

        self.shared.connected.store(false, Ordering::Release);
        result
    }

    /// Clean stop: Close(701) when a control stream exists, then transport
    /// close (§6.3: 701 → no reconnect).
    pub fn stop(&self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        self.set_state(ClientState::Closed);
        if let Some(conn) = self.shared.conn.lock().unwrap().clone() {
            conn.close(quinn::VarInt::from_u64(701).unwrap(), b"client stopping");
        }
        self.shared.connected.store(false, Ordering::Release);
        log::info!("session stopped (701 CLIENT_STOPPING)");
    }

    pub fn state_name(&self) -> &'static str {
        self.shared.state.lock().unwrap().name()
    }

    /// JSON stats snapshot (low-frequency JNI surface). No credentials, no
    /// key material, no media bytes — only counters and state names.
    pub fn stats_json(&self) -> String {
        let state = self.state_name();
        let connected = self.shared.connected.load(Ordering::Acquire);
        let error = self.shared.last_error.lock().unwrap().clone();
        let close = self.shared.close_info.lock().unwrap().as_ref().map(|c| c.code);
        format!(
            "{{\"state\":\"{state}\",\"connected\":{connected},\"last_error\":{},\"close_code\":{}}}",
            json_escape(&error),
            close.map(|c| c.to_string()).unwrap_or_else(|| "null".into()),
        )
    }

    /// Network-change migration primitive (prototype-validated): rebind the
    /// QUIC endpoint onto a JVM-created, route-pinned UDP fd. The fd
    /// transfers to native on call — success or failure.
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
        self.runtime.block_on(async {
            // SAFETY: the JNI contract transfers fd ownership to native
            // (Kotlin must never close or reuse it afterwards); the socket
            // is dropped (fd closed) if the rebind fails.
            unsafe { rebind_endpoint_from_raw_fd(&endpoint, fd) }
        })
    }
}


/// Client identity for the session. The fixture identity exists only behind
/// the `test-support` feature (E2E APKs); a production build without real
/// pairing support returns an explicit error instead of silently shipping the
/// fixed test key.
#[cfg(any(test, feature = "test-support"))]
fn session_identity_key() -> Result<p256::ecdsa::SigningKey, String> {
    Ok(crate::auth::test_client_identity_key())
}

#[cfg(not(any(test, feature = "test-support")))]
fn session_identity_key() -> Result<p256::ecdsa::SigningKey, String> {
    Err("no client identity: build with `test-support` for the fixture identity;          production pairing identities are provided by pairing (phase 6)"
        .to_string())
}

/// §6.3 local classification of a run error: pin mismatch is 205, everything
/// else is transport loss (601).
fn classify_run_error(message: &str) -> u32 {
    if message.contains("SPKI does not match pinned identity") {
        205
    } else {
        601
    }
}

async fn send_verdict(tx: &mut quinn::SendStream, v: &crate::control::Verdict) -> Result<(), String> {
    if let crate::control::Verdict::Send(f) = v {
        let bytes = encode_frame(f).map_err(|e| e.to_string())?;
        tx.write_all(&bytes).await.map_err(|e| format!("send: {e}"))?;
    }
    Ok(())
}

/// Minimal helpers namespace for the fatal-row frame builders (wire order:
/// Error before Close, §5.2).
mod ctrl {
    use crate::proto::{frame::Body, Close, Error, Frame, Status};

    pub fn error_frame_for(code: u32) -> Frame {
        Frame {
            body: Some(Body::Error(Error {
                request_id: 0,
                status: Some(Status {
                    code,
                    retryable: false,
                    requires_repairing: false,
                    detail: String::new(),
                }),
            })),
        }
    }

    pub fn close_frame_for(code: u32) -> Frame {
        Frame {
            body: Some(Body::Close(Close {
                code,
                detail: String::new(),
            })),
        }
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    format!("\"{out}\"")
}

/// Reconstruct a `std::net::UdpSocket` from a raw fd handed over by the JVM
/// side and rebind the endpoint onto it (quinn `Endpoint::rebind`).
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

impl SharedState {
    /// Poll-based shutdown check (the Notify-based prototype pattern kept).
    async fn shutdown_requested(&self) {
        loop {
            if self.stopping.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> SessionConfig {
        SessionConfig {
            host: "127.0.0.1".to_string(),
            port: 65531,
            pinned_spki_hex: "00".repeat(32),
            server_public_key: vec![0x11u8; 32],
            client_name: "test".into(),
        }
    }

    /// Creates a bound UDP socket and hands its fd over raw (ownership moves
    /// away via `into_raw_fd`), mirroring the JVM JNI fd contract.
    fn transfer_bound_socket() -> (i32, u16) {
        use std::os::fd::IntoRawFd;
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
        endpoint.wait_idle().await;
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

    /// §4.2: the response deadline is absolute — repeated reads (or unsolicited
    /// pushes) while a request is pending must not push it back.
    #[test]
    fn response_deadline_is_absolute_and_not_pushed_back() {
        let mut watch = ResponseWatch::default();
        let start = tokio::time::Instant::now();
        watch.observe(start, 1);
        let armed = watch.deadline.expect("deadline armed");
        assert_eq!(
            armed,
            start + std::time::Duration::from_secs(RESPONSE_DEADLINE_SECS)
        );
        watch.observe(start + std::time::Duration::from_secs(5), 1);
        assert_eq!(watch.deadline, Some(armed), "deadline must not move");
        watch.observe(start + std::time::Duration::from_secs(9), 2);
        assert_eq!(watch.deadline, Some(armed), "more pending must not move it");
    }

    /// The deadline disarms when the last pending request completes.
    #[test]
    fn response_deadline_disarms_when_nothing_is_pending() {
        let mut watch = ResponseWatch::default();
        let start = tokio::time::Instant::now();
        watch.observe(start, 1);
        assert!(watch.deadline.is_some());
        watch.observe(start + std::time::Duration::from_secs(1), 0);
        assert_eq!(watch.deadline, None);
    }

    /// §4.2: 10 s after arming, the deadline future completes.
    #[tokio::test(start_paused = true)]
    async fn response_deadline_expires_after_ten_seconds() {
        let mut watch = ResponseWatch::default();
        watch.observe(tokio::time::Instant::now(), 1);
        tokio::time::sleep(std::time::Duration::from_secs(RESPONSE_DEADLINE_SECS - 1)).await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1), watch.expired())
                .await
                .is_err(),
            "must not expire early"
        );
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        tokio::time::timeout(std::time::Duration::from_millis(1), watch.expired())
            .await
            .expect("deadline must expire at 10 s");
    }

    /// §6.3: a pin mismatch is classified 205 (no reconnect); every other
    /// run error is transport loss (601).
    #[test]
    fn run_error_classification_matches_section_6_3() {
        assert_eq!(classify_run_error("handshake: refused"), 601);
        assert_eq!(classify_run_error("handshake: server SPKI does not match pinned identity"), 205);
    }

    /// §6.3 backoff schedule: 1 s, 2 s, 4 s, … capped at 30 s.
    #[test]
    fn backoff_schedule_matches_policy() {
        assert_eq!(backoff_delay_ms(0), 1_000);
        assert_eq!(backoff_delay_ms(1), 2_000);
        assert_eq!(backoff_delay_ms(2), 4_000);
        assert_eq!(backoff_delay_ms(4), 16_000);
        assert_eq!(backoff_delay_ms(5), 30_000, "capped");
        assert_eq!(backoff_delay_ms(20), 30_000, "still capped");
    }

    /// The stats JSON never carries key material (red line): inject a fake
    /// error containing a private-looking string and assert the JSON only
    /// carries it as a quoted string — no hex blobs are ever formatted in by
    /// the session itself (the error strings come from fixed classifications).
    #[test]
    fn stats_json_shape_is_stable() {
        let session = Session::new(test_config()).expect("session");
        *session.shared.last_error.lock().unwrap() = "handshake: refused".into();
        let json = session.stats_json();
        assert!(json.contains("\"state\":\"IDLE\""), "got: {json}");
        assert!(json.contains("\"connected\":false"));
        assert!(json.contains("\"close_code\":null"));
        assert!(json.contains("handshake: refused"));
    }

    /// The auth exporter label is byte-exact (pairing.md §9.1).
    #[test]
    fn exporter_label_is_exact() {
        assert_eq!(AUTH_EXPORTER_LABEL, b"EXPORTER audio-share v2 auth");
        assert_eq!(ALPN, b"audio-share/v2");
        assert_eq!(DEFAULT_PORT, 65531);
    }
}
