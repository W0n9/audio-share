//! audio-share wire-protocol-v2 transport prototype — Android client native core.
//!
//! Throwaway code for ticket 01: validates quinn/rustls transport viability on
//! Android. NOT production code, NOT the normative v2 layout.

pub mod codec;
pub mod control;
pub mod jni;
pub mod media;
pub mod ring;
pub mod session;
pub mod stats;
pub mod verify;

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert!(true);
    }
}
