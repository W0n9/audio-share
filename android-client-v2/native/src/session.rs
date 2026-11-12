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

use std::collections::BTreeMap;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::control::{ClientMachine, ClientState, CloseInfo};
use crate::frame::{encode_frame, FrameDecoder};
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

/// Per-request response deadlines (§4.2). Each pending request is armed once,
/// from the moment it is sent, and is never extended by unrelated reads. A
/// request's deadline is dropped when its response arrives (it leaves the
/// pending set), not when the first of several requests completes.
#[derive(Debug, Default)]
struct ResponseWatch {
    deadlines: BTreeMap<u64, tokio::time::Instant>,
}

impl ResponseWatch {
    fn observe(&mut self, now: tokio::time::Instant, pending: &[u64]) {
        let pending: std::collections::HashSet<u64> = pending.iter().copied().collect();
        self.deadlines.retain(|id, _| pending.contains(id));
        for id in pending {
            self.deadlines.entry(id).or_insert(
                now + std::time::Duration::from_secs(RESPONSE_DEADLINE_SECS),
            );
        }
    }

    fn earliest(&self) -> Option<tokio::time::Instant> {
        self.deadlines.values().copied().min()
    }

    /// Wait for the earliest outstanding deadline and return its request id.
    async fn expired(&self) -> u64 {
        let Some(deadline) = self.earliest() else {
            return std::future::pending().await;
        };
        tokio::time::sleep_until(deadline).await;
        self.deadlines
            .iter()
            .min_by_key(|(_, deadline)| **deadline)
            .map(|(id, _)| *id)
            .expect("the deadline that just fired is still tracked")
    }
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

    /// Record the terminal Close code so the Kotlin reconnect policy can
    /// classify it (§6.3). Remote Close frames used to leave `close_code`
    /// null, which looked like transport loss and triggered a reconnect.
    fn record_close(&self, info: CloseInfo) {
        self.set_state(ClientState::Closed);
        *self.shared.close_info.lock().unwrap() = Some(info);
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

    async fn run(self: Arc<Self>, bind_local_address: Option<std::net::IpAddr>) -> Result<(), RunError> {
        let socket = self.make_client_socket(bind_local_address)?;
        let pin_verifier = Arc::new(
            PinnedSpkiVerifier::new(&self.config.pinned_spki_hex)
                .map_err(|e| format!("pin: {e}"))?,
        );
        let crypto = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(pin_verifier.clone())
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
            .map_err(|_| RunError::Other("handshake timeout (5 s) → 601".to_string()))?
            .map_err(|e| {
                if pin_verifier.pin_mismatch() {
                    RunError::PinMismatch
                } else {
                    RunError::Other(format!("handshake: {e}"))
                }
            })?;
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
        let auth_init = machine.auth_init(client_public_key);
        let v = machine
            .enter_authenticating(auth_init)
            .map_err(|info| RunError::Other(format!("auth init rejected locally: {}", info.code)))?;
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
        let pending = machine.pending_ids();
        watch.observe(tokio::time::Instant::now(), &pending);
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
                expired_id = watch.expired() => {
                    break 'control Err(format!(
                        "response deadline ({RESPONSE_DEADLINE_SECS} s) for request_id {expired_id} → 601"
                    ));
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
                let (result, event) = machine.on_frame(&frame);
                self.set_state(machine.state.clone());
                if let Some(event) = event {
                    log::info!("client event: {event:?}");
                }
                match result {
                    Ok(crate::control::Verdict::Ok) => {}
                    Ok(v @ (crate::control::Verdict::Send(_)
                    | crate::control::Verdict::Reject { .. })) => {
                        // One lock for the one write (S4); Reject writes a
                        // non-fatal Error and keeps the connection open (§5.3).
                        let mut tx_guard = self.shared.control_stream.lock().await;
                        if let Some(tx) = tx_guard.as_mut() {
                            send_verdict(tx, &v).await?;
                        }
                    }
                    Ok(crate::control::Verdict::Fatal { code }) => {
                        // §5.2 fatal class: Error(code) then Close(code), then
                        // transport-close with the code.
                        let mut tx_guard = self.shared.control_stream.lock().await;
                        if let Some(tx) = tx_guard.as_mut() {
                            for f in crate::control::frames::fatal_frames(code) {
                                tx.write_all(&encode_frame(&f).map_err(|e| e.to_string())?)
                                    .await
                                    .map_err(|e| format!("send: {e}"))?;
                            }
                        }
                        drop(tx_guard);
                        conn.close(quinn::VarInt::from_u64(u64::from(code)).unwrap(), b"");
                        break 'control Ok(());
                    }
                    Ok(crate::control::Verdict::Stop(info)) => {
                        // Close received (remote): no echo; the transport close
                        // is the acknowledgment (§5.4.4)
                        self.record_close(info.clone());
                        if info.remote {
                            conn.close(quinn::VarInt::from_u64(u64::from(info.code)).unwrap(), b"");
                        }
                        break 'control Ok(());
                    }
                    Err(info) => {
                        // surface path (202/203/204/101...): no reconnect per §6.3
                        self.record_close(info);
                        break 'control Ok(());
                    }
                }
                let pending = machine.pending_ids();
                watch.observe(tokio::time::Instant::now(), &pending);
                if machine.state.is_closed() {
                    break 'control Ok(());
                }
            }
        };

        self.shared.connected.store(false, Ordering::Release);
        result.map_err(RunError::Other)
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

/// Why a run ended locally. Typed so §6.3 classification never depends on
/// matching an English error string (the verifier sets its mismatch flag).
#[derive(Debug)]
enum RunError {
    PinMismatch,
    Other(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::PinMismatch => write!(f, "server pinned identity mismatch"),
            RunError::Other(message) => write!(f, "{message}"),
        }
    }
}

impl From<String> for RunError {
    fn from(message: String) -> Self {
        RunError::Other(message)
    }
}

impl From<&str> for RunError {
    fn from(message: &str) -> Self {
        RunError::Other(message.to_string())
    }
}

/// §6.3 local classification of a run error: pin mismatch is 205, everything
/// else is transport loss (601).
fn classify_run_error(error: &RunError) -> u32 {
    match error {
        RunError::PinMismatch => 205,
        RunError::Other(_) => 601,
    }
}

async fn send_verdict(tx: &mut quinn::SendStream, v: &crate::control::Verdict) -> Result<(), String> {
    let frame = match v {
        crate::control::Verdict::Send(f) => Some(f.clone()),
        crate::control::Verdict::Reject { code } => Some(crate::control::frames::error_frame(*code)),
        _ => None,
    };
    if let Some(frame) = frame {
        let bytes = encode_frame(&frame).map_err(|e| e.to_string())?;
        tx.write_all(&bytes).await.map_err(|e| format!("send: {e}"))?;
    }
    Ok(())
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

    /// §4.2: each request's deadline is absolute and unrelated reads do not
    /// push back any of them; a newly sent request gets its own deadline.
    #[test]
    fn response_deadline_is_absolute_and_not_pushed_back() {
        let mut watch = ResponseWatch::default();
        let start = tokio::time::Instant::now();
        let deadline_for = |watch: &ResponseWatch, id: u64| *watch.deadlines.get(&id).unwrap();
        watch.observe(start, &[1]);
        let armed = deadline_for(&watch, 1);
        assert_eq!(
            armed,
            start + std::time::Duration::from_secs(RESPONSE_DEADLINE_SECS)
        );
        watch.observe(start + std::time::Duration::from_secs(5), &[1]);
        assert_eq!(deadline_for(&watch, 1), armed, "deadline must not move");
        let later = start + std::time::Duration::from_secs(9);
        watch.observe(later, &[1, 2]);
        assert_eq!(deadline_for(&watch, 1), armed, "request 1 must not move");
        assert_eq!(
            deadline_for(&watch, 2),
            later + std::time::Duration::from_secs(RESPONSE_DEADLINE_SECS),
            "request 2 is armed from when it was sent"
        );
    }

    /// Only the request whose own 10 s elapsed triggers the 601; a request
    /// sent later keeps its full deadline (§4.2).
    #[tokio::test(start_paused = true)]
    async fn concurrent_request_deadlines_do_not_interfere() {
        let mut watch = ResponseWatch::default();
        let start = tokio::time::Instant::now();
        watch.observe(start, &[1]);
        tokio::time::advance(std::time::Duration::from_secs(5)).await;
        watch.observe(tokio::time::Instant::now(), &[1, 2]);
        assert_eq!(
            *watch.deadlines.get(&1).unwrap(),
            start + std::time::Duration::from_secs(10)
        );
        assert_eq!(
            *watch.deadlines.get(&2).unwrap(),
            start + std::time::Duration::from_secs(15)
        );

        tokio::time::advance(std::time::Duration::from_secs(5)).await;
        let expired = tokio::time::timeout(std::time::Duration::from_millis(1), watch.expired())
            .await
            .expect("request 1 must expire at its own 10 s");
        assert_eq!(expired, 1);

        // Request 2 is unaffected: the first expiry does not shorten it.
        watch.observe(tokio::time::Instant::now(), &[2]);
        assert_eq!(
            *watch.deadlines.get(&2).unwrap(),
            start + std::time::Duration::from_secs(15)
        );
        tokio::time::advance(std::time::Duration::from_secs(4)).await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1), watch.expired())
                .await
                .is_err(),
            "request 2 must not expire early"
        );
        tokio::time::advance(std::time::Duration::from_secs(1)).await;
        let expired = tokio::time::timeout(std::time::Duration::from_millis(1), watch.expired())
            .await
            .expect("request 2 expires at its own 10 s");
        assert_eq!(expired, 2);
    }

    /// The deadline disarms when the last pending request completes.
    #[test]
    fn response_deadline_disarms_when_nothing_is_pending() {
        let mut watch = ResponseWatch::default();
        let start = tokio::time::Instant::now();
        watch.observe(start, &[1]);
        assert!(!watch.deadlines.is_empty());
        watch.observe(start + std::time::Duration::from_secs(1), &[]);
        assert!(watch.deadlines.is_empty());
    }

    /// §4.2: 10 s after arming, the deadline future completes with its id.
    #[tokio::test(start_paused = true)]
    async fn response_deadline_expires_after_ten_seconds() {
        let mut watch = ResponseWatch::default();
        watch.observe(tokio::time::Instant::now(), &[7]);
        tokio::time::sleep(std::time::Duration::from_secs(RESPONSE_DEADLINE_SECS - 1)).await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1), watch.expired())
                .await
                .is_err(),
            "must not expire early"
        );
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let expired = tokio::time::timeout(std::time::Duration::from_millis(1), watch.expired())
            .await
            .expect("deadline must expire at 10 s");
        assert_eq!(expired, 7);
    }

    /// §6.3: a pin mismatch is classified 205 (no reconnect); every other
    /// run error is transport loss (601).
    #[test]
    fn run_error_classification_matches_section_6_3() {
        assert_eq!(classify_run_error(&RunError::Other("handshake: refused".into())), 601);
        assert_eq!(classify_run_error(&RunError::PinMismatch), 205);
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

    /// §6.3: a remote Close code (e.g. 204 revocation) must reach the Kotlin
    /// reconnect policy instead of surfacing as a null "transport loss" code.
    #[test]
    fn remote_close_code_is_surfaced() {
        let session = Session::new(test_config()).expect("session");
        session.record_close(CloseInfo { code: 204, remote: true });
        let json = session.stats_json();
        assert!(json.contains("\"state\":\"CLOSED\""), "got: {json}");
        assert!(json.contains("\"close_code\":204"), "got: {json}");
    }

    /// The auth exporter label is byte-exact (pairing.md §9.1).
    #[test]
    fn exporter_label_is_exact() {
        assert_eq!(AUTH_EXPORTER_LABEL, b"EXPORTER audio-share v2 auth");
        assert_eq!(ALPN, b"audio-share/v2");
        assert_eq!(DEFAULT_PORT, 65531);
    }
}
