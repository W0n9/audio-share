//! Core error type.
//!
//! The core never exits the process and never panics on expected failures:
//! fallible public APIs return [`Result`] so the Server GUI and `as-cmd` can
//! report problems without killing their process.

use std::fmt;

use crate::protocol::ProtocolError;

/// Result alias used by every fallible core API.
pub type Result<T> = std::result::Result<T, Error>;

/// A recoverable Server Core failure.
#[derive(Debug)]
pub enum Error {
    /// A capture backend rejected its configuration or failed to start.
    Capture(String),
    /// A configuration value is unusable (bad address, zero block alignment).
    InvalidConfig(String),
    /// A host name could not be resolved to a socket address.
    Resolve(String),
    /// An operating-system operation failed.
    Io {
        /// What the core was doing, e.g. `bind TCP listener`.
        operation: &'static str,
        /// The underlying OS error.
        source: std::io::Error,
    },
    /// A Wire Protocol frame was malformed.
    Protocol(ProtocolError),
}

impl Error {
    /// Build an [`Error::Io`] with human-readable context.
    pub(crate) fn io(operation: &'static str, source: std::io::Error) -> Self {
        Self::Io { operation, source }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Capture(message) => write!(f, "capture failed: {message}"),
            Self::InvalidConfig(message) => write!(f, "invalid configuration: {message}"),
            Self::Resolve(message) => write!(f, "cannot resolve address: {message}"),
            Self::Io { operation, source } => write!(f, "{operation}: {source}"),
            Self::Protocol(error) => write!(f, "wire protocol error: {error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Protocol(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Self::io("I/O error", source)
    }
}

impl From<ProtocolError> for Error {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}
