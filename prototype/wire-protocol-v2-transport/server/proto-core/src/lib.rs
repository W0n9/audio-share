//! Shared primitives for the wire-protocol-v2 transport prototype.
//!
//! Throwaway code answering a transport question only; it does NOT freeze the
//! v2 normative wire layout. See `.scratch/wire-protocol-v2/prototype-brief.md`.

pub mod frame;
pub mod media;
pub mod spki;
