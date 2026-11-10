//! Linux help links shown in place of the Windows-only maintenance actions.
//!
//! Repair Firewall and the Sound Panel do not exist on Linux. The Server GUI
//! offers documentation instead, opened through the Tauri opener plugin. The
//! topic-to-URL mapping lives here so it is unit-tested without a display and
//! the native command can reject arbitrary URLs from the webview.

/// Topic identifier used by the frontend for the PipeWire documentation.
pub const PIPEWIRE_ID: &str = "pipewire";

/// Topic identifier used by the frontend for desktop sound settings help.
pub const DESKTOP_SOUND_ID: &str = "desktopSound";

/// PipeWire documentation entry point.
pub const PIPEWIRE_DOCS_URL: &str = "https://docs.pipewire.org/";

/// Desktop sound settings guidance (GNOME Help, Ubuntu's default desktop).
pub const DESKTOP_SOUND_HELP_URL: &str =
    "https://help.gnome.org/users/gnome-help/stable/sound-volume.html";

/// Documentation topics offered by the Linux settings view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpTopic {
    /// PipeWire documentation.
    PipeWire,
    /// Desktop sound settings guidance.
    DesktopSound,
}

impl HelpTopic {
    /// Every topic, in display order.
    pub const ALL: [HelpTopic; 2] = [HelpTopic::PipeWire, HelpTopic::DesktopSound];

    /// The stable identifier exchanged with the frontend.
    pub const fn id(self) -> &'static str {
        match self {
            Self::PipeWire => PIPEWIRE_ID,
            Self::DesktopSound => DESKTOP_SOUND_ID,
        }
    }

    /// Resolve a frontend identifier; unknown values are rejected.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            PIPEWIRE_ID => Some(Self::PipeWire),
            DESKTOP_SOUND_ID => Some(Self::DesktopSound),
            _ => None,
        }
    }

    /// The documentation URL opened by the opener plugin.
    pub const fn url(self) -> &'static str {
        match self {
            Self::PipeWire => PIPEWIRE_DOCS_URL,
            Self::DesktopSound => DESKTOP_SOUND_HELP_URL,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_maps_to_its_documentation_url() {
        assert_eq!(
            HelpTopic::PipeWire.url(),
            "https://docs.pipewire.org/",
            "PipeWire documentation URL"
        );
        assert_eq!(
            HelpTopic::DesktopSound.url(),
            "https://help.gnome.org/users/gnome-help/stable/sound-volume.html",
            "desktop sound settings help URL"
        );
    }

    #[test]
    fn frontend_identifiers_round_trip() {
        for topic in HelpTopic::ALL {
            assert_eq!(HelpTopic::parse(topic.id()), Some(topic));
        }
        assert_eq!(
            HelpTopic::ALL.map(HelpTopic::id),
            ["pipewire", "desktopSound"]
        );
    }

    #[test]
    fn unknown_or_arbitrary_topics_are_rejected() {
        for value in [
            "",
            "PipeWire",
            "pipewire ",
            "desktop-sound",
            "default",
            "https://example.test/",
        ] {
            assert_eq!(HelpTopic::parse(value), None, "topic {value:?}");
        }
    }

    #[test]
    fn urls_are_https_documentation_links_without_query_strings() {
        for topic in HelpTopic::ALL {
            let url = topic.url();
            assert!(url.starts_with("https://"), "{url}");
            assert!(!url.contains('?'), "unexpected query string in {url}");
            assert!(!url.contains('#'), "unexpected fragment in {url}");
            assert!(!url.contains(' '), "unexpected space in {url}");
        }
        let urls: Vec<&str> = HelpTopic::ALL.iter().map(|topic| topic.url()).collect();
        let mut unique = urls.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), urls.len(), "duplicate help URLs");
    }
}
