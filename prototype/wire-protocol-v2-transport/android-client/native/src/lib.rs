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

/// android_logger init (device only). Called from createSession.
#[cfg(target_os = "android")]
pub fn init_android_logger() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Info),
        );
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert!(true);
    }
}
