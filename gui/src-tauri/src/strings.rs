//! UI strings shared by the Rust tray and the Svelte frontend.
//!
//! The frontend's `src/locales/en-US.json` is the single source of truth; it is
//! embedded at compile time so tray labels and webview text cannot drift.

use std::collections::HashMap;
use std::sync::OnceLock;

const EN_US: &str = include_str!("../../src/locales/en-US.json");

fn resources() -> &'static HashMap<String, String> {
    static RESOURCES: OnceLock<HashMap<String, String>> = OnceLock::new();
    RESOURCES.get_or_init(|| {
        serde_json::from_str(EN_US).expect("gui/src/locales/en-US.json must be a flat string map")
    })
}

/// Look up an English string by flat dotted key, returning the key itself when
/// it is missing.
pub fn t(key: &str) -> String {
    resources()
        .get(key)
        .cloned()
        .unwrap_or_else(|| key.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_labels_match_the_frontend_resource() {
        assert_eq!(t("app.title"), "Audio Share Server");
        assert_eq!(t("tray.show"), "Show");
        assert_eq!(t("tray.start"), "Start");
        assert_eq!(t("tray.stop"), "Stop");
        assert_eq!(t("tray.exit"), "Exit");
    }

    #[test]
    fn missing_keys_fall_back_to_the_key() {
        assert_eq!(t("no.such.key"), "no.such.key");
    }

    #[test]
    fn every_value_is_a_non_empty_string() {
        for (key, value) in resources() {
            assert!(!value.is_empty(), "empty value for {key}");
        }
    }
}
