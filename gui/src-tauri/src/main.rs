#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri shell for the Audio Share Server GUI.
//!
//! The window, tray, and commands are thin adapters over the display-free
//! `ServerController` in the library half of this crate. Ticket 09 loads and
//! migrates `config.ini` before the UI starts, persists settings changes, and
//! applies the configured close behavior. Ticket 10 adds launch behavior,
//! autostart, i18n, update checks, and About at the marked extension points.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use audio_share_core::capture::{platform_backend, synthetic_backend, CaptureBackend, Endpoint};
use audio_share_core::config::{self, LoadOptions};
use audio_share_core::paths;
use audio_share_server_gui::autostart;
use audio_share_server_gui::controller::ServerController;
use audio_share_server_gui::help::HelpTopic;
use audio_share_server_gui::i18n::{self, Locale};
use audio_share_server_gui::logging;
use audio_share_server_gui::model::{
    bootstrap, notice_messages, tray_enabled, ApplicationSettings, Bootstrap, Notice, ServerState,
    ServerStatus, Settings,
};
use audio_share_server_gui::options::GuiOptions;
use audio_share_server_gui::strings;
use audio_share_server_gui::update::{self, UpdateCheckResult};
use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent,
};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

/// State managed by Tauri and reached from commands and tray callbacks.
struct AppState {
    controller: Arc<ServerController>,
    options: GuiOptions,
    /// Last successful endpoint enumeration; `None` when enumeration failed, so
    /// settings saves are not rejected against a list we never obtained.
    endpoints: Arc<Mutex<Option<Vec<Endpoint>>>>,
    /// Migration/recovery/startup warnings rendered as localized notices.
    notices: Mutex<Vec<Notice>>,
    /// Newest update found by a background or manual check.
    update: Mutex<Option<UpdateCheckResult>>,
    /// Language selected at startup (also installed in `strings`).
    locale: Locale,
}

/// Tray menu items kept in managed state so status changes can toggle them.
struct TrayItems {
    start: MenuItem<tauri::Wry>,
    stop: MenuItem<tauri::Wry>,
}

/// Endpoint shape exposed to the frontend (`id`/`name`).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EndpointDto {
    id: String,
    name: String,
}

impl From<Endpoint> for EndpointDto {
    fn from(endpoint: Endpoint) -> Self {
        Self {
            id: endpoint.id,
            name: endpoint.name,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn hide_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

/// Build the capture backend for a start/enumeration request. `--fake-capture`
/// is the hardware-free debug path; normal runs use the platform backend.
fn capture_backend(options: &GuiOptions) -> Result<Arc<dyn CaptureBackend>, String> {
    if options.fake_capture {
        synthetic_backend()
            .map(|backend| Arc::new(backend) as Arc<dyn CaptureBackend>)
            .map_err(|error| error.to_string())
    } else {
        platform_backend().map_err(|error| error.to_string())
    }
}

/// Load the persisted settings and enumerate endpoints before the UI starts.
///
/// Endpoint enumeration (PipeWire/WASAPI) can block, so `main` runs this on a
/// dedicated thread before building the Tauri application; the webview is never
/// waiting on audio discovery.
fn load_startup(
    options: GuiOptions,
    paths: paths::Paths,
) -> (Settings, Vec<Notice>, Option<Vec<Endpoint>>) {
    let config_path = paths.config.clone();
    let mut notices = Vec::new();
    let endpoints = match capture_backend(&options) {
        Ok(backend) => match backend.endpoints() {
            Ok(endpoints) => Some(endpoints),
            Err(error) => {
                notices.push(Notice::new("endpointListFailed").with("message", error.to_string()));
                None
            }
        },
        Err(error) => {
            notices.push(Notice::new("captureInitFailed").with("message", error.to_string()));
            None
        }
    };

    match config::load(
        &config_path,
        LoadOptions {
            legacy: paths.legacy_config.as_deref(),
            endpoints: endpoints.as_deref(),
        },
    ) {
        Ok(outcome) => {
            notices.extend(notice_messages(&outcome.notices));
            (outcome.config, notices, endpoints)
        }
        Err(error) => {
            logging::error(format!(
                "cannot load configuration {}: {error}",
                config_path.display()
            ));
            notices.push(Notice::new("loadFailed").with("message", error.to_string()));
            (Settings::default(), notices, endpoints)
        }
    }
}

/// Shared start path for commands and the tray menu.
async fn start_with_app(app: &AppHandle) -> Result<ServerStatus, String> {
    let (controller, backend) = {
        let state = app.state::<AppState>();
        (
            Arc::clone(&state.controller),
            capture_backend(&state.options)?,
        )
    };
    controller.start(backend).await
}

/// Shared stop path for commands and the tray menu.
async fn stop_with_app(app: &AppHandle) -> Result<ServerStatus, String> {
    let controller = Arc::clone(&app.state::<AppState>().controller);
    controller.stop().await
}

/// The release endpoint; `AUDIO_SHARE_UPDATE_URL` is a test seam for replayed
/// GitHub responses and is not used by normal installs.
fn update_endpoint() -> String {
    std::env::var("AUDIO_SHARE_UPDATE_URL").unwrap_or_else(|_| update::API_URL.to_owned())
}

/// Perform one update check and publish a newer release to the UI/tray.
///
/// The blocking HTTP request runs on the blocking pool. A newer release is
/// stored for the bootstrap payload, emitted as `update:available`, and shown
/// as a native notification with the raw tag. Failures are returned so the
/// manual path can show them; the automatic path logs and ignores them.
async fn run_update_check(app: &AppHandle) -> Result<UpdateCheckResult, String> {
    let url = update_endpoint();
    let result =
        match tauri::async_runtime::spawn_blocking(move || update::fetch_latest(&url)).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                logging::error(format!("update check failed: {error}"));
                return Err(error);
            }
            Err(error) => {
                let message = format!("update check task failed: {error}");
                logging::error(&message);
                return Err(message);
            }
        };

    match &result {
        UpdateCheckResult::UpToDate => logging::info("update check: up to date"),
        UpdateCheckResult::Newer { version, .. } => {
            logging::info(format!("update check: release {version} is available"));
        }
    }

    let state = app.state::<AppState>();
    *lock(&state.update) = match &result {
        UpdateCheckResult::Newer { .. } => Some(result.clone()),
        UpdateCheckResult::UpToDate => None,
    };

    if let UpdateCheckResult::Newer { version, .. } = &result {
        let _ = app.emit("update:available", result.clone());
        let title = strings::t("update.notificationTitle");
        let body = strings::t_args("update.notificationBody", &[("version", version)]);
        if let Err(error) = app.notification().builder().title(title).body(body).show() {
            // A missing notification daemon (headless CI) must not break the
            // check; the in-app banner is the primary UX.
            logging::error(format!("update notification failed: {error}"));
        }
    }
    Ok(result)
}

/// Run an automatic update check every 3 hours while `AutoUpdate` is enabled.
///
/// v0.3.4 parity: the first automatic check happens one full interval after
/// launch; there is no automatic startup check. Manual checks are separate.
/// The setting is re-read on every tick, so toggling it takes effect without a
/// restart.
fn spawn_update_loop(app: &tauri::App) {
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(update::AUTOMATIC_CHECK_INTERVAL).await;
            let enabled = handle.state::<AppState>().controller.settings().auto_update;
            if update::should_run_automatic_check(enabled) {
                // run_update_check logs its own failure; the loop must survive.
                let _ = run_update_check(&handle).await;
            }
        }
    });
}

/// Make the OS login entry match `AutoRun`; report failures in the UI.
fn reconcile_autostart(app: &tauri::App) {
    let enabled = app.state::<AppState>().controller.settings().autostart;
    if let Err(error) = autostart::configure(enabled) {
        logging::error(format!("autostart reconciliation failed: {error}"));
        let state = app.state::<AppState>();
        lock(&state.notices).push(Notice::new("autostartFailed").with("message", error));
    }
}

/// Apply `WhenAppStart` after setup, asynchronously and without blocking.
fn apply_startup_behavior(app: &tauri::App) {
    let settings = app.state::<AppState>().controller.settings();
    if !settings.when_start.starts_server(settings.running) {
        return;
    }
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = start_with_app(&handle).await {
            logging::error(format!("startup server start failed: {error}"));
        }
    });
}

fn update_tray(app: &AppHandle, status: &ServerStatus) {
    if let Some(items) = app.try_state::<TrayItems>() {
        let enabled = tray_enabled(status.state);
        let _ = items.start.set_enabled(enabled.start);
        let _ = items.stop.set_enabled(enabled.stop);
    }
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show_i = MenuItem::with_id(app, "show", strings::t("tray.show"), true, None::<&str>)?;
    let start_i = MenuItem::with_id(app, "start", strings::t("tray.start"), true, None::<&str>)?;
    let stop_i = MenuItem::with_id(app, "stop", strings::t("tray.stop"), false, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let exit_i = MenuItem::with_id(app, "exit", strings::t("tray.exit"), true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_i, &start_i, &stop_i, &separator, &exit_i])?;

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("bundle.icon must include a PNG (icons/icon.png)");

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip(strings::t("app.title"))
        .menu(&menu)
        // Windows left-click toggles the window, so the menu must not also
        // open on left click. Linux delivers no click events; right-click
        // opens this menu.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "start" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = start_with_app(&app).await {
                        logging::error(format!("tray start failed: {error}"));
                    }
                });
            }
            "stop" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = stop_with_app(&app).await {
                        logging::error(format!("tray stop failed: {error}"));
                    }
                });
            }
            // Tray Exit always quits, regardless of WhenClose. Stop the core
            // gracefully first (bounded) so clients see a clean close.
            "exit" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    stop_core_if_running(&app, Duration::from_secs(3)).await;
                    app.exit(0);
                });
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Tauri does not emit tray icon events on Linux (StatusNotifierItem).
            // On Windows, left click toggles window visibility.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    if window.is_visible().unwrap_or(false) {
                        let _ = window.hide();
                    } else {
                        show_main_window(app);
                    }
                }
            }
        })
        .build(app)?;

    app.manage(TrayItems {
        start: start_i,
        stop: stop_i,
    });

    Ok(())
}

/// Stop a running core with a bounded wait; used by Exit paths.
async fn stop_core_if_running(app: &AppHandle, timeout: Duration) {
    let controller = Arc::clone(&app.state::<AppState>().controller);
    if controller.status().state == ServerState::Running {
        let _ = tokio::time::timeout(timeout, controller.stop()).await;
    }
}

/// Apply `[App Settings] WhenClose`: 0 stops the core and exits, non-zero hides
/// the window and keeps the server running. This applies only to the window
/// close request; the tray Exit command always quits.
fn install_close_handler(app: &tauri::App) {
    if let Some(window) = app.get_webview_window("main") {
        let app_handle = app.handle().clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let when_close = app_handle
                    .state::<AppState>()
                    .controller
                    .settings()
                    .when_close;
                if when_close.exits_on_close() {
                    let handle = app_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        // Bounded so a wedged capture backend cannot trap the
                        // user in a closing app.
                        stop_core_if_running(&handle, Duration::from_secs(3)).await;
                        handle.exit(0);
                    });
                } else {
                    hide_main_window(&app_handle);
                }
            }
        });
    }
}

fn install_controller_listener(app: &tauri::App) {
    let controller = Arc::clone(&app.state::<AppState>().controller);
    let handle = app.handle().clone();
    controller.set_listener(Arc::new(move |status| {
        update_tray(&handle, status);
        // Fire-and-forget; never blocks a tray callback or the controller.
        let _ = handle.emit("server:state-changed", status.clone());
    }));
}

#[tauri::command]
async fn get_bootstrap(state: State<'_, AppState>) -> Result<Bootstrap, String> {
    let controller = Arc::clone(&state.controller);
    let notices = lock(&state.notices).clone();
    let update = lock(&state.update).clone();
    Ok(bootstrap(
        &controller.settings(),
        &controller.status(),
        &notices,
        update,
        state.locale,
    ))
}

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    Ok(state.controller.settings())
}

/// Validate, persist atomically, and activate new settings. Refused unless the
/// server is stopped; the endpoint list (when known) is checked here.
#[tauri::command]
async fn save_settings(state: State<'_, AppState>, settings: Settings) -> Result<Settings, String> {
    let endpoints = lock(&state.endpoints).clone();
    state
        .controller
        .save_settings_with_endpoints(settings, endpoints.as_deref())
}

/// Persist application settings even while the server runs. Autostart is
/// applied to the OS first so a failure leaves the config untouched.
#[tauri::command]
async fn save_application_settings(
    app: AppHandle,
    settings: ApplicationSettings,
) -> Result<Settings, String> {
    settings.validate()?;
    let controller = {
        let state = app.state::<AppState>();
        Arc::clone(&state.controller)
    };
    let previous_autostart = controller.settings().autostart;
    if settings.autostart != previous_autostart {
        if let Err(error) = autostart::configure(settings.autostart) {
            return Err(strings::t_args("error.autostart", &[("message", &error)]));
        }
    }
    controller.save_application_settings(settings)
}

/// Manual update check. The caller shows "No Update" or a readable error.
#[tauri::command]
async fn check_update(app: AppHandle) -> Result<UpdateCheckResult, String> {
    run_update_check(&app).await
}

/// Rebuild the inbound TCP and UDP firewall rules for this executable.
///
/// Windows only: the active profile mask is read through COM and the elevated
/// script runs on the blocking pool, so neither UAC nor netsh blocks the UI.
/// Other platforms hide the settings row and receive an unsupported error.
#[tauri::command]
async fn repair_firewall() -> Result<(), String> {
    #[cfg(windows)]
    {
        let outcome = tauri::async_runtime::spawn_blocking(
            audio_share_server_gui::windows_maintenance::repair_firewall_blocking,
        )
        .await
        .map_err(|error| {
            audio_share_server_gui::maintenance::MaintenanceError::ElevationFailed(
                error.to_string(),
            )
            .localized(strings::locale())
        })?;
        outcome.map_err(|error| error.localized(strings::locale()))
    }
    #[cfg(not(windows))]
    {
        Err(strings::t("error.maintenance.unsupported"))
    }
}

/// Open `control.exe /name Microsoft.Sound` without waiting for it.
///
/// Windows only; the v0.3.4 GUI waited for the control panel to close.
#[tauri::command]
async fn open_sound_panel() -> Result<(), String> {
    #[cfg(windows)]
    {
        audio_share_server_gui::windows_maintenance::open_sound_panel()
            .map_err(|error| error.localized(strings::locale()))
    }
    #[cfg(not(windows))]
    {
        Err(strings::t("error.maintenance.unsupported"))
    }
}

/// Restore the Server fields to their defaults and write them.
#[tauri::command]
async fn reset_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    state.controller.reset_settings()
}

/// Enumerate the real capture endpoints off the UI thread.
#[tauri::command]
async fn list_endpoints(state: State<'_, AppState>) -> Result<Vec<EndpointDto>, String> {
    let options = state.options;
    let cache = Arc::clone(&state.endpoints);
    let endpoints = tauri::async_runtime::spawn_blocking(move || {
        let backend = capture_backend(&options)?;
        backend
            .endpoints()
            .map_err(|error| format!("cannot list audio endpoints: {error}"))
    })
    .await
    .map_err(|error| format!("audio endpoint task failed: {error}"))??;
    *lock(&cache) = Some(endpoints.clone());
    Ok(endpoints.into_iter().map(EndpointDto::from).collect())
}

/// Async command: the core start (capture negotiation, socket bind) runs off
/// the UI thread and the controller never holds a lock across `.await`.
#[tauri::command]
async fn start_server(app: AppHandle) -> Result<ServerStatus, String> {
    start_with_app(&app).await
}

/// Async command: stopping joins capture and network tasks off the UI thread.
#[tauri::command]
async fn stop_server(app: AppHandle) -> Result<ServerStatus, String> {
    stop_with_app(&app).await
}

/// Open the documentation that replaces the Windows-only maintenance actions
/// on Linux. The frontend sends a topic id, never a URL, so the webview cannot
/// use this command to open arbitrary pages.
#[tauri::command]
async fn open_help(app: AppHandle, topic: String) -> Result<(), String> {
    let topic = HelpTopic::parse(&topic).ok_or_else(|| format!("unknown help topic {topic:?}"))?;
    app.opener()
        .open_url(topic.url(), None::<&str>)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn hide_window(app: AppHandle) -> Result<(), String> {
    hide_main_window(&app);
    Ok(())
}

#[tauri::command]
async fn exit_app(app: AppHandle) -> Result<(), String> {
    // Best effort: close client sessions before the process exits. Bounded so
    // a wedged capture backend cannot trap the user in a closing app.
    stop_core_if_running(&app, Duration::from_secs(3)).await;
    app.exit(0);
    Ok(())
}

fn main() {
    // Install the log file before any startup work so early failures are
    // recorded. v0.3.4's GUI created `server.log` next to `config.ini` and
    // truncated it at launch; the rewrite keeps that behavior.
    let resolved_paths = paths::paths();
    logging::init(&resolved_paths.log);
    logging::info(format!(
        "Audio Share Server {} starting",
        env!("CARGO_PKG_VERSION")
    ));
    logging::info(format!(
        "configuration file: {}",
        resolved_paths.config.display()
    ));

    // Windows: resolve WebView2 before any startup work. Tauri would otherwise
    // fail late (and, in a release build, panic after its own error dialog).
    // The prompt lets the user open the download page; a missing runtime is an
    // expected condition, not a crash, so exit cleanly.
    #[cfg(windows)]
    if !audio_share_server_gui::webview2_check::preflight() {
        logging::error("Microsoft Edge WebView2 Runtime is not available; exiting");
        std::process::exit(0);
    }

    let options = GuiOptions::parse(std::env::args().skip(1));

    // Settings load and endpoint enumeration run before the Tauri UI exists;
    // the dedicated thread keeps audio discovery off the main/UI thread.
    let config_path = resolved_paths.config.clone();
    let (settings, startup_notices, endpoints) =
        std::thread::spawn(move || load_startup(options, resolved_paths))
            .join()
            .unwrap_or_else(|_| {
                logging::error("startup task panicked; using default settings");
                (
                    Settings::default(),
                    vec![Notice::new("startupFailed")],
                    None,
                )
            });

    for notice in &startup_notices {
        logging::info(format!(
            "configuration notice: {} {:?}",
            notice.code, notice.params
        ));
    }

    // The language is read once before the tray exists; changing it in the UI
    // persists the setting and takes effect on the next launch.
    let locale = i18n::resolve_configured(&settings.language);
    strings::set_locale(locale);

    tauri::Builder::default()
        // Single-instance MUST be the first plugin registered.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // Autostart may launch a second instance with hide args; it must
            // not steal focus from the user's current window.
            if args
                .iter()
                .any(|arg| matches!(arg.as_str(), "--hide" | "-hide" | "/hide"))
            {
                return;
            }
            show_main_window(app);
            let _ = app.emit("app:second-instance", ());
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .manage(AppState {
            controller: Arc::new(ServerController::with_config(settings, config_path)),
            options,
            endpoints: Arc::new(Mutex::new(endpoints)),
            notices: Mutex::new(startup_notices),
            update: Mutex::new(None),
            locale,
        })
        .invoke_handler(tauri::generate_handler![
            get_bootstrap,
            get_settings,
            save_settings,
            save_application_settings,
            reset_settings,
            list_endpoints,
            start_server,
            stop_server,
            open_help,
            hide_window,
            exit_app,
            check_update,
            repair_firewall,
            open_sound_panel
        ])
        .setup(move |app| {
            install_controller_listener(app);
            build_tray(app)?;
            install_close_handler(app);

            // The window starts hidden in config (no flash at login). Show it
            // for normal launches; autostart passes --hide.
            if options.hide {
                hide_main_window(app.handle());
            } else {
                show_main_window(app.handle());
            }

            // AutoRun is the source of truth: create or remove the OS entry at
            // every launch, reporting failures in the UI notice area.
            reconcile_autostart(app);
            apply_startup_behavior(app);
            spawn_update_loop(app);

            // Headless CI smoke test: prove the window/webview exists, then
            // exit 0. Output format is asserted by docs/server-rewrite tests.
            if options.smoke_test {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(4));
                    if let Some(window) = handle.get_webview_window("main") {
                        println!(
                            "SMOKE visible={:?} size={:?} title={:?}",
                            window.is_visible(),
                            window.inner_size(),
                            window.title()
                        );
                    } else {
                        println!("SMOKE window-missing");
                    }
                    handle.exit(0);
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Audio Share Server");
}
