//! audio-share wire-protocol-v2 transport prototype — Android client native core.
//!
//! Throwaway code for ticket 01: validates quinn/rustls transport viability on
//! Android. NOT production code, NOT the normative v2 layout.

pub mod codec;
pub mod control;
pub mod media;
pub mod verify;

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert!(true);
    }
}
