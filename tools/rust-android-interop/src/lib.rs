//! Test-only helpers for the Rust Server ↔ Android Client interop harness.
//!
//! Nothing here is compiled into either product: this crate exists solely to
//! hold the two real implementations in one host process so a test can encode
//! on one end and decode on the other.

use std::path::PathBuf;

/// Absolute path to the repository's frozen golden vectors.
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/wire-protocol-v2")
}

/// Load one `fixtures/wire-protocol-v2/media/*.json` vector.
pub fn media_fixture(id: &str) -> serde_json::Value {
    let path = fixtures_dir().join("media").join(format!("{id}.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).expect("media fixture"))
        .expect("media fixture json")
}

/// Load one `fixtures/wire-protocol-v2/control/*.json` vector.
pub fn control_fixture(id: &str) -> serde_json::Value {
    let path = fixtures_dir().join("control").join(format!("{id}.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).expect("control fixture"))
        .expect("control fixture json")
}

/// Decode a lowercase hex string into bytes.
pub fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "even-length hex");
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex byte"))
        .collect()
}

/// The `(datagram_hex, length)` pairs of every datagram step in a media vector.
pub fn media_datagrams(value: &serde_json::Value) -> Vec<(String, usize)> {
    value["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .filter(|step| step["kind"] == "datagram")
        .map(|step| {
            (
                step["datagram_hex"].as_str().expect("hex").to_owned(),
                step["length"].as_u64().expect("length") as usize,
            )
        })
        .collect()
}

/// The `(body_hex, frame_hex)` pairs of every control-frame step in a vector.
pub fn control_frames(value: &serde_json::Value) -> Vec<(String, String)> {
    value["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .filter(|step| step["kind"] == "frame")
        .map(|step| {
            (
                step["body_hex"].as_str().expect("body_hex").to_owned(),
                step["frame_hex"].as_str().expect("frame_hex").to_owned(),
            )
        })
        .collect()
}
