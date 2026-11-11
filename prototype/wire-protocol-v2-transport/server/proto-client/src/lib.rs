//! wire-protocol-v2 transport prototype — desktop test client core.
//!
//! Connects with a pinned server SPKI (sha256 over the certificate's
//! SubjectPublicKeyInfo DER via a custom rustls verifier), runs the brief's
//! control sequence over the single bidirectional stream, and verifies media
//! datagram continuity (sequence, timestamp, deterministic PCM pattern).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use proto_core::frame::{self, CloseReason, Frame};
use proto_core::media::{frame_bytes, sample_value, MediaHeader};
use proto_core::spki::{extract_spki_der, spki_sha256, to_hex};
use quinn::{Connection, RecvStream, SendStream};
use rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::DigitallySignedStruct;
use tokio::time::timeout;

/// Desktop test client configuration.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub server_addr: SocketAddr,
    /// Expected `sha256(SPKI DER)` of the server certificate.
    pub pinned_spki: [u8; 32],
    /// ALPN to offer (default `audio-share/v2`; wrong values are for negative
    /// tests and must fail the handshake).
    pub alpn: String,
    pub credential: String,
    pub connect_timeout: Duration,
    /// DPLPMTUD on the client side; disabled by default for the same reason
    /// as the server (deterministic datagram-size invariant).
    pub mtud_discovery: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("handshake failed: {0}")]
    Handshake(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("protocol violation: {0}")]
    Protocol(String),
    #[error("server rejected auth: {code:?} (then {close_after:?})")]
    AuthRejected {
        code: frame::AuthErrCode,
        close_after: Option<CloseReason>,
    },
    #[error("unexpected frame: {0:?}")]
    UnexpectedFrame(Frame),
    #[error("connection closed by peer without a CLOSE frame (transport: {0})")]
    ClosedWithoutFrame(String),
    #[error("timed out")]
    Timeout,
}

/// Established test client: owns the single bidirectional control stream.
#[derive(Debug)]
pub struct ClientConn {
    conn: Connection,
    tx: SendStream,
    rx: RecvStream,
}

/// Handshake with the pinned SPKI verifier, TLS 1.3 only, then open the
/// single bidirectional control stream.
///
/// 0-RTT: quinn 0.11 exposes no client-side API to send early data at all,
/// and this client never enables resumption with early data — early data is
/// structurally impossible here (see README for the full evidence path).
pub async fn connect(cfg: &ClientConfig) -> Result<ClientConn, ClientError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Arc::new(SpkiPinVerifier {
        pinned: cfg.pinned_spki,
        provider: provider.clone(),
    });
    let tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| ClientError::Protocol(format!("tls versions: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    let mut tls = tls;
    tls.alpn_protocols = vec![cfg.alpn.clone().into_bytes()];
    let quic_tls = quinn::crypto::rustls::QuicClientConfig::try_from(tls)
        .map_err(|e| ClientError::Protocol(format!("quic tls config: {e}")))?;
    let mut client_config = quinn::ClientConfig::new(Arc::new(quic_tls));
    let mut transport = quinn::TransportConfig::default();
    if !cfg.mtud_discovery {
        transport.mtu_discovery_config(None);
    }
    client_config.transport_config(Arc::new(transport));

    // Bind the wildcard address of the server's address family: an IPv4-only
    // bind cannot send to an IPv6 remote (quinn rejects the family mismatch).
    let bind_addr = if cfg.server_addr.is_ipv4() {
        SocketAddr::from(([0u8, 0, 0, 0], 0))
    } else {
        SocketAddr::from(([0u16; 8], 0))
    };
    let mut endpoint = quinn::Endpoint::client(bind_addr).map_err(|e| {
        ClientError::Protocol(format!("client endpoint: {e}"))
    })?;
    endpoint.set_default_client_config(client_config);
    let connecting = endpoint
        .connect(cfg.server_addr, "localhost")
        .map_err(|e| ClientError::Protocol(format!("connect: {e}")))?;
    let conn = timeout(cfg.connect_timeout, connecting)
        .await
        .map_err(|_| ClientError::Timeout)?
        .map_err(|e| ClientError::Handshake(e.to_string()))?;
    let (tx, rx) = conn
        .open_bi()
        .await
        .map_err(|e| ClientError::Transport(format!("open control stream: {e}")))?;
    Ok(ClientConn { conn, tx, rx })
}

impl ClientConn {
    /// The server's QUIC connection (for datagram IO and stats).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Current maximum datagram size the server may use towards us; queried
    /// live, never assumed.
    pub fn max_datagram_size(&self) -> Option<usize> {
        self.conn.max_datagram_size()
    }

    pub async fn send_frame(&mut self, frame: &Frame) -> Result<(), ClientError> {
        let bytes = frame::encode(frame)
            .map_err(|e| ClientError::Protocol(format!("encode: {e}")))?;
        self.send_raw(&bytes).await
    }

    /// Sends arbitrary pre-encoded wire bytes — the negative protocol tests
    /// need to speak frames the codec cannot express (unknown types, bad
    /// payload lengths, oversized lengths).
    pub async fn send_raw(&mut self, bytes: &[u8]) -> Result<(), ClientError> {
        self.tx
            .write_all(bytes)
            .await
            .map_err(|e| ClientError::Transport(format!("write: {e}")))
    }

    pub async fn read_frame(&mut self) -> Result<Frame, ClientError> {
        let mut len_buf = [0u8; 4];
        match self.rx.read_exact(&mut len_buf).await {
            Ok(()) => {}
            Err(quinn::ReadExactError::FinishedEarly(_)) => {
                return Err(ClientError::ClosedWithoutFrame("control stream EOF".into()))
            }
            Err(e) => return Err(ClientError::Transport(format!("read len: {e}"))),
        }
        let frame_len = u32::from_be_bytes(len_buf);
        if frame_len == 0 || frame_len > frame::MAX_FRAME_LEN {
            return Err(ClientError::Protocol(format!(
                "frame_len {frame_len} outside 1..={}",
                frame::MAX_FRAME_LEN
            )));
        }
        let mut ty_buf = [0u8; 1];
        self.rx
            .read_exact(&mut ty_buf)
            .await
            .map_err(|e| ClientError::Transport(format!("read type: {e}")))?;
        let mut payload = vec![0u8; (frame_len - 1) as usize];
        self.rx
            .read_exact(&mut payload)
            .await
            .map_err(|e| ClientError::Transport(format!("read payload: {e}")))?;
        frame::parse(ty_buf[0], &payload)
            .map_err(|e| ClientError::Protocol(format!("parse: {e}")))
    }

    /// Sends AUTH and expects AUTH_OK. On AUTH_ERR, also consumes the
    /// following CLOSE frame if one arrives.
    pub async fn auth(&mut self, credential: &str) -> Result<u32, ClientError> {
        self.send_frame(&Frame::Auth {
            token: credential.to_string(),
        })
        .await?;
        match self.read_frame().await? {
            Frame::AuthOk { session_id } => Ok(session_id),
            Frame::AuthErr(code) => {
                let close_after = match timeout(Duration::from_secs(2), self.read_frame()).await {
                    Ok(Ok(Frame::Close(reason))) => Some(reason),
                    _ => None,
                };
                Err(ClientError::AuthRejected { code, close_after })
            }
            other => Err(ClientError::UnexpectedFrame(other)),
        }
    }

    /// Sends SUBSCRIBE and expects MEDIA_START.
    pub async fn subscribe(&mut self) -> Result<frame::MediaStartPayload, ClientError> {
        self.send_frame(&Frame::Subscribe).await?;
        match self.read_frame().await? {
            Frame::MediaStart(payload) => Ok(payload),
            other => Err(ClientError::UnexpectedFrame(other)),
        }
    }

    /// Sends UNSUBSCRIBE (the prototype does not ack it on the wire).
    pub async fn unsubscribe(&mut self) -> Result<(), ClientError> {
        self.send_frame(&Frame::Unsubscribe).await
    }

    /// Sends CLOSE(NORMAL) — the client-initiated goodbye.
    pub async fn send_close_normal(&mut self) -> Result<(), ClientError> {
        self.send_frame(&Frame::Close(CloseReason::Normal)).await
    }

    /// Reads frames until a CLOSE frame arrives (or the transport closes),
    /// half-closes our side, and waits briefly for the transport close.
    pub async fn expect_close(&mut self, within: Duration) -> Result<CloseReason, ClientError> {
        let deadline = Instant::now() + within;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ClientError::Timeout);
            }
            match timeout(remaining, self.read_frame()).await {
                Err(_) => return Err(ClientError::Timeout),
                Ok(Ok(Frame::Close(reason))) => {
                    // We acknowledged the goodbye; let the server see our EOF.
                    let _ = self.tx.finish();
                    let _ = timeout(Duration::from_secs(1), self.conn.closed()).await;
                    return Ok(reason);
                }
                Ok(Ok(_)) => continue,
                Ok(Err(e)) => return Err(e),
            }
        }
    }

    /// Next media datagram (unreliable; may fail if the connection closes).
    pub async fn read_datagram(&self) -> Result<bytes::Bytes, ClientError> {
        self.conn
            .read_datagram()
            .await
            .map_err(|e| ClientError::Transport(format!("read_datagram: {e}")))
    }

    /// Waits for the QUIC transport to close, returning application error code
    /// and reason text when the peer closed the application, or a description
    /// of the transport-level close otherwise.
    pub async fn wait_transport_closed(
        &self,
        within: Duration,
    ) -> Result<(u64, String), ClientError> {
        let error = timeout(within, self.conn.closed())
            .await
            .map_err(|_| ClientError::Timeout)?;
        match error {
            quinn::ConnectionError::ApplicationClosed(close) => Ok((
                close.error_code.into_inner(),
                String::from_utf8_lossy(&close.reason).into_owned(),
            )),
            other => Ok((u64::MAX, other.to_string())),
        }
    }
}

/// SPKI-pinning server certificate verifier: accepts exactly one server whose
/// leaf certificate's SubjectPublicKeyInfo hashes to the pinned value.
///
/// Everything else about the certificate (name, chain, validity) is ignored by
/// design — the pin IS the identity for this prototype, mirroring how the
/// Android client will pin after pairing.
#[derive(Debug)]
struct SpkiPinVerifier {
    pinned: [u8; 32],
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for SpkiPinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let spki = extract_spki_der(end_entity.as_ref()).map_err(|e| {
            rustls::Error::General(format!("SPKI extraction failed: {e}"))
        })?;
        let actual = spki_sha256(&spki);
        if actual == self.pinned {
            Ok(ServerCertVerified::assertion())
        } else {
            tracing::debug!(
                actual = %to_hex(&actual),
                "server SPKI sha256 does not match pinned value"
            );
            Err(rustls::Error::General(
                "server SPKI sha256 does not match pinned value".to_string(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        digitsig: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            digitsig,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        digitsig: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            digitsig,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Verifies media datagram continuity: sequence, timestamp advance, and the
/// deterministic PCM pattern.
///
/// This is the client-side (and e2e-test) view of the media plane: counters
/// are accumulated, never fatal — the prototype reports loss/reorder instead
/// of failing on it.
#[derive(Debug, Clone)]
pub struct MediaVerifier {
    pub channels: u8,
    pub datagrams: u64,
    pub bytes: u64,
    pub sample_frames: u64,
    pub gaps: u64,
    pub duplicates: u64,
    pub pattern_mismatches: u64,
    pub timestamp_mismatches: u64,
    pub oversized: u64,
    pub misaligned: u64,
    pub too_short: u64,
    pub instance_changes: u64,
    pub max_datagram_len: usize,
    instance: u32,
    next_seq: u32,
    next_ts: u64,
    anchored: bool,
}

impl MediaVerifier {
    pub fn new(channels: u8) -> Self {
        Self {
            channels,
            datagrams: 0,
            bytes: 0,
            sample_frames: 0,
            gaps: 0,
            duplicates: 0,
            pattern_mismatches: 0,
            timestamp_mismatches: 0,
            oversized: 0,
            misaligned: 0,
            too_short: 0,
            instance_changes: 0,
            max_datagram_len: 0,
            instance: 0,
            next_seq: 0,
            next_ts: 0,
            anchored: false,
        }
    }

    /// Checks one received datagram against `max_datagram_size` (the value
    /// queried from the connection, never assumed).
    pub fn check(&mut self, datagram: &[u8], max_datagram_size: usize) {
        self.datagrams += 1;
        self.bytes += datagram.len() as u64;
        self.max_datagram_len = self.max_datagram_len.max(datagram.len());
        if datagram.len() > max_datagram_size {
            self.oversized += 1;
        }
        let Ok((header, pcm)) = MediaHeader::parse(datagram) else {
            self.too_short += 1;
            return;
        };
        let fb = frame_bytes(self.channels);
        if pcm.len() % fb != 0 {
            self.misaligned += 1;
        }

        if !self.anchored {
            self.instance = header.stream_instance_id;
            self.anchored = true;
            self.next_seq = header.sequence;
        } else if header.stream_instance_id != self.instance {
            // A capture restart: reset expectations, count the change.
            self.instance_changes += 1;
            self.instance = header.stream_instance_id;
            self.next_seq = header.sequence;
            self.next_ts = 0;
        }

        if header.sequence == self.next_seq {
            if self.next_ts != 0 && header.timestamp != self.next_ts {
                self.timestamp_mismatches += 1;
            }
        } else if header.sequence > self.next_seq {
            self.gaps += (header.sequence - self.next_seq) as u64;
        } else {
            self.duplicates += 1;
        }

        // Pattern verification uses absolute sample positions from the
        // timestamp, so it works across gaps and instance restarts.
        for (index, chunk) in pcm.chunks_exact(4).enumerate() {
            let frame_index = (index / self.channels as usize) as u64;
            let expected = sample_value(header.timestamp + frame_index);
            let got = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if got.to_bits() != expected.to_bits() {
                self.pattern_mismatches += 1;
            }
        }
        self.sample_frames += (pcm.len() / fb) as u64;

        if header.sequence >= self.next_seq {
            self.next_seq = header.sequence.wrapping_add(1);
            self.next_ts = header.timestamp + (pcm.len() / fb) as u64;
        }
    }
}
