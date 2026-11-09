//! Library half of the Audio Share Server GUI shell.
//!
//! Everything here is testable without a display: the server controller and
//! its state machine, settings-to-core mapping, CLI option parsing, and the
//! shared UI strings. `main.rs` is the thin Tauri wiring around it.

pub mod autostart;
pub mod controller;
pub mod help;
pub mod i18n;
pub mod maintenance;
pub mod model;
pub mod options;
pub mod strings;
pub mod update;
pub mod webview2_check;

#[cfg(windows)]
pub mod windows_maintenance;
