#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri shell for the Audio Share Server GUI.
//!
//! The window, tray, and commands are thin adapters over the display-free
//! `ServerController` in the library half of this crate. Ticket 08 keeps
//! settings in memory; tickets 09/10 add persistence, lifecycle policy,
//! autostart UI, and update checks at the marked extension points.

use std::sync::Arc;
use std::time::Duration;

use audio_share_core::capture::{platform_backend, synthetic_backend, CaptureBackend};
use audio_share_server_gui::controller::ServerController;
use audio_share_server_gui::model::{
    bootstrap, tray_enabled, Bootstrap, ServerState, ServerStatus, Settings,
};
use audio_share_server_gui::options::GuiOptions;
use audio_share_server_gui::strings::t;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent,
};

/// State managed by Tauri and reached from commands and tray callbacks.
struct AppState {
    controller: Arc<ServerController>,
    options: GuiOptions,
}

/// Tray menu items kept in managed state so status changes can toggle them.
struct TrayItems {
    start: MenuItem<tauri::Wry>,
    stop: MenuItem<tauri::Wry>,
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

/// Build the capture backend for a start request. `--fake-capture` is the
/// hardware-free debug path; normal runs use the platform backend.
fn capture_backend(options: &GuiOptions) -> Result<Arc<dyn CaptureBackend>, String> {
    if options.fake_capture {
        synthetic_backend()
            .map(|backend| Arc::new(backend) as Arc<dyn CaptureBackend>)
            .map_err(|error| error.to_string())
    } else {
        platform_backend().map_err(|error| error.to_string())
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

fn install_close_handler(app: &tauri::App) {
    if let Some(window) = app.get_webview_window("main") {
        let window_for_event = window.clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Ticket 08 default: close hides to the tray and the server
                // keeps running. Ticket 09 branches here on the onClose
                // setting (Exit vs Minimize to tray).
                api.prevent_close();
                let _ = window_for_event.hide();
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
    Ok(bootstrap(&controller.settings(), &controller.status()))
}

#[tauri::command]
async fn get_status(state: State<'_, AppState>) -> Result<ServerStatus, String> {
    Ok(state.controller.status())
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
    let settings = Settings::default();

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
            controller: Arc::new(ServerController::new(settings)),
            options,
        })
        .invoke_handler(tauri::generate_handler![
            get_bootstrap,
            get_status,
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
