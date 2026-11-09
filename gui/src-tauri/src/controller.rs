//! Start/Stop controller shared by the Tauri commands and the tray menu.
//!
//! The controller owns the optional [`ServerHandle`] and the published status.
//! It is deliberately free of Tauri types: the shell installs a listener that
//! updates the tray and emits `server:state-changed`. Every lock is dropped
//! before an `.await`, so the UI thread and tray callbacks never wait on
//! capture or network startup.

use std::sync::{Arc, Mutex};

use audio_share_core::capture::CaptureBackend;
use audio_share_core::server::{Server, ServerHandle};

use crate::model::{ServerState, ServerStatus, Settings};

/// Called after every published status change, outside the controller lock.
pub type StatusListener = Arc<dyn Fn(&ServerStatus) + Send + Sync>;

struct ControllerState {
    settings: Settings,
    status: ServerStatus,
    handle: Option<ServerHandle>,
}

/// Owns the running server and the status shown in the window and tray.
pub struct ServerController {
    state: Mutex<ControllerState>,
    listener: Mutex<Option<StatusListener>>,
}

impl ServerController {
    /// Create a stopped controller with the given in-memory settings.
    pub fn new(settings: Settings) -> Self {
        Self {
            state: Mutex::new(ControllerState {
                settings,
                status: ServerStatus::stopped(),
                handle: None,
            }),
            listener: Mutex::new(None),
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

    /// The current in-memory settings snapshot.
    pub fn settings(&self) -> Settings {
        lock(&self.state).settings.clone()
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
                let mut state = lock(&self.state);
                state.handle = Some(handle);
                state.status = status.clone();
                drop(state);
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
                let status = ServerStatus::stopped();
                lock(&self.state).status = status.clone();
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
        synthetic_backend, CaptureConfig, CaptureSession, Endpoint, FrameSink,
    };
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
            encoding: "default".to_owned(),
        }
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
    fn listener_is_optional() {
        // No listener installed: publishing must not panic.
        let controller = ServerController::new(test_settings());
        controller.publish(&ServerStatus::stopped());
    }
}
