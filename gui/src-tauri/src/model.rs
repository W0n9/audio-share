//! Pure GUI model: server status, persisted settings, and their mapping to the
//! Audio Share Core API. No Tauri types live here, so everything is unit-tested
//! without a display.
//!
//! Settings are the core [`audio_share_core::config::Config`] type: the config
//! file, the GUI, and the controller all share one definition, and its serde
//! spelling (`camelCase`, lowercase enums) is the frontend contract pinned by
//! the tests below.

use std::collections::BTreeMap;

use audio_share_core::config::{
    ConfigNotice, WhenAppStart, WhenClose, CONFIG_VERSION, DEFAULT_LANGUAGE,
};
use serde::{Deserialize, Serialize};

use crate::i18n::Locale;
use crate::update::UpdateCheckResult;

pub use audio_share_core::config::{Config as Settings, EncodingSetting};

/// v0.3.4's default Server port.
pub use audio_share_core::config::DEFAULT_PORT;

/// Where the controller currently is in the Start/Stop lifecycle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerState {
    /// No server is running.
    #[default]
    Stopped,
    /// Capture/socket startup is in progress.
    Starting,
    /// The server is accepting clients.
    Running,
    /// Shutdown is in progress.
    Stopping,
}

/// A snapshot of the controller state published to the UI and tray.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    /// Current lifecycle state.
    pub state: ServerState,
    /// Actual listen address while running, e.g. `192.168.1.5:65530`.
    pub address: Option<String>,
    /// Last readable failure; cleared when a new start succeeds.
    pub last_error: Option<String>,
}

impl ServerStatus {
    /// The initial stopped state.
    pub fn stopped() -> Self {
        Self {
            state: ServerState::Stopped,
            address: None,
            last_error: None,
        }
    }

    /// A server that is starting.
    pub fn starting() -> Self {
        Self {
            state: ServerState::Starting,
            address: None,
            last_error: None,
        }
    }

    /// A running server at `address`.
    pub fn running(address: String) -> Self {
        Self {
            state: ServerState::Running,
            address: Some(address),
            last_error: None,
        }
    }

    /// A server that is stopping; keeps the previous address for display.
    pub fn stopping(address: Option<String>) -> Self {
        Self {
            state: ServerState::Stopping,
            address,
            last_error: None,
        }
    }

    /// A failed start: stopped, with a readable error.
    pub fn failed(message: String) -> Self {
        Self {
            state: ServerState::Stopped,
            address: None,
            last_error: Some(message),
        }
    }
}

/// Which tray menu commands are available in a lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrayEnabled {
    /// Whether Start may be chosen.
    pub start: bool,
    /// Whether Stop may be chosen.
    pub stop: bool,
}

/// Derive tray menu availability from the lifecycle state. Start/Stop are
/// disabled while a transition is in flight.
pub fn tray_enabled(state: ServerState) -> TrayEnabled {
    match state {
        ServerState::Stopped => TrayEnabled {
            start: true,
            stop: false,
        },
        ServerState::Running => TrayEnabled {
            start: false,
            stop: true,
        },
        ServerState::Starting | ServerState::Stopping => TrayEnabled {
            start: false,
            stop: false,
        },
    }
}

/// One-time data the frontend needs to render the shell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    /// Application version (`0.4.0`).
    pub version: String,
    /// `windows`, `linux`, or `unknown`.
    pub platform: String,
    /// Locale selected at startup (`en-US` or `zh-CN`).
    pub locale: String,
    /// Current controller status.
    pub status: ServerStatus,
    /// Persisted settings.
    pub settings: Settings,
    /// Structured migration/recovery warnings; empty when clean.
    pub config_notices: Vec<Notice>,
    /// An update already found by a background check, if any.
    pub update: Option<UpdateCheckResult>,
}

/// A localized message the frontend renders with `notice.<code>`.
///
/// Rust never sends English prose for these; the resource files own the text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    /// Message code such as `migrated` or `autostartFailed`.
    pub code: String,
    /// Placeholder values for the localized template.
    pub params: BTreeMap<String, String>,
}

impl Notice {
    /// Start a notice with no parameters.
    pub fn new(code: &str) -> Self {
        Self {
            code: code.to_owned(),
            params: BTreeMap::new(),
        }
    }

    /// Add one placeholder value.
    pub fn with(mut self, key: &str, value: impl Into<String>) -> Self {
        self.params.insert(key.to_owned(), value.into());
        self
    }
}

impl From<&ConfigNotice> for Notice {
    fn from(notice: &ConfigNotice) -> Self {
        match notice {
            ConfigNotice::Migrated { from } => Notice::new("migrated")
                .with("from", from.to_string())
                .with("to", CONFIG_VERSION.to_string()),
            ConfigNotice::FutureVersion { version } => {
                Notice::new("futureVersion").with("version", version.to_string())
            }
            ConfigNotice::Recovered => Notice::new("recovered"),
            ConfigNotice::LegacyMigrated { from } => {
                Notice::new("legacyMigrated").with("path", from.to_string_lossy())
            }
            ConfigNotice::InvalidValue { key, value } => Notice::new("invalidValue")
                .with("key", key)
                .with("value", value),
            ConfigNotice::UnknownEndpoint { endpoint } => {
                Notice::new("unknownEndpoint").with("endpoint", endpoint)
            }
        }
    }
}

/// Application-level settings that may change while the server is running.
///
/// Server fields (host/port/endpoint/encoding) stay locked while running, per
/// v0.3.4; autostart, launch/close behavior, update checks, and language are
/// independent of the running server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationSettings {
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
}

impl From<&Settings> for ApplicationSettings {
    fn from(settings: &Settings) -> Self {
        Self {
            autostart: settings.autostart,
            when_start: settings.when_start,
            when_close: settings.when_close,
            auto_update: settings.auto_update,
            language: settings.language.clone(),
        }
    }
}

impl ApplicationSettings {
    /// Check the values the UI can write.
    pub fn validate(&self) -> Result<(), String> {
        let language = self.language.trim();
        if language.is_empty()
            || language.eq_ignore_ascii_case(DEFAULT_LANGUAGE)
            || Locale::parse(language).is_some()
        {
            Ok(())
        } else {
            Err(format!(
                "unsupported language {:?}: expected default, en-US, or zh-CN",
                self.language
            ))
        }
    }
}

/// Build the bootstrap payload for the frontend.
pub fn bootstrap(
    settings: &Settings,
    status: &ServerStatus,
    notices: &[Notice],
    update: Option<UpdateCheckResult>,
    locale: Locale,
) -> Bootstrap {
    Bootstrap {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        platform: platform_name().to_owned(),
        locale: locale.tag().to_owned(),
        status: status.clone(),
        settings: settings.clone(),
        config_notices: notices.to_vec(),
        update,
    }
}

/// The current platform label used by the frontend to hide Windows-only rows.
pub const fn platform_name() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

/// Reduce a settings/config notice list to structured, localizable notices.
pub fn notice_messages(notices: &[ConfigNotice]) -> Vec<Notice> {
    notices.iter().map(Notice::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_share_core::capture::{CaptureConfig, EncodingPreference};
    use audio_share_core::protocol::{HEARTBEAT_INTERVAL, SESSION_TIMEOUT};

    #[test]
    fn default_settings_match_the_043_baseline() {
        let settings = Settings::default();
        assert_eq!(settings.port, 65_530);
        assert_eq!(settings.endpoint, "default");
        assert_eq!(settings.encoding, EncodingSetting::Default);
        assert_eq!(
            settings.when_start,
            audio_share_core::config::WhenAppStart::Nothing
        );
        assert_eq!(
            settings.when_close,
            audio_share_core::config::WhenClose::Minimize
        );
        assert!(!settings.autostart);
        assert!(!settings.auto_update);
        assert_eq!(settings.language, "default");
        assert!(!settings.running);
        assert!(!settings.host.is_empty(), "a bind host is always offered");
    }

    #[test]
    fn settings_map_to_capture_config_with_auto_channels_and_rate() {
        let settings = Settings {
            host: "127.0.0.1".to_owned(),
            port: 12_345,
            endpoint: "sink-7".to_owned(),
            encoding: EncodingSetting::S24,
            ..Settings::default()
        };
        let capture = settings.capture_config();
        assert_eq!(
            capture,
            CaptureConfig {
                endpoint_id: "sink-7".to_owned(),
                encoding: EncodingPreference::S24,
                channels: 0,
                sample_rate: 0,
            }
        );
    }

    #[test]
    fn every_encoding_maps_to_the_matching_capture_preference() {
        for (setting, preference) in [
            (EncodingSetting::Default, EncodingPreference::Default),
            (EncodingSetting::F32, EncodingPreference::F32),
            (EncodingSetting::S8, EncodingPreference::S8),
            (EncodingSetting::S16, EncodingPreference::S16),
            (EncodingSetting::S24, EncodingPreference::S24),
            (EncodingSetting::S32, EncodingPreference::S32),
        ] {
            assert_eq!(setting.preference(), preference);
        }
    }

    #[test]
    fn settings_map_to_server_config() {
        let settings = Settings {
            host: "127.0.0.1".to_owned(),
            port: 12_345,
            ..Settings::default()
        };
        let server = settings.server_config();
        assert_eq!(server.host, "127.0.0.1");
        assert_eq!(server.port, 12_345);
        assert_eq!(server.heartbeat_interval, HEARTBEAT_INTERVAL);
        assert_eq!(server.heartbeat_timeout, SESSION_TIMEOUT);
    }

    #[test]
    fn settings_json_uses_the_frontend_field_names() {
        let settings = Settings {
            host: "127.0.0.1".to_owned(),
            port: 65_000,
            endpoint: "sink-3".to_owned(),
            encoding: EncodingSetting::S16,
            autostart: true,
            when_start: audio_share_core::config::WhenAppStart::KeepLast,
            when_close: audio_share_core::config::WhenClose::Exit,
            auto_update: true,
            language: "zh-CN".to_owned(),
            running: true,
        };
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(json["host"], "127.0.0.1");
        assert_eq!(json["port"], 65_000);
        assert_eq!(json["endpoint"], "sink-3");
        assert_eq!(json["encoding"], "s16");
        assert_eq!(json["autostart"], true);
        assert_eq!(json["whenStart"], "keepLast");
        assert_eq!(json["whenClose"], "exit");
        assert_eq!(json["autoUpdate"], true);
        assert_eq!(json["language"], "zh-CN");
        assert_eq!(json["running"], true);

        let round_trip: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(round_trip, settings);
    }

    #[test]
    fn status_json_uses_the_frontend_field_names() {
        let running = ServerStatus::running("10.0.0.2:65530".to_owned());
        let json = serde_json::to_value(&running).unwrap();
        assert_eq!(json["state"], "running");
        assert_eq!(json["address"], "10.0.0.2:65530");
        assert!(json["lastError"].is_null());

        let failed = ServerStatus::failed("bind failed".to_owned());
        let json = serde_json::to_value(&failed).unwrap();
        assert_eq!(json["state"], "stopped");
        assert_eq!(json["lastError"], "bind failed");
    }

    #[test]
    fn tray_commands_follow_the_lifecycle_state() {
        assert_eq!(
            tray_enabled(ServerState::Stopped),
            TrayEnabled {
                start: true,
                stop: false
            }
        );
        assert_eq!(
            tray_enabled(ServerState::Starting),
            TrayEnabled {
                start: false,
                stop: false
            }
        );
        assert_eq!(
            tray_enabled(ServerState::Running),
            TrayEnabled {
                start: false,
                stop: true
            }
        );
        assert_eq!(
            tray_enabled(ServerState::Stopping),
            TrayEnabled {
                start: false,
                stop: false
            }
        );
    }

    #[test]
    fn bootstrap_carries_version_platform_locale_state_settings_and_notices() {
        let settings = Settings::default();
        let status = ServerStatus::starting();
        let notices = notice_messages(&[
            ConfigNotice::Migrated { from: 1 },
            ConfigNotice::UnknownEndpoint {
                endpoint: "gone".to_owned(),
            },
        ]);
        let update = Some(crate::update::UpdateCheckResult::Newer {
            version: "v0.4.1".to_owned(),
            url: "https://example.test/v0.4.1".to_owned(),
        });
        let data = bootstrap(&settings, &status, &notices, update.clone(), Locale::ZhCn);
        assert_eq!(data.version, "0.4.0");
        assert_eq!(data.platform, platform_name());
        assert_eq!(data.locale, "zh-CN");
        assert_eq!(data.status, status);
        assert_eq!(data.settings, settings);
        assert_eq!(data.update, update);
        assert_eq!(data.config_notices.len(), 2);
        assert_eq!(data.config_notices[0].code, "migrated");
        assert_eq!(data.config_notices[0].params["from"], "1");
        assert_eq!(data.config_notices[0].params["to"], "2");
        assert_eq!(data.config_notices[1].code, "unknownEndpoint");
        assert_eq!(data.config_notices[1].params["endpoint"], "gone");

        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(json["locale"], "zh-CN");
        assert_eq!(json["configNotices"][0]["code"], "migrated");
        assert_eq!(json["configNotices"][0]["params"]["from"], "1");
        assert_eq!(json["update"]["status"], "newer");
    }

    #[test]
    fn an_empty_notice_list_means_no_banner() {
        let data = bootstrap(
            &Settings::default(),
            &ServerStatus::stopped(),
            &[],
            None,
            Locale::EnUs,
        );
        assert!(data.config_notices.is_empty());
        assert!(data.update.is_none());
        assert!(notice_messages(&[]).is_empty());
    }

    #[test]
    fn every_config_notice_maps_to_a_structured_code() {
        let notices = notice_messages(&[
            ConfigNotice::Migrated { from: 0 },
            ConfigNotice::FutureVersion { version: 3 },
            ConfigNotice::Recovered,
            ConfigNotice::LegacyMigrated {
                from: "/old/config.ini".into(),
            },
            ConfigNotice::InvalidValue {
                key: "port".to_owned(),
                value: "x".to_owned(),
            },
            ConfigNotice::UnknownEndpoint {
                endpoint: "gone".to_owned(),
            },
        ]);
        let codes: Vec<&str> = notices.iter().map(|notice| notice.code.as_str()).collect();
        assert_eq!(
            codes,
            vec![
                "migrated",
                "futureVersion",
                "recovered",
                "legacyMigrated",
                "invalidValue",
                "unknownEndpoint"
            ]
        );
        assert_eq!(notices[3].params["path"], "/old/config.ini");
        assert_eq!(notices[4].params["key"], "port");
        assert_eq!(notices[4].params["value"], "x");
    }

    #[test]
    fn application_settings_validate_supported_languages() {
        let base = ApplicationSettings::from(&Settings::default());
        assert!(base.validate().is_ok());
        for language in ["default", "en-US", "zh-CN", "EN-us"] {
            assert!(
                ApplicationSettings {
                    language: language.to_owned(),
                    ..base.clone()
                }
                .validate()
                .is_ok(),
                "{language} must be accepted"
            );
        }
        let error = ApplicationSettings {
            language: "fr-FR".to_owned(),
            ..base.clone()
        }
        .validate()
        .expect_err("unknown language must be rejected");
        assert!(error.contains("language"), "error was {error}");
    }

    #[test]
    fn application_settings_json_uses_the_frontend_contract() {
        let settings = Settings {
            autostart: true,
            when_start: WhenAppStart::KeepLast,
            when_close: WhenClose::Exit,
            auto_update: true,
            language: "zh-CN".to_owned(),
            ..Settings::default()
        };
        let json = serde_json::to_value(ApplicationSettings::from(&settings)).unwrap();
        assert_eq!(json["autostart"], true);
        assert_eq!(json["whenStart"], "keepLast");
        assert_eq!(json["whenClose"], "exit");
        assert_eq!(json["autoUpdate"], true);
        assert_eq!(json["language"], "zh-CN");
    }
}
