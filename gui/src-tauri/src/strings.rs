//! UI strings shared by the Rust tray/notifications and the Svelte frontend.
//!
//! `gui/src/locales/en-US.json` and `zh-CN.json` are the single source of
//! truth; both are embedded at compile time so every language the webview can
//! show is also available to the native shell. The startup language is
//! resolved once in `main` before the tray is built.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use crate::i18n::Locale;

const EN_US: &str = include_str!("../../src/locales/en-US.json");
const ZH_CN: &str = include_str!("../../src/locales/zh-CN.json");

fn resources(locale: Locale) -> &'static HashMap<String, String> {
    static EN_US_RESOURCES: OnceLock<HashMap<String, String>> = OnceLock::new();
    static ZH_CN_RESOURCES: OnceLock<HashMap<String, String>> = OnceLock::new();
    match locale {
        Locale::EnUs => EN_US_RESOURCES.get_or_init(|| parse(EN_US, "en-US.json")),
        Locale::ZhCn => ZH_CN_RESOURCES.get_or_init(|| parse(ZH_CN, "zh-CN.json")),
    }
}

fn parse(source: &str, name: &str) -> HashMap<String, String> {
    serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("gui/src/locales/{name} must be a flat string map: {error}"))
}

static CURRENT_LOCALE: AtomicU8 = AtomicU8::new(0);

/// Select the language used by [`t`] and [`t_args`].
pub fn set_locale(locale: Locale) {
    CURRENT_LOCALE.store(
        match locale {
            Locale::EnUs => 0,
            Locale::ZhCn => 1,
        },
        Ordering::SeqCst,
    );
}

/// The currently selected native-shell language.
pub fn locale() -> Locale {
    match CURRENT_LOCALE.load(Ordering::SeqCst) {
        1 => Locale::ZhCn,
        _ => Locale::EnUs,
    }
}

/// Look up a string in the selected language, falling back to English and then
/// to the key itself when it is missing.
pub fn t(key: &str) -> String {
    t_for(locale(), key)
}

/// Look up a string in an explicit language.
pub fn t_for(locale: Locale, key: &str) -> String {
    resources(locale)
        .get(key)
        .or_else(|| resources(Locale::EnUs).get(key))
        .cloned()
        .unwrap_or_else(|| key.to_owned())
}

/// Look up a string and replace `{name}` placeholders.
pub fn t_args(key: &str, args: &[(&str, &str)]) -> String {
    let template = t(key);
    interpolate(&template, args)
}

/// Look up a string in an explicit language and interpolate placeholders.
pub fn t_args_for(locale: Locale, key: &str, args: &[(&str, &str)]) -> String {
    interpolate(&t_for(locale, key), args)
}

fn interpolate(template: &str, args: &[(&str, &str)]) -> String {
    let mut result = template.to_owned();
    for (name, value) in args {
        result = result.replace(&format!("{{{name}}}"), value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_languages_are_key_complete() {
        let english = resources(Locale::EnUs);
        let chinese = resources(Locale::ZhCn);
        let mut english_keys: Vec<&String> = english.keys().collect();
        let mut chinese_keys: Vec<&String> = chinese.keys().collect();
        english_keys.sort();
        chinese_keys.sort();
        assert_eq!(english_keys, chinese_keys, "locale key sets must match");
        assert!(!english_keys.is_empty());
    }

    #[test]
    fn every_value_is_a_non_empty_string() {
        for locale in [Locale::EnUs, Locale::ZhCn] {
            for (key, value) in resources(locale) {
                assert!(!value.is_empty(), "empty value for {key} in {locale:?}");
            }
        }
    }

    #[test]
    fn tray_labels_match_the_frontend_resource() {
        assert_eq!(t_for(Locale::EnUs, "app.title"), "Audio Share Server");
        assert_eq!(t_for(Locale::EnUs, "tray.show"), "Show");
        assert_eq!(t_for(Locale::EnUs, "tray.start"), "Start");
        assert_eq!(t_for(Locale::EnUs, "tray.stop"), "Stop");
        assert_eq!(t_for(Locale::EnUs, "tray.exit"), "Exit");
        assert_eq!(t_for(Locale::ZhCn, "tray.show"), "显示");
        assert_eq!(t_for(Locale::ZhCn, "tray.exit"), "退出");
    }

    #[test]
    fn missing_keys_fall_back_to_the_key() {
        assert_eq!(t_for(Locale::EnUs, "no.such.key"), "no.such.key");
        assert_eq!(t_for(Locale::ZhCn, "no.such.key"), "no.such.key");
    }

    #[test]
    fn placeholders_are_interpolated() {
        assert_eq!(
            t_args_for(Locale::EnUs, "update.available", &[("version", "v0.4.1")]),
            "A new version is available: v0.4.1"
        );
        assert_eq!(
            t_args_for(Locale::ZhCn, "update.available", &[("version", "v0.4.1")]),
            "有新版本可用：v0.4.1"
        );
    }

    #[test]
    fn locale_selection_affects_t() {
        // This test owns the global selection only briefly; assertions compare
        // explicit-language lookups so parallel tests cannot observe it.
        set_locale(Locale::ZhCn);
        assert_eq!(t("status.running"), "服务器运行中");
        set_locale(Locale::EnUs);
        assert_eq!(t("status.running"), "Server running");
    }
}
