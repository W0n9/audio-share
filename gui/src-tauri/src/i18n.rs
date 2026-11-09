//! Interface language selection shared by the webview, tray, and notifications.
//!
//! `[App] language` holds `default`, `en-US`, or `zh-CN`. It is read once at
//! startup; the UI does not switch live. `default` follows the system locale:
//! any `zh*` locale uses Simplified Chinese, everything else uses English.

use serde::{Deserialize, Serialize};

/// A shipped interface language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Locale {
    /// English (United States).
    #[serde(rename = "en-US")]
    EnUs,
    /// Simplified Chinese (China).
    #[serde(rename = "zh-CN")]
    ZhCn,
}

impl Locale {
    /// The BCP 47 tag used on the wire and in the settings file.
    pub const fn tag(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::ZhCn => "zh-CN",
        }
    }

    /// Parse an exact configured/resolved tag.
    pub fn parse(tag: &str) -> Option<Self> {
        match tag.to_ascii_lowercase().as_str() {
            "en-us" => Some(Self::EnUs),
            "zh-cn" => Some(Self::ZhCn),
            _ => None,
        }
    }
}

/// Resolve `[App] language` against the system locale.
///
/// Recognized explicit values win. `default`, empty, and unknown values follow
/// the system locale (matching the old "unknown resources fall back" behavior
/// without letting a corrupt value hide the interface language).
pub fn resolve_language(setting: &str, system_locale: Option<&str>) -> Locale {
    if let Some(locale) = Locale::parse(setting.trim()) {
        return locale;
    }
    match system_locale {
        Some(locale) if locale.to_ascii_lowercase().starts_with("zh") => Locale::ZhCn,
        _ => Locale::EnUs,
    }
}

/// The current system locale via the platform API/environment.
pub fn system_locale() -> Option<String> {
    sys_locale::get_locale()
}

/// Resolve the language for a settings value against the current system.
pub fn resolve_configured(setting: &str) -> Locale {
    resolve_language(setting, system_locale().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_follows_chinese_system_locales() {
        for system in ["zh-CN", "zh-Hans-CN", "zh_CN.UTF-8", "zh-TW", "zh"] {
            assert_eq!(
                resolve_language("default", Some(system)),
                Locale::ZhCn,
                "system locale {system}"
            );
        }
    }

    #[test]
    fn default_uses_english_for_everything_else_and_missing_locales() {
        for system in ["en-US", "en-GB", "fr-FR", "ja-JP", ""] {
            assert_eq!(
                resolve_language("default", Some(system)),
                Locale::EnUs,
                "system locale {system}"
            );
        }
        assert_eq!(resolve_language("default", None), Locale::EnUs);
    }

    #[test]
    fn explicit_languages_win_over_the_system_locale() {
        assert_eq!(resolve_language("zh-CN", Some("en-US")), Locale::ZhCn);
        assert_eq!(resolve_language("en-US", Some("zh-CN")), Locale::EnUs);
        assert_eq!(resolve_language(" ZH-cn ", Some("en-US")), Locale::ZhCn);
        assert_eq!(resolve_language("En-Us", Some("zh-CN")), Locale::EnUs);
    }

    #[test]
    fn unknown_values_follow_the_system_locale() {
        assert_eq!(resolve_language("fr-FR", Some("zh-Hans")), Locale::ZhCn);
        assert_eq!(resolve_language("", Some("en-US")), Locale::EnUs);
        assert_eq!(resolve_language("not-a-locale", None), Locale::EnUs);
    }

    #[test]
    fn tags_round_trip() {
        for locale in [Locale::EnUs, Locale::ZhCn] {
            assert_eq!(Locale::parse(locale.tag()), Some(locale));
        }
    }
}
