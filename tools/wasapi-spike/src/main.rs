//! Windows WASAPI shared-mode loopback capture spike.
//!
//! The real tool is Windows-only. On other platforms this binary exists so that
//! a workspace-wide `cargo build` keeps working; it prints a clear error and
//! exits with status 1.

#[cfg(any(windows, test))]
mod logic;

#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    win::main();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("wasapi-spike: Windows only (WASAPI is not available on this platform)");
    std::process::exit(1);
}
