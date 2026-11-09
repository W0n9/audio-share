//! Start/Stop controller shared by the Tauri commands and the tray menu.
//!
//! The controller owns the optional [`ServerHandle`], the persisted settings,
//! and the published status. It is deliberately free of Tauri types: the shell
//! installs a listener that updates the tray and emits `server:state-changed`.
//! Every lock is dropped before an `.await`, so the UI thread and tray
//! callbacks never wait on capture or network startup.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use audio_share_core::capture::{CaptureBackend, Endpoint};
use audio_share_core::config::{self, DEFAULT_ENDPOINT};
use audio_share_core::server::{Server, ServerHandle};

use crate::model::{ApplicationSettings, ServerState, ServerStatus, Settings};

/// Called after every published status change, outside the controller lock.
pub type StatusListener = Arc<dyn Fn(&ServerStatus) + Send + Sync>;

struct ControllerState {
    settings: Settings,
    status: ServerStatus,
    handle: Option<ServerHandle>,
}

/// Owns the running server, the persisted settings, and the displayed status.
pub struct ServerController {
    state: Mutex<ControllerState>,
    listener: Mutex<Option<StatusListener>>,
    config_path: Option<PathBuf>,
}

impl ServerController {
    /// Create a stopped controller whose settings are not persisted.
    pub fn new(settings: Settings) -> Self {
        Self::build(settings, None)
    }

    /// Create a stopped controller that writes settings to `config_path`.
    pub fn with_config(settings: Settings, config_path: PathBuf) -> Self {
        Self::build(settings, Some(config_path))
    }

    fn build(settings: Settings, config_path: Option<PathBuf>) -> Self {
        Self {
            state: Mutex::new(ControllerState {
                settings,
                status: ServerStatus::stopped(),
                handle: None,
            }),
            listener: Mutex::new(None),
            config_path,
        }
    }

    /// Install (or replace) the status listener used by the Tauri shell.
    pub fn set_listener(&self, listener: StatusListener) {
        *lock(&self.listener) = Some(listener);
    }

    /// The current status snapshot.
    pub fn status(&self) -> ServerStatus {
        lock(&self.state).status.clone()
    }

    /// The current settings snapshot.
    pub fn settings(&self) -> Settings {
        lock(&self.state).settings.clone()
    }

    /// The config file written by this controller, when persistence is enabled.
    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    /// Persist new settings while the server is stopped.
    ///
    /// `[App] Running` is owned by the controller (successful start/stop), so a
    /// stale value from the UI never overwrites it. The settings are validated
    /// and written atomically before they become active.
    pub fn save_settings(&self, settings: Settings) -> Result<Settings, String> {
        self.save_settings_with_endpoints(settings, None)
    }

    /// Like [`Self::save_settings`], additionally rejecting an endpoint ID that
    /// is not in the live backend list (MFC parity; `default` is always valid).
    pub fn save_settings_with_endpoints(
        &self,
        mut settings: Settings,
        endpoints: Option<&[Endpoint]>,
    ) -> Result<Settings, String> {
        let mut state = lock(&self.state);
        if state.status.state != ServerState::Stopped {
            return Err("stop the server before changing settings".to_owned());
        }
        settings.running = state.settings.running;
        settings.validate().map_err(|error| error.to_string())?;
        if let Some(endpoints) = endpoints {
            let known = settings.endpoint == DEFAULT_ENDPOINT
                || endpoints
                    .iter()
                    .any(|endpoint| endpoint.id == settings.endpoint);
            if !known {
                return Err(format!(
                    "audio endpoint {:?} is not available",
                    settings.endpoint
                ));
            }
        }
        self.persist(&settings)?;
        state.settings = settings.clone();
        Ok(settings)
    }

    /// Persist application-level settings without touching Server fields.
    ///
    /// v0.3.4 keeps autostart, launch/close behavior, update checks, and
    /// language editable while the server runs, so this deliberately does not
    /// require the stopped state. `[App] Running` and the Server fields come
    /// from the controller, never from the caller.
    pub fn save_application_settings(
        &self,
        application: ApplicationSettings,
    ) -> Result<Settings, String> {
        application.validate()?;
        let mut state = lock(&self.state);
        let mut settings = state.settings.clone();
        settings.autostart = application.autostart;
        settings.when_start = application.when_start;
        settings.when_close = application.when_close;
        settings.auto_update = application.auto_update;
        settings.language = application.language.trim().to_owned();
        self.persist(&settings)?;
        state.settings = settings.clone();
        Ok(settings)
    }

    /// Restore the Server-field factory defaults and write them.
    ///
    /// This mirrors the MFC Reset button: host/port/endpoint/encoding return to
    /// their defaults; application settings (autostart, launch/close behavior,
    /// update checks, language) are not touched.
    pub fn reset_settings(&self) -> Result<Settings, String> {
        let mut state = lock(&self.state);
        if state.status.state != ServerState::Stopped {
            return Err("stop the server before resetting settings".to_owned());
        }
        let current = &state.settings;
        let reset = Settings {
            autostart: current.autostart,
            when_start: current.when_start,
            when_close: current.when_close,
            auto_update: current.auto_update,
            language: current.language.clone(),
            running: current.running,
            ..Settings::default()
        };
        self.persist(&reset)?;
        state.settings = reset.clone();
        Ok(reset)
    }

    /// Start the server with `backend`, publishing every state transition.
    ///
    /// The state lock is held only to validate the transition, then to install
    /// the handle; capture negotiation and socket binding run unlocked.
    pub async fn start(&self, backend: Arc<dyn CaptureBackend>) -> Result<ServerStatus, String> {
        let (server_config, capture_config, starting) = {
            let mut state = lock(&self.state);
            match state.status.state {
                ServerState::Stopped => {}
                ServerState::Starting => {
                    return Err("server is already starting".to_owned());
                }
                ServerState::Running => {
                    return Err("server is already running".to_owned());
                }
                ServerState::Stopping => {
                    return Err("server is still stopping".to_owned());
                }
            }
            let settings = state.settings.clone();
            state.status = ServerStatus::starting();
            (
                settings.server_config(),
                settings.capture_config(),
                state.status.clone(),
            )
        };
        self.publish(&starting);

        match Server::start(server_config, backend, capture_config).await {
            Ok(handle) => {
                let status = ServerStatus::running(handle.local_addr().to_string());
                let persisted = {
                    let mut state = lock(&self.state);
                    state.handle = Some(handle);
                    state.status = status.clone();
                    state.settings.running = true;
                    state.settings.clone()
                };
                self.persist_best_effort(&persisted);
                self.publish(&status);
                Ok(status)
            }
            Err(error) => {
                let message = error.to_string();
                let status = ServerStatus::failed(message.clone());
                lock(&self.state).status = status.clone();
                self.publish(&status);
                Err(message)
            }
        }
    }

    /// Stop the running server, publishing Stopping and then Stopped.
    pub async fn stop(&self) -> Result<ServerStatus, String> {
        let (handle, stopping) = {
            let mut state = lock(&self.state);
            match state.status.state {
                ServerState::Running => {}
                ServerState::Starting => {
                    return Err("server is still starting; wait for it to finish".to_owned());
                }
                ServerState::Stopping => {
                    return Err("server is already stopping".to_owned());
                }
                ServerState::Stopped => {
                    return Err("server is not running".to_owned());
                }
            }
            let Some(handle) = state.handle.take() else {
                let status = ServerStatus::failed("server handle was missing".to_owned());
                state.status = status.clone();
                drop(state);
                self.publish(&status);
                return Err("server is not running".to_owned());
            };
            state.status = ServerStatus::stopping(state.status.address.clone());
            (handle, state.status.clone())
        };
        self.publish(&stopping);

        match handle.stop().await {
            Ok(()) => {
                let persisted = {
                    let mut state = lock(&self.state);
                    state.status = ServerStatus::stopped();
                    state.settings.running = false;
                    state.settings.clone()
                };
                self.persist_best_effort(&persisted);
                let status = ServerStatus::stopped();
                self.publish(&status);
                Ok(status)
            }
            Err(error) => {
                let message = error.to_string();
                let status = ServerStatus::failed(message.clone());
                lock(&self.state).status = status.clone();
                self.publish(&status);
                Err(message)
            }
        }
    }

    /// Write `settings` to the config file when persistence is enabled.
    fn persist(&self, settings: &Settings) -> Result<(), String> {
        match &self.config_path {
            Some(path) => config::save(path, settings)
                .map_err(|error| format!("cannot save settings: {error}")),
            None => Ok(()),
        }
    }

    /// Start/stop succeeded; a persistence failure must not change that result.
    fn persist_best_effort(&self, settings: &Settings) {
        if let Err(error) = self.persist(settings) {
            eprintln!("AudioShareServer: {error}");
        }
    }

    /// Publish `status` to the installed listener, if any.
    fn publish(&self, status: &ServerStatus) {
        let listener = lock(&self.listener).clone();
        if let Some(listener) = listener {
            listener(status);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::Duration;

    use audio_share_core::capture::{
        synthetic_backend, CaptureConfig, CaptureSession, Endpoint, FrameSink, SyntheticCapture,
    };
    use audio_share_core::config::{load, EncodingSetting, LoadOptions};
    use audio_share_core::protocol::{encode_command, Command};
    use audio_share_core::Error;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    use super::*;

    fn test_settings() -> Settings {
        Settings {
            host: "127.0.0.1".to_owned(),
            port: 0,
            endpoint: "default".to_owned(),
            ..Settings::default()
        }
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("bind a free port")
            .local_addr()
            .expect("local address")
            .port()
    }

    fn persisted_settings() -> Settings {
        Settings {
            host: "127.0.0.1".to_owned(),
            port: free_port(),
            endpoint: "default".to_owned(),
            ..Settings::default()
        }
    }

    fn load_config(path: &Path) -> Settings {
        load(path, LoadOptions::default())
            .expect("load persisted config")
            .config
    }

    fn synthetic() -> Arc<dyn CaptureBackend> {
        Arc::new(synthetic_backend().expect("synthetic backend"))
    }

    fn recording_controller() -> (Arc<ServerController>, Arc<Mutex<Vec<ServerState>>>) {
        let controller = Arc::new(ServerController::new(test_settings()));
        let states = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&states);
        controller.set_listener(Arc::new(move |status| {
            recorded.lock().unwrap().push(status.state);
        }));
        (controller, states)
    }

    #[test]
    fn starts_stopped_with_the_configured_settings() {
        let controller = ServerController::new(test_settings());
        assert_eq!(controller.status(), ServerStatus::stopped());
        assert_eq!(controller.settings(), test_settings());
    }

    #[test]
    fn save_settings_updates_the_controller_and_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ini");
        let controller = ServerController::with_config(test_settings(), path.clone());
        let updated = Settings {
            host: "127.0.0.1".to_owned(),
            port: 12_345,
            endpoint: "default".to_owned(),
            encoding: EncodingSetting::S16,
            ..Settings::default()
        };

        let saved = controller
            .save_settings(updated.clone())
            .expect("save settings");

        assert_eq!(saved, updated);
        assert_eq!(controller.settings(), updated);
        let on_disk = load_config(&path);
        assert_eq!(on_disk.port, 12_345);
        assert_eq!(on_disk.encoding, EncodingSetting::S16);
        assert!(
            !on_disk.running,
            "saving settings must not claim the server runs"
        );
    }

    #[test]
    fn save_settings_is_refused_while_the_server_is_not_stopped() {
        let controller = ServerController::new(test_settings());
        lock(&controller.state).status = ServerStatus::running("127.0.0.1:1".to_owned());

        let error = controller
            .save_settings(Settings::default())
            .expect_err("running controllers must refuse settings changes");

        assert!(error.contains("stop the server"), "error was {error}");
    }

    #[test]
    fn save_settings_rejects_unusable_values_without_touching_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ini");
        let controller = ServerController::with_config(test_settings(), path.clone());
        controller
            .save_settings(Settings {
                port: 12_345,
                ..test_settings()
            })
            .unwrap();
        let before = std::fs::read(&path).unwrap();

        for (candidate, expected) in [
            (
                Settings {
                    host: "  ".to_owned(),
                    ..test_settings()
                },
                "host",
            ),
            (
                Settings {
                    port: 0,
                    ..test_settings()
                },
                "port",
            ),
        ] {
            let error = controller
                .save_settings(candidate)
                .expect_err("invalid settings must be refused");
            assert!(error.contains(expected), "error was {error}");
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
    }

    #[test]
    fn save_settings_rejects_an_unknown_endpoint_when_a_list_is_available() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ini");
        let controller = ServerController::with_config(test_settings(), path.clone());
        let endpoints = vec![Endpoint {
            id: "sink-1".to_owned(),
            name: "Sink One".to_owned(),
        }];

        let error = controller
            .save_settings_with_endpoints(
                Settings {
                    port: 12_345,
                    endpoint: "gone".to_owned(),
                    ..test_settings()
                },
                Some(&endpoints),
            )
            .expect_err("unknown endpoint must be refused");
        assert!(error.contains("gone"), "error was {error}");

        let accepted = controller
            .save_settings_with_endpoints(
                Settings {
                    port: 12_345,
                    endpoint: "sink-1".to_owned(),
                    ..test_settings()
                },
                Some(&endpoints),
            )
            .expect("a known endpoint is accepted");
        assert_eq!(accepted.endpoint, "sink-1");
        assert_eq!(load_config(&path).endpoint, "sink-1");
    }

    #[test]
    fn reset_restores_server_defaults_and_preserves_application_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ini");
        let controller = ServerController::with_config(test_settings(), path.clone());
        controller
            .save_settings(Settings {
                host: "127.0.0.1".to_owned(),
                port: 12_345,
                endpoint: "default".to_owned(),
                encoding: EncodingSetting::S32,
                autostart: true,
                when_start: audio_share_core::config::WhenAppStart::Start,
                when_close: audio_share_core::config::WhenClose::Exit,
                auto_update: true,
                language: "zh-CN".to_owned(),
                running: false,
            })
            .unwrap();

        let reset = controller.reset_settings().expect("reset settings");

        let defaults = Settings::default();
        assert_eq!(reset.host, defaults.host);
        assert_eq!(reset.port, 65_530);
        assert_eq!(reset.endpoint, "default");
        assert_eq!(reset.encoding, EncodingSetting::Default);
        assert!(reset.autostart);
        assert_eq!(
            reset.when_start,
            audio_share_core::config::WhenAppStart::Start
        );
        assert_eq!(reset.when_close, audio_share_core::config::WhenClose::Exit);
        assert!(reset.auto_update);
        assert_eq!(reset.language, "zh-CN");
        assert_eq!(controller.settings(), reset);

        let on_disk = load_config(&path);
        assert_eq!(on_disk.port, 65_530);
        assert_eq!(on_disk.encoding, EncodingSetting::Default);
        assert!(on_disk.autostart);
    }

    #[tokio::test]
    async fn successful_start_and_stop_persist_the_running_flag() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ini");
        let settings = persisted_settings();
        let controller = ServerController::with_config(settings, path.clone());

        controller.start(synthetic()).await.expect("start");
        assert!(
            load_config(&path).running,
            "a successful start must write Running=1"
        );

        controller.stop().await.expect("stop");
        assert!(
            !load_config(&path).running,
            "a successful stop must write Running=0"
        );
    }

    #[tokio::test]
    async fn a_failed_start_does_not_persist_running() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ini");
        let controller = ServerController::with_config(persisted_settings(), path.clone());

        controller
            .start(Arc::new(FailingCapture) as Arc<dyn CaptureBackend>)
            .await
            .expect_err("start must fail");

        assert!(!load_config(&path).running);
    }

    struct RecordingCapture {
        inner: SyntheticCapture,
        seen: Arc<Mutex<Vec<CaptureConfig>>>,
    }

    impl RecordingCapture {
        fn new() -> Self {
            Self {
                inner: synthetic_backend().expect("synthetic backend"),
                seen: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl CaptureBackend for RecordingCapture {
        fn endpoints(&self) -> audio_share_core::Result<Vec<Endpoint>> {
            self.inner.endpoints()
        }

        fn default_endpoint(&self) -> audio_share_core::Result<Option<Endpoint>> {
            self.inner.default_endpoint()
        }

        fn start(
            &self,
            config: &CaptureConfig,
            sink: FrameSink,
        ) -> audio_share_core::Result<CaptureSession> {
            self.seen.lock().unwrap().push(config.clone());
            self.inner.start(config, sink)
        }
    }

    #[tokio::test]
    async fn start_uses_the_settings_saved_while_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ini");
        let controller = ServerController::with_config(test_settings(), path.clone());
        let port = free_port();
        let updated = Settings {
            host: "127.0.0.1".to_owned(),
            port,
            endpoint: "synthetic".to_owned(),
            encoding: EncodingSetting::S24,
            ..Settings::default()
        };
        let endpoints = vec![Endpoint {
            id: "synthetic".to_owned(),
            name: "Synthetic Capture".to_owned(),
        }];
        controller
            .save_settings_with_endpoints(updated, Some(&endpoints))
            .expect("save settings");

        let recorder = Arc::new(RecordingCapture::new());
        let status = controller
            .start(Arc::clone(&recorder) as Arc<dyn CaptureBackend>)
            .await
            .expect("start with saved settings");

        assert_eq!(
            status.address.as_deref(),
            Some(format!("127.0.0.1:{port}").as_str())
        );
        {
            let seen = recorder.seen.lock().unwrap();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].endpoint_id, "synthetic");
            assert_eq!(
                seen[0].encoding,
                audio_share_core::capture::EncodingPreference::S24
            );
        }
        controller.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn start_then_stop_publishes_running_address_and_stopped() {
        let (controller, states) = recording_controller();
        let running = controller.start(synthetic()).await.unwrap();
        assert_eq!(running.state, ServerState::Running);
        let address = running.address.clone().expect("running address");
        assert!(
            address.starts_with("127.0.0.1:"),
            "unexpected address {address}"
        );
        assert_eq!(controller.status(), running);

        let stopped = controller.stop().await.unwrap();
        assert_eq!(stopped, ServerStatus::stopped());
        assert_eq!(
            *states.lock().unwrap(),
            vec![
                ServerState::Starting,
                ServerState::Running,
                ServerState::Stopping,
                ServerState::Stopped,
            ]
        );
    }

    #[tokio::test]
    async fn the_running_server_answers_get_format_and_closes_on_stop() {
        let controller = ServerController::new(test_settings());
        let running = controller.start(synthetic()).await.unwrap();
        let address: SocketAddr = running.address.unwrap().parse().unwrap();

        let mut client = TcpStream::connect(address).await.unwrap();
        client
            .write_all(&encode_command(Command::GetFormat))
            .await
            .unwrap();
        let mut header = [0u8; 8];
        client.read_exact(&mut header).await.unwrap();
        assert_eq!(
            u32::from_le_bytes(header[0..4].try_into().unwrap()),
            Command::GetFormat as u32
        );
        let size = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let mut payload = vec![0u8; size as usize];
        client.read_exact(&mut payload).await.unwrap();

        controller.stop().await.unwrap();

        let closed = tokio::time::timeout(Duration::from_secs(2), client.read(&mut [0u8; 1]))
            .await
            .expect("connection must close after stop");
        assert!(
            matches!(closed, Ok(0) | Err(_)),
            "expected EOF or reset, got {closed:?}"
        );
    }

    #[tokio::test]
    async fn a_second_start_while_running_is_rejected_without_stopping_the_server() {
        let controller = ServerController::new(test_settings());
        let running = controller.start(synthetic()).await.unwrap();
        let error = controller.start(synthetic()).await.unwrap_err();
        assert!(error.contains("already running"), "error was {error}");
        assert_eq!(controller.status(), running);
        controller.stop().await.unwrap();
    }

    #[tokio::test]
    async fn stop_without_a_server_is_rejected_and_keeps_the_stopped_state() {
        let controller = ServerController::new(test_settings());
        let error = controller.stop().await.unwrap_err();
        assert!(error.contains("not running"), "error was {error}");
        assert_eq!(controller.status(), ServerStatus::stopped());
    }

    struct FailingCapture;

    impl CaptureBackend for FailingCapture {
        fn endpoints(&self) -> audio_share_core::Result<Vec<Endpoint>> {
            Ok(Vec::new())
        }

        fn default_endpoint(&self) -> audio_share_core::Result<Option<Endpoint>> {
            Ok(None)
        }

        fn start(
            &self,
            _config: &CaptureConfig,
            _sink: FrameSink,
        ) -> audio_share_core::Result<CaptureSession> {
            Err(Error::Capture("no capture device".to_owned()))
        }
    }

    #[tokio::test]
    async fn a_failed_start_publishes_a_readable_error_and_recovers() {
        let (controller, states) = recording_controller();
        let error = controller
            .start(Arc::new(FailingCapture) as Arc<dyn CaptureBackend>)
            .await
            .unwrap_err();
        assert!(error.contains("no capture device"), "error was {error}");
        let status = controller.status();
        assert_eq!(status.state, ServerState::Stopped);
        assert_eq!(
            status.last_error.as_deref(),
            Some("capture failed: no capture device")
        );
        assert_eq!(
            *states.lock().unwrap(),
            vec![ServerState::Starting, ServerState::Stopped]
        );

        // The controller must accept a retry after a failed start.
        let running = controller.start(synthetic()).await.unwrap();
        assert_eq!(running.state, ServerState::Running);
        assert_eq!(running.last_error, None);
        controller.stop().await.unwrap();
    }

    #[tokio::test]
    async fn stop_during_start_is_rejected_readably() {
        // This exercises the state guard directly: the controller is Starting
        // and has no handle yet, so Stop must not block on a missing handle.
        let controller = ServerController::new(test_settings());
        lock(&controller.state).status = ServerStatus::starting();
        let error = controller.stop().await.unwrap_err();
        assert!(error.contains("starting"), "error was {error}");
        assert_eq!(controller.status().state, ServerState::Starting);
    }

    #[test]
    fn application_settings_can_change_while_running_and_preserve_server_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ini");
        let settings = persisted_settings();
        let controller = ServerController::with_config(settings.clone(), path.clone());
        lock(&controller.state).status = ServerStatus::running("127.0.0.1:1".to_owned());
        lock(&controller.state).settings.running = true;

        let updated = controller
            .save_application_settings(ApplicationSettings {
                autostart: true,
                when_start: audio_share_core::config::WhenAppStart::KeepLast,
                when_close: audio_share_core::config::WhenClose::Exit,
                auto_update: true,
                language: "zh-CN".to_owned(),
            })
            .expect("application settings save while running");

        assert_eq!(updated.host, "127.0.0.1");
        assert_eq!(updated.port, settings.port);
        assert_eq!(updated.endpoint, "default");
        assert!(updated.running, "controller-owned Running must survive");
        assert!(updated.autostart);
        assert_eq!(
            updated.when_start,
            audio_share_core::config::WhenAppStart::KeepLast
        );
        assert_eq!(
            updated.when_close,
            audio_share_core::config::WhenClose::Exit
        );
        assert!(updated.auto_update);
        assert_eq!(updated.language, "zh-CN");
        assert_eq!(controller.status().state, ServerState::Running);

        let on_disk = load_config(&path);
        assert_eq!(on_disk.language, "zh-CN");
        assert!(on_disk.autostart);
        assert_eq!(
            on_disk.when_close,
            audio_share_core::config::WhenClose::Exit
        );
    }

    #[test]
    fn application_settings_reject_an_unsupported_language_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ini");
        let controller = ServerController::with_config(persisted_settings(), path.clone());
        controller.save_settings(persisted_settings()).unwrap();
        let before = std::fs::read(&path).unwrap();

        let error = controller
            .save_application_settings(ApplicationSettings {
                language: "fr-FR".to_owned(),
                ..ApplicationSettings::from(&test_settings())
            })
            .expect_err("unknown language must be refused");
        assert!(error.contains("language"), "error was {error}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(controller.settings().language, "default");
    }

    #[test]
    fn listener_is_optional() {
        // No listener installed: publishing must not panic.
        let controller = ServerController::new(test_settings());
        controller.publish(&ServerStatus::stopped());
    }
}
