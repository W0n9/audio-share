//! Server Core for Audio Share.
//!
//! This crate owns the frozen v0.3.4 Wire Protocol types, framing helpers, and
//! MTU segmentation. Capture and the live network server arrive in a later
//! ticket; the protocol surface here is shared by `as-cmd`, the Server GUI, and
//! the protocol harness.

pub mod protocol;
