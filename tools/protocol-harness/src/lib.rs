//! Scripted Wire Protocol client, fixture recorder/replayer, and fake v0.3.4
//! server for hardware-free tests.
//!
//! The binary in `src/main.rs` is a thin CLI over these modules; the library
//! target lets integration tests drive the same replay path against `as-cmd`.

pub mod fake_server;
pub mod fixture;
pub mod process;
pub mod record;
pub mod replay;
pub mod synth;
