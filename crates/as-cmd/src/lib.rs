//! v2 控制平面与共享 capture 的生产接线。
//!
//! These adapters live in `v2-runtime` because the Server GUI and `as-cmd`
//! host the same v2 runtime. The re-exports keep the historical `as_cmd::*`
//! paths used by the process-level tests and the Rust↔Android interop harness.

pub use v2_runtime::{
    capture_media_source, v2_capture_snapshot, v2_capture_source, CaptureMediaSource,
};
