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
use crate::media_session::{MediaSession, PlaybackReport};
use crate::pairing::{
    ClientPairingMode, PairedCredential, PairingExporter, PairingInfo, PairingPhase, SharedExporter,
};
use crate::signer::SharedSigner;
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
/// pairing.md §5: the complete pairing exchange deadline (10 minutes).
const PAIRING_DEADLINE_SECS: u64 = 10 * 60;
/// §4.2: per-request response deadline.
const RESPONSE_DEADLINE_SECS: u64 = 10;
/// §3/D3: idle timeout and keep-alive, both sides.
const IDLE_TIMEOUT_SECS: u64 = 30;
const KEEP_ALIVE_SECS: u64 = 10;
/// How often the control loop checks the media cadences (§9 5 s reports; the
/// §7.6 give-up bound). Short enough to react promptly, long enough to stay
/// off the hot path.
const MEDIA_POLL: std::time::Duration = std::time::Duration::from_millis(250);
/// §9 `buffer_target_ms` reported for the M5-03 jitter buffer (D12: 60 ms).
/// Mirrors `JitterConfig::default().target_ms`; the buffer now really exists.
const BUFFER_TARGET_MS: u32 = 60;

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
            self.deadlines
                .entry(id)
                .or_insert(now + std::time::Duration::from_secs(RESPONSE_DEADLINE_SECS));
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

/// How this session proves its identity.
pub enum SessionIdentity {
    /// Paired reconnection: pinned Server SPKI + stored Server key + the
    /// Client identity signer (Keystore in production).
    Auth {
        /// The stored pin (64 hex chars).
        pinned_spki_hex: String,
        /// The stored raw 32-byte Ed25519 Server identity key.
        server_public_key: Vec<u8>,
        /// The Client identity (non-exportable in production).
        signer: SharedSigner,
    },
    /// QR pairing: pinned Server SPKI + nonce; sends `PairingRequest`.
    PairQr {
        /// Pin derived from the QR key.
        pinned_spki_hex: String,
        /// The QR key (raw 32-byte Ed25519).
        server_public_key: Vec<u8>,
        /// The single-use 16-byte nonce.
        nonce: Vec<u8>,
        /// Display name offered to the Server (≤ 63 bytes).
        display_name: String,
        /// The Client identity signer.
        signer: SharedSigner,
    },
    /// SAS pairing: first contact without a pin (TOFU + human comparison).
    PairSas {
        /// Display name offered to the Server (≤ 63 bytes).
        display_name: String,
        /// The Client identity signer.
        signer: SharedSigner,
    },
}

impl SessionIdentity {
    fn is_pairing(&self) -> bool {
        matches!(
            self,
            SessionIdentity::PairQr { .. } | SessionIdentity::PairSas { .. }
        )
    }
}

/// Session configuration.
pub struct SessionConfig {
    pub host: String,
    pub port: u16,
    /// Client display name (≤ 63 bytes UTF-8, pairing.md §6.2). Used by the
    /// test-support convenience session; pairing modes carry their own name.
    pub client_name: String,
    /// How the session establishes/proves its identity.
    pub identity: SessionIdentity,
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
    /// Connection handle for the datagram pump and migration.
    pub conn: Mutex<Option<quinn::Connection>>,
    /// Endpoint kept alive for the connection lifetime (rebind primitive).
    pub endpoint: Mutex<Option<quinn::Endpoint>>,
    /// The active media delivery state (PLAYING only). Shared with the JNI
    /// surface so Kotlin's ring cursor and playback reports reach the receiver.
    pub media: Mutex<Option<Arc<Mutex<MediaSession>>>>,
    /// The JVM-owned direct ByteBuffer attached once per session. Media never
    /// crosses JNI per datagram: this is the ring's backing memory.
    pub ring: Mutex<Option<RingAttachment>>,
    /// Pairing display material / phase (no secrets beyond SAS/fingerprint,
    /// which are user-visible display material and are never logged).
    pub pairing: Mutex<Option<PairingInfo>>,
    /// The credential to persist after `PairingResult(OK)`.
    pub credential: Mutex<Option<PairedCredential>>,
    /// Requests a user-confirmed pairing from the control loop.
    confirm_tx: tokio::sync::mpsc::Sender<()>,
    /// The control loop takes this once at start.
    confirm_rx: Mutex<Option<tokio::sync::mpsc::Receiver<()>>>,
}

/// The native side of the one-time direct-`ByteBuffer` attachment.
#[derive(Clone, Copy)]
pub struct RingAttachment {
    /// Base address of the JVM's direct buffer (valid for the session's life).
    pub base: *mut u8,
    /// Writable capacity in bytes.
    pub capacity: usize,
}

// The buffer is owned by the JVM for the whole session; native only writes to
// it. Access is serialized by the media mutex.
unsafe impl Send for RingAttachment {}

pub struct Session {
    pub config: SessionConfig,
    pub shared: Arc<SharedState>,
    runtime: tokio::runtime::Runtime,
}

/// Terminal event emitted by the session's public lifecycle boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    Closed(CloseInfo),
}

/// A malformed control frame is a protocol-terminal event, not network loss.
pub fn session_event_for_frame_error(_error: &crate::frame::FramedError) -> SessionEvent {
    SessionEvent::Closed(CloseInfo {
        code: 401,
        remote: false,
    })
}

impl Session {
    /// Build the session (no I/O yet). Called from the JNI thread.
    pub fn new(config: SessionConfig) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| format!("tokio runtime: {e}"))?;
        let (confirm_tx, confirm_rx) = tokio::sync::mpsc::channel(4);
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
                media: Mutex::new(None),
                ring: Mutex::new(None),
                pairing: Mutex::new(None),
                credential: Mutex::new(None),
                confirm_tx,
                confirm_rx: Mutex::new(Some(confirm_rx)),
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
        // Write the close info before publishing CLOSED. Otherwise a reader
        // can observe CLOSED with close_code == null and misclassify a remote
        // close as transport loss (ticket 15, behaviour-neutral cleanup).
        *self.shared.close_info.lock().unwrap() = Some(info);
        self.set_state(ClientState::Closed);
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
    pub fn connect(
        self: &Arc<Self>,
        bind_local_address: Option<std::net::IpAddr>,
    ) -> Result<(), String> {
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

    async fn run(
        self: Arc<Self>,
        bind_local_address: Option<std::net::IpAddr>,
    ) -> Result<(), RunError> {
        let socket = self.make_client_socket(bind_local_address)?;
        let pin_verifier = Arc::new(match &self.config.identity {
            // SAS first contact is TOFU-with-verification: no pin yet, but the
            // presented SPKI is recorded so it can be persisted after pairing.
            SessionIdentity::PairSas { .. } => PinnedSpkiVerifier::unpinned_accepting(),
            SessionIdentity::Auth {
                pinned_spki_hex, ..
            }
            | SessionIdentity::PairQr {
                pinned_spki_hex, ..
            } => PinnedSpkiVerifier::new(pinned_spki_hex).map_err(|e| format!("pin: {e}"))?,
        });
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
        transport.keep_alive_interval(Some(std::time::Duration::from_secs(KEEP_ALIVE_SECS)));
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

        log::info!(
            "connecting {}:{} ALPN {}",
            self.config.host,
            self.config.port,
            String::from_utf8_lossy(ALPN)
        );
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

        // One reliable bidirectional control stream, opened by the Client
        // immediately after the handshake (spec §3).
        let (mut tx, mut rx) = conn.open_bi().await.map_err(|e| format!("open_bi: {e}"))?;

        // The two identity paths: paired reconnection sends AuthInit; QR/SAS
        // sends PairingRequest. Pairing uses a per-connection exporter seam so
        // the pure state machine can derive binding/SAS without quinn.
        let mut machine = ClientMachine::new();
        let pairing_exporter: Option<SharedExporter> = if self.config.identity.is_pairing() {
            Some(Arc::new(QuicPairingExporter { conn: conn.clone() }))
        } else {
            None
        };
        let verdict = match &self.config.identity {
            SessionIdentity::Auth {
                server_public_key,
                signer,
                ..
            } => {
                let mut exporter = vec![0u8; 32];
                conn.export_keying_material(&mut exporter, AUTH_EXPORTER_LABEL, &[])
                    .map_err(|_| "exporter: not available".to_string())?;
                machine
                    .set_auth_context(server_public_key.clone(), exporter, Arc::clone(signer))
                    .map_err(|e| format!("signer: {e}"))?;
                let client_public_key = signer
                    .public_key_sec1()
                    .map_err(|e| format!("signer public key: {e}"))?;
                let auth_init = machine.auth_init(client_public_key);
                machine.enter_authenticating(auth_init).map_err(|info| {
                    RunError::Other(format!("auth init rejected locally: {}", info.code))
                })?
            }
            SessionIdentity::PairQr {
                server_public_key,
                nonce,
                display_name,
                signer,
                ..
            } => {
                let mut client_random = [0u8; 32];
                rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut client_random);
                machine
                    .begin_pairing(
                        ClientPairingMode::Qr,
                        nonce.clone(),
                        client_random,
                        display_name.clone(),
                        Arc::clone(signer),
                        pairing_exporter.clone().expect("pairing exporter"),
                        Some(server_public_key.clone()),
                    )
                    .map_err(|info| {
                        RunError::Other(format!("pairing init rejected locally: {}", info.code))
                    })?
            }
            SessionIdentity::PairSas {
                display_name,
                signer,
            } => {
                let mut client_random = [0u8; 32];
                rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut client_random);
                machine
                    .begin_pairing(
                        ClientPairingMode::Sas,
                        Vec::new(),
                        client_random,
                        display_name.clone(),
                        Arc::clone(signer),
                        pairing_exporter.clone().expect("pairing exporter"),
                        None,
                    )
                    .map_err(|info| {
                        RunError::Other(format!("pairing init rejected locally: {}", info.code))
                    })?
            }
        };
        send_verdict(&mut tx, &verdict).await?;
        self.set_state(machine.state.clone());
        *self.shared.control_stream.lock().await = Some(tx);
        let confirm_rx = self
            .shared
            .confirm_rx
            .lock()
            .unwrap()
            .take()
            .ok_or("confirm channel already taken")?;
        let mut confirm_rx = confirm_rx;

        // ---- control loop ---------------------------------------------------
        let mut decoder = FrameDecoder::new();
        let mut rx_buf = vec![0u8; 8192];
        // Absolute auth deadline: armed once, never rebuilt by a read (§3).
        let auth_deadline =
            tokio::time::Instant::now() + std::time::Duration::from_secs(AUTH_DEADLINE_SECS);
        // The pairing exchange may wait for a human on both ends; it owns the
        // 10-minute deadline instead of the 10 s auth deadline (pairing.md §5).
        let pairing_deadline =
            tokio::time::Instant::now() + std::time::Duration::from_secs(PAIRING_DEADLINE_SECS);
        let mut watch = ResponseWatch::default();
        let pending = machine.pending_ids();
        watch.observe(tokio::time::Instant::now(), &pending);
        // Media cadence (§5.3 READY subscribe, §9 5 s diagnostics) and the
        // §7.6 unknown-configuration give-up clock.
        let mut media_control = crate::media_control::MediaControl::new();
        let mut media_tick = tokio::time::interval(MEDIA_POLL);
        media_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let media_clock = std::time::Instant::now();
        let result: Result<(), String> = 'control: loop {
            let n = tokio::select! {
                read = rx.read(&mut rx_buf) => {
                    match read {
                        Ok(Some(n)) => n,
                        Ok(None) => break 'control Err("control stream closed by server".into()),
                        Err(e) => break 'control Err(format!("control read: {e}")),
                    }
                }
                _ = tokio::time::sleep_until(auth_deadline), if machine.state == ClientState::Authenticating && machine.pairing_info().is_none() => {
                    break 'control Err(format!("auth exchange deadline ({AUTH_DEADLINE_SECS} s) → 601"));
                }
                _ = tokio::time::sleep_until(pairing_deadline), if machine.pairing_info().is_some() => {
                    if let Some(info) = machine.pairing_info() {
                        if info.phase != PairingPhase::Complete {
                            break 'control Err("pairing deadline (10 min) → 206".to_string());
                        }
                    }
                    continue;
                }
                confirmed = confirm_rx.recv(), if machine.pairing_info().is_some() => {
                    if confirmed.is_some() {
                        match machine.confirm_pairing() {
                            Ok(v) => {
                                let mut tx_guard = self.shared.control_stream.lock().await;
                                if let Some(tx) = tx_guard.as_mut() {
                                    send_verdict(tx, &v).await?;
                                }
                                drop(tx_guard);
                                self.set_state(machine.state.clone());
                                *self.shared.pairing.lock().unwrap() =
                                    machine.pairing_info().cloned();
                            }
                            Err(info) => {
                                self.record_error(format!(
                                    "pairing confirm rejected locally: {}",
                                    info.code
                                ));
                                self.record_close(info);
                                break 'control Ok(());
                            }
                        }
                    }
                    continue;
                }
                expired_id = watch.expired() => {
                    break 'control Err(format!(
                        "response deadline ({RESPONSE_DEADLINE_SECS} s) for request_id {expired_id} → 601"
                    ));
                }
                _ = self.shared.shutdown_requested() => {
                    // §5.3/§9: entering CLOSING sends one best-effort final
                    // PlaybackDiagnostics before Close(701), then stops. The
                    // final snapshot must never wait for, or be blocked by,
                    // the 5 s periodic cadence or an in-flight report.
                    machine.state = ClientState::Closing;
                    self.set_state(machine.state.clone());
                    if let crate::media_control::MediaAction::ReportDiagnostics =
                        media_control.on_tick(&machine.state, machine.config.is_some(), std::time::Instant::now())
                    {
                        let mut tx_guard = self.shared.control_stream.lock().await;
                        if let Some(tx) = tx_guard.as_mut() {
                            let _ = self.write_diagnostics(tx).await;
                        }
                        drop(tx_guard);
                        media_control.report_finished();
                    }
                    // Close is one-way (§5.4 invariant 4); no echo expected.
                    let close = crate::control::frames::close_frame(701);
                    let mut tx_guard = self.shared.control_stream.lock().await;
                    if let (Some(tx), Ok(bytes)) = (tx_guard.as_mut(), encode_frame(&close)) {
                        let _ = tx.write_all(&bytes).await;
                    }
                    drop(tx_guard);
                    break 'control Ok(());
                }
                datagram = conn.read_datagram() => {
                    match datagram {
                        Ok(datagram) => {
                            let now_ms = media_clock.elapsed().as_millis() as u64;
                            let current = self.shared.media.lock().unwrap().clone();
                            let gave_up = if let Some(media) = current {
                                let mut media = media.lock().unwrap();
                                let _outcome = media.on_datagram(now_ms, &datagram);
                                media.gave_up_unknown_config()
                            } else {
                                false
                            };
                            if gave_up {
                                // §7.6 step 3: Error(403)+Close(403), then
                                // transport-close is the acknowledgment.
                                let verdict = machine.fatal_unsupported_media();
                                self.set_state(machine.state.clone());
                                self.write_fatal(&verdict).await?;
                                break 'control Ok(());
                            }
                            continue;
                        }
                        Err(error) => {
                            log::debug!("datagram receive ended: {error}");
                            break 'control Err(format!("datagram: {error}"));
                        }
                    }
                }
                _ = media_tick.tick() => {
                    let now_ms = media_clock.elapsed().as_millis() as u64;
                    // §7.6 give-up clock advances even without new datagrams.
                    // The tick returns true exactly when the 2 s bound is
                    // crossed while the hold is still active, so the session
                    // emits 403 once and does not wait for another datagram.
                    let gave_up = if let Some(media) = self.shared.media.lock().unwrap().clone() {
                        media.lock().unwrap().tick(now_ms)
                    } else {
                        false
                    };
                    if gave_up {
                        let verdict = machine.fatal_unsupported_media();
                        self.set_state(machine.state.clone());
                        self.write_fatal(&verdict).await?;
                        break 'control Ok(());
                    }
                    match media_control.on_tick(
                        &machine.state,
                        machine.config.is_some(),
                        std::time::Instant::now(),
                    ) {
                        crate::media_control::MediaAction::Subscribe => {
                            let verdict = machine.request_subscribe();
                            let mut tx_guard = self.shared.control_stream.lock().await;
                            if let Some(tx) = tx_guard.as_mut() {
                                send_verdict(tx, &verdict).await?;
                            }
                            drop(tx_guard);
                            self.set_state(machine.state.clone());
                            let pending = machine.pending_ids();
                            watch.observe(tokio::time::Instant::now(), &pending);
                        }
                        crate::media_control::MediaAction::ReportDiagnostics => {
                            // §9: send the snapshot on the SAME tick that
                            // produced it. Previously this branch `continue`d
                            // and the send only ran after a Server control
                            // frame arrived — during PLAYING (no control
                            // traffic) the 5 s report never went out and the
                            // pending flag never cleared.
                            let mut tx_guard = self.shared.control_stream.lock().await;
                            if let Some(tx) = tx_guard.as_mut() {
                                self.write_diagnostics(tx).await;
                            }
                            drop(tx_guard);
                            media_control.report_finished();
                        }
                        crate::media_control::MediaAction::None => {}
                    }
                    continue;
                }
            };
            let frames = match decoder.push(&rx_buf[..n]) {
                Ok(f) => f,
                Err(e) => {
                    let SessionEvent::Closed(info) = session_event_for_frame_error(&e);
                    self.record_error(format!("frame decode: {e}"));
                    self.record_close(info);
                    if let Some(mut tx) = self.shared.control_stream.lock().await.take() {
                        for frame in crate::control::frames::fatal_frames(401) {
                            let bytes = encode_frame(&frame).map_err(|error| error.to_string())?;
                            tx.write_all(&bytes)
                                .await
                                .map_err(|error| format!("send: {error}"))?;
                        }
                    }
                    conn.close(quinn::VarInt::from_u32(401), b"malformed message");
                    break 'control Ok(());
                }
            };
            for frame in frames {
                // request-id pairing sanity (§4.2): a response whose request_id
                // matches no pending request is dropped without state change —
                // the machine folds everything; unsolicited requests in illegal
                // states are answered by the machine (never silently ignored).
                let (result, event) = machine.on_frame(&frame);
                self.set_state(machine.state.clone());
                if let Some(info) = machine.pairing_info() {
                    *self.shared.pairing.lock().unwrap() = Some(info.clone());
                }
                if let Some(event) = event {
                    log::info!("client event: {event:?}");
                    match event {
                        crate::control::Event::PairingReady(info) => {
                            // Belt and braces: the certificate the TLS layer
                            // accepted must be the key the Server named in
                            // PairingResponse. In SAS mode this is the TOFU pin
                            // we will persist; in QR mode the TLS pin already
                            // enforced it.
                            if let (Some(expected), Some(observed)) = (
                                info.server_spki_sha256_hex.as_ref(),
                                pin_verifier.observed_pin(),
                            ) {
                                let observed_hex = observed
                                    .iter()
                                    .map(|b| format!("{b:02x}"))
                                    .collect::<String>();
                                if &observed_hex != expected {
                                    self.record_error(
                                        "pairing: presented SPKI does not match the Server key"
                                            .to_string(),
                                    );
                                    self.record_close(CloseInfo {
                                        code: 205,
                                        remote: false,
                                    });
                                    break 'control Ok(());
                                }
                            }
                            *self.shared.pairing.lock().unwrap() = Some(info);
                        }
                        crate::control::Event::Paired(credential) => {
                            *self.shared.credential.lock().unwrap() = Some(credential);
                            *self.shared.pairing.lock().unwrap() = machine.pairing_info().cloned();
                        }
                        crate::control::Event::StreamStarted(anchor) => {
                            // §5.3: initialize per-instance media state; the
                            // reliable push is the only epoch authority.
                            let ring = self.shared.ring.lock().unwrap().map(|attachment| {
                                // SAFETY: the JVM owns the buffer for the whole
                                // session; the attachment is set once.
                                unsafe {
                                    crate::ring::MediaRing::attach(
                                        attachment.base,
                                        attachment.capacity,
                                    )
                                }
                            });
                            let started = machine.config.as_ref().and_then(|configuration| {
                                MediaSession::start(
                                    anchor.stream_instance_id,
                                    anchor.config_generation,
                                    anchor.initial_timestamp,
                                    configuration,
                                    ring,
                                )
                            });
                            if let Some(media) = started {
                                *self.shared.media.lock().unwrap() =
                                    Some(Arc::new(Mutex::new(media)));
                            } else {
                                // §5.3 READY: a configuration this Client cannot
                                // decode is Error(403)+Close(403).
                                let verdict = machine.fatal_unsupported_media();
                                self.set_state(machine.state.clone());
                                if let crate::control::Verdict::Fatal { code } = verdict {
                                    let mut tx_guard = self.shared.control_stream.lock().await;
                                    if let Some(tx) = tx_guard.as_mut() {
                                        for f in crate::control::frames::fatal_frames(code) {
                                            tx.write_all(
                                                &encode_frame(&f).map_err(|e| e.to_string())?,
                                            )
                                            .await
                                            .map_err(|e| format!("send: {e}"))?;
                                        }
                                    }
                                    self.record_close(crate::control::CloseInfo {
                                        code,
                                        remote: false,
                                    });
                                    break 'control Ok(());
                                }
                            }
                        }
                        crate::control::Event::StreamEnded(_) => {
                            *self.shared.media.lock().unwrap() = None;
                            media_control.stream_ended();
                        }
                        crate::control::Event::ConfigurationPublished(_) => {
                            let current = self.shared.media.lock().unwrap().clone();
                            if let (Some(media), Some(configuration)) =
                                (current, machine.config.as_ref())
                            {
                                media.lock().unwrap().apply_configuration(configuration);
                            }
                        }
                        _ => {}
                    }
                }
                match result {
                    Ok(crate::control::Verdict::Ok) => {}
                    Ok(
                        v @ (crate::control::Verdict::Send(_)
                        | crate::control::Verdict::Reject { .. }),
                    ) => {
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
                        self.record_close(crate::control::CloseInfo {
                            code,
                            remote: false,
                        });
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
        // Record the terminal user-stop close before tearing state down, so
        // `close_code == null` only ever means "no outcome recorded yet".
        *self.shared.close_info.lock().unwrap() = Some(CloseInfo {
            code: 701,
            remote: false,
        });
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

    /// Confirm a verified pairing (the user pressed "Confirm pairing").
    ///
    /// This only queues the confirmation; the control loop builds and sends
    /// `PairingConfirm` so the machine stays single-owner.
    pub fn confirm_pairing(&self) -> Result<bool, String> {
        let phase = self
            .shared
            .pairing
            .lock()
            .unwrap()
            .as_ref()
            .map(|info| info.phase);
        if phase != Some(PairingPhase::AwaitingConfirmation) {
            return Err("no verified pairing is awaiting confirmation".to_string());
        }
        self.shared
            .confirm_tx
            .try_send(())
            .map_err(|e| format!("confirm channel: {e}"))?;
        Ok(true)
    }

    /// Pairing display material as JSON (no private keys, never logged).
    /// Empty string when this session is not a pairing session.
    pub fn pairing_json(&self) -> String {
        let Some(info) = self.shared.pairing.lock().unwrap().clone() else {
            return String::new();
        };
        let opt = |value: Option<&String>| match value {
            Some(v) => json_escape(v),
            None => "null".to_string(),
        };
        format!(
            "{{\"mode\":\"{}\",\"phase\":\"{}\",\"sas\":{},\"fingerprint\":{},\"server_public_key\":{},\"server_spki_sha256\":{},\"client_id\":{},\"display_name\":{},\"error_code\":{}}}",
            info.mode.as_str(),
            info.phase.name(),
            opt(info.sas.as_ref()),
            opt(info.fingerprint.as_ref()),
            opt(info.server_public_key_hex.as_ref()),
            opt(info.server_spki_sha256_hex.as_ref()),
            opt(info.client_id.as_ref()),
            opt(info.display_name.as_ref()),
            info.error_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "null".to_string()),
        )
    }

    /// The credential to persist after `PairingResult(OK)`, as JSON. Empty
    /// until the Server has persisted this identity.
    pub fn credential_json(&self) -> String {
        let Some(credential) = self.shared.credential.lock().unwrap().clone() else {
            return String::new();
        };
        format!(
            "{{\"server_spki_sha256\":{},\"server_public_key\":{},\"client_id\":{},\"display_name\":{}}}",
            json_escape(&credential.server_spki_sha256_hex),
            json_escape(&credential.server_public_key_hex),
            json_escape(&credential.client_id),
            json_escape(&credential.display_name),
        )
    }

    /// Attach the JVM's direct ByteBuffer as the media ring, once per session.
    pub fn attach_ring(&self, attachment: RingAttachment) -> Result<bool, String> {
        let mut ring = self.shared.ring.lock().unwrap();
        if ring.is_some() {
            return Ok(false);
        }
        *ring = Some(attachment);
        // A session already playing (attach raced with StreamStarted) gets the
        // ring immediately; otherwise the StreamStarted handler picks it up.
        if let Some(media) = self.shared.media.lock().unwrap().as_ref() {
            // SAFETY: the JVM keeps the buffer alive for the whole session.
            let ring =
                unsafe { crate::ring::MediaRing::attach(attachment.base, attachment.capacity) };
            media.lock().unwrap().set_ring(ring);
        }
        Ok(true)
    }

    /// Monotonic count of bytes written into the media ring (-1 if none yet).
    pub fn ring_position(&self) -> i64 {
        let media = self.shared.media.lock().unwrap().clone();
        let Some(media) = media else {
            return -1;
        };
        let media = media.lock().unwrap();
        media
            .ring_write_position()
            .map(|position| i64::try_from(position).unwrap_or(i64::MAX))
            .unwrap_or(-1)
    }

    /// The active media format as JSON for the Kotlin playback side, or ""
    /// before PLAYING. The six keys are the ticket-14 JNI contract — no extra
    /// fields, no key material, no PCM. Delivered-frame counts stay native
    /// (receiver statistics), they are not a playback-format fact.
    pub fn media_format_json(&self) -> String {
        let media = self.shared.media.lock().unwrap().clone();
        let Some(media) = media else {
            return String::new();
        };
        let media = media.lock().unwrap();
        let geometry = media.geometry();
        format!(
            "{{\"instance\":{},\"generation\":{},\"sample_rate\":{},\"channels\":{},\"sample_format\":{},\"anchor\":{}}}",
            media.stream_instance_id(),
            media.config_generation(),
            geometry.sample_rate,
            geometry.channels,
            geometry.sample_format.wire_value(),
            media.anchor(),
        )
    }

    /// Record the platform playback facts (§9: the only playback evidence).
    pub fn record_playback(&self, submitted_bytes: u64, underruns: u64, buffer_current_ms: u32) {
        if let Some(media) = self.shared.media.lock().unwrap().clone() {
            media.lock().unwrap().record_playback(PlaybackReport {
                submitted_bytes,
                underruns,
                buffer_current_ms,
            });
        }
    }

    /// Record frames trimmed by the Kotlin reader's §8 hard-limit re-anchor.
    /// M5-04 counts the deletion as `ReAnchor` concealment and advances the
    /// submitted position over the skipped media.
    pub fn record_trim(&self, frames: u64) {
        if let Some(media) = self.shared.media.lock().unwrap().clone() {
            media.lock().unwrap().record_trim(frames);
        }
    }

    /// Take the pending whole-frame drift adjustment (+ insert, − delete).
    /// The player applies it and reports back through
    /// [`Self::record_drift_intervention`].
    pub fn take_drift_intervention(&self) -> i32 {
        self.shared
            .media
            .lock()
            .unwrap()
            .clone()
            .map(|media| media.lock().unwrap().take_drift_intervention())
            .unwrap_or(0)
    }

    /// Acknowledge what the player actually applied (positive insert,
    /// negative delete). This is the §8 concealment accounting for drift.
    pub fn record_drift_intervention(&self, frames: i32) {
        if let Some(media) = self.shared.media.lock().unwrap().clone() {
            media.lock().unwrap().record_drift_intervention(frames);
        }
    }

    /// §8 drift estimate in ppm, or `None` before a full 10 s window.
    pub fn drift_ppm(&self) -> Option<i32> {
        self.shared
            .media
            .lock()
            .unwrap()
            .clone()
            .and_then(|media| media.lock().unwrap().drift_ppm())
    }

    /// Smooth ±100 ppm playback-rate correction currently commanded.
    pub fn drift_correction_ppm(&self) -> i32 {
        self.shared
            .media
            .lock()
            .unwrap()
            .clone()
            .map(|media| media.lock().unwrap().drift_correction_ppm())
            .unwrap_or(0)
    }

    /// Build and write ONE §9 diagnostics snapshot from the active media
    /// session. Extracted so the 5 s tick (which sees no Server control frame)
    /// and the CLOSING path use the same writer, and so the behaviour is
    /// testable over a real async stream without running the whole session.
    /// Returns `true` when a snapshot was produced (even if the write failed —
    /// statistics failures never block media, §9).
    async fn write_diagnostics<W>(&self, tx: &mut W) -> bool
    where
        W: tokio::io::AsyncWriteExt + Unpin,
    {
        let current = self.shared.media.lock().unwrap().clone();
        let Some(media) = current else {
            return false;
        };
        let (diagnostics, drift_ppm, correction_ppm) = {
            let mut media = media.lock().unwrap();
            let diagnostics = media.diagnostics(BUFFER_TARGET_MS);
            (diagnostics, media.drift_ppm(), media.drift_correction_ppm())
        };
        // §9 has NO drift field (and adding one is a protocol change needing
        // maintainer adjudication), so the §8 estimate reaches L1 tooling
        // through this log line and the `driftPpm` JNI accessor instead.
        if let Some(drift_ppm) = drift_ppm {
            log::info!(
                "event = {} drift_ppm={} correction_ppm={} buffer_current_ms={} buffer_target_ms={}",
                "playback.drift",
                drift_ppm,
                correction_ppm,
                diagnostics.buffer_current_ms,
                diagnostics.buffer_target_ms,
            );
        }
        let frame = crate::proto::Frame {
            body: Some(crate::proto::frame::Body::PlaybackDiagnostics(diagnostics)),
        };
        let Ok(bytes) = encode_frame(&frame) else {
            log::warn!("diagnostics encode failed");
            return true;
        };
        // Best-effort: a broken stream does not turn into a session error.
        let _ = tx.write_all(&bytes).await;
        true
    }

    /// Write the §5.2 fatal pair (Error + Close) for a locally raised verdict.
    async fn write_fatal(&self, verdict: &crate::control::Verdict) -> Result<(), String> {
        let crate::control::Verdict::Fatal { code } = verdict else {
            return Ok(());
        };
        let code = *code;
        let mut tx_guard = self.shared.control_stream.lock().await;
        if let Some(tx) = tx_guard.as_mut() {
            for frame in crate::control::frames::fatal_frames(code) {
                let bytes = encode_frame(&frame).map_err(|e| e.to_string())?;
                tx.write_all(&bytes)
                    .await
                    .map_err(|e| format!("send: {e}"))?;
            }
        }
        drop(tx_guard);
        self.record_close(crate::control::CloseInfo {
            code,
            remote: false,
        });
        Ok(())
    }

    /// JSON stats snapshot (low-frequency JNI surface). No credentials, no
    /// key material, no media bytes — only counters and state names.
    pub fn stats_json(&self) -> String {
        let state = self.state_name();
        let connected = self.shared.connected.load(Ordering::Acquire);
        let error = self.shared.last_error.lock().unwrap().clone();
        let close = self
            .shared
            .close_info
            .lock()
            .unwrap()
            .as_ref()
            .map(|c| c.code);
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

/// The test-support identity signer. A production build (no `test-support`)
/// has no fixture key path at all: production identities always arrive from
/// Kotlin via the signer upcall.
#[cfg(any(test, feature = "test-support"))]
pub fn test_support_signer() -> Result<SharedSigner, String> {
    Ok(Arc::new(crate::signer::SoftwareSigner::new(
        crate::auth::test_client_identity_key(),
    )))
}

#[cfg(not(any(test, feature = "test-support")))]
pub fn test_support_signer() -> Result<SharedSigner, String> {
    Err("no test identity in this build (enable `test-support` for the fixture key)".to_string())
}

/// A per-connection TLS-Exporter adapter over quinn. Pairing derives both the
/// 48-byte binding and the 32-byte SAS seed from this exact handshake.
struct QuicPairingExporter {
    conn: quinn::Connection,
}

impl PairingExporter for QuicPairingExporter {
    fn export(&self, label: &[u8], context: &[u8], len: usize) -> Result<Vec<u8>, String> {
        let mut out = vec![0u8; len];
        self.conn
            .export_keying_material(&mut out, label, context)
            .map_err(|_| "TLS exporter unavailable".to_string())?;
        Ok(out)
    }
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

async fn send_verdict(
    tx: &mut quinn::SendStream,
    v: &crate::control::Verdict,
) -> Result<(), String> {
    let frame = match v {
        crate::control::Verdict::Send(f) => Some(f.clone()),
        crate::control::Verdict::Reject { code } => {
            Some(crate::control::frames::error_frame(*code))
        }
        _ => None,
    };
    if let Some(frame) = frame {
        let bytes = encode_frame(&frame).map_err(|e| e.to_string())?;
        tx.write_all(&bytes)
            .await
            .map_err(|e| format!("send: {e}"))?;
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
            client_name: "test".into(),
            identity: SessionIdentity::Auth {
                pinned_spki_hex: "00".repeat(32),
                server_public_key: vec![0x11u8; 32],
                signer: crate::signer::test_signer(),
            },
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
            endpoint
                .local_addr()
                .expect("local addr after rebind")
                .port(),
            old_port,
            "endpoint must now use the second socket"
        );
        assert_eq!(
            endpoint
                .local_addr()
                .expect("local addr after rebind")
                .port(),
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

    #[test]
    fn malformed_remote_frame_is_401_not_network_failure() {
        let mut decoder = FrameDecoder::new();
        let error = decoder.push(&0u32.to_be_bytes()).unwrap_err();
        assert_eq!(
            session_event_for_frame_error(&error),
            SessionEvent::Closed(CloseInfo {
                code: 401,
                remote: false
            })
        );
    }

    /// §6.3: a pin mismatch is classified 205 (no reconnect); every other
    /// run error is transport loss (601).
    #[test]
    fn run_error_classification_matches_section_6_3() {
        assert_eq!(
            classify_run_error(&RunError::Other("handshake: refused".into())),
            601
        );
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
        session.record_close(CloseInfo {
            code: 204,
            remote: true,
        });
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

    fn playing_session() -> Session {
        let session = Session::new(test_config()).expect("session");
        let configuration = crate::proto::Configuration {
            config_generation: 1,
            codec: 1,
            pcm: Some(crate::proto::PcmParameters {
                sample_rate: 48_000,
                channels: 2,
                sample_format: 1,
            }),
        };
        let media = MediaSession::start(4, 1, 96_000, &configuration, None).expect("geometry");
        *session.shared.media.lock().unwrap() = Some(Arc::new(Mutex::new(media)));
        session
    }

    /// §9 regression: a diagnostics report is written on the tick that
    /// produces it, WITHOUT waiting for any Server control frame. This is the
    /// defect the Spec-axis review found: the old loop `continue`d past the
    /// send, so PLAYING sessions (no control traffic) never reported.
    #[test]
    fn diagnostics_are_written_without_a_server_control_frame() {
        let session = playing_session();
        // Real playback evidence: the platform accepted 64 frames.
        session.record_playback(8 * 64, 1, 25);
        // Session owns a multi-thread runtime; run the async writer on it
        // instead of nesting a second #[tokio::test] runtime.
        session.runtime.block_on(async {
            let (mut writer, mut reader) = tokio::io::duplex(4096);
            assert!(session.write_diagnostics(&mut writer).await);
            drop(writer);

            let mut bytes = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut bytes)
                .await
                .expect("read diagnostics frame");
            assert!(!bytes.is_empty(), "one framed message must be written");
            let frames = FrameDecoder::new()
                .push(&bytes)
                .expect("Android decodes its own §9 frame");
            assert_eq!(frames.len(), 1);
            match frames[0].body.as_ref().expect("body") {
                crate::proto::frame::Body::PlaybackDiagnostics(d) => {
                    assert_eq!(d.stream_instance_id, 4);
                    assert_eq!(d.config_generation, 1);
                    assert_eq!(d.underruns, 1);
                    assert_eq!(d.buffer_current_ms, 25);
                    assert_eq!(
                        d.submitted_position, 96_064,
                        "anchor + platform-accepted frames (§9)"
                    );
                }
                other => panic!("expected PlaybackDiagnostics, got {other:?}"),
            }
        });
    }

    /// A session with no active media produces no diagnostics frame (§9:
    /// there is nothing to report, and no fabricated snapshot).
    #[test]
    fn no_active_media_writes_no_diagnostics() {
        let session = Session::new(test_config()).expect("session");
        session.runtime.block_on(async {
            let (mut writer, _reader) = tokio::io::duplex(64);
            assert!(!session.write_diagnostics(&mut writer).await);
        });
    }

    /// The JNI media surface reports the active instance/generation/geometry
    /// and nothing before PLAYING.
    #[test]
    fn media_format_json_describes_the_active_stream() {
        let session = Session::new(test_config()).expect("session");
        assert_eq!(session.media_format_json(), "", "no media before PLAYING");
        assert_eq!(session.ring_position(), -1, "no ring before PLAYING");

        let session = playing_session();
        let json = session.media_format_json();
        assert!(json.contains("\"instance\":4"), "got: {json}");
        assert!(json.contains("\"generation\":1"), "got: {json}");
        assert!(json.contains("\"sample_rate\":48000"), "got: {json}");
        assert!(json.contains("\"channels\":2"), "got: {json}");
        assert!(json.contains("\"sample_format\":1"), "got: {json}");
        assert!(json.contains("\"anchor\":96000"), "got: {json}");
        // Ticket-14 JNI contract: exactly these six keys, no extras.
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let keys: Vec<&str> = parsed
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys.len(), 6, "no extra fields: {json}");
        for key in [
            "instance",
            "generation",
            "sample_rate",
            "channels",
            "sample_format",
            "anchor",
        ] {
            assert!(keys.contains(&key), "missing {key}: {json}");
        }
    }

    /// Playback reports reach the active media session and drive the reported
    /// submitted position (§9: only platform-accepted bytes count).
    #[test]
    fn playback_reports_flow_into_the_media_session() {
        let session = playing_session();
        session.record_playback(8 * 64, 2, 40);
        let media = session.shared.media.lock().unwrap().clone().expect("media");
        let media = media.lock().unwrap();
        assert_eq!(media.anchor(), 96_000);
        assert_eq!(media.submitted_position(), 96_064);
        let mut media = media;
        let diagnostics = media.diagnostics(0);
        assert_eq!(diagnostics.underruns, 2);
        assert_eq!(diagnostics.buffer_current_ms, 40);
        assert_eq!(diagnostics.submitted_position, 96_064);
    }

    /// Attaching the ring is a one-time operation (media never crosses JNI per
    /// datagram, so the buffer identity must not change mid-session).
    #[test]
    fn the_media_ring_is_attached_once() {
        let session = Session::new(test_config()).expect("session");
        let mut backing = vec![0u8; 1024];
        let attachment = RingAttachment {
            base: backing.as_mut_ptr(),
            capacity: backing.len(),
        };
        assert_eq!(session.attach_ring(attachment), Ok(true));
        assert_eq!(
            session.attach_ring(attachment),
            Ok(false),
            "a second attach must be refused"
        );
    }
}
