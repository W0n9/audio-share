//! `config.ini` parsing, migration, and atomic persistence.
//!
//! The Server GUI keeps the v0.3.4 file format so an older build can still read
//! a config written by 0.4.0. Resolution order for the file itself lives in
//! [`crate::paths`]. This module owns the Win32-compatible INI semantics:
//! section/key names are matched case-insensitively, values are trimmed, and
//! surrounding quotes are stripped. Unknown sections, keys, comments, and
//! spelling are preserved on rewrite.
//!
//! Version 2 is the rewrite's format. Missing/legacy versions are migrated in
//! place after backing up the damaged/future file, and writes are atomic:
//! same-directory temporary file, fsync, then rename/replace.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::capture::{CaptureConfig, EncodingPreference, Endpoint};
use crate::error::{Error, Result};
use crate::protocol::{HEARTBEAT_INTERVAL, SESSION_TIMEOUT};
use crate::server::{default_host, ServerConfig};

/// Current `[App] configVersion` written by this implementation.
pub const CONFIG_VERSION: i32 = 2;

/// v0.3.4's default Server port.
pub const DEFAULT_PORT: u16 = 65_530;

/// The value that selects the system default audio endpoint.
pub const DEFAULT_ENDPOINT: &str = "default";

/// The value that follows the system locale.
pub const DEFAULT_LANGUAGE: &str = "default";

/// Legacy `[Capture] encoding` numbers.
///
/// The numbers are frozen for downgrade compatibility: 0 = default,
/// 1 = invalid/unused, 2 = f32, 3 = s8, 4 = s16, 5 = s24, 6 = s32.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EncodingSetting {
    /// Let the endpoint choose (legacy 0).
    #[default]
    Default,
    /// 32-bit floating-point PCM (legacy 2).
    F32,
    /// 8-bit integer PCM (legacy 3).
    S8,
    /// 16-bit integer PCM (legacy 4).
    S16,
    /// 24-bit integer PCM (legacy 5).
    S24,
    /// 32-bit integer PCM (legacy 6).
    S32,
}

impl EncodingSetting {
    /// Parse a legacy integer; `None` for the unused `1` and out-of-range values.
    pub const fn from_legacy(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Default),
            2 => Some(Self::F32),
            3 => Some(Self::S8),
            4 => Some(Self::S16),
            5 => Some(Self::S24),
            6 => Some(Self::S32),
            _ => None,
        }
    }

    /// The legacy integer written to `config.ini`.
    pub const fn legacy_value(self) -> i32 {
        match self {
            Self::Default => 0,
            Self::F32 => 2,
            Self::S8 => 3,
            Self::S16 => 4,
            Self::S24 => 5,
            Self::S32 => 6,
        }
    }

    /// Parse a user-facing encoding name (`default`, `f32`, `s8`, ...).
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "default" | "" => Some(Self::Default),
            "f32" => Some(Self::F32),
            "s8" => Some(Self::S8),
            "s16" => Some(Self::S16),
            "s24" => Some(Self::S24),
            "s32" => Some(Self::S32),
            _ => None,
        }
    }

    /// The stable user-facing name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::F32 => "f32",
            Self::S8 => "s8",
            Self::S16 => "s16",
            Self::S24 => "s24",
            Self::S32 => "s32",
        }
    }

    /// The capture-backend preference this setting requests.
    pub const fn preference(self) -> EncodingPreference {
        match self {
            Self::Default => EncodingPreference::Default,
            Self::F32 => EncodingPreference::F32,
            Self::S8 => EncodingPreference::S8,
            Self::S16 => EncodingPreference::S16,
            Self::S24 => EncodingPreference::S24,
            Self::S32 => EncodingPreference::S32,
        }
    }
}

impl std::fmt::Display for EncodingSetting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// `[App Settings] WhenAppStart` values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WhenAppStart {
    /// Do not start the server at launch (legacy 0).
    #[default]
    Nothing,
    /// Start the server at launch (legacy 1).
    Start,
    /// Start only when `[App] Running` was true (legacy 2).
    KeepLast,
}

impl WhenAppStart {
    /// Parse a legacy integer; unknown values mean "do nothing" (MFC parity).
    pub const fn from_legacy(value: i32) -> Self {
        match value {
            1 => Self::Start,
            2 => Self::KeepLast,
            _ => Self::Nothing,
        }
    }

    /// The legacy integer written to `config.ini`.
    pub const fn legacy_value(self) -> i32 {
        match self {
            Self::Nothing => 0,
            Self::Start => 1,
            Self::KeepLast => 2,
        }
    }

    /// Whether launch should start the server, given the persisted
    /// `[App] Running` flag from the previous session.
    pub const fn starts_server(self, last_running: bool) -> bool {
        match self {
            Self::Nothing => false,
            Self::Start => true,
            Self::KeepLast => last_running,
        }
    }
}

/// `[App Settings] WhenClose` values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WhenClose {
    /// Exit the application (legacy 0).
    Exit,
    /// Hide the window and keep the server running (legacy non-zero).
    #[default]
    Minimize,
}

impl WhenClose {
    /// Parse a legacy integer: `0` exits, any non-zero value minimizes.
    pub const fn from_legacy(value: i32) -> Self {
        if value == 0 {
            Self::Exit
        } else {
            Self::Minimize
        }
    }

    /// The legacy integer written to `config.ini`.
    pub const fn legacy_value(self) -> i32 {
        match self {
            Self::Exit => 0,
            Self::Minimize => 1,
        }
    }

    /// Whether a window close request should stop the core and exit.
    pub const fn exits_on_close(self) -> bool {
        matches!(self, Self::Exit)
    }
}

/// All persisted Server settings.
///
/// Field names and JSON spelling are the GUI's data contract; `camelCase`
/// converts `when_start` to `whenStart`, matching `gui/src/lib/ipc.ts`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Bind host. Defaults to the first usable local address.
    pub host: String,
    /// Bind port. Defaults to 65530.
    pub port: u16,
    /// Endpoint ID, or `"default"` for the system default.
    pub endpoint: String,
    /// Requested capture encoding.
    pub encoding: EncodingSetting,
    /// Start at login.
    pub autostart: bool,
    /// Server behavior at application launch.
    pub when_start: WhenAppStart,
    /// Window-close behavior.
    pub when_close: WhenClose,
    /// Periodic update checks.
    pub auto_update: bool,
    /// `default`, `en-US`, or `zh-CN`.
    pub language: String,
    /// Persisted last server state for `WhenAppStart=keepLast`.
    pub running: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: default_host()
                .map(|address| address.to_string())
                .unwrap_or_else(|| "0.0.0.0".to_owned()),
            port: DEFAULT_PORT,
            endpoint: DEFAULT_ENDPOINT.to_owned(),
            encoding: EncodingSetting::Default,
            autostart: false,
            when_start: WhenAppStart::Nothing,
            when_close: WhenClose::Minimize,
            auto_update: false,
            language: DEFAULT_LANGUAGE.to_owned(),
            running: false,
        }
    }
}

impl Config {
    /// Check the values that must be usable before starting or persisting.
    pub fn validate(&self) -> Result<()> {
        if self.host.trim().is_empty() {
            return Err(Error::InvalidConfig("host must not be empty".to_owned()));
        }
        if self.port == 0 {
            return Err(Error::InvalidConfig(
                "port must be between 1 and 65535".to_owned(),
            ));
        }
        if self.endpoint.trim().is_empty() {
            return Err(Error::InvalidConfig(
                "endpoint must not be empty".to_owned(),
            ));
        }
        Ok(())
    }

    /// Map these settings onto a Core capture request.
    pub fn capture_config(&self) -> CaptureConfig {
        CaptureConfig {
            endpoint_id: self.endpoint.clone(),
            encoding: self.encoding.preference(),
            channels: 0,
            sample_rate: 0,
        }
    }

    /// Map these settings onto a Core server configuration.
    pub fn server_config(&self) -> ServerConfig {
        ServerConfig {
            host: self.host.clone(),
            port: self.port,
            heartbeat_interval: HEARTBEAT_INTERVAL,
            heartbeat_timeout: SESSION_TIMEOUT,
        }
    }
}

/// An observable outcome of loading or migrating a config file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigNotice {
    /// A legacy file was upgraded to [`CONFIG_VERSION`].
    Migrated {
        /// The version found on disk (0 when the key was missing).
        from: i32,
    },
    /// The file was written by a newer build; it was read but not overwritten.
    FutureVersion {
        /// The version found on disk.
        version: i32,
    },
    /// The file was unreadable; defaults were restored from a backup.
    Recovered,
    /// Values were copied from a legacy executable-directory config.
    LegacyMigrated {
        /// The read-only legacy file.
        from: PathBuf,
    },
    /// A recognized key had an unusable value and the default was used.
    InvalidValue {
        /// The key name, e.g. `port`.
        key: String,
        /// The raw value found in the file.
        value: String,
    },
    /// The configured audio endpoint is no longer available.
    UnknownEndpoint {
        /// The endpoint ID found in the file.
        endpoint: String,
    },
}

impl std::fmt::Display for ConfigNotice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Migrated { from } => write!(
                f,
                "Configuration migrated from version {from} to {CONFIG_VERSION}."
            ),
            Self::FutureVersion { version } => write!(
                f,
                "Configuration version {version} is newer than this application; \
                 settings were read without being rewritten and a backup was kept."
            ),
            Self::Recovered => f.write_str(
                "The configuration file was damaged; defaults were restored and a backup \
                 was saved as config.ini.bak.",
            ),
            Self::LegacyMigrated { from } => write!(
                f,
                "Settings were migrated from the read-only location {}.",
                from.display()
            ),
            Self::InvalidValue { key, value } => write!(
                f,
                "The {key} value {value:?} is invalid; the default was used."
            ),
            Self::UnknownEndpoint { endpoint } => write!(
                f,
                "Audio endpoint {endpoint:?} is no longer available; the default endpoint \
                 was selected."
            ),
        }
    }
}

/// Optional inputs for [`load`].
#[derive(Clone, Copy, Debug, Default)]
pub struct LoadOptions<'a> {
    /// A legacy executable-directory config to migrate when `path` is absent.
    pub legacy: Option<&'a Path>,
    /// Live endpoints used to validate `[Capture] endpoint`; `None` skips the
    /// check (for example when enumeration itself failed).
    pub endpoints: Option<&'a [Endpoint]>,
}

/// The loaded settings plus any migration/recovery information for the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadOutcome {
    /// Effective settings.
    pub config: Config,
    /// Non-empty when the user should be informed about a migration or recovery.
    pub notices: Vec<ConfigNotice>,
}

/// The backup path used for a damaged or future-version config file.
pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "config.ini".into());
    name.push(".bak");
    path.with_file_name(name)
}

/// Load `path`, migrating v0.3.4 values and reporting every correction.
///
/// A fresh file is created with defaults. A file with no `configVersion` or
/// version 1 is migrated to version 2. A version above 2 is read without being
/// rewritten (a backup is kept). Damaged files are copied to `config.ini.bak`
/// and replaced with defaults.
pub fn load(path: &Path, options: LoadOptions<'_>) -> Result<LoadOutcome> {
    let mut notices = Vec::new();
    let mut source_path = path.to_path_buf();
    let mut from_legacy = false;

    let bytes = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match options.legacy {
            Some(legacy) if legacy != path => match fs::read(legacy) {
                Ok(bytes) => {
                    source_path = legacy.to_path_buf();
                    from_legacy = true;
                    Some(bytes)
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(Error::io("read legacy config", error));
                }
            },
            _ => None,
        },
        Err(error) => return Err(Error::io("read config", error)),
    };

    if from_legacy {
        notices.push(ConfigNotice::LegacyMigrated {
            from: source_path.clone(),
        });
    }

    let Some(bytes) = bytes else {
        // Fresh install: write a canonical version-2 file with defaults.
        let config = Config::default();
        let mut document = IniDocument::default();
        apply_config(&mut document, &config);
        write_document(path, &document)?;
        return Ok(LoadOutcome { config, notices });
    };

    let Some(text) = decode_ini(&bytes) else {
        return recover(path, &bytes, notices);
    };
    let document = IniDocument::parse(&text);

    let version = match document.get("App", "configVersion") {
        None => 0,
        Some(raw) => match raw.parse::<i32>() {
            Ok(version) => version,
            Err(_) => return recover(path, &bytes, notices),
        },
    };

    let (config, corrected) = recognized_config(&document, options.endpoints, &mut notices);

    if version > CONFIG_VERSION {
        notices.push(ConfigNotice::FutureVersion { version });
        backup_bytes(path, &bytes)?;
        if from_legacy {
            // Preserve the legacy values at the writable location without
            // rewriting them; the next version of the app will understand them.
            atomic_write(path, &bytes)?;
        }
        return Ok(LoadOutcome { config, notices });
    }

    if version < CONFIG_VERSION {
        notices.push(ConfigNotice::Migrated { from: version });
    }

    let needs_write = version < CONFIG_VERSION || corrected || from_legacy;
    if needs_write {
        let mut document = document;
        apply_config(&mut document, &config);
        write_document(path, &document)?;
    }

    Ok(LoadOutcome { config, notices })
}

/// Persist `config` atomically, preserving unknown sections, keys, and comments
/// from an existing file.
pub fn save(path: &Path, config: &Config) -> Result<()> {
    config.validate()?;
    let text = fs::read(path)
        .ok()
        .and_then(|bytes| decode_ini(&bytes))
        .unwrap_or_default();
    let mut document = IniDocument::parse(&text);
    apply_config(&mut document, config);
    write_document(path, &document)
}

fn recover(path: &Path, bytes: &[u8], notices: Vec<ConfigNotice>) -> Result<LoadOutcome> {
    let mut notices = notices;
    notices.push(ConfigNotice::Recovered);
    backup_bytes(path, bytes)?;
    let config = Config::default();
    let mut document = IniDocument::default();
    apply_config(&mut document, &config);
    write_document(path, &document)?;
    Ok(LoadOutcome { config, notices })
}

/// Read every recognized key, applying defaults and recording corrections.
fn recognized_config(
    document: &IniDocument,
    endpoints: Option<&[Endpoint]>,
    notices: &mut Vec<ConfigNotice>,
) -> (Config, bool) {
    let mut config = Config::default();
    let mut corrected = false;

    if let Some(value) = document.get("Network", "host") {
        if value.trim().is_empty() {
            notices.push(ConfigNotice::InvalidValue {
                key: "host".to_owned(),
                value: value.to_owned(),
            });
            corrected = true;
        } else {
            config.host = value.to_owned();
        }
    }

    if let Some(value) = document.get("Network", "port") {
        match value.parse::<u16>() {
            Ok(port) if port != 0 => config.port = port,
            _ => {
                notices.push(ConfigNotice::InvalidValue {
                    key: "port".to_owned(),
                    value: value.to_owned(),
                });
                corrected = true;
            }
        }
    }

    if let Some(value) = document.get("Capture", "endpoint") {
        if value.trim().is_empty() {
            notices.push(ConfigNotice::InvalidValue {
                key: "endpoint".to_owned(),
                value: value.to_owned(),
            });
            corrected = true;
        } else {
            config.endpoint = value.to_owned();
        }
    }

    if let Some(value) = document.get("Capture", "encoding") {
        match value
            .parse::<i32>()
            .ok()
            .and_then(EncodingSetting::from_legacy)
        {
            Some(encoding) => config.encoding = encoding,
            None => {
                notices.push(ConfigNotice::InvalidValue {
                    key: "encoding".to_owned(),
                    value: value.to_owned(),
                });
                corrected = true;
            }
        }
    }

    if let Some(value) = document.get("App", "language") {
        if !value.trim().is_empty() {
            config.language = value.to_owned();
        }
    }
    if let Some(value) = document.get("App", "Running") {
        config.running = parse_int(value).is_some_and(|value| value != 0);
    }
    if let Some(value) = document.get("App Settings", "AutoRun") {
        config.autostart = parse_int(value).is_some_and(|value| value != 0);
    }
    if let Some(value) = document.get("App Settings", "WhenAppStart") {
        config.when_start = WhenAppStart::from_legacy(parse_int(value).unwrap_or(0));
    }
    if let Some(value) = document.get("App Settings", "WhenClose") {
        // GetProfileInt returns 0 for unparseable values, and 0 means exit.
        config.when_close = WhenClose::from_legacy(parse_int(value).unwrap_or(0));
    }
    if let Some(value) = document.get("App Settings", "AutoUpdate") {
        config.auto_update = parse_int(value).is_some_and(|value| value != 0);
    }

    if let Some(endpoints) = endpoints {
        let known = config.endpoint == DEFAULT_ENDPOINT
            || endpoints
                .iter()
                .any(|endpoint| endpoint.id == config.endpoint);
        if !known {
            notices.push(ConfigNotice::UnknownEndpoint {
                endpoint: config.endpoint.clone(),
            });
            config.endpoint = DEFAULT_ENDPOINT.to_owned();
            corrected = true;
        }
    }

    (config, corrected)
}

fn parse_int(value: &str) -> Option<i32> {
    value.trim().parse::<i32>().ok()
}

fn apply_config(document: &mut IniDocument, config: &Config) {
    document.set("App", "configVersion", &CONFIG_VERSION.to_string());
    document.set("App", "language", &config.language);
    document.set("App", "Running", bool_int(config.running));
    document.set("Network", "host", &config.host);
    document.set("Network", "port", &config.port.to_string());
    document.set("Capture", "endpoint", &config.endpoint);
    document.set(
        "Capture",
        "encoding",
        &config.encoding.legacy_value().to_string(),
    );
    document.set("App Settings", "AutoRun", bool_int(config.autostart));
    document.set(
        "App Settings",
        "WhenAppStart",
        &config.when_start.legacy_value().to_string(),
    );
    document.set(
        "App Settings",
        "WhenClose",
        &config.when_close.legacy_value().to_string(),
    );
    document.set("App Settings", "AutoUpdate", bool_int(config.auto_update));
}

fn bool_int(value: bool) -> &'static str {
    if value {
        "1"
    } else {
        "0"
    }
}

/// Decode an INI byte string. `None` means the file is damaged.
fn decode_ini(bytes: &[u8]) -> Option<String> {
    let text = if let Some(body) = bytes.strip_prefix(&[0xff, 0xfe]) {
        if body.len() % 2 != 0 {
            return None;
        }
        let units: Vec<u16> = body
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        let body = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        String::from_utf8_lossy(body).into_owned()
    };
    if text.contains('\0') {
        None
    } else {
        Some(text)
    }
}

#[derive(Clone, Debug)]
enum IniLine {
    Raw(String),
    Section {
        name: String,
        raw: String,
    },
    Property {
        section: Option<String>,
        key: String,
        raw: String,
    },
}

/// A line-preserving INI editor.
///
/// Only recognized keys are rewritten; every other line is emitted verbatim so
/// comments, unknown keys, ordering, and key spelling survive.
#[derive(Clone, Debug, Default)]
struct IniDocument {
    lines: Vec<IniLine>,
}

impl IniDocument {
    fn parse(text: &str) -> Self {
        let mut lines = Vec::new();
        let mut section: Option<String> = None;
        for line in text.lines() {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let trimmed = line.trim();
            if let Some(name) = trimmed
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            {
                let name = name.trim().to_owned();
                section = Some(name.clone());
                lines.push(IniLine::Section {
                    name,
                    raw: line.to_owned(),
                });
            } else if !trimmed.is_empty()
                && !trimmed.starts_with(';')
                && !trimmed.starts_with('#')
                && trimmed.contains('=')
            {
                let (key, _) = line.split_once('=').expect("checked above");
                lines.push(IniLine::Property {
                    section: section.clone(),
                    key: key.trim().to_owned(),
                    raw: line.to_owned(),
                });
            } else {
                lines.push(IniLine::Raw(line.to_owned()));
            }
        }
        Self { lines }
    }

    fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.lines.iter().find_map(|line| match line {
            IniLine::Property {
                section: Some(found_section),
                key: found_key,
                raw,
            } if found_section.eq_ignore_ascii_case(section)
                && found_key.eq_ignore_ascii_case(key) =>
            {
                Some(property_value(raw))
            }
            _ => None,
        })
    }

    fn set(&mut self, section: &str, key: &str, value: &str) {
        if let Some(index) = self.lines.iter().position(|line| match line {
            IniLine::Property {
                section: Some(found_section),
                key: found_key,
                ..
            } => found_section.eq_ignore_ascii_case(section) && found_key.eq_ignore_ascii_case(key),
            _ => false,
        }) {
            if let IniLine::Property { raw, .. } = &self.lines[index] {
                if property_value(raw) == value {
                    return;
                }
            }
            let original_key = match &self.lines[index] {
                IniLine::Property { key, .. } => key.clone(),
                _ => unreachable!("position matched a property"),
            };
            self.lines[index] = IniLine::Property {
                section: Some(section.to_owned()),
                key: original_key.clone(),
                raw: format!("{original_key}={value}"),
            };
            return;
        }

        let section_index = self.lines.iter().position(|line| match line {
            IniLine::Section { name, .. } => name.eq_ignore_ascii_case(section),
            _ => false,
        });
        let insert_at = match section_index {
            Some(start) => {
                let mut end = start + 1;
                while end < self.lines.len() && !matches!(self.lines[end], IniLine::Section { .. })
                {
                    end += 1;
                }
                end
            }
            None => {
                if self
                    .lines
                    .last()
                    .is_some_and(|line| !matches!(line, IniLine::Raw(raw) if raw.is_empty()))
                {
                    self.lines.push(IniLine::Raw(String::new()));
                }
                self.lines.push(IniLine::Section {
                    name: section.to_owned(),
                    raw: format!("[{section}]"),
                });
                self.lines.len()
            }
        };
        self.lines.insert(
            insert_at,
            IniLine::Property {
                section: Some(section.to_owned()),
                key: key.to_owned(),
                raw: format!("{key}={value}"),
            },
        );
    }

    fn render(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            match line {
                IniLine::Raw(raw) => out.push_str(raw),
                IniLine::Section { raw, .. } => out.push_str(raw),
                IniLine::Property { raw, .. } => out.push_str(raw),
            }
            out.push('\n');
        }
        out
    }
}

/// The Win32-style value: text after the first `=`, trimmed, quotes stripped.
fn property_value(raw: &str) -> &str {
    let (_, value) = raw.split_once('=').unwrap_or(("", raw));
    strip_quotes(value.trim())
}

fn strip_quotes(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn write_document(path: &Path, document: &IniDocument) -> Result<()> {
    atomic_write(path, document.render().as_bytes())
}

fn backup_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_write(&backup_path(path), bytes)
}

/// Write `bytes` via a same-directory temporary file, fsync, then replace.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| Error::io("create config directory", error))?;

    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config.ini".to_owned());
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".{file_name}.tmp-{}-{sequence}",
        std::process::id()
    ));

    let write_result = (|| -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace_file(&temporary, path)?;
        sync_directory(parent)?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result.map_err(|error| Error::io("write config atomically", error))
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let source = wide(source);
    let destination = wide(destination);
    match unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } {
        Ok(()) => Ok(()),
        Err(_) => Err(io::Error::last_os_error()),
    }
}

#[cfg(unix)]
fn sync_directory(directory: &Path) -> io::Result<()> {
    fs::File::open(directory)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_directory: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_document_is_case_insensitive_and_preserves_unknown_lines() {
        let mut document = IniDocument::parse(
            "[NETWORK]\nHOST = \"10.0.0.1\" \n; keep me\nport=1\n\n[Other]\nX=y\n",
        );
        assert_eq!(document.get("network", "host"), Some("10.0.0.1"));
        assert_eq!(document.get("Network", "port"), Some("1"));

        document.set("network", "host", "10.0.0.2");
        document.set("network", "port", "1"); // unchanged: raw line stays verbatim

        let rendered = document.render();
        assert!(rendered.contains("HOST=10.0.0.2"));
        assert!(rendered.contains("; keep me"));
        assert!(rendered.contains("port=1"));
        assert!(rendered.contains("[Other]\nX=y"));
    }

    #[test]
    fn new_keys_are_appended_to_their_section_in_order() {
        let mut document = IniDocument::parse("[App]\nconfigVersion=2\n");
        document.set("App", "language", "default");
        document.set("Network", "host", "127.0.0.1");
        document.set("Network", "port", "65530");

        let rendered = document.render();
        assert_eq!(
            rendered,
            "[App]\nconfigVersion=2\nlanguage=default\n\n[Network]\nhost=127.0.0.1\nport=65530\n"
        );
    }

    #[test]
    fn a_corrupt_utf16_boundary_is_rejected() {
        assert!(decode_ini(&[0xff, 0xfe, 0x41]).is_none());
        assert!(decode_ini(b"[App]\nconfigVersion=2\n\0").is_none());
        assert_eq!(decode_ini(&[0xef, 0xbb, 0xbf, b'A']).as_deref(), Some("A"));
    }

    #[test]
    fn backup_name_appends_bak_to_the_full_file_name() {
        assert_eq!(
            backup_path(Path::new("/tmp/config.ini")),
            PathBuf::from("/tmp/config.ini.bak")
        );
        assert_eq!(
            backup_path(Path::new("/tmp/custom-config.ini")),
            PathBuf::from("/tmp/custom-config.ini.bak")
        );
    }
}
