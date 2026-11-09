//! Server Core for Audio Share.
//!
//! This crate owns the frozen v0.3.4 Wire Protocol types, framing helpers,
//! MTU segmentation, capture backends, and the TCP/UDP server. `as-cmd`, the
//! Server GUI, and the protocol harness all link it directly.

pub mod capture;
pub mod error;
pub mod protocol;
pub mod server;

pub use error::{Error, Result};
