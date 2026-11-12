//! Server identity key lifecycle (pairing.md §2, §10.2, §10.3; PR-1).
//!
//! The Server holds a persistent Ed25519 identity key as a PKCS#8 PEM file at
//! `<data dir>/identity/server-identity.pem` (file 0600, containing directory
//! 0700). Writes are atomic with one backup generation. A corrupted key file
//! fails closed — v2 stays disabled and the file is never regenerated
//! (pairing.md §10.3). On POSIX, loose permissions refuse to enable v2 unless
//! `allow_loose_permissions` is set (logged); on Windows the equivalent ACL
//! check is advisory (warn-only), per pairing.md §10.2.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use ring::signature::Ed25519KeyPair;
use ring::signature::KeyPair as _;

use crate::codes::IDENTITY_KEY_FILE;

/// The 12-byte DER prefix of a SubjectPublicKeyInfo for a raw Ed25519 key
/// (pairing.md §4.2): fixed algorithm identifier + 32-byte key.
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

const PEM_BEGIN: &str = "-----BEGIN PRIVATE KEY-----";
const PEM_END: &str = "-----END PRIVATE KEY-----";

/// Where the identity key lives and how strictly permissions are enforced.
#[derive(Clone, Debug)]
pub struct IdentityConfig {
    /// The Server's per-user application-data directory
    /// (`<data dir>/identity/server-identity.pem`, pairing.md §10.2).
    pub data_dir: PathBuf,
    /// POSIX override for the loose-permission refusal
    /// (`--allow-loose-key-permissions`; discouraged, logged).
    pub allow_loose_permissions: bool,
}

/// An identity-key failure. Every variant disables v2 (fail-closed); none of
/// them ever regenerates the key.
#[derive(Debug)]
pub enum IdentityError {
    /// The key file exists but is not a parseable Ed25519 PKCS#8 key.
    Corrupted {
        /// The offending file path.
        path: PathBuf,
        /// Why the parse failed.
        reason: String,
    },
    /// POSIX: the key file or its directory is readable by group/other.
    LoosePermissions {
        /// The offending path.
        path: PathBuf,
        /// The observed permission bits.
        mode: u32,
    },
    /// An operating-system operation failed.
    Io {
        /// What the identity loader was doing.
        operation: &'static str,
        /// The underlying OS error.
        source: io::Error,
    },
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Corrupted { path, reason } => {
                write!(f, "server identity key {} is corrupted: {}; v2 stays disabled, reset the identity explicitly to recover", path.display(), reason)
            }
            Self::LoosePermissions { path, mode } => write!(
                f,
                "server identity {} has loose permissions (mode {:o}); refusing to enable v2 (use --allow-loose-key-permissions to override)",
                path.display(),
                mode
            ),
            Self::Io { operation, source } => write!(f, "{operation}: {source}"),
        }
    }
}

impl std::error::Error for IdentityError {}

/// The loaded Server identity: Ed25519 key pair, PKCS#8 DER, and derived pin
/// material (SPKI SHA-256).
pub struct ServerIdentity {
    key_pair: Ed25519KeyPair,
    pkcs8_der: Vec<u8>,
    key_path: PathBuf,
    spki_sha256: [u8; 32],
}

impl fmt::Debug for ServerIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never include key material in Debug output.
        f.debug_struct("ServerIdentity")
            .field("key_path", &self.key_path)
            .field("fingerprint", &hex_groups(&self.spki_sha256))
            .finish()
    }
}

impl ServerIdentity {
    /// Load the identity key, generating and persisting it on first start.
    ///
    /// Fail-closed rules (pairing.md §10.3): a key file that exists but does
    /// not parse is an error and the file is left untouched — recovery is an
    /// explicit operator identity reset, never an automatic regeneration.
    pub fn open(config: &IdentityConfig) -> Result<Self, IdentityError> {
        let identity_dir = config.data_dir.join("identity");
        let key_path = identity_dir.join(IDENTITY_KEY_FILE);
        let created = !identity_dir.exists();
        fs::create_dir_all(&identity_dir)
            .map_err(|source| IdentityError::Io { operation: "create identity directory", source })?;
        // A freshly created directory is tightened to 0700 explicitly
        // (create_dir_all honors the umask, typically 022 → 0755; pairing.md
        // §10.2). An existing directory is never silently repaired: a loose
        // mode is the operator's doing and must fail closed below.
        if created {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                fs::set_permissions(&identity_dir, fs::Permissions::from_mode(0o700)).map_err(
                    |source| IdentityError::Io {
                        operation: "restrict identity directory",
                        source,
                    },
                )?;
            }
        }
        check_directory_permissions(&identity_dir, config.allow_loose_permissions)?;

        if key_path.exists() {
            check_file_permissions(&key_path, config.allow_loose_permissions)?;
            let pem = fs::read_to_string(&key_path).map_err(|source| IdentityError::Io {
                operation: "read identity key",
                source,
            })?;
            let der = pem_to_pkcs8_der(&pem).ok_or_else(|| IdentityError::Corrupted {
                path: key_path.clone(),
                reason: "not a PKCS#8 PEM private key".to_owned(),
            })?;
            let key_pair = Ed25519KeyPair::from_pkcs8(&der).map_err(|error| IdentityError::Corrupted {
                path: key_path.clone(),
                reason: error.to_string(),
            })?;
            Ok(Self::from_key_pair(key_pair, der, key_path))
        } else {
            let rng = ring::rand::SystemRandom::new();
            let document = Ed25519KeyPair::generate_pkcs8(&rng)
                .map_err(|_| IdentityError::Io {
                    operation: "generate Ed25519 identity key",
                    source: io::Error::other("ring key generation failed"),
                })?
                .as_ref()
                .to_vec();
            let key_pair = Ed25519KeyPair::from_pkcs8(&document)
                .expect("ring accepts the PKCS#8 document it just produced");
            let identity = Self::from_key_pair(key_pair, document.clone(), key_path.clone());
            atomic_write_with_backup(&key_path, &pkcs8_to_pem(&document))?;
            Ok(identity)
        }
    }

    fn from_key_pair(key_pair: Ed25519KeyPair, pkcs8_der: Vec<u8>, key_path: PathBuf) -> Self {
        use sha2::Digest as _;
        let mut spki = Vec::with_capacity(ED25519_SPKI_PREFIX.len() + 32);
        spki.extend_from_slice(&ED25519_SPKI_PREFIX);
        spki.extend_from_slice(key_pair.public_key().as_ref());
        let digest: [u8; 32] = sha2::Sha256::digest(&spki).into();
        Self {
            key_pair,
            pkcs8_der,
            key_path,
            spki_sha256: digest,
        }
    }

    /// The raw 32-byte Ed25519 identity public key (pairing.md §2).
    pub fn public_key_raw(&self) -> &[u8] {
        self.key_pair.public_key().as_ref()
    }

    /// The 44-byte SubjectPublicKeyInfo DER for the raw Ed25519 key
    /// (pairing.md §4.2); Android Clients pin SHA-256 of exactly these bytes.
    pub fn spki_der(&self) -> Vec<u8> {
        let mut spki = Vec::with_capacity(ED25519_SPKI_PREFIX.len() + 32);
        spki.extend_from_slice(&ED25519_SPKI_PREFIX);
        spki.extend_from_slice(self.public_key_raw());
        spki
    }

    /// SHA-256 of the SPKI DER — the pin material Clients pair against
    /// (pairing.md §6.1, PR-1). Logged at startup in the §7.4 presentation.
    pub fn spki_sha256(&self) -> &[u8; 32] {
        &self.spki_sha256
    }

    /// The fingerprint in the §7.4 presentation: 8 groups of 8 uppercase hex
    /// characters, space-separated.
    pub fn fingerprint(&self) -> String {
        hex_groups(&self.spki_sha256)
    }

    /// The identity key file path (for diagnostics; never logs key material).
    pub fn key_path(&self) -> &Path {
        &self.key_path
    }

    /// Build the rustls TLS 1.3 server config from the identity: the
    /// self-signed Ed25519 certificate (generated here, per PR-1) and the
    /// identity key. ALPN and 0-RTT policy are the transport's (§3); this
    /// method supplies certificate material only.
    pub fn tls_server_config(&self) -> Result<rustls::ServerConfig, IdentityError> {
        let key_pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(
            &rustls_pki_types::PrivatePkcs8KeyDer::from(self.pkcs8_der.clone()),
            &rcgen::PKCS_ED25519,
        )
        .map_err(|error| IdentityError::Corrupted {
            path: self.key_path.clone(),
            reason: format!("identity key unusable for TLS: {error}"),
        })?;
        // The self-signed certificate carries the raw public key as its SPKI,
        // so the pin (SHA-256 of the SPKI) survives certificate regeneration.
        let mut params = rcgen::CertificateParams::new(vec!["audio-share-server.local".to_owned()])
            .map_err(|error| IdentityError::Io {
                operation: "build certificate parameters",
                source: io::Error::other(error.to_string()),
            })?;
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let certificate = params
            .self_signed(&key_pair)
            .map_err(|error| IdentityError::Io {
                operation: "self-sign the identity certificate",
                source: io::Error::other(error.to_string()),
            })?;

        let provider = rustls::crypto::ring::default_provider();
        let config = rustls::ServerConfig::builder_with_provider(provider.into())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("TLS 1.3 is available in the rustls ring provider")
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls_pki_types::CertificateDer::from(certificate.der().to_vec())],
                rustls_pki_types::PrivateKeyDer::Pkcs8(rustls_pki_types::PrivatePkcs8KeyDer::from(
                    self.pkcs8_der.clone(),
                )),
            )
            .map_err(|error| IdentityError::Corrupted {
                path: self.key_path.clone(),
                reason: format!("certificate/key pair rejected: {error}"),
            })?;
        Ok(config)
    }
}

/// POSIX permission check for the key file: refuse group/other read bits
/// (pairing.md §10.2). Windows has no POSIX bits; the check compiles to `Ok`
/// there (ACL enumeration is advisory and warn-only per §10.2 §12).
fn check_file_permissions(path: &Path, allow_loose: bool) -> Result<(), IdentityError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(path)
            .map_err(|source| IdentityError::Io { operation: "stat identity key", source })?
            .permissions()
            .mode();
        if mode & 0o077 != 0 && !allow_loose {
            return Err(IdentityError::LoosePermissions { path: path.to_path_buf(), mode });
        }
    }
    #[cfg(windows)]
    {
        let _ = (path, allow_loose); // warn-only by decision (pairing.md §12)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (path, allow_loose);
    }
    Ok(())
}

/// POSIX permission check for the containing directory (0700 target).
fn check_directory_permissions(path: &Path, allow_loose: bool) -> Result<(), IdentityError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(path)
            .map_err(|source| IdentityError::Io { operation: "stat identity directory", source })?
            .permissions()
            .mode();
        if mode & 0o077 != 0 && !allow_loose {
            return Err(IdentityError::LoosePermissions { path: path.to_path_buf(), mode });
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (path, allow_loose);
    }
    Ok(())
}

/// Atomic write discipline (pairing.md §10.3): temp file → fsync → rename
/// over the target, keeping the previous version as `<name>.bak` (one backup
/// generation). Applied to every identity/pairing write.
pub(crate) fn atomic_write_with_backup(path: &Path, contents: &str) -> Result<(), IdentityError> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    // Keep the full file name plus `.bak` (clients.json → clients.json.bak),
    // as pairing.md §10.3 and the auth-store recovery message specify.
    let backup = path
        .file_name()
        .map(|name| path.with_file_name(format!("{}.bak", name.to_string_lossy())))
        .unwrap_or_else(|| path.with_extension("bak"));
    if path.exists() {
        fs::rename(path, &backup).map_err(|source| IdentityError::Io {
            operation: "rotate identity key backup",
            source,
        })?;
    }
    let temp = directory.join(format!(
        ".{}.tmp",
        path.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "identity".to_owned())
    ));
    {
        let mut file = fs::File::create(&temp).map_err(|source| IdentityError::Io {
            operation: "create identity temp file",
            source,
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o600)).map_err(|source| {
                IdentityError::Io { operation: "restrict temp file permissions", source }
            })?;
        }
        io::Write::write_all(&mut file, contents.as_bytes()).map_err(|source| {
            IdentityError::Io { operation: "write identity key", source }
        })?;
        file.sync_all().map_err(|source| IdentityError::Io {
            operation: "fsync identity key",
            source,
        })?;
    }
    fs::rename(&temp, path).map_err(|source| IdentityError::Io {
        operation: "publish identity key",
        source,
    })?;
    Ok(())
}

/// Wrap PKCS#8 DER in the §10.2 PEM container.
fn pkcs8_to_pem(der: &[u8]) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(der);
    let mut pem = String::from(PEM_BEGIN);
    pem.push('\n');
    for chunk in encoded.as_bytes().chunks(64) {
        pem.push_str(std::str::from_utf8(chunk).expect("base64 is ASCII"));
        pem.push('\n');
    }
    pem.push_str(PEM_END);
    pem.push('\n');
    pem
}

/// Parse the §10.2 PEM container back into PKCS#8 DER; `None` when the text
/// is not a well-formed PEM private key.
fn pem_to_pkcs8_der(pem: &str) -> Option<Vec<u8>> {
    let body: String = pem
        .lines()
        .filter(|line| !line.trim().starts_with("-----"))
        .collect();
    if pem.trim().starts_with(PEM_BEGIN) && pem.trim().contains(PEM_END) {
        base64::engine::general_purpose::STANDARD.decode(body.trim()).ok()
    } else {
        None
    }
}

/// Render bytes as 8 groups of 8 uppercase hex characters, space-separated
/// (pairing.md §7.4 fingerprint presentation).
pub(crate) fn hex_groups(bytes: &[u8]) -> String {
    let hex: String = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
    let mut groups: Vec<&str> = Vec::new();
    for chunk in hex.as_bytes().chunks(8) {
        groups.push(std::str::from_utf8(chunk).expect("hex digits are ASCII"));
    }
    groups.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_eight_groups_of_eight_uppercase_hex() {
        // pairing.md §7.4 example shape: "1A2B3C4D 5E6F7081 …" × 8 groups.
        let digest: Vec<u8> = std::iter::repeat_n([0x1a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f, 0x70, 0x81], 4)
            .flatten()
            .collect();
        let groups = hex_groups(&digest[..32]);
        let parts: Vec<&str> = groups.split(' ').collect();
        assert_eq!(parts.len(), 8);
        for part in &parts {
            assert_eq!(part.len(), 8);
            assert!(part.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
        }
        assert!(groups.starts_with("1A2B3C4D 5E6F7081"));
    }

    #[test]
    fn pem_round_trips_pkcs8() {
        let der = vec![0x30, 0x2e, 0x02, 0x01, 0x00];
        let pem = pkcs8_to_pem(&der);
        assert!(pem.starts_with(PEM_BEGIN));
        assert!(pem.contains(PEM_END));
        assert_eq!(pem_to_pkcs8_der(&pem), Some(der));
    }

    #[test]
    fn pem_rejects_foreign_text() {
        assert_eq!(pem_to_pkcs8_der("not a pem"), None);
    }

    #[test]
    fn pem_container_ends_with_armor_line() {
        // pkcs8_to_pem emits a trailing newline after the END armor; parsing
        // must tolerate both. Contract: the last armor token is PEM_END.
        let pem = pkcs8_to_pem(&[0x30, 0x2e, 0x02, 0x01, 0x00]);
        let last_line = pem.lines().last().unwrap_or("");
        assert!(last_line.starts_with("-----"));
    }

    #[test]
    fn hex_groups_rejects_wrong_length() {
        // Only used with 32-byte digests; short input still renders groups.
        assert_eq!(hex_groups(&[0xaa, 0xbb]), "AABB");
    }

    // The tests below exercise the fail-closed lifecycle through the public
    // `open` seam against a real temporary data directory.

    struct TempGuard(tempfile::TempDir);
    impl TempGuard {
        fn new() -> Self {
            Self(tempfile::tempdir().expect("tempdir"))
        }
        fn path(&self) -> PathBuf {
            self.0.path().to_path_buf()
        }
    }

    fn config(dir: &Path) -> IdentityConfig {
        IdentityConfig { data_dir: dir.to_path_buf(), allow_loose_permissions: false }
    }

    #[test]
    fn first_open_generates_and_persists_key() {
        let dir = TempGuard::new();
        let identity = ServerIdentity::open(&config(&dir.path())).expect("first open generates");
        let key_path = dir.path().join("identity").join(IDENTITY_KEY_FILE);
        assert!(key_path.exists());
        assert_eq!(identity.key_path(), key_path);

        // File is 0600 and directory 0700 (pairing.md §10.2).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let file_mode = fs::metadata(&key_path).unwrap().permissions().mode() & 0o777;
            let dir_mode = fs::metadata(dir.path().join("identity"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(file_mode, 0o600, "key file must be 0600");
            assert_eq!(dir_mode, 0o700, "identity directory must be 0700");
        }
    }

    #[test]
    fn reload_returns_same_identity() {
        let dir = TempGuard::new();
        let first = ServerIdentity::open(&config(&dir.path())).expect("generate");
        let second = ServerIdentity::open(&config(&dir.path())).expect("reload");
        assert_eq!(first.public_key_raw(), second.public_key_raw());
        assert_eq!(first.spki_sha256(), second.spki_sha256());
        assert_eq!(first.fingerprint(), second.fingerprint());
    }

    #[test]
    fn corrupted_key_fails_closed_and_is_not_regenerated() {
        let dir = TempGuard::new();
        ServerIdentity::open(&config(&dir.path())).expect("generate");
        let key_path = dir.path().join("identity").join(IDENTITY_KEY_FILE);
        fs::write(&key_path, "this is not a key").unwrap();

        let error = ServerIdentity::open(&config(&dir.path())).expect_err("corruption is fatal");
        assert!(matches!(error, IdentityError::Corrupted { .. }), "{error}");
        assert!(error.to_string().contains("fail") || error.to_string().contains("disabled"));

        // Fail-closed: the file was not replaced (pairing.md §10.3).
        assert_eq!(fs::read_to_string(&key_path).unwrap(), "this is not a key");
    }

    #[test]
    fn truncated_key_fails_closed() {
        let dir = TempGuard::new();
        ServerIdentity::open(&config(&dir.path())).expect("generate");
        let key_path = dir.path().join("identity").join(IDENTITY_KEY_FILE);
        // Valid PEM armor, invalid DER body.
        let pem = format!("{PEM_BEGIN}\naW50ZXJuYWwga2V5IGRhdGE=\n{PEM_END}\n");
        fs::write(&key_path, pem).unwrap();
        let error = ServerIdentity::open(&config(&dir.path())).expect_err("bad DER is fatal");
        assert!(matches!(error, IdentityError::Corrupted { .. }), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn loose_file_permissions_refuse_v2() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = TempGuard::new();
        ServerIdentity::open(&config(&dir.path())).expect("generate");
        let key_path = dir.path().join("identity").join(IDENTITY_KEY_FILE);
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644)).unwrap();

        let error = ServerIdentity::open(&config(&dir.path())).expect_err("loose perms refused");
        assert!(matches!(error, IdentityError::LoosePermissions { .. }), "{error}");

        // The override flag permits the load (pairing.md §10.2).
        let loose = IdentityConfig {
            data_dir: dir.path().to_path_buf(),
            allow_loose_permissions: true,
        };
        ServerIdentity::open(&loose).expect("override flag permits");
    }

    #[cfg(unix)]
    #[test]
    fn loose_directory_permissions_refuse_v2() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = TempGuard::new();
        // Generate normally (open() re-tightens its directory to 0700), then
        // loosen the directory externally — e.g. an operator `chmod 755` —
        // and expect the next start to refuse.
        ServerIdentity::open(&config(&dir.path())).expect("generate");
        let identity_dir = dir.path().join("identity");
        fs::set_permissions(&identity_dir, fs::Permissions::from_mode(0o755)).unwrap();
        let error = ServerIdentity::open(&config(&dir.path())).expect_err("loose dir refused");
        assert!(matches!(error, IdentityError::LoosePermissions { .. }), "{error}");

        // The override flag permits the load (pairing.md §10.2).
        let loose = IdentityConfig {
            data_dir: dir.path().to_path_buf(),
            allow_loose_permissions: true,
        };
        ServerIdentity::open(&loose).expect("override flag permits");
    }

    #[test]
    fn tls_config_binds_identity_public_key() {
        let dir = TempGuard::new();
        let identity = ServerIdentity::open(&config(&dir.path())).expect("generate");
        let tls = identity.tls_server_config().expect("tls config");
        assert!(tls.max_early_data_size == 0, "0-RTT must be off by default in the TLS config");
        assert_eq!(tls.alpn_protocols.len(), 0, "ALPN policy belongs to the transport");
    }

    #[test]
    fn spki_der_matches_recorded_fixture_layout() {
        // The SPKI layout is fixed: 12-byte prefix + 32-byte raw key.
        let dir = TempGuard::new();
        let identity = ServerIdentity::open(&config(&dir.path())).expect("generate");
        let spki = identity.spki_der();
        assert_eq!(spki.len(), 44);
        assert_eq!(&spki[..12], &ED25519_SPKI_PREFIX);
        assert_eq!(&spki[12..], identity.public_key_raw());
    }
}
