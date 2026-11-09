//! Windows WASAPI shared-mode loopback capture.
//!
//! The platform-neutral format negotiation logic lives in [`format`] and is
//! compiled and unit-tested on every host. The COM capture backend is
//! Windows-only and lives in `backend`.
//!
//! All COM objects (`DeviceEnumerator`, `Device`, `AudioClient`,
//! `AudioCaptureClient`) are created, used, and dropped on one dedicated STA-free
//! MTA thread per operation. The exported [`WasapiCapture`] backend stores only
//! owned data, so it can be shared across threads.

// The pure rules are exercised by unit tests on every host; on non-Windows
// builds the Windows backend that consumes them is absent.
#[cfg_attr(not(windows), allow(dead_code))]
mod format;

#[cfg(windows)]
mod backend;

#[cfg(windows)]
pub use backend::WasapiCapture;
