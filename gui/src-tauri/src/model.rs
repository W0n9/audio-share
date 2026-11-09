//! Pure GUI model: server status, in-memory settings, and their mapping to the
//! Audio Share Core API. No Tauri types live here, so everything is unit-tested
//! without a display.

use audio_share_core::capture::{CaptureConfig, EncodingPreference};
use audio_share_core::protocol::{HEARTBEAT_INTERVAL, SESSION_TIMEOUT};
use audio_share_core::server::ServerConfig;
use serde::{Deserialize, Serialize};

/// v0.3.4's default Server port.
pub const DEFAULT_PORT: u16 = 65_530;

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

/// In-memory GUI settings for ticket 08. Config persistence arrives in ticket
/// 09; these defaults match the v0.3.4/new-install baseline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Bind host.
    pub host: String,
    /// Bind port.
    pub port: u16,
    /// Endpoint ID; `"default"` selects the system default endpoint.
    pub endpoint: String,
    /// Encoding preference: `default`, `f32`, `s8`, `s16`, `s24`, or `s32`.
    pub encoding: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: DEFAULT_PORT,
            endpoint: "default".to_owned(),
            encoding: "default".to_owned(),
        }
    }
}

impl Settings {
    /// Map the GUI settings onto a Core capture request.
    pub fn capture_config(&self) -> CaptureConfig {
        CaptureConfig {
            endpoint_id: self.endpoint.clone(),
            encoding: parse_encoding(&self.encoding),
            channels: 0,
            sample_rate: 0,
        }
    }

    /// Map the GUI settings onto a Core server configuration.
    pub fn server_config(&self) -> ServerConfig {
        ServerConfig {
            host: self.host.clone(),
            port: self.port,
            heartbeat_interval: HEARTBEAT_INTERVAL,
            heartbeat_timeout: SESSION_TIMEOUT,
        }
    }
}

/// The first usable default bind address, falling back to all interfaces.
fn default_host() -> String {
    let host = ServerConfig::default().host;
    if host.is_empty() {
        "0.0.0.0".to_owned()
    } else {
        host
    }
}

/// Parse a GUI encoding name. Unknown or empty values ask the endpoint for its
/// default format, matching the v0.3.4 `default` encoding value.
pub fn parse_encoding(value: &str) -> EncodingPreference {
    match value.trim().to_ascii_lowercase().as_str() {
        "f32" => EncodingPreference::F32,
        "s8" => EncodingPreference::S8,
        "s16" => EncodingPreference::S16,
        "s24" => EncodingPreference::S24,
        "s32" => EncodingPreference::S32,
        _ => EncodingPreference::Default,
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
    /// Current controller status.
    pub status: ServerStatus,
    /// Current in-memory settings.
    pub settings: Settings,
}

/// Build the bootstrap payload for the frontend.
pub fn bootstrap(settings: &Settings, status: &ServerStatus) -> Bootstrap {
    Bootstrap {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        platform: platform_name().to_owned(),
        status: status.clone(),
        settings: settings.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_match_the_043_baseline() {
        let settings = Settings::default();
        assert_eq!(settings.port, 65_530);
        assert_eq!(settings.endpoint, "default");
        assert_eq!(settings.encoding, "default");
        assert!(!settings.host.is_empty(), "a bind host is always offered");
    }

    #[test]
    fn encoding_names_map_to_capture_preferences() {
        assert_eq!(parse_encoding("default"), EncodingPreference::Default);
        assert_eq!(parse_encoding("f32"), EncodingPreference::F32);
        assert_eq!(parse_encoding("s8"), EncodingPreference::S8);
        assert_eq!(parse_encoding("s16"), EncodingPreference::S16);
        assert_eq!(parse_encoding("s24"), EncodingPreference::S24);
        assert_eq!(parse_encoding("s32"), EncodingPreference::S32);
        assert_eq!(parse_encoding("  F32 "), EncodingPreference::F32);
        assert_eq!(parse_encoding(""), EncodingPreference::Default);
        assert_eq!(parse_encoding("bogus"), EncodingPreference::Default);
    }

    #[test]
    fn settings_map_to_capture_config_with_auto_channels_and_rate() {
        let settings = Settings {
            host: "127.0.0.1".to_owned(),
            port: 12_345,
            endpoint: "sink-7".to_owned(),
            encoding: "s24".to_owned(),
        };
        let capture = settings.capture_config();
        assert_eq!(capture.endpoint_id, "sink-7");
        assert_eq!(capture.encoding, EncodingPreference::S24);
        assert_eq!(capture.channels, 0, "channels are endpoint-selected");
        assert_eq!(capture.sample_rate, 0, "sample rate is endpoint-selected");
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
    fn bootstrap_carries_version_platform_and_state() {
        let settings = Settings::default();
        let status = ServerStatus::starting();
        let data = bootstrap(&settings, &status);
        assert_eq!(data.version, "0.4.0");
        assert_eq!(data.platform, platform_name());
        assert_eq!(data.status, status);
        assert_eq!(data.settings, settings);
    }
}
