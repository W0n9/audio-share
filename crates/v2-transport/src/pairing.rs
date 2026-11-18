//! Production pairing core (pairing.md §4–§11; ticket 19).
//!
//! Protocol-neutral, synchronous state shared by the CLI, the Server GUI, and
//! the per-connection driver:
//!
//! - QR payload encode/parse (§4) and the binding-context TLV canonicalization
//!   (§6.3);
//! - SAS derivation and presentation (§7.1, §7.3);
//! - the memory-only, single-use, short-lived pairing nonce table (§5);
//! - the admin-opened SAS admission window (§7.3);
//! - the TM-5 pairing-failure rate limit (5 per sliding 60 s per address),
//!   independent of the authentication guard;
//! - the explicit local-approval decision (§5.2, pairing.md §6.3.1);
//! - persistence through [`AuthorizationStore::upsert`] only (§8.1).
//!
//! Red line: nonces, SAS values, binding values, and keys are never exposed in
//! `Debug` output or log lines. The driver logs only registry codes and masked
//! source addresses.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use sha2::Digest as _;
use tokio::sync::broadcast;

use crate::auth;
use crate::auth_store::{AuthorizationStore, ClientState, StoreError, StoredClient};
use crate::codes::PROTOCOL_MAJOR;
use crate::identity::{hex_groups, IdentityConfig, IdentityError, ServerIdentity};
use crate::limits::AddressRateGuard;
use crate::schema::{PairingMode, PairingRequest};

/// The QR payload URI scheme (§4.1).
pub const QR_URI_SCHEME: &str = "audioshare-pair";
/// The payload-format tag; a shape change must bump this (`p2`).
pub const QR_PAYLOAD_TAG: &str = "p1";

/// TLS-Exporter label for `binding` (48 bytes; pairing.md §6.3).
pub const PAIRING_EXPORTER_LABEL: &[u8] = b"EXPORTER audio-share v2 pairing";
/// TLS-Exporter label for `sas_seed` (32 bytes; pairing.md §6.3).
pub const SAS_EXPORTER_LABEL: &[u8] = b"EXPORTER audio-share v2 sas";

/// Pairing nonce length in bytes (§5: 128 bits).
pub const NONCE_LEN: usize = 16;
/// Nonce lifetime from generation, on the monotonic clock (§5).
pub const NONCE_TTL: Duration = Duration::from_secs(10 * 60);
/// At most this many outstanding nonces; the next evicts the oldest (§5).
pub const MAX_OUTSTANDING_NONCES: usize = 8;
/// At most this many concurrent pairing sessions (TM-5, pairing.md §5).
pub const MAX_CONCURRENT_PAIRINGS: usize = 8;
/// The admin-opened SAS window lifetime (the pairing deadline, §7.3).
pub const SAS_WINDOW_TTL: Duration = Duration::from_secs(10 * 60);
/// Pairing failure limit per source address per sliding window (TM-5).
pub const PAIRING_FAILURES_PER_WINDOW: u32 = 5;
/// The TM-5 sliding window.
pub const PAIRING_FAILURE_WINDOW: Duration = Duration::from_secs(60);
/// The complete pairing exchange deadline (§5.2: 10 min → 206).
pub const PAIRING_DEADLINE: Duration = Duration::from_secs(10 * 60);

/// The exported `binding` length.
pub const BINDING_LEN: usize = 48;
/// The exported `sas_seed` length.
pub const SAS_SEED_LEN: usize = 32;

/// A monotonic clock seam: production uses [`Instant::now`], tests advance a
/// manual clock without sleeping.
pub trait Clock: Send + Sync + fmt::Debug {
    /// The current monotonic instant.
    fn now(&self) -> Instant;
}

/// The production clock.
#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Per-connection TLS-Exporter seam. The driver implements this over
/// `quinn::Connection::export_keying_material`; tests inject the golden-vector
/// stand-ins. The state machine never depends on quinn.
pub trait PairingExporter: Send + Sync + fmt::Debug {
    /// Export `len` bytes for `label` bound to `context`, or `None` when the
    /// connection cannot provide exporter material.
    fn export(&self, label: &[u8], context: &[u8], len: usize) -> Option<Vec<u8>>;
}

/// A decoded/validated QR payload (§4.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QrPayload {
    /// The Server address hint (`host:port`), never a trusted identity.
    pub addr_hint: String,
    /// The raw 32-byte Ed25519 identity public key (`key`).
    pub server_public_key: [u8; 32],
    /// The 16-byte single-use nonce.
    pub nonce: [u8; NONCE_LEN],
}

/// A QR payload parsing failure. Unknown fields and unknown shapes are a
/// payload-format change, never silently ignored (§4.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QrPayloadError {
    /// Not the `audioshare-pair://p1` shape.
    Shape,
    /// A `pm` other than the ALPN major.
    ProtocolMajor(u32),
    /// A malformed or unsupported `addr` hint.
    Addr(String),
    /// A missing required query field.
    MissingField(&'static str),
    /// A duplicate query field.
    DuplicateField(String),
    /// An unknown query field (payload-format change).
    UnknownField(String),
    /// Base64url decoding failed for a field.
    Base64 {
        /// The field name.
        field: &'static str,
        /// The decoder's reason.
        reason: String,
    },
    /// The decoded key is not 32 bytes.
    KeyLength {
        /// The observed length.
        found: usize,
    },
    /// The decoded nonce is not 16 bytes.
    NonceLength {
        /// The observed length.
        found: usize,
    },
}

impl fmt::Display for QrPayloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shape => write!(f, "not a audioshare-pair://p1 payload"),
            Self::ProtocolMajor(major) => {
                write!(f, "pairing payload protocol major {major} is not 2")
            }
            Self::Addr(reason) => write!(f, "invalid address hint: {reason}"),
            Self::MissingField(field) => write!(f, "missing query field {field}"),
            Self::DuplicateField(field) => write!(f, "duplicate query field {field}"),
            Self::UnknownField(field) => write!(
                f,
                "unknown query field {field:?}; unknown payload shapes require a format bump"
            ),
            Self::Base64 { field, reason } => write!(f, "{field} is not base64url: {reason}"),
            Self::KeyLength { found } => write!(f, "key is {found} bytes; expected 32"),
            Self::NonceLength { found } => write!(f, "nonce is {found} bytes; expected 16"),
        }
    }
}

impl std::error::Error for QrPayloadError {}

/// Encode the §4.1 QR payload URI (generation order is fixed; parsers accept
/// any order).
pub fn encode_qr_uri(
    addr_hint: &str,
    server_public_key: &[u8; 32],
    nonce: &[u8; NONCE_LEN],
) -> String {
    let key = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(server_public_key);
    let nonce = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(nonce);
    format!(
        "{QR_URI_SCHEME}://{QR_PAYLOAD_TAG}?pm={PROTOCOL_MAJOR}&addr={addr_hint}&key={key}&nonce={nonce}"
    )
}

/// Parse and validate the §4.1 QR payload URI.
///
/// Parsers accept fields in any order and reject unknown fields (§4.1). The
/// address is a hint: it is validated for shape only and is never an identity.
pub fn parse_qr_uri(uri: &str) -> Result<QrPayload, QrPayloadError> {
    let prefix = format!("{QR_URI_SCHEME}://{QR_PAYLOAD_TAG}?");
    let query = uri.strip_prefix(&prefix).ok_or(QrPayloadError::Shape)?;
    if query.is_empty() {
        return Err(QrPayloadError::Shape);
    }
    let mut pm: Option<u32> = None;
    let mut addr: Option<String> = None;
    let mut key: Option<Vec<u8>> = None;
    let mut nonce: Option<Vec<u8>> = None;
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').ok_or(QrPayloadError::Shape)?;
        if name.is_empty() {
            return Err(QrPayloadError::Shape);
        }
        if value.contains('%') {
            return Err(QrPayloadError::Addr(
                "percent-encoding is not permitted in a pairing payload".to_owned(),
            ));
        }
        match name {
            "pm" => {
                if pm.is_some() {
                    return Err(QrPayloadError::DuplicateField("pm".to_owned()));
                }
                pm = Some(value.parse().map_err(|_| QrPayloadError::Shape)?);
            }
            "addr" => {
                if addr.is_some() {
                    return Err(QrPayloadError::DuplicateField("addr".to_owned()));
                }
                validate_addr_hint(value)?;
                addr = Some(value.to_owned());
            }
            "key" => {
                if key.is_some() {
                    return Err(QrPayloadError::DuplicateField("key".to_owned()));
                }
                key = Some(
                    base64::engine::general_purpose::URL_SAFE_NO_PAD
                        .decode(value)
                        .map_err(|error| QrPayloadError::Base64 {
                            field: "key",
                            reason: error.to_string(),
                        })?,
                );
            }
            "nonce" => {
                if nonce.is_some() {
                    return Err(QrPayloadError::DuplicateField("nonce".to_owned()));
                }
                nonce = Some(
                    base64::engine::general_purpose::URL_SAFE_NO_PAD
                        .decode(value)
                        .map_err(|error| QrPayloadError::Base64 {
                            field: "nonce",
                            reason: error.to_string(),
                        })?,
                );
            }
            other => return Err(QrPayloadError::UnknownField(other.to_owned())),
        }
    }
    let major = pm.ok_or(QrPayloadError::MissingField("pm"))?;
    if major != PROTOCOL_MAJOR {
        return Err(QrPayloadError::ProtocolMajor(major));
    }
    let addr_hint = addr.ok_or(QrPayloadError::MissingField("addr"))?;
    let key = key.ok_or(QrPayloadError::MissingField("key"))?;
    if key.len() != 32 {
        return Err(QrPayloadError::KeyLength { found: key.len() });
    }
    let nonce = nonce.ok_or(QrPayloadError::MissingField("nonce"))?;
    if nonce.len() != NONCE_LEN {
        return Err(QrPayloadError::NonceLength { found: nonce.len() });
    }
    Ok(QrPayload {
        addr_hint,
        server_public_key: key.try_into().expect("length checked"),
        nonce: nonce.try_into().expect("length checked"),
    })
}

/// Validate the §4.2 `addr` hint: IPv4 dotted quad, bracketed IPv6, or a DNS
/// name, followed by `:` and a port in 1–65535. No percent-encoding.
fn validate_addr_hint(value: &str) -> Result<(), QrPayloadError> {
    if value.is_empty() || value.contains('%') || value.chars().any(char::is_whitespace) {
        return Err(QrPayloadError::Addr(value.to_owned()));
    }
    let (host, port) = if let Some(rest) = value.strip_prefix('[') {
        let (host, suffix) = rest
            .split_once(']')
            .ok_or_else(|| QrPayloadError::Addr(value.to_owned()))?;
        host.parse::<std::net::Ipv6Addr>()
            .map_err(|_| QrPayloadError::Addr(value.to_owned()))?;
        let port = suffix
            .strip_prefix(':')
            .ok_or_else(|| QrPayloadError::Addr(value.to_owned()))?;
        (host, port)
    } else {
        let (host, port) = value
            .rsplit_once(':')
            .ok_or_else(|| QrPayloadError::Addr(value.to_owned()))?;
        if host.is_empty() || host.contains(':') {
            return Err(QrPayloadError::Addr(value.to_owned()));
        }
        if !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        {
            return Err(QrPayloadError::Addr(value.to_owned()));
        }
        (host, port)
    };
    if host.is_empty() {
        return Err(QrPayloadError::Addr(value.to_owned()));
    }
    let port: u32 = port
        .parse()
        .map_err(|_| QrPayloadError::Addr(value.to_owned()))?;
    if port == 0 || port > u16::MAX as u32 {
        return Err(QrPayloadError::Addr(value.to_owned()));
    }
    Ok(())
}

/// The §6.3 transcript binding: SHA-256 over the canonical TLV sequence.
///
/// `TLV(1,mode) || TLV(2,protocol_major) || [TLV(3,pairing_nonce) in QR mode]
///  || TLV(4,client_random) || TLV(5,server_random) || TLV(6,client_pubkey)
///  || TLV(7,server_pubkey) || TLV(8,capabilities_canonical)
///  || TLV(9,client_display_name)`.
#[allow(
    clippy::too_many_arguments,
    reason = "the normative §6.3 transcript has nine fixed inputs; a struct would obscure the order"
)]
pub fn binding_context(
    mode: PairingMode,
    pairing_nonce: &[u8],
    client_random: &[u8],
    server_random: &[u8],
    client_public_key: &[u8],
    server_public_key: &[u8],
    capabilities: &[u32],
    client_display_name: &str,
) -> Result<[u8; 32], PairingError> {
    let mode_text = match mode {
        PairingMode::Qr => {
            if pairing_nonce.len() != NONCE_LEN {
                return Err(PairingError::Malformed("pairing_nonce must be 16 bytes"));
            }
            "qr"
        }
        PairingMode::Sas => {
            if !pairing_nonce.is_empty() {
                return Err(PairingError::Malformed("sas mode must omit pairing_nonce"));
            }
            "sas"
        }
        PairingMode::Unspecified => return Err(PairingError::Mode),
    };
    if client_random.len() != 32 {
        return Err(PairingError::Malformed("client_random must be 32 bytes"));
    }
    if server_random.len() != 32 {
        return Err(PairingError::Malformed("server_random must be 32 bytes"));
    }
    if auth::parse_client_public_key(client_public_key).is_err() {
        return Err(PairingError::Malformed(
            "client_public_key must be a 33-byte SEC1 point",
        ));
    }
    if server_public_key.len() != 32 {
        return Err(PairingError::Malformed(
            "server_public_key must be 32 bytes",
        ));
    }
    auth::validate_capability_list_bounded(capabilities, u8::MAX as usize)
        .map_err(|_| PairingError::Malformed("capabilities must be ascending and unique"))?;
    let mut transcript = Vec::new();
    transcript.extend_from_slice(&auth::tlv(1, mode_text.as_bytes()));
    // The fixed 4-byte big-endian major, never a varint (pairing.md §6.3).
    transcript.extend_from_slice(&auth::tlv(2, &PROTOCOL_MAJOR.to_be_bytes()));
    if mode == PairingMode::Qr {
        transcript.extend_from_slice(&auth::tlv(3, pairing_nonce));
    }
    transcript.extend_from_slice(&auth::tlv(4, client_random));
    transcript.extend_from_slice(&auth::tlv(5, server_random));
    transcript.extend_from_slice(&auth::tlv(6, client_public_key));
    transcript.extend_from_slice(&auth::tlv(7, server_public_key));
    transcript.extend_from_slice(&auth::tlv(8, &auth::canonical_capabilities(capabilities)));
    transcript.extend_from_slice(&auth::tlv(9, client_display_name.as_bytes()));
    Ok(sha2::Sha256::digest(&transcript).into())
}

/// `SAS = decimal(BE u32 of sas_seed[0..4] mod 1_000_000)` (§7.1).
pub fn derive_sas_code(sas_seed: &[u8]) -> Option<u32> {
    let first = sas_seed.get(..4)?;
    let value = u32::from_be_bytes(first.try_into().expect("four bytes"));
    Some(value % 1_000_000)
}

/// Render a SAS as the §7.3 `123 456` grouping.
pub fn format_sas(code: u32) -> String {
    format!("{:03} {:03}", (code / 1000) % 1000, code % 1000)
}

/// An invitation created by an admin action.
#[derive(Clone, Debug)]
pub struct Invitation {
    /// QR or SAS.
    pub mode: PairingMode,
    /// The 16-byte nonce (QR) or empty (SAS).
    pub nonce: Vec<u8>,
    /// The monotonic expiry instant.
    pub expires_at: Instant,
}

/// The lifecycle of the most recently opened QR invitation, as shown to the
/// local operator. This is a local admin view; no wire message carries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvitationState {
    /// The nonce is still outstanding and its lifetime has not elapsed.
    Active,
    /// The first well-formed pairing request consumed the nonce, regardless of
    /// the pairing's eventual outcome (§5).
    Consumed,
    /// The lifetime elapsed without a request consuming the nonce.
    Expired,
}

/// A request for the operator's explicit local approval (§5.2).
///
/// `approval_id` is a process-local handle; it is what `approve`, `decline`,
/// `cancel_approval`, and every admin surface use. The wire-level
/// `request_id` stays inside the state machine and never reaches this type:
/// two concurrent connections may legitimately both use `request_id = 1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalRequest {
    /// Process-unique local approval handle.
    pub approval_id: u64,
    /// The canonical client id (hex).
    pub client_id: String,
    /// The accepted display name.
    pub display_name: String,
    /// QR or SAS.
    pub mode: PairingMode,
    /// The source address (unmasked: shown to the local operator only).
    pub address: String,
    /// The per-connection SAS, when the connection supplied one.
    pub sas: Option<String>,
}

/// A live pairing as presented to the local operator.
///
/// Published as soon as the Server has successfully written `PairingResponse`
/// (pairing.md §6.2, §7.3), before the Client's `PairingConfirm`. The wire
/// protocol is unchanged: this is a local operator view only. It carries the
/// same process-local `approval_id` the post-confirm approval card will use,
/// so every clear path (approve, decline, cancel, deadline, connection loss)
/// can drop both with one id.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingPresented {
    /// Process-unique local approval handle (shared with the approval card).
    pub approval_id: u64,
    /// QR or SAS.
    pub mode: PairingMode,
    /// The canonical client id (hex).
    pub client_id: String,
    /// The accepted display name.
    pub display_name: String,
    /// The source address (unmasked: shown to the local operator only).
    pub address: String,
    /// The per-connection SAS, when the connection supplied one.
    pub sas: Option<String>,
    /// The §7.4 Server SPKI fingerprint.
    pub fingerprint: String,
    /// The monotonic pairing deadline instant.
    pub expires_at: Instant,
}

impl fmt::Debug for PairingPresented {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Red line: never expose the SAS through Debug/log output.
        f.debug_struct("PairingPresented")
            .field("approval_id", &self.approval_id)
            .field("mode", &self.mode)
            .field("client_id", &self.client_id)
            .field("display_name", &self.display_name)
            .field("address", &self.address)
            .field("sas", &self.sas.as_ref().map(|_| "<redacted>"))
            .field("fingerprint", &self.fingerprint)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// A pairing failure. Pre-authentication failures never carry detail on the
/// wire; these variants exist for local classification and redacted logs.
#[derive(Debug)]
pub enum PairingError {
    /// The nonce is unknown, was already used, or came after eviction.
    NonceInvalid,
    /// The nonce existed but its 10-minute lifetime elapsed.
    NonceExpired,
    /// No admin-opened SAS window is active.
    SasWindowClosed,
    /// The pairing mode is unspecified or outside the registry.
    Mode,
    /// `protocol_major` is not the ALPN major.
    Major,
    /// A structural validation failure (lengths, omitted nonce, capabilities).
    Malformed(&'static str),
    /// The connection-scoped TLS exporter was unavailable.
    ExporterUnavailable,
    /// The source address exhausted its TM-5 pairing-failure budget.
    RateLimited,
    /// Eight pairing sessions are already in flight (TM-5 concurrency bound).
    ConcurrentLimit,
    /// Persistence failed; v2 fails closed.
    Store(StoreError),
    /// Identity provisioning failed.
    Identity(IdentityError),
}

impl fmt::Display for PairingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonceInvalid => write!(f, "pairing nonce is unknown, used, or evicted"),
            Self::NonceExpired => write!(f, "pairing nonce expired"),
            Self::SasWindowClosed => write!(f, "no SAS pairing window is open"),
            Self::Mode => write!(f, "pairing mode is unspecified"),
            Self::Major => write!(f, "pairing protocol major is not 2"),
            Self::Malformed(reason) => write!(f, "malformed pairing input: {reason}"),
            Self::ExporterUnavailable => write!(f, "TLS exporter unavailable for pairing"),
            Self::RateLimited => write!(f, "pairing failure rate exceeded for this source"),
            Self::ConcurrentLimit => write!(f, "eight pairing sessions are already in flight"),
            Self::Store(error) => write!(f, "authorization store: {error}"),
            Self::Identity(error) => write!(f, "server identity: {error}"),
        }
    }
}

impl std::error::Error for PairingError {}

impl From<StoreError> for PairingError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<IdentityError> for PairingError {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error)
    }
}

struct NonceEntry {
    nonce: [u8; NONCE_LEN],
    expires_at: Instant,
}

/// The latest QR invitation, retained after consumption so the local admin
/// surface can report `consumed` instead of losing the nonce (§5).
struct CurrentQrInvitation {
    nonce: [u8; NONCE_LEN],
    expires_at: Instant,
    consumed: bool,
}

struct PendingApproval {
    request: ApprovalRequest,
    decide: Box<dyn FnOnce(bool) + Send>,
}

/// An RAII slot in the TM-5 concurrent-pairing bound.
///
/// The slot is acquired by [`PairingCore::admit_request`] and released when the
/// owning pending exchange is resolved: approval, decline, deadline, or
/// connection loss all drop the permit. Holding this value keeps the slot.
pub struct PairingPermit {
    active: Arc<AtomicUsize>,
}

impl fmt::Debug for PairingPermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairingPermit").finish_non_exhaustive()
    }
}

impl Drop for PairingPermit {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The production Pairing Core shared by the CLI, GUI, and driver.
pub struct PairingCore {
    server_public_key: [u8; 32],
    store: Arc<AuthorizationStore>,
    clock: Arc<dyn Clock>,
    nonces: Mutex<VecDeque<NonceEntry>>,
    /// The most recent QR invitation, retained for the local admin state.
    current_qr: Mutex<Option<CurrentQrInvitation>>,
    sas_window: Mutex<Option<Instant>>,
    failures: Mutex<AddressRateGuard>,
    approvals: Mutex<HashMap<u64, PendingApproval>>,
    /// Live pairings published after `PairingResponse`, keyed by the same
    /// process-local `approval_id` used by the approval card.
    presented: Mutex<HashMap<u64, PairingPresented>>,
    /// Process-unique approval handles; never derived from the wire.
    approval_seq: AtomicU64,
    /// Live pairing sessions holding a TM-5 concurrency slot.
    active_pairings: Arc<AtomicUsize>,
    approval_tx: broadcast::Sender<ApprovalRequest>,
    presented_tx: broadcast::Sender<PairingPresented>,
    paired_tx: broadcast::Sender<StoredClient>,
}

impl fmt::Debug for PairingCore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Public metadata only: never nonce, SAS, binding, or key values.
        f.debug_struct("PairingCore")
            .field("fingerprint", &self.fingerprint())
            .field("nonces", &self.nonces.lock().map(|n| n.len()).unwrap_or(0))
            .field("sas_window_open", &self.sas_window_open())
            .field("pending_approvals", &self.pending_approvals().len())
            .field("presented", &self.presented().len())
            .field("clients", &self.store.clients().len())
            .finish()
    }
}

impl PairingCore {
    /// Build the core for a Server identity public key and shared store.
    ///
    /// # Panics
    ///
    /// Panics when `server_public_key` is not the 32-byte raw Ed25519 key:
    /// this is a programmer error in the Server wiring, never wire input.
    pub fn new(server_public_key: Vec<u8>, store: Arc<AuthorizationStore>) -> Self {
        Self::with_clock(server_public_key, store, Arc::new(SystemClock))
    }

    /// Build the core with an injected monotonic clock (tests).
    ///
    /// # Panics
    ///
    /// Panics when `server_public_key` is not 32 bytes.
    pub fn with_clock(
        server_public_key: Vec<u8>,
        store: Arc<AuthorizationStore>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let (approval_tx, _) = broadcast::channel(16);
        let (presented_tx, _) = broadcast::channel(16);
        let (paired_tx, _) = broadcast::channel(16);
        Self {
            server_public_key: server_public_key
                .try_into()
                .expect("server identity public key is 32 bytes"),
            store,
            clock,
            nonces: Mutex::new(VecDeque::new()),
            current_qr: Mutex::new(None),
            sas_window: Mutex::new(None),
            failures: Mutex::new(AddressRateGuard::new(
                PAIRING_FAILURE_WINDOW,
                PAIRING_FAILURES_PER_WINDOW,
            )),
            approvals: Mutex::new(HashMap::new()),
            presented: Mutex::new(HashMap::new()),
            approval_seq: AtomicU64::new(0),
            active_pairings: Arc::new(AtomicUsize::new(0)),
            approval_tx,
            presented_tx,
            paired_tx,
        }
    }

    /// Build from a loaded Server identity.
    pub fn from_identity(identity: &ServerIdentity, store: Arc<AuthorizationStore>) -> Self {
        Self::new(identity.public_key_raw().to_vec(), store)
    }

    /// The raw 32-byte Ed25519 identity public key.
    pub fn server_public_key(&self) -> &[u8; 32] {
        &self.server_public_key
    }

    /// The §7.4 fingerprint of the Server identity SPKI.
    pub fn fingerprint(&self) -> String {
        let mut spki = Vec::with_capacity(44);
        spki.extend_from_slice(&[
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
        ]);
        spki.extend_from_slice(&self.server_public_key);
        let digest: [u8; 32] = sha2::Sha256::digest(&spki).into();
        hex_groups(&digest)
    }

    /// The shared authorization store.
    pub fn store(&self) -> &Arc<AuthorizationStore> {
        &self.store
    }

    /// Open a QR invitation: a fresh single-use nonce, valid for 10 minutes.
    pub fn begin_qr(&self, _addr_hint: &str) -> Invitation {
        let nonce = random_nonce();
        let expires_at = self.clock.now() + NONCE_TTL;
        {
            let mut nonces = self.nonces.lock().expect("pairing nonce lock");
            while nonces.len() >= MAX_OUTSTANDING_NONCES {
                nonces.pop_front();
            }
            nonces.push_back(NonceEntry { nonce, expires_at });
        }
        *self.current_qr.lock().expect("current QR invitation lock") = Some(CurrentQrInvitation {
            nonce,
            expires_at,
            consumed: false,
        });
        Invitation {
            mode: PairingMode::Qr,
            nonce: nonce.to_vec(),
            expires_at,
        }
    }

    /// The lifecycle state of the most recently opened QR invitation.
    ///
    /// A consumed invitation stays [`InvitationState::Consumed`] even after its
    /// original expiry: the single-use rule is stronger than the lifetime rule,
    /// and the local admin surface must not auto-renew a failed attempt (§5).
    pub fn invitation_state(&self) -> Option<InvitationState> {
        let current = self.current_qr.lock().expect("current QR invitation lock");
        current.as_ref().map(|invitation| {
            if invitation.consumed {
                InvitationState::Consumed
            } else if self.clock.now() >= invitation.expires_at {
                InvitationState::Expired
            } else {
                InvitationState::Active
            }
        })
    }

    /// Open the admin SAS admission window (no nonce; §7.3).
    pub fn begin_sas(&self) -> Invitation {
        let expires_at = self.clock.now() + SAS_WINDOW_TTL;
        *self.sas_window.lock().expect("sas window lock") = Some(expires_at);
        Invitation {
            mode: PairingMode::Sas,
            nonce: Vec::new(),
            expires_at,
        }
    }

    /// Test-support: install a fixed nonce (golden-vector replay). Production
    /// nonces come only from [`Self::begin_qr`]'s CSPRNG.
    #[cfg(feature = "test-support")]
    pub fn insert_nonce_for_tests(&self, nonce: [u8; NONCE_LEN], expires_at: Instant) {
        let mut nonces = self.nonces.lock().expect("pairing nonce lock");
        while nonces.len() >= MAX_OUTSTANDING_NONCES {
            nonces.pop_front();
        }
        nonces.push_back(NonceEntry { nonce, expires_at });
    }

    /// Close the SAS window and decline every undecided approval.
    ///
    /// Each pending connection's decision callback is invoked with `false`, so
    /// a verified Client receives 201+Close(201) immediately instead of
    /// waiting for the 10-minute pairing deadline. Nonces are left to expire
    /// (closing a dialog does not consume them; §5).
    pub fn cancel(&self) {
        *self.sas_window.lock().expect("sas window lock") = None;
        self.presented
            .lock()
            .expect("pairing presented lock")
            .clear();
        let pending: Vec<_> = self
            .approvals
            .lock()
            .expect("pairing approval lock")
            .drain()
            .map(|(_, pending)| pending)
            .collect();
        for pending in pending {
            (pending.decide)(false);
        }
    }

    /// The QR URI for an invitation.
    pub fn qr_uri(&self, invitation: &Invitation, addr_hint: &str) -> Option<String> {
        if invitation.mode != PairingMode::Qr {
            return None;
        }
        let nonce: [u8; NONCE_LEN] = invitation.nonce.as_slice().try_into().ok()?;
        Some(encode_qr_uri(addr_hint, &self.server_public_key, &nonce))
    }

    /// Admit a PairingRequest against the QR nonce table or the SAS window.
    ///
    /// A valid nonce is consumed by this first well-formed request regardless
    /// of the pairing's eventual outcome (§5). Structural failures must be
    /// rejected before this call so they never burn a nonce.
    ///
    /// On success the returned [`PairingPermit`] holds one of the
    /// [`MAX_CONCURRENT_PAIRINGS`] slots; the caller must keep it alive until
    /// the pending exchange is resolved. A ninth concurrent admission returns
    /// [`PairingError::ConcurrentLimit`] (the state machine answers 302).
    pub fn admit_request(&self, request: &PairingRequest) -> Result<PairingPermit, PairingError> {
        let permit = self
            .acquire_pairing_permit()
            .ok_or(PairingError::ConcurrentLimit)?;
        match PairingMode::try_from(request.pairing_mode) {
            Ok(PairingMode::Qr) => self.admit_qr(&request.pairing_nonce)?,
            Ok(PairingMode::Sas) => {
                if !request.pairing_nonce.is_empty() {
                    return Err(PairingError::Malformed("sas mode must omit pairing_nonce"));
                }
                self.admit_sas()?;
            }
            Ok(PairingMode::Unspecified) | Err(_) => return Err(PairingError::Mode),
        }
        Ok(permit)
    }

    /// Reserve one of the [`MAX_CONCURRENT_PAIRINGS`] slots without blocking.
    fn acquire_pairing_permit(&self) -> Option<PairingPermit> {
        let mut active = self.active_pairings.load(Ordering::Acquire);
        loop {
            if active >= MAX_CONCURRENT_PAIRINGS {
                return None;
            }
            match self.active_pairings.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Some(PairingPermit {
                        active: Arc::clone(&self.active_pairings),
                    })
                }
                Err(current) => active = current,
            }
        }
    }

    fn admit_qr(&self, nonce: &[u8]) -> Result<(), PairingError> {
        if nonce.len() != NONCE_LEN {
            return Err(PairingError::Malformed("pairing_nonce must be 16 bytes"));
        }
        let now = self.clock.now();
        let admitted = {
            let mut nonces = self.nonces.lock().expect("pairing nonce lock");
            let index = nonces
                .iter()
                .position(|entry| entry.nonce.as_slice() == nonce);
            match index {
                Some(index) => {
                    let entry = nonces.remove(index).expect("index found");
                    if now >= entry.expires_at {
                        Err(PairingError::NonceExpired)
                    } else {
                        Ok(())
                    }
                }
                None => Err(PairingError::NonceInvalid),
            }
        };
        if admitted.is_ok() {
            let mut current = self.current_qr.lock().expect("current QR invitation lock");
            if let Some(invitation) = current.as_mut() {
                if invitation.nonce.as_slice() == nonce {
                    invitation.consumed = true;
                }
            }
        }
        admitted
    }

    fn admit_sas(&self) -> Result<(), PairingError> {
        let now = self.clock.now();
        let mut window = self.sas_window.lock().expect("sas window lock");
        match *window {
            Some(expires_at) if now < expires_at => Ok(()),
            Some(_) => {
                *window = None;
                Err(PairingError::SasWindowClosed)
            }
            None => Err(PairingError::SasWindowClosed),
        }
    }

    /// Whether the admin SAS window is currently open.
    pub fn sas_window_open(&self) -> bool {
        let now = self.clock.now();
        self.sas_window
            .lock()
            .expect("sas window lock")
            .is_some_and(|expires_at| now < expires_at)
    }

    /// Record one pairing failure against a source address; `true` when the
    /// address reached its TM-5 budget (the caller answers Close(302)).
    pub fn note_failure(&self, address: IpAddr) -> bool {
        self.failures
            .lock()
            .expect("pairing failure lock")
            .record(address, self.clock.now())
    }

    /// Whether the address is already over its TM-5 budget.
    pub fn is_limited(&self, address: IpAddr) -> bool {
        self.failures
            .lock()
            .expect("pairing failure lock")
            .is_limited(address, self.clock.now())
    }

    /// Allocate the next process-unique approval handle.
    ///
    /// The handle is deliberately independent of the connection's wire
    /// `request_id`; two concurrent phones may both use `request_id = 1`.
    pub(crate) fn next_approval_id(&self) -> u64 {
        self.approval_seq.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Publish a live pairing after `PairingResponse` was written.
    ///
    /// At most one entry exists per `approval_id`; a re-publish replaces it.
    /// The broadcast lets a one-shot CLI surface print the code immediately
    /// instead of polling.
    pub fn publish_presented(&self, presented: PairingPresented) {
        let approval_id = presented.approval_id;
        self.presented
            .lock()
            .expect("pairing presented lock")
            .insert(approval_id, presented.clone());
        let _ = self.presented_tx.send(presented);
    }

    /// Every live presented pairing, ordered by process-local handle.
    pub fn presented(&self) -> Vec<PairingPresented> {
        let mut presented: Vec<_> = self
            .presented
            .lock()
            .expect("pairing presented lock")
            .values()
            .cloned()
            .collect();
        presented.sort_by_key(|request| request.approval_id);
        presented
    }

    /// Drop one live presented pairing (deadline, decline, connection loss).
    fn clear_presented(&self, approval_id: u64) {
        self.presented
            .lock()
            .expect("pairing presented lock")
            .remove(&approval_id);
    }

    /// Subscribe to live pairing presentations (one receiver per admin surface).
    pub fn subscribe_presented(&self) -> broadcast::Receiver<PairingPresented> {
        self.presented_tx.subscribe()
    }

    /// Register a verified-but-unapproved pairing and notify observers.
    ///
    /// The decision callback is invoked at most once, by [`Self::approve`],
    /// [`Self::decline`], or [`Self::cancel`]; the state machine never blocks
    /// on human input. Returns the request's local `approval_id` (assigning
    /// one when the caller left it at zero).
    pub fn request_approval(
        &self,
        mut request: ApprovalRequest,
        decide: Box<dyn FnOnce(bool) + Send>,
    ) -> u64 {
        if request.approval_id == 0 {
            request.approval_id = self.next_approval_id();
        }
        let approval_id = request.approval_id;
        self.approvals
            .lock()
            .expect("pairing approval lock")
            .insert(
                approval_id,
                PendingApproval {
                    request: request.clone(),
                    decide,
                },
            );
        // No subscribers is fine: the pending entry remains for an explicit
        // approve/decline by approval id.
        let _ = self.approval_tx.send(request);
        approval_id
    }

    /// Approve one pending pairing; `false` when the id is not pending.
    pub fn approve(&self, approval_id: u64) -> bool {
        let pending = self
            .approvals
            .lock()
            .expect("pairing approval lock")
            .remove(&approval_id);
        match pending {
            Some(pending) => {
                crate::session_log::pairing_accepted(&pending.request.client_id);
                self.clear_presented(approval_id);
                (pending.decide)(true);
                true
            }
            None => false,
        }
    }

    /// Decline one pending pairing; `false` when the id is not pending.
    pub fn decline(&self, approval_id: u64) -> bool {
        let pending = self
            .approvals
            .lock()
            .expect("pairing approval lock")
            .remove(&approval_id);
        match pending {
            Some(pending) => {
                crate::session_log::pairing_declined(&pending.request.client_id);
                self.clear_presented(approval_id);
                (pending.decide)(false);
                true
            }
            None => false,
        }
    }

    /// Drop a pending approval without deciding (session ended or timed out).
    pub fn cancel_approval(&self, approval_id: u64) {
        self.approvals
            .lock()
            .expect("pairing approval lock")
            .remove(&approval_id);
        self.clear_presented(approval_id);
    }

    /// Every pending approval, for local admin surfaces.
    pub fn pending_approvals(&self) -> Vec<ApprovalRequest> {
        let mut requests: Vec<_> = self
            .approvals
            .lock()
            .expect("pairing approval lock")
            .values()
            .map(|pending| pending.request.clone())
            .collect();
        requests.sort_by_key(|request| request.approval_id);
        requests
    }

    /// Subscribe to incoming approval requests (one receiver per admin
    /// surface). Requests sent before subscribing remain pending by id.
    pub fn subscribe_approvals(&self) -> broadcast::Receiver<ApprovalRequest> {
        self.approval_tx.subscribe()
    }

    /// Subscribe to successful persistent pairings.
    pub fn subscribe_paired(&self) -> broadcast::Receiver<StoredClient> {
        self.paired_tx.subscribe()
    }

    /// Persist a successful pairing through [`AuthorizationStore::upsert`]
    /// (the production path; never `insert_for_tests`).
    ///
    /// This method only writes the record. The success broadcast is a separate
    /// step ([`Self::notify_paired`]) so the driver can fire it *after* the
    /// `PairingResult` frame has been written to the control stream: a one-shot
    /// consumer (such as `as-cmd pair`) can then stop the Server without
    /// racing the success result.
    pub fn persist_client(
        &self,
        public_key: &[u8],
        display_name: &str,
    ) -> Result<StoredClient, PairingError> {
        if auth::parse_client_public_key(public_key).is_err() {
            return Err(PairingError::Malformed(
                "client_public_key must be a 33-byte SEC1 point",
            ));
        }
        let display_name = normalize_display_name(display_name);
        let now = rfc3339_now();
        let created_at = self
            .store
            .record_for(public_key)
            .map(|record| record.created_at)
            .filter(|created| !created.is_empty())
            .unwrap_or_else(|| now.clone());
        let client_id: String = auth::client_id(public_key)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let client = StoredClient {
            client_id,
            public_key: public_key.to_vec(),
            display_name,
            state: ClientState::Active,
            created_at,
            last_seen_at: now,
        };
        self.store.upsert(client.clone())?;
        Ok(client)
    }

    /// Announce a persisted pairing to local admin surfaces.
    ///
    /// Call this only after the `PairingResult(OK)` frame for that pairing has
    /// been written successfully; a failed write must not notify anyone.
    pub fn notify_paired(&self, client: StoredClient) {
        let _ = self.paired_tx.send(client);
    }

    /// List authorized clients (admin surface).
    pub fn clients(&self) -> Vec<StoredClient> {
        self.store.clients()
    }

    /// Revoke a client by canonical `client_id`; keeps the record for audit.
    pub fn revoke(&self, client_id: &str) -> Result<StoredClient, StoreError> {
        let client = self.store.revoke_client(client_id)?;
        crate::session_log::client_revoked(&client.client_id);
        Ok(client)
    }
}

/// Normalize an accepted display name: an empty name becomes a fixed local
/// label and an over-long name is truncated on a UTF-8 boundary (pairing.md
/// §6.2/§8.1 cap 63 bytes).
///
/// Shared by the state machine (PairingResponse) and persistence so the
/// accepted wire value and the stored value can never diverge.
pub(crate) fn normalize_display_name(name: &str) -> String {
    if name.is_empty() {
        return "Unknown device".to_owned();
    }
    if name.len() <= 63 {
        return name.to_owned();
    }
    let mut end = 63;
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_owned()
}

fn random_nonce() -> [u8; NONCE_LEN] {
    use ring::rand::{SecureRandom as _, SystemRandom};
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .expect("OS CSPRNG is available");
    nonce
}

/// Current UTC time in RFC 3339 (`YYYY-MM-DDTHH:MM:SSZ`), the §8.1 format.
pub fn rfc3339_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = seconds.div_euclid(86_400);
    let second_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = second_of_day / 3_600;
    let minute = (second_of_day % 3_600) / 60;
    let second = second_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Whether a string has the canonical RFC 3339 UTC shape the store writes.
pub(crate) fn is_rfc3339_utc(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'Z'
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || byte.is_ascii_digit()
        })
}

/// Convert days since 1970-01-01 to a civil (year, month, day).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = (days - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

/// The fail-closed identity reset (pairing.md §8.3/§10.3).
///
/// Two files cannot be replaced in one atomic step. The order is deliberate:
///
/// 1. **Clear every Client record first** and persist the empty store. A crash
///    here leaves the old identity with no authorized Clients: every old
///    pairing fails closed and no Client is silently retained.
/// 2. **Then write the new identity key** atomically (with one backup). A
///    crash here leaves the old key with an empty store — still fail-closed.
///
/// The reverse order could leave old Client records authorized under a new
/// key after a crash, which is exactly the state this operation must prevent.
pub fn reset_identity(
    config: &IdentityConfig,
    store: &AuthorizationStore,
) -> Result<ServerIdentity, PairingError> {
    store.clear_all()?;
    let identity = ServerIdentity::reset(config)?;
    crate::session_log::identity_reset(&identity.fingerprint());
    Ok(identity)
}
