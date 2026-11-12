//! The server-side authorization store (pairing.md §8.1, §10.2, §10.3; PR-10).
//!
//! One JSON file, `pairing/clients.json` in the Server data directory: public
//! keys and metadata only — no secrets. `client_id` is derived (first 8 bytes
//! of SHA-256(public_key)), never allocated. Writes are atomic with one backup
//! generation (`clients.json.bak`); a store that fails validation at load
//! fails closed (v2 auth disabled, never regenerated, pairing.md §10.3).
//!
//! POSIX permissions follow §10.2: file 0600, containing directory 0700.
//!
//! `insert_for_tests` is test-only (pairing.md §11.3): production pairing
//! persistence arrives with the pairing phase (ticket 04's production flow).

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// The store file name inside `<data dir>/pairing/` (pairing.md §10.2).
pub const CLIENTS_FILE: &str = "clients.json";

/// A Client authorization record (pairing.md §8.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredClient {
    /// First 8 bytes of SHA-256(public_key), hex (§8.1) — derived, not
    /// allocated, so concurrent writers cannot collide.
    pub client_id: String,
    /// The 33-byte SEC1-compressed P-256 public key.
    pub public_key: Vec<u8>,
    /// UTF-8 display name, ≤ 63 bytes.
    pub display_name: String,
    /// `active` or `revoked` (§8.1); revoke keeps the record for audit.
    pub state: ClientState,
}

/// The record's authorization state (§8.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientState {
    /// Authorized: authentication may succeed.
    Active,
    /// Revoked: refused immediately (§9.2); kept for audit.
    Revoked,
}

/// The on-disk JSON schema (`version` 1; §8.1).
#[derive(Serialize, Deserialize)]
struct StoreFile {
    version: u32,
    clients: Vec<StoredClientJson>,
}

#[derive(Serialize, Deserialize)]
struct StoredClientJson {
    client_id: String,
    /// base64url (RFC 4648 §5, no padding) of the 33-byte compressed point.
    public_key: String,
    display_name: String,
    /// "active" | "revoked".
    state: String,
}

impl StoredClient {
    fn to_json(&self) -> StoredClientJson {
        StoredClientJson {
            client_id: self.client_id.clone(),
            public_key: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&self.public_key),
            display_name: self.display_name.clone(),
            state: match self.state {
                ClientState::Active => "active".to_owned(),
                ClientState::Revoked => "revoked".to_owned(),
            },
        }
    }

    fn from_json(json: StoredClientJson) -> Result<Self, String> {
        if json.state != "active" && json.state != "revoked" {
            return Err(format!("unknown client state {:?}", json.state));
        }
        let public_key = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&json.public_key)
            .map_err(|error| format!("public_key is not base64url: {error}"))?;
        // pairing.md §8.1: the stored key is a 33-byte SEC1-compressed P-256
        // point. Decoding validates the point, so the store can never hold a
        // credential that `AuthInit` could not later verify (fail closed).
        if crate::auth::parse_client_public_key(&public_key).is_err() {
            return Err(
                "public_key is not a 33-byte SEC1-compressed P-256 point".to_owned()
            );
        }
        // client_id is derived, never allocated: first 8 bytes of
        // SHA-256(public_key), lowercase hex (§8.1).
        let derived: String = crate::auth::client_id(&public_key)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if !json.client_id.eq_ignore_ascii_case(&derived) {
            return Err(format!(
                "client_id {:?} does not match the derived value {derived:?}",
                json.client_id
            ));
        }
        if json.display_name.len() > 63 {
            return Err(format!(
                "display_name is {} bytes; the limit is 63 UTF-8 bytes",
                json.display_name.len()
            ));
        }
        Ok(Self {
            client_id: json.client_id,
            public_key,
            display_name: json.display_name,
            state: if json.state == "active" {
                ClientState::Active
            } else {
                ClientState::Revoked
            },
        })
    }
}

/// The outcome of consulting the store at `AuthInit`/`AuthProof` (§9.2: the
/// authorization state is consulted at every authentication; no cache outlives
/// the store).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Authorization {
    /// The identity matches no record → 202 AUTHORIZATION_UNKNOWN.
    Unknown(#[allow(dead_code)] Vec<u8>),
    /// The identity matches an active record.
    Authorized(StoredClient),
    /// The identity matches a revoked record → 204 AUTHORIZATION_DENIED.
    Revoked(StoredClient),
}

/// A store failure. Every variant fails closed (v2 auth disabled).
#[derive(Debug)]
pub enum StoreError {
    /// The file exists but does not validate (bad JSON, bad schema).
    Corrupted {
        /// The offending path.
        path: PathBuf,
        /// Why validation failed.
        reason: String,
    },
    /// An OS operation failed.
    Io {
        /// What the store was doing.
        operation: &'static str,
        /// The underlying error.
        source: io::Error,
    },
    /// The caller named a public key with no record (operator mistake, not
    /// corruption — nothing on disk is wrong).
    UnknownClient {
        /// The key that was not found.
        public_key: Vec<u8>,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Corrupted { path, reason } => write!(
                f,
                "authorization store {} fails validation: {reason}; v2 auth fails closed — restore clients.json.bak or replace the file manually, then restart (pairing.md §10.3)",
                path.display()
            ),
            Self::Io { operation, source } => write!(f, "{operation}: {source}"),
            Self::UnknownClient { public_key } => write!(
                f,
                "revoke of an unknown public key ({} bytes)",
                public_key.len()
            ),
        }
    }
}

impl std::error::Error for StoreError {}

/// The in-memory view over `clients.json` (§8.1). The running Server Core owns
/// it; admin surfaces operate this state (pairing.md §11.1).
///
/// Interior mutability because the accept loop, the session registry, and the
/// admin surfaces share one store (`Arc<AuthorizationStore>`, pairing.md
/// §11.1); all mutations persist atomically (§10.3).
pub struct AuthorizationStore {
    path: Option<PathBuf>,
    clients: std::sync::RwLock<Vec<StoredClient>>,
}

impl fmt::Debug for AuthorizationStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Public metadata only — never key material in Debug output.
        f.debug_struct("AuthorizationStore")
            .field("path", &self.path)
            .field(
                "clients",
                &self
                    .clients
                    .read()
                    .map(|clients| clients.iter().map(|c| c.client_id.clone()).collect::<Vec<_>>())
                    .unwrap_or_default(),
            )
            .finish()
    }
}

impl AuthorizationStore {
    /// Open (or lazily create) the store at `<data dir>/pairing/clients.json`.
    ///
    /// A missing file starts an empty store (nothing is written until the
    /// first mutation). A file that fails validation is a fail-closed error —
    /// never regenerated, never silently truncated (§10.3).
    pub fn open(data_dir: &Path) -> Result<Self, StoreError> {
        let pairing_dir = data_dir.join("pairing");
        let path = pairing_dir.join(CLIENTS_FILE);
        if !path.exists() {
            return Ok(Self { path: Some(path), clients: std::sync::RwLock::new(Vec::new()) });
        }
        let text = fs::read_to_string(&path).map_err(|source| StoreError::Io {
            operation: "read authorization store",
            source,
        })?;
        let clients = parse_store_text(&text).map_err(|reason| StoreError::Corrupted {
            path: path.clone(),
            reason,
        })?;
        Ok(Self { path: Some(path), clients: std::sync::RwLock::new(clients) })
    }

    /// A memory-only store (tests / isolated configurations).
    pub fn in_memory() -> Self {
        Self { path: None, clients: std::sync::RwLock::new(Vec::new()) }
    }

    /// Consult the authorization state for this public key (§9.2: at every
    /// authentication, never a cached answer).
    pub fn authorize(&self, public_key: &[u8]) -> Authorization {
        let guard = self.clients.read().expect("authorization store lock");
        match guard.iter().find(|client| client.public_key == public_key) {
            Some(client) => match client.state {
                ClientState::Active => Authorization::Authorized(client.clone()),
                ClientState::Revoked => Authorization::Revoked(client.clone()),
            },
            None => Authorization::Unknown(public_key.to_vec()),
        }
    }

    /// The record for a public key, when one exists.
    pub fn record_for(&self, public_key: &[u8]) -> Option<StoredClient> {
        let guard = self.clients.read().expect("authorization store lock");
        guard.iter().find(|client| client.public_key == public_key).cloned()
    }

    /// Every record (admin surfaces / audit).
    pub fn clients(&self) -> Vec<StoredClient> {
        self.clients.read().expect("authorization store lock").clone()
    }

    /// Add or replace a record (keyed by public key; the store never holds two
    /// records for one key — pairing.md §8.3: replacement is an explicit admin
    /// action, and this is it). Persists atomically.
    pub fn upsert(&self, client: StoredClient) -> Result<(), StoreError> {
        {
            let mut guard = self.clients.write().expect("authorization store lock");
            match guard.iter_mut().find(|existing| existing.public_key == client.public_key) {
                Some(existing) => *existing = client.clone(),
                None => guard.push(client.clone()),
            }
        }
        self.persist()
    }

    /// Insert into the store and persist. Test-support credential preload
    /// (ticket 07's test configuration) routes through here so the persisted
    /// file and the in-memory view stay identical; gated per pairing.md §11.3.
    #[cfg(feature = "test-support")]
    pub fn insert_for_tests(&self, client: StoredClient) {
        self.upsert(client).expect("persist test credential");
    }

    /// Revoke a record by public key (§9.2: revoke keeps the record, state
    /// becomes `revoked`); persists immediately. Unknown keys are an error.
    pub fn revoke(&self, public_key: &[u8]) -> Result<StoredClient, StoreError> {
        let updated = {
            let mut guard = self.clients.write().expect("authorization store lock");
            let found = guard
                .iter()
                .find(|client| client.public_key == public_key)
                .ok_or_else(|| StoreError::UnknownClient { public_key: public_key.to_vec() })?
                .clone();
            let updated = StoredClient { state: ClientState::Revoked, ..found };
            if let Some(slot) = guard.iter_mut().find(|client| client.public_key == public_key) {
                *slot = updated.clone();
            }
            updated
        };
        self.persist()?;
        Ok(updated)
    }

    /// Whether the store was loaded from a file (a memory-only store skips
    /// persistence).
    pub fn is_persistent(&self) -> bool {
        self.path.is_some()
    }

    /// Atomic write + one backup generation (§10.3). Memory-only stores skip.
    fn persist(&self) -> Result<(), StoreError> {
        let Some(path) = &self.path else { return Ok(()) };
        let pairing_dir = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(pairing_dir).map_err(|source| StoreError::Io {
            operation: "create pairing directory",
            source,
        })?;
        tighten_dir(pairing_dir)?;
        let clients = self.clients();
        let file = StoreFile {
            version: 1,
            clients: clients.iter().map(StoredClient::to_json).collect(),
        };
        let text = serde_json::to_string_pretty(&file).map_err(|error| StoreError::Corrupted {
            path: path.clone(),
            reason: format!("serialize: {error}"),
        })?;
        crate::identity::atomic_write_with_backup(path, &text).map_err(|error| StoreError::Io {
            operation: "persist authorization store",
            source: io::Error::other(error.to_string()),
        })?;
        Ok(())
    }
}

fn parse_store_text(text: &str) -> Result<Vec<StoredClient>, String> {
    let file: StoreFile = serde_json::from_str(text).map_err(|error| format!("not valid JSON: {error}"))?;
    if file.version != 1 {
        return Err(format!("unsupported store version {}", file.version));
    }
    let clients = file
        .clients
        .into_iter()
        .map(StoredClient::from_json)
        .collect::<Result<Vec<_>, _>>()?;
    // client_id is the unique identity key (§8.1); two records claiming the
    // same derived id would make authorization order-dependent.
    let mut seen = std::collections::HashSet::with_capacity(clients.len());
    for client in &clients {
        if !seen.insert(client.client_id.to_ascii_lowercase()) {
            return Err(format!("duplicate client_id {:?}", client.client_id));
        }
    }
    Ok(clients)
}

#[cfg(unix)]
fn tighten_dir(dir: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(|source| {
        StoreError::Io { operation: "restrict pairing directory", source }
    })
}

#[cfg(not(unix))]
fn tighten_dir(_dir: &Path) -> Result<(), StoreError> {
    Ok(())
}
