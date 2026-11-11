//! Shared primitives for the wire-protocol-v2 transport prototype.
//!
//! Throwaway code answering a transport question only; it does NOT freeze the
//! v2 normative wire layout. See the prototype brief
//! (`prototype/wire-protocol-v2-transport/BRIEF.md`).

pub mod frame;
pub mod media;
pub mod spki;
