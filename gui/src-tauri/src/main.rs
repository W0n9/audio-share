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
use audio_share_core::config::{self, LoadOptions, WhenClose};
use audio_share_core::paths;
use audio_share_server_gui::controller::ServerController;
use audio_share_server_gui::model::{
    bootstrap, notice_messages, tray_enabled, Bootstrap, ServerState, ServerStatus, Settings,
};
use audio_share_server_gui::options::GuiOptions;
use audio_share_server_gui::strings::t;
use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent,
};

/// State managed by Tauri and reached from commands and tray callbacks.
struct AppState {
    controller: Arc<ServerController>,
    options: GuiOptions,
    /// Last successful endpoint enumeration; `None` when enumeration failed, so
    /// settings saves are not rejected against a list we never obtained.
    endpoints: Arc<Mutex<Option<Vec<Endpoint>>>>,
    /// Migration/recovery warnings collected before the webview started.
    startup_notices: Vec<String>,
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
) -> (
    Settings,
    Vec<String>,
    Option<Vec<Endpoint>>,
    std::path::PathBuf,
) {
    let paths = paths::paths();
    let config_path = paths.config.clone();
    let mut notices = Vec::new();
    let endpoints = match capture_backend(&options) {
        Ok(backend) => match backend.endpoints() {
            Ok(endpoints) => Some(endpoints),
            Err(error) => {
                notices.push(format!("Could not list audio endpoints: {error}"));
                None
            }
        },
        Err(error) => {
            notices.push(format!("Could not initialize audio capture: {error}"));
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
            (outcome.config, notices, endpoints, config_path)
        }
        Err(error) => {
            notices.push(format!("Could not load settings: {error}"));
            (Settings::default(), notices, endpoints, config_path)
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

fn update_tray(app: &AppHandle, status: &ServerStatus) {
    if let Some(items) = app.try_state::<TrayItems>() {
        let enabled = tray_enabled(status.state);
        let _ = items.start.set_enabled(enabled.start);
        let _ = items.stop.set_enabled(enabled.stop);
    }
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show_i = MenuItem::with_id(app, "show", t("tray.show"), true, None::<&str>)?;
    let start_i = MenuItem::with_id(app, "start", t("tray.start"), true, None::<&str>)?;
    let stop_i = MenuItem::with_id(app, "stop", t("tray.stop"), false, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let exit_i = MenuItem::with_id(app, "exit", t("tray.exit"), true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_i, &start_i, &stop_i, &separator, &exit_i])?;

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("bundle.icon must include a PNG (icons/icon.png)");

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip(t("app.title"))
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
                        eprintln!("AudioShareServer: tray start failed: {error}");
                    }
                });
            }
            "stop" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = stop_with_app(&app).await {
                        eprintln!("AudioShareServer: tray stop failed: {error}");
                    }
                });
            }
            // app.exit bypasses CloseRequested; window.close() would be turned
            // into hide-to-tray by the close handler.
            "exit" => app.exit(0),
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

/// Apply `[App Settings] WhenClose`: 0 stops the core and exits, non-zero hides
/// the window and keeps the server running.
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
                match when_close {
                    WhenClose::Minimize => hide_main_window(&app_handle),
                    WhenClose::Exit => {
                        let handle = app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            let controller = Arc::clone(&handle.state::<AppState>().controller);
                            if controller.status().state == ServerState::Running {
                                // Bounded so a wedged capture backend cannot trap
                                // the user in a closing app.
                                let _ =
                                    tokio::time::timeout(Duration::from_secs(3), controller.stop())
                                        .await;
                            }
                            handle.exit(0);
                        });
                    }
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
    Ok(bootstrap(
        &controller.settings(),
        &controller.status(),
        &state.startup_notices,
    ))
}

#[tauri::command]
async fn get_status(state: State<'_, AppState>) -> Result<ServerStatus, String> {
    Ok(state.controller.status())
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

#[tauri::command]
async fn hide_window(app: AppHandle) -> Result<(), String> {
    hide_main_window(&app);
    Ok(())
}

#[tauri::command]
async fn exit_app(app: AppHandle) -> Result<(), String> {
    let controller = {
        let state = app.state::<AppState>();
        Arc::clone(&state.controller)
    };
    // Best effort: close client sessions before the process exits. Bounded so
    // a wedged capture backend cannot trap the user in a closing app.
    if controller.status().state == ServerState::Running {
        let _ = tokio::time::timeout(Duration::from_secs(3), controller.stop()).await;
    }
    app.exit(0);
    Ok(())
}

fn main() {
    let options = GuiOptions::parse(std::env::args().skip(1));

    // Settings load and endpoint enumeration run before the Tauri UI exists;
    // the dedicated thread keeps audio discovery off the main/UI thread.
    let (settings, startup_notices, endpoints, config_path) =
        std::thread::spawn(move || load_startup(options))
            .join()
            .unwrap_or_else(|_| {
                (
                    Settings::default(),
                    vec!["Could not load settings: startup task failed".to_owned()],
                    None,
                    paths::paths().config,
                )
            });

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
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hide"]),
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .manage(AppState {
            controller: Arc::new(ServerController::with_config(settings, config_path)),
            options,
            endpoints: Arc::new(Mutex::new(endpoints)),
            startup_notices,
        })
        .invoke_handler(tauri::generate_handler![
            get_bootstrap,
            get_status,
            get_settings,
            save_settings,
            reset_settings,
            list_endpoints,
            start_server,
            stop_server,
            hide_window,
            exit_app
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
