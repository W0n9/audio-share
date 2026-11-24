//! Start/Stop controller shared by the Tauri commands and the tray menu.
//!
//! The controller owns the optional [`ServerHandle`], the persisted settings,
//! and the published status. It is deliberately free of Tauri types: the shell
//! installs a listener that updates the tray and emits `server:state-changed`.
//! Every lock is dropped before an `.await`, so the UI thread and tray
//! callbacks never wait on capture or network startup.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use audio_share_core::capture::{CaptureBackend, Endpoint};
use audio_share_core::config::{self, DEFAULT_ENDPOINT};
use audio_share_core::server::{Server, ServerHandle};
use v2_runtime::admin::{
    ClientStateView, ClientView, InvitationStateView, PairingAdmin, PendingRequestView,
    PresentedRequestView,
};

use crate::logging;
use crate::model::{
    ApplicationSettings, ClientInfo, PairingInvitation, PairingInvitationState, PairingLiveInfo,
    PairingRequestInfo, PairingStatus, ServerState, ServerStatus, Settings, V2Status,
};
use crate::v2::{PairingModeArg, V2Runtime, V2StartInputs, V2Starter, DEFAULT_V2_PORT};

/// Called after every published status change, outside the controller lock.
pub type StatusListener = Arc<dyn Fn(&ServerStatus) + Send + Sync>;

#[derive(Clone)]
struct ActiveInvitation {
    mode: String,
    qr_uri: Option<String>,
    qr_svg: Option<String>,
    fingerprint: String,
    expires_at: Instant,
}

struct ControllerState {
    settings: Settings,
    status: ServerStatus,
    handle: Option<ServerHandle>,
    /// The shared v2 runtime, present only while v1 is running and v2 started.
    v2: Option<Arc<dyn V2Runtime>>,
    /// The Pairing Core adapter owned by the running v2 runtime.
    v2_admin: Option<Arc<dyn PairingAdmin>>,
    /// The invitation created by the last `pairing_begin`; nonces expire on
    /// their own, closing the dialog does not consume them (pairing.md §5).
    invitation: Option<ActiveInvitation>,
    /// Set when an automatic refresh failed; cleared by a manual begin/cancel.
    auto_renew_blocked: bool,
    /// Guards against two concurrent polls renewing the same invitation.
    auto_renew_in_flight: bool,
}

/// Owns the running server, the persisted settings, and the displayed status.
pub struct ServerController {
    state: Mutex<ControllerState>,
    listener: Mutex<Option<StatusListener>>,
    config_path: Option<PathBuf>,
    /// Installed by the Tauri shell; tests inject a fake stater.
    v2_starter: Mutex<Option<V2Starter>>,
    v2_port: Mutex<u16>,
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

    /// Create a controller that also starts the shared v2 runtime by default.
    pub fn with_config_and_v2(
        settings: Settings,
        config_path: PathBuf,
        starter: V2Starter,
        v2_port: u16,
    ) -> Self {
        let controller = Self::with_config(settings, config_path);
        controller.enable_v2(starter, v2_port);
        controller
    }

    /// Install the v2 starter and port. Called before the first start; tests
    /// use this to inject a fake runtime.
    pub fn enable_v2(&self, starter: V2Starter, v2_port: u16) {
        *lock(&self.v2_starter) = Some(starter);
        *lock(&self.v2_port) = v2_port;
    }

    fn build(settings: Settings, config_path: Option<PathBuf>) -> Self {
        Self {
            state: Mutex::new(ControllerState {
                settings,
                status: ServerStatus::stopped(),
                handle: None,
                v2: None,
                v2_admin: None,
                invitation: None,
                auto_renew_blocked: false,
                auto_renew_in_flight: false,
            }),
            listener: Mutex::new(None),
            config_path,
            v2_starter: Mutex::new(None),
            v2_port: Mutex::new(DEFAULT_V2_PORT),
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
        settings.host = normalize_gui_host(&settings.host)?;
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
    /// require the stopped state. The GUI theme rides along as an application
    /// setting. `[App] Running` and the Server fields come from the
    /// controller, never from the caller.
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
        settings.theme = application.theme;
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
            theme: current.theme,
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
        let (server_config, capture_config, configured_host, starting) = {
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
                settings.host.clone(),
                state.status.clone(),
            )
        };
        logging::info(format!(
            "server starting: {}:{}",
            server_config.host, server_config.port
        ));
        self.publish(&starting);

        match Server::start(server_config, backend, capture_config).await {
            Ok(handle) => {
                // v1 is up. Start the shared v2 runtime on the same capture.
                // A v2 failure is reported independently and never stops v1.
                let starter = lock(&self.v2_starter).clone();
                let (v2_status, v2_runtime, v2_admin) = match starter {
                    None => (V2Status::disabled(), None, None),
                    Some(starter) => {
                        let data_dir = self
                            .config_path
                            .as_ref()
                            .map(|path| v2_runtime::data_dir_for_config(path))
                            .unwrap_or_else(|| PathBuf::from("."));
                        let inputs = V2StartInputs {
                            data_dir,
                            allow_loose_permissions: false,
                            port: *lock(&self.v2_port),
                            host: configured_host.clone(),
                            capture: handle.capture_observer(),
                            media_tap: handle.media_tap().clone(),
                        };
                        match starter(inputs).await {
                            Ok(runtime) => {
                                let admin = runtime.admin();
                                logging::info(format!(
                                    "v2 runtime running on {} (fingerprint {})",
                                    runtime.local_addr(),
                                    admin.fingerprint()
                                ));
                                (
                                    V2Status::running(
                                        runtime.local_addr().to_string(),
                                        admin.fingerprint(),
                                    ),
                                    Some(runtime),
                                    Some(admin),
                                )
                            }
                            Err(message) => {
                                logging::error(format!(
                                    "v2 runtime failed to start (v1 stays running): {message}"
                                ));
                                (V2Status::failed(message), None, None)
                            }
                        }
                    }
                };

                let mut status = ServerStatus::running(handle.local_addr().to_string());
                status.v2 = v2_status;
                logging::info(format!(
                    "server running on {}",
                    status.address.as_deref().unwrap_or("unknown")
                ));
                let persisted = {
                    let mut state = lock(&self.state);
                    state.handle = Some(handle);
                    state.v2 = v2_runtime;
                    state.v2_admin = v2_admin;
                    state.invitation = None;
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
                logging::error(format!("server start failed: {message}"));
                let status = ServerStatus::failed(message.clone());
                lock(&self.state).status = status.clone();
                self.publish(&status);
                Err(message)
            }
        }
    }

    /// Stop the running server, publishing Stopping and then Stopped.
    ///
    /// v2 is stopped first (closing the QUIC endpoint and any pairing dialog),
    /// then v1; the reverse order would leave v2 accepting on a server the UI
    /// already reports as stopping.
    pub async fn stop(&self) -> Result<ServerStatus, String> {
        let (handle, v2, admin, stopping) = {
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
            let v2 = state.v2.take();
            let admin = state.v2_admin.take();
            state.invitation = None;
            let mut stopping = ServerStatus::stopping(state.status.address.clone());
            stopping.v2 = state.status.v2.clone();
            state.status = stopping.clone();
            (handle, v2, admin, stopping)
        };
        logging::info("server stopping");
        if let Some(admin) = &admin {
            admin.cancel();
        }
        if let Some(v2) = v2 {
            v2.stop();
            logging::info("v2 listener stopped before v1");
        }
        self.publish(&stopping);

        match handle.stop().await {
            Ok(()) => {
                logging::info("server stopped");
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
                logging::error(format!("server stop failed: {message}"));
                let status = ServerStatus::failed(message.clone());
                lock(&self.state).status = status.clone();
                self.publish(&status);
                Err(message)
            }
        }
    }

    /// The pairing dialog's polled state: invitation, pending requests, and
    /// the independent v2 status. Always succeeds; when v2 is unavailable the
    /// `v2` block explains why and the dialog stays inert.
    pub fn pairing_status(&self) -> PairingStatus {
        let now = Instant::now();
        let (v2, admin, active) = {
            let state = lock(&self.state);
            (
                state.status.v2.clone(),
                state.v2_admin.clone(),
                state.invitation.clone(),
            )
        };
        let invitation = active.map(|active| {
            let state = match active.mode.as_str() {
                "qr" => match admin.as_ref().and_then(|admin| admin.invitation_state()) {
                    Some(InvitationStateView::Active) => PairingInvitationState::Active,
                    Some(InvitationStateView::Consumed) => PairingInvitationState::Consumed,
                    // Expired, or the core no longer holds this invitation
                    // (for example a core restart): the QR is not usable.
                    Some(InvitationStateView::Expired) | None => PairingInvitationState::Expired,
                },
                _ if active.expires_at > now => PairingInvitationState::Active,
                _ => PairingInvitationState::Expired,
            };
            PairingInvitation {
                expires_in_secs: if state == PairingInvitationState::Active {
                    active.expires_at.saturating_duration_since(now).as_secs()
                } else {
                    0
                },
                mode: active.mode,
                qr_uri: active.qr_uri,
                qr_svg: active.qr_svg,
                fingerprint: active.fingerprint,
                state,
            }
        });
        let pending = admin
            .as_ref()
            .map(|admin| admin.pending())
            .unwrap_or_default()
            .into_iter()
            .map(pending_info)
            .collect::<Vec<_>>();
        let live = admin
            .as_ref()
            .map(|admin| admin.presented())
            .unwrap_or_default()
            .into_iter()
            .map(live_info)
            .collect::<Vec<_>>();
        let invitation_active = invitation
            .as_ref()
            .is_some_and(|invitation| invitation.state == PairingInvitationState::Active);
        PairingStatus {
            active: invitation_active || !live.is_empty() || !pending.is_empty(),
            invitation,
            live,
            pending,
            v2,
        }
    }

    /// Poll the pairing dialog and renew a naturally expired QR invitation.
    ///
    /// The store calls this only while the dialog is open. A consumed
    /// invitation is never renewed automatically: a failed attempt needs an
    /// explicit refresh (§5). A failed automatic renewal is not retried until
    /// the operator refreshes or cancels.
    pub fn pairing_poll(&self) -> Result<PairingStatus, String> {
        let status = self.pairing_status();
        let (admin, mode) = {
            let state = lock(&self.state);
            (
                state.v2_admin.clone(),
                state
                    .invitation
                    .as_ref()
                    .map(|invitation| invitation.mode.clone()),
            )
        };
        // Renewal is driven by the core's natural-expiry state, never by the
        // locally truncated countdown and never when the core has no state
        // (for example after a core restart): that is not a natural expiry.
        let core_expired = mode.as_deref() == Some("qr")
            && admin.as_ref().and_then(|admin| admin.invitation_state())
                == Some(InvitationStateView::Expired);
        let should_renew = {
            let mut state = lock(&self.state);
            let should = core_expired
                && status.live.is_empty()
                && status.pending.is_empty()
                && !state.auto_renew_blocked
                && !state.auto_renew_in_flight;
            if should {
                state.auto_renew_in_flight = true;
            }
            should
        };
        if !should_renew {
            let mut state = lock(&self.state);
            if !status
                .invitation
                .as_ref()
                .is_some_and(|invitation| invitation.state == PairingInvitationState::Expired)
            {
                state.auto_renew_blocked = false;
            }
            return Ok(status);
        }
        let result = self.pairing_begin(PairingModeArg::Qr);
        let mut state = lock(&self.state);
        state.auto_renew_in_flight = false;
        state.auto_renew_blocked = result.is_err();
        drop(state);
        result
    }

    /// Begin a pairing invitation from the GUI dialog.
    pub fn pairing_begin(&self, mode: PairingModeArg) -> Result<PairingStatus, String> {
        let admin = self.pairing_admin()?;
        let view = admin
            .begin(mode.into())
            .map_err(|error| format!("cannot begin pairing: {error}"))?;
        let expires_at = Instant::now() + Duration::from_secs(view.expires_in_secs.max(1));
        {
            let mut state = lock(&self.state);
            state.invitation = Some(ActiveInvitation {
                mode: mode_name(view.mode),
                qr_uri: view.qr_uri,
                qr_svg: view.qr_svg,
                fingerprint: view.fingerprint,
                expires_at,
            });
            state.auto_renew_blocked = false;
        }
        Ok(self.pairing_status())
    }

    /// Close the pairing dialog's invitation and pending approvals.
    ///
    /// Per pairing.md §5, outstanding QR nonces are *not* consumed by closing
    /// the dialog; they expire on their own. The core's cancel closes the SAS
    /// admission window and declines every undecided approval, so a waiting
    /// Client receives 201+Close(201) immediately.
    pub fn pairing_cancel(&self) -> Result<PairingStatus, String> {
        let admin = self.pairing_admin()?;
        admin.cancel();
        {
            let mut state = lock(&self.state);
            state.invitation = None;
            state.auto_renew_blocked = false;
        }
        Ok(self.pairing_status())
    }

    /// Approve one pending pairing by request id.
    pub fn pairing_approve(&self, approval_id: u64) -> Result<PairingStatus, String> {
        let admin = self.pairing_admin()?;
        if !admin.approve(approval_id) {
            return Err("that pairing request is no longer pending".to_owned());
        }
        Ok(self.pairing_status())
    }

    /// Approve one pending pairing with the explicit Restore decision (ADR-0006).
    pub fn pairing_restore(&self, approval_id: u64) -> Result<PairingStatus, String> {
        let admin = self.pairing_admin()?;
        if !admin.restore(approval_id) {
            return Err("that pairing request is no longer pending".to_owned());
        }
        Ok(self.pairing_status())
    }

    /// Decline one pending pairing by request id.
    pub fn pairing_decline(&self, approval_id: u64) -> Result<PairingStatus, String> {
        let admin = self.pairing_admin()?;
        if !admin.decline(approval_id) {
            return Err("that pairing request is no longer pending".to_owned());
        }
        Ok(self.pairing_status())
    }

    /// The paired-device list (active and revoked records).
    pub fn clients_list(&self) -> Result<Vec<ClientInfo>, String> {
        let admin = self.pairing_admin()?;
        Ok(admin.clients().into_iter().map(client_info).collect())
    }

    /// Revoke one Client. The driver's authorization poll closes an affected
    /// active session within `REVOCATION_POLL_INTERVAL` (pairing.md §9.2).
    pub fn clients_revoke(&self, client_id: &str) -> Result<ClientInfo, String> {
        let admin = self.pairing_admin()?;
        admin
            .revoke(client_id)
            .map(client_info)
            .map_err(|error| format!("cannot revoke client: {error}"))
    }

    /// Permanently delete one **revoked** Client record (ADR-0006).
    ///
    /// An active record is refused by the core so a single click cannot both
    /// terminate a session and erase the audit row.
    pub fn clients_delete(&self, client_id: &str) -> Result<ClientInfo, String> {
        let admin = self.pairing_admin()?;
        admin
            .delete(client_id)
            .map(client_info)
            .map_err(|error| format!("cannot delete client: {error}"))
    }

    /// The running Pairing Core adapter, or a readable reason it is absent.
    fn pairing_admin(&self) -> Result<Arc<dyn PairingAdmin>, String> {
        let state = lock(&self.state);
        if state.status.state != ServerState::Running {
            return Err("start the server before pairing devices".to_owned());
        }
        match &state.v2_admin {
            Some(admin) => Ok(Arc::clone(admin)),
            None => Err(match &state.status.v2.last_error {
                Some(error) => format!("v2 is not available: {error}"),
                None => "v2 is disabled".to_owned(),
            }),
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
            logging::error(&error);
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

fn mode_name(mode: v2_runtime::admin::RequestedPairingMode) -> String {
    match mode {
        v2_runtime::admin::RequestedPairingMode::Qr => "qr".to_owned(),
        v2_runtime::admin::RequestedPairingMode::Sas => "sas".to_owned(),
    }
}

fn pending_info(request: PendingRequestView) -> PairingRequestInfo {
    PairingRequestInfo {
        approval_id: request.approval_id,
        client_id: request.client_id,
        display_name: request.display_name,
        mode: mode_name(request.mode),
        address: request.address,
        sas: request.sas,
        restore_candidate: request.restore_candidate,
    }
}

fn live_info(request: PresentedRequestView) -> PairingLiveInfo {
    PairingLiveInfo {
        approval_id: request.approval_id,
        client_id: request.client_id,
        display_name: request.display_name,
        mode: mode_name(request.mode),
        address: request.address,
        sas: request.sas,
        fingerprint: request.fingerprint,
        expires_in_secs: request.expires_in_secs,
        restore_candidate: request.restore_candidate,
    }
}

fn client_info(client: ClientView) -> ClientInfo {
    ClientInfo {
        client_id: client.client_id,
        display_name: client.display_name,
        state: match client.state {
            ClientStateView::Active => "active".to_owned(),
            ClientStateView::Revoked => "revoked".to_owned(),
        },
        created_at: client.created_at,
        last_seen_at: client.last_seen_at,
    }
}

/// Trim and validate a Host entered in the GUI, returning the value to store.
///
/// Only the GUI save path calls this. A hand-edited `config.ini` may still
/// carry a scoped link-local Host: the config loader accepts it and the QR
/// path falls back to the default address. Rejecting it here keeps the UI from
/// persisting a value that can never produce a valid pairing payload, and
/// bracketed IPv6 input (`[addr]`) is stored unbracketed so the QR hint adds
/// exactly one pair of brackets.
fn normalize_gui_host(host: &str) -> Result<String, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("host must not be empty".to_owned());
    }
    if host.contains('%') {
        return Err("host must not include a %scope suffix".to_owned());
    }
    let host = match host.strip_prefix('[') {
        Some(rest) => {
            let inner = rest.strip_suffix(']').ok_or_else(invalid_gui_host_error)?;
            inner
                .parse::<std::net::Ipv6Addr>()
                .map_err(|_| invalid_gui_host_error())?;
            inner
        }
        None => host,
    };
    if host.parse::<std::net::IpAddr>().is_ok() || is_valid_hostname(host) {
        return Ok(host.to_owned());
    }
    Err(invalid_gui_host_error())
}

fn invalid_gui_host_error() -> String {
    "host must be an IPv4/IPv6 address or a hostname".to_owned()
}

/// Whether `host` has DNS hostname shape: dot-separated labels of 1-63 ASCII
/// letters, digits, or hyphens, not starting or ending with a hyphen.
///
/// This is the alphabet the pairing payload validator accepts; underscores,
/// spaces, and other URL punctuation are refused.
fn is_valid_hostname(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && label
                .as_bytes()
                .first()
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
            && label
                .as_bytes()
                .last()
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
    })
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use audio_share_core::capture::{
        synthetic_backend, CaptureConfig, CaptureSession, Endpoint, FrameSink, SyntheticCapture,
    };
    use audio_share_core::config::{load, EncodingSetting, LoadOptions, ThemeSetting};
    use audio_share_core::protocol::{encode_command, Command};
    use audio_share_core::Error;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use v2_runtime::admin::{
        ClientStateView, ClientView, InvitationStateView, InvitationView, PairingAdmin,
        PendingRequestView, PresentedRequestView, RequestedPairingMode,
    };

    use super::*;
    use crate::v2::{PairingModeArg, V2Runtime};

    #[derive(Debug, Default)]
    struct FakeAdminState {
        calls: Vec<String>,
        pending: Vec<PendingRequestView>,
        presented: Vec<PresentedRequestView>,
        clients: Vec<ClientView>,
        invitation_state: Option<InvitationStateView>,
    }

    #[derive(Debug, Clone, Default)]
    struct FakeAdmin {
        state: Arc<Mutex<FakeAdminState>>,
    }

    impl FakeAdmin {
        fn new(state: Arc<Mutex<FakeAdminState>>) -> Self {
            Self { state }
        }

        fn calls(&self) -> Vec<String> {
            lock(&self.state).calls.clone()
        }
    }

    impl PairingAdmin for FakeAdmin {
        fn begin(&self, mode: RequestedPairingMode) -> Result<InvitationView, String> {
            let mut state = lock(&self.state);
            state.calls.push(format!("begin:{mode:?}"));
            state.invitation_state = Some(InvitationStateView::Active);
            let view = InvitationView {
                mode,
                qr_uri: (mode == RequestedPairingMode::Qr).then(|| {
                    "audioshare-pair://p1?pm=2&addr=127.0.0.1:65531&key=x&nonce=y".to_owned()
                }),
                qr_svg: (mode == RequestedPairingMode::Qr).then(|| "<svg/>".to_owned()),
                fingerprint: "FP".to_owned(),
                expires_in_secs: 600,
            };
            Ok(view)
        }

        fn invitation_state(&self) -> Option<InvitationStateView> {
            lock(&self.state).invitation_state
        }

        fn cancel(&self) {
            let mut state = lock(&self.state);
            state.calls.push("cancel".to_owned());
            state.presented.clear();
            state.invitation_state = None;
        }

        fn pending(&self) -> Vec<PendingRequestView> {
            lock(&self.state).pending.clone()
        }

        fn presented(&self) -> Vec<PresentedRequestView> {
            lock(&self.state).calls.push("presented".to_owned());
            lock(&self.state).presented.clone()
        }

        fn approve(&self, approval_id: u64) -> bool {
            lock(&self.state)
                .calls
                .push(format!("approve:{approval_id}"));
            let mut state = lock(&self.state);
            match state
                .pending
                .iter()
                .position(|request| request.approval_id == approval_id)
            {
                Some(index) => {
                    state.pending.remove(index);
                    state
                        .presented
                        .retain(|request| request.approval_id != approval_id);
                    true
                }
                None => false,
            }
        }

        fn restore(&self, approval_id: u64) -> bool {
            lock(&self.state)
                .calls
                .push(format!("restore:{approval_id}"));
            let mut state = lock(&self.state);
            match state
                .pending
                .iter()
                .position(|request| request.approval_id == approval_id)
            {
                Some(index) => {
                    state.pending.remove(index);
                    state
                        .presented
                        .retain(|request| request.approval_id != approval_id);
                    true
                }
                None => false,
            }
        }

        fn decline(&self, approval_id: u64) -> bool {
            lock(&self.state)
                .calls
                .push(format!("decline:{approval_id}"));
            let mut state = lock(&self.state);
            match state
                .pending
                .iter()
                .position(|request| request.approval_id == approval_id)
            {
                Some(index) => {
                    state.pending.remove(index);
                    state
                        .presented
                        .retain(|request| request.approval_id != approval_id);
                    true
                }
                None => false,
            }
        }

        fn clients(&self) -> Vec<ClientView> {
            lock(&self.state).calls.push("clients".to_owned());
            lock(&self.state).clients.clone()
        }

        fn revoke(&self, client_id: &str) -> Result<ClientView, String> {
            lock(&self.state).calls.push(format!("revoke:{client_id}"));
            let mut state = lock(&self.state);
            let Some(client) = state
                .clients
                .iter_mut()
                .find(|client| client.client_id == client_id)
            else {
                return Err("unknown client".to_owned());
            };
            client.state = ClientStateView::Revoked;
            Ok(client.clone())
        }

        fn delete(&self, client_id: &str) -> Result<ClientView, String> {
            lock(&self.state).calls.push(format!("delete:{client_id}"));
            let mut state = lock(&self.state);
            let Some(position) = state
                .clients
                .iter()
                .position(|client| client.client_id == client_id)
            else {
                return Err("unknown client".to_owned());
            };
            if state.clients[position].state == ClientStateView::Active {
                return Err("client is active; revoke it before deleting".to_owned());
            }
            Ok(state.clients.remove(position))
        }

        fn fingerprint(&self) -> String {
            "FP".to_owned()
        }
    }

    #[derive(Debug)]
    struct FakeRuntime {
        admin: Arc<dyn PairingAdmin>,
        stopped: Arc<AtomicBool>,
    }

    impl V2Runtime for FakeRuntime {
        fn local_addr(&self) -> SocketAddr {
            "127.0.0.1:65531".parse().unwrap()
        }

        fn admin(&self) -> Arc<dyn PairingAdmin> {
            Arc::clone(&self.admin)
        }

        fn stop(&self) {
            self.stopped.store(true, Ordering::SeqCst);
        }
    }

    fn ok_starter(runtime: Arc<FakeRuntime>) -> V2Starter {
        Arc::new(move |_inputs| {
            let runtime = Arc::clone(&runtime);
            Box::pin(async move { Ok(runtime as Arc<dyn V2Runtime>) })
        })
    }

    fn failing_starter(message: &str) -> V2Starter {
        let message = message.to_owned();
        Arc::new(move |_inputs| {
            let message = message.clone();
            Box::pin(async move { Err(message) })
        })
    }

    fn fake_v2() -> (
        FakeAdmin,
        Arc<Mutex<FakeAdminState>>,
        V2Starter,
        Arc<AtomicBool>,
    ) {
        let admin_state = Arc::new(Mutex::new(FakeAdminState {
            clients: vec![ClientView {
                client_id: "0011223344556677".to_owned(),
                display_name: "Pixel 8".to_owned(),
                state: ClientStateView::Active,
                created_at: "2026-11-16T10:00:00Z".to_owned(),
                last_seen_at: "2026-11-16T10:00:00Z".to_owned(),
            }],
            ..FakeAdminState::default()
        }));
        let admin = FakeAdmin::new(Arc::clone(&admin_state));
        let stopped = Arc::new(AtomicBool::new(false));
        let runtime = Arc::new(FakeRuntime {
            admin: Arc::new(admin.clone()),
            stopped: Arc::clone(&stopped),
        });
        (admin, admin_state, ok_starter(runtime), stopped)
    }

    fn pending_request() -> PendingRequestView {
        PendingRequestView {
            approval_id: 7,
            client_id: "0011223344556677".to_owned(),
            display_name: "Pixel 8".to_owned(),
            mode: RequestedPairingMode::Qr,
            address: "192.168.1.44:50000".to_owned(),
            sas: Some("123 456".to_owned()),
            restore_candidate: None,
        }
    }

    fn live_request() -> PresentedRequestView {
        PresentedRequestView {
            approval_id: 7,
            client_id: "0011223344556677".to_owned(),
            display_name: "Pixel 8".to_owned(),
            mode: RequestedPairingMode::Qr,
            address: "192.168.1.44:50000".to_owned(),
            sas: Some("123 456".to_owned()),
            fingerprint: "FP".to_owned(),
            expires_in_secs: 600,
            restore_candidate: None,
        }
    }

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
    fn save_settings_normalizes_a_bracketed_ipv6_host_for_the_qr_hint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ini");
        let controller = ServerController::with_config(test_settings(), path.clone());

        let saved = controller
            .save_settings(Settings {
                host: "  [2001:db8::5]  ".to_owned(),
                port: 12_345,
                ..test_settings()
            })
            .expect("a bracketed IPv6 host is normalized and accepted");

        assert_eq!(saved.host, "2001:db8::5");
        assert_eq!(controller.settings().host, "2001:db8::5");
        assert_eq!(load_config(&path).host, "2001:db8::5");
        assert_eq!(
            v2_runtime::qr_addr_hint(&saved.host, 65_531).as_deref(),
            Some("[2001:db8::5]:65531"),
            "the saved unbracketed host still yields a bracketed QR addr"
        );
    }

    #[test]
    fn save_settings_rejects_scoped_and_invalid_hosts_without_touching_the_file() {
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

        for (host, expected) in [
            ("fe80::1%eth0", "scope"),
            ("[fe80::1%eth0]", "scope"),
            ("bad host", "host"),
            ("host_with_underscore", "host"),
            ("[not-an-ipv6]", "host"),
            ("", "host"),
            ("   ", "host"),
        ] {
            let error = controller
                .save_settings(Settings {
                    host: host.to_owned(),
                    port: 12_345,
                    ..test_settings()
                })
                .expect_err("an unusable host must be refused");
            assert!(error.contains(expected), "host {host:?} error was {error}");
            assert_eq!(
                std::fs::read(&path).unwrap(),
                before,
                "host {host:?} must not touch the file"
            );
        }
    }

    #[test]
    fn save_settings_accepts_hostnames_wildcards_and_ip_literals() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ini");
        let controller = ServerController::with_config(test_settings(), path.clone());

        for (host, expected) in [
            ("audio-share.local", "audio-share.local"),
            ("my-host", "my-host"),
            ("0.0.0.0", "0.0.0.0"),
            ("::", "::"),
            ("[::]", "::"),
            ("[2409:8a34:1234:5678::1]", "2409:8a34:1234:5678::1"),
        ] {
            let saved = controller
                .save_settings(Settings {
                    host: host.to_owned(),
                    port: 12_345,
                    ..test_settings()
                })
                .unwrap_or_else(|error| panic!("host {host:?} must be accepted: {error}"));
            assert_eq!(saved.host, expected);
            assert_eq!(load_config(&path).host, expected);
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
                theme: ThemeSetting::Dark,
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
        assert_eq!(
            reset.theme,
            ThemeSetting::Dark,
            "reset must preserve the theme like every application setting"
        );
        assert_eq!(controller.settings(), reset);

        let on_disk = load_config(&path);
        assert_eq!(on_disk.port, 65_530);
        assert_eq!(on_disk.encoding, EncodingSetting::Default);
        assert!(on_disk.autostart);
        assert_eq!(on_disk.theme, ThemeSetting::Dark);
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
                theme: ThemeSetting::Light,
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
        assert_eq!(
            updated.theme,
            ThemeSetting::Light,
            "the theme is an application setting and must save while running"
        );
        assert_eq!(controller.status().state, ServerState::Running);

        let on_disk = load_config(&path);
        assert_eq!(on_disk.language, "zh-CN");
        assert!(on_disk.autostart);
        assert_eq!(
            on_disk.when_close,
            audio_share_core::config::WhenClose::Exit
        );
        assert_eq!(on_disk.theme, ThemeSetting::Light);
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

    #[tokio::test]
    async fn start_brings_up_v2_and_stop_closes_it_first() {
        let (admin, _state, starter, stopped) = fake_v2();
        let controller = ServerController::new(test_settings());
        controller.enable_v2(starter, 0);

        let status = controller.start(synthetic()).await.expect("start");
        assert_eq!(status.state, ServerState::Running);
        assert_eq!(status.v2.state, crate::model::V2State::Running);
        assert_eq!(status.v2.address.as_deref(), Some("127.0.0.1:65531"));
        assert_eq!(status.v2.fingerprint.as_deref(), Some("FP"));
        assert_eq!(status.v2.last_error, None);

        let paired = controller.pairing_begin(PairingModeArg::Qr).expect("begin");
        assert!(paired.active);
        assert!(admin.calls().iter().any(|call| call == "begin:Qr"));

        controller.stop().await.expect("stop");
        assert!(
            stopped.load(Ordering::SeqCst),
            "stop must close the v2 runtime"
        );
        assert_eq!(
            controller.status().v2.state,
            crate::model::V2State::Disabled
        );
    }

    #[tokio::test]
    async fn start_passes_the_configured_host_and_the_v2_port_to_the_starter() {
        let seen: Arc<Mutex<Option<(String, u16)>>> = Arc::new(Mutex::new(None));
        let recorder = Arc::clone(&seen);
        let starter: V2Starter = Arc::new(move |inputs| {
            *lock(&recorder) = Some((inputs.host.clone(), inputs.port));
            Box::pin(async { Err("not starting in this test".to_owned()) })
        });
        let mut settings = test_settings();
        // `test_settings` uses port 0 for v1; the starter must see the v2 port.
        settings.host = "127.0.0.1".to_owned();
        let controller = ServerController::new(settings);
        controller.enable_v2(starter, 65_531);

        controller.start(synthetic()).await.expect("v1 starts");
        assert_eq!(
            *lock(&seen),
            Some(("127.0.0.1".to_owned(), 65_531)),
            "the starter receives settings.host and the v2 port"
        );

        controller.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn a_v2_start_failure_keeps_v1_running_and_reports_separately() {
        let controller = ServerController::new(test_settings());
        controller.enable_v2(failing_starter("bind refused"), 0);

        let status = controller
            .start(synthetic())
            .await
            .expect("v1 still starts");
        assert_eq!(status.state, ServerState::Running);
        assert_eq!(status.v2.state, crate::model::V2State::Error);
        assert_eq!(status.v2.last_error.as_deref(), Some("bind refused"));
        assert_eq!(
            status.last_error, None,
            "a v2 error must not masquerade as a v1 error"
        );
        assert_eq!(controller.status(), status);

        let error = controller
            .pairing_begin(PairingModeArg::Qr)
            .expect_err("pairing needs v2");
        assert!(error.contains("bind refused"), "error was {error}");
        controller.stop().await.expect("stop");
    }

    #[test]
    fn pairing_is_refused_until_the_server_runs() {
        let controller = ServerController::new(test_settings());
        let error = controller
            .pairing_begin(PairingModeArg::Qr)
            .expect_err("stopped controller");
        assert!(error.contains("start the server"), "error was {error}");
        let status = controller.pairing_status();
        assert!(!status.active);
        assert!(status.pending.is_empty());
    }

    #[tokio::test]
    async fn pairing_status_reports_live_pairings_until_they_clear() {
        let (_admin, admin_state, starter, _stopped) = fake_v2();
        let controller = ServerController::new(test_settings());
        controller.enable_v2(starter, 0);
        controller.start(synthetic()).await.expect("start");

        let status = controller
            .pairing_begin(PairingModeArg::Sas)
            .expect("begin SAS");
        assert!(status.live.is_empty());

        // Client connected (PairingResponse written) but has not confirmed:
        // the live view is already visible and keeps the dialog active.
        lock(&admin_state).presented.push(live_request());
        let status = controller.pairing_status();
        assert!(status.active, "a live pairing keeps the dialog active");
        assert!(status.pending.is_empty(), "no approval card before confirm");
        assert_eq!(status.live.len(), 1);
        assert_eq!(status.live[0].approval_id, 7);
        assert_eq!(status.live[0].display_name, "Pixel 8");
        assert_eq!(status.live[0].sas.as_deref(), Some("123 456"));
        assert_eq!(status.live[0].client_id, "0011223344556677");
        assert_eq!(status.live[0].address, "192.168.1.44:50000");
        assert_eq!(status.live[0].fingerprint, "FP");
        assert_eq!(status.live[0].expires_in_secs, 600);

        // PairingConfirm has arrived: the approval card joins the live view.
        lock(&admin_state).pending.push(pending_request());
        let status = controller.pairing_status();
        assert_eq!(status.live.len(), 1);
        assert_eq!(status.pending.len(), 1);

        // Approving clears both the approval card and the live view.
        let status = controller.pairing_approve(7).expect("approve");
        assert!(status.live.is_empty());
        assert!(status.pending.is_empty());

        // Cancel must clear a live view that has not reached approval.
        controller
            .pairing_begin(PairingModeArg::Sas)
            .expect("begin SAS again");
        lock(&admin_state).presented.push(live_request());
        let status = controller.pairing_cancel().expect("cancel");
        assert!(status.live.is_empty());
        assert!(!status.active);

        // Decline clears the live view too.
        controller
            .pairing_begin(PairingModeArg::Qr)
            .expect("begin QR again");
        lock(&admin_state).presented.push(live_request());
        lock(&admin_state).pending.push(pending_request());
        let status = controller.pairing_decline(7).expect("decline");
        assert!(status.live.is_empty());
        assert!(status.pending.is_empty());

        // Deadline/connection-loss clearing (driver side) drops the live view;
        // the controller reflects that on the next poll.
        controller
            .pairing_begin(PairingModeArg::Sas)
            .expect("begin SAS third time");
        lock(&admin_state).presented.push(live_request());
        assert_eq!(controller.pairing_status().live.len(), 1);
        lock(&admin_state).presented.clear();
        assert!(controller.pairing_status().live.is_empty());

        controller.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn pairing_status_forwards_restore_candidate_for_revoked_identities() {
        let (_admin, admin_state, starter, _stopped) = fake_v2();
        let controller = ServerController::new(test_settings());
        controller.enable_v2(starter, 0);
        controller.start(synthetic()).await.expect("start");

        // A plain identity keeps `None`: the dialog shows the normal card.
        lock(&admin_state).pending.push(pending_request());
        lock(&admin_state).presented.push(live_request());
        let status = controller.pairing_status();
        assert_eq!(status.pending[0].restore_candidate, None);
        assert_eq!(status.live[0].restore_candidate, None);

        // A revoked identity forwards the saved display name to both views so
        // the dialog can ask for an explicit Restore (ADR-0006).
        lock(&admin_state).pending.clear();
        lock(&admin_state).presented.clear();
        let mut pending = pending_request();
        pending.restore_candidate = Some("Pixel 8 (saved)".to_owned());
        lock(&admin_state).pending.push(pending);
        let mut live = live_request();
        live.restore_candidate = Some("Pixel 8 (saved)".to_owned());
        lock(&admin_state).presented.push(live);

        let status = controller.pairing_status();
        assert_eq!(
            status.pending[0].restore_candidate.as_deref(),
            Some("Pixel 8 (saved)")
        );
        assert_eq!(
            status.live[0].restore_candidate.as_deref(),
            Some("Pixel 8 (saved)")
        );

        controller.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn pairing_begin_status_approve_and_decline_route_to_the_core() {
        let (admin, admin_state, starter, _stopped) = fake_v2();
        let controller = ServerController::new(test_settings());
        controller.enable_v2(starter, 0);
        controller.start(synthetic()).await.expect("start");

        let status = controller
            .pairing_begin(PairingModeArg::Qr)
            .expect("begin QR");
        assert!(status.active);
        let invitation = status.invitation.expect("invitation");
        assert_eq!(invitation.mode, "qr");
        assert_eq!(invitation.state, PairingInvitationState::Active);
        assert!(invitation.qr_uri.is_some());
        assert_eq!(invitation.qr_svg.as_deref(), Some("<svg/>"));
        assert_eq!(invitation.fingerprint, "FP");
        assert!(invitation.expires_in_secs > 0);
        assert!(admin.calls().iter().any(|call| call == "begin:Qr"));

        lock(&admin_state).pending.push(pending_request());
        let status = controller.pairing_status();
        assert!(status.active);
        assert_eq!(status.pending.len(), 1);
        assert_eq!(status.pending[0].display_name, "Pixel 8");
        assert_eq!(status.pending[0].sas.as_deref(), Some("123 456"));
        assert_eq!(status.pending[0].client_id, "0011223344556677");

        let status = controller.pairing_approve(7).expect("approve");
        assert!(status.pending.is_empty());
        assert!(admin.calls().iter().any(|call| call == "approve:7"));

        let error = controller
            .pairing_approve(7)
            .expect_err("approval is single-use");
        assert!(error.contains("no longer pending"), "error was {error}");

        lock(&admin_state).pending.push(PendingRequestView {
            approval_id: 8,
            ..pending_request()
        });
        let status = controller.pairing_decline(8).expect("decline");
        assert!(status.pending.is_empty());
        assert!(admin.calls().iter().any(|call| call == "decline:8"));

        controller.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn pairing_cancel_clears_the_dialog_invitation() {
        let (_admin, _state, starter, _stopped) = fake_v2();
        let controller = ServerController::new(test_settings());
        controller.enable_v2(starter, 0);
        controller.start(synthetic()).await.expect("start");
        assert!(
            controller
                .pairing_begin(PairingModeArg::Sas)
                .expect("begin SAS")
                .active
        );

        let status = controller.pairing_cancel().expect("cancel");
        assert!(!status.active);
        assert!(status.invitation.is_none());

        controller.stop().await.expect("stop");
        assert!(!controller.pairing_status().active);
    }

    #[tokio::test]
    async fn pairing_status_marks_expired_invitations_without_dropping_them() {
        let (admin, _state, starter, _stopped) = fake_v2();
        let controller = ServerController::new(test_settings());
        controller.enable_v2(starter, 0);
        controller.start(synthetic()).await.expect("start");
        {
            let mut state = lock(&controller.state);
            state.v2_admin = Some(Arc::new(admin));
            state.invitation = Some(ActiveInvitation {
                mode: "qr".to_owned(),
                qr_uri: Some("audioshare-pair://p1?…".to_owned()),
                qr_svg: None,
                fingerprint: "FP".to_owned(),
                expires_at: Instant::now() - Duration::from_secs(1),
            });
        }

        let status = controller.pairing_status();
        assert!(!status.active, "an expired invitation is not active");
        let invitation = status
            .invitation
            .expect("the expired invitation stays visible for refresh");
        assert_eq!(invitation.state, PairingInvitationState::Expired);
        assert_eq!(invitation.expires_in_secs, 0);
        controller.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn pairing_poll_never_renews_a_consumed_invitation() {
        let (admin, admin_state, starter, _stopped) = fake_v2();
        let controller = ServerController::new(test_settings());
        controller.enable_v2(starter, 0);
        controller.start(synthetic()).await.expect("start");

        controller
            .pairing_begin(PairingModeArg::Qr)
            .expect("begin QR");
        lock(&admin_state).invitation_state = Some(InvitationStateView::Consumed);

        let status = controller.pairing_poll().expect("poll");
        let invitation = status
            .invitation
            .expect("the consumed invitation stays visible for refresh");
        assert_eq!(invitation.state, PairingInvitationState::Consumed);
        assert!(!status.active);
        assert_eq!(
            begin_qr_count(&admin),
            1,
            "a consumed invitation is never renewed automatically"
        );

        controller.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn pairing_poll_renews_a_naturally_expired_invitation_once() {
        let (admin, admin_state, starter, _stopped) = fake_v2();
        let controller = ServerController::new(test_settings());
        controller.enable_v2(starter, 0);
        controller.start(synthetic()).await.expect("start");

        controller
            .pairing_begin(PairingModeArg::Qr)
            .expect("begin QR");
        lock(&admin_state).invitation_state = Some(InvitationStateView::Expired);

        let status = controller.pairing_poll().expect("poll renews");
        let invitation = status.invitation.expect("renewed invitation");
        assert_eq!(invitation.state, PairingInvitationState::Active);
        assert!(status.active);
        assert_eq!(
            begin_qr_count(&admin),
            2,
            "one manual begin plus one automatic renewal"
        );

        let status = controller.pairing_poll().expect("second poll");
        assert_eq!(
            status.invitation.expect("still active").state,
            PairingInvitationState::Active,
            "the renewed invitation is not renewed again"
        );
        assert_eq!(begin_qr_count(&admin), 2);

        controller.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn pairing_poll_does_not_renew_while_a_pairing_is_live() {
        let (admin, admin_state, starter, _stopped) = fake_v2();
        let controller = ServerController::new(test_settings());
        controller.enable_v2(starter, 0);
        controller.start(synthetic()).await.expect("start");

        controller
            .pairing_begin(PairingModeArg::Qr)
            .expect("begin QR");
        {
            let mut state = lock(&admin_state);
            state.invitation_state = Some(InvitationStateView::Expired);
            state.presented.push(live_request());
        }

        let status = controller.pairing_poll().expect("poll");
        assert!(status.active, "a live pairing keeps the dialog active");
        assert_eq!(
            status.invitation.expect("invitation").state,
            PairingInvitationState::Expired,
            "the expired QR is shown as expired, not silently replaced"
        );
        assert_eq!(begin_qr_count(&admin), 1);

        controller.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn core_active_outranks_a_truncated_local_deadline() {
        let (admin, admin_state, starter, _stopped) = fake_v2();
        let controller = ServerController::new(test_settings());
        controller.enable_v2(starter, 0);
        controller.start(synthetic()).await.expect("start");

        lock(&admin_state).invitation_state = Some(InvitationStateView::Active);
        {
            let mut state = lock(&controller.state);
            state.v2_admin = Some(Arc::new(admin.clone()));
            state.invitation = Some(ActiveInvitation {
                mode: "qr".to_owned(),
                qr_uri: Some("audioshare-pair://p1?…".to_owned()),
                qr_svg: None,
                fingerprint: "FP".to_owned(),
                expires_at: Instant::now() - Duration::from_secs(1),
            });
        }

        let status = controller.pairing_status();
        assert_eq!(
            status.invitation.expect("invitation").state,
            PairingInvitationState::Active,
            "the core is the single source of truth for the QR lifecycle"
        );
        let status = controller.pairing_poll().expect("poll");
        assert_eq!(
            status.invitation.expect("invitation").state,
            PairingInvitationState::Active
        );
        assert_eq!(
            begin_qr_count(&admin),
            0,
            "the truncated local countdown must not renew before the core expires"
        );

        controller.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn pairing_poll_does_not_renew_an_unknown_core_invitation() {
        let (admin, admin_state, starter, _stopped) = fake_v2();
        let controller = ServerController::new(test_settings());
        controller.enable_v2(starter, 0);
        controller.start(synthetic()).await.expect("start");

        controller
            .pairing_begin(PairingModeArg::Qr)
            .expect("begin QR");
        lock(&admin_state).invitation_state = None;

        let status = controller.pairing_poll().expect("poll");
        assert_eq!(
            status.invitation.expect("invitation").state,
            PairingInvitationState::Expired,
            "an unknown core state is shown as expired"
        );
        assert_eq!(
            begin_qr_count(&admin),
            1,
            "only a natural expiration reported by the core triggers renewal"
        );

        controller.stop().await.expect("stop");
    }

    fn begin_qr_count(admin: &FakeAdmin) -> usize {
        admin
            .calls()
            .iter()
            .filter(|call| call.as_str() == "begin:Qr")
            .count()
    }

    #[tokio::test]
    async fn clients_list_and_revoke_route_to_the_shared_core() {
        let (admin, _state, starter, _stopped) = fake_v2();
        let controller = ServerController::new(test_settings());
        controller.enable_v2(starter, 0);
        controller.start(synthetic()).await.expect("start");

        let clients = controller.clients_list().expect("list");
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].display_name, "Pixel 8");
        assert_eq!(clients[0].state, "active");

        let revoked = controller
            .clients_revoke("0011223344556677")
            .expect("revoke");
        assert_eq!(revoked.client_id, "0011223344556677");
        assert_eq!(revoked.state, "revoked");
        assert!(admin
            .calls()
            .iter()
            .any(|call| call == "revoke:0011223344556677"));

        let error = controller
            .clients_revoke("deadbeefdeadbeef")
            .expect_err("unknown client");
        assert!(error.contains("unknown client"), "error was {error}");
        controller.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn clients_delete_routes_to_the_shared_core_and_refuses_active_records() {
        let (admin, _state, starter, _stopped) = fake_v2();
        let controller = ServerController::new(test_settings());
        controller.enable_v2(starter, 0);
        controller.start(synthetic()).await.expect("start");

        // ADR-0006: an active record must be revoked first, so one click cannot
        // both close a session and erase the audit row.
        let error = controller
            .clients_delete("0011223344556677")
            .expect_err("active record");
        assert!(error.contains("revoke"), "error was {error}");
        assert_eq!(controller.clients_list().expect("list").len(), 1);

        controller
            .clients_revoke("0011223344556677")
            .expect("revoke");
        let deleted = controller
            .clients_delete("0011223344556677")
            .expect("delete revoked");
        assert_eq!(deleted.client_id, "0011223344556677");
        assert_eq!(deleted.state, "revoked");
        assert!(admin
            .calls()
            .iter()
            .any(|call| call == "delete:0011223344556677"));
        assert!(controller.clients_list().expect("list").is_empty());

        let error = controller
            .clients_delete("deadbeefdeadbeef")
            .expect_err("unknown client");
        assert!(error.contains("unknown client"), "error was {error}");
        controller.stop().await.expect("stop");
    }

    #[tokio::test]
    async fn device_commands_are_refused_after_stop() {
        let (_admin, _state, starter, _stopped) = fake_v2();
        let controller = ServerController::new(test_settings());
        controller.enable_v2(starter, 0);
        controller.start(synthetic()).await.expect("start");
        controller.stop().await.expect("stop");

        for error in [
            controller
                .clients_list()
                .expect_err("list after stop")
                .to_string(),
            controller
                .clients_delete("0011223344556677")
                .expect_err("delete after stop")
                .to_string(),
            controller
                .pairing_cancel()
                .expect_err("cancel after stop")
                .to_string(),
        ] {
            assert!(error.contains("start the server"), "error was {error}");
        }
    }
}
