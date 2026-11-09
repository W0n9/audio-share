//! GUI command-line options. Pure parsing so it can be unit-tested without
//! starting Tauri.

/// Debug/headless flags accepted by the GUI binary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GuiOptions {
    /// Use deterministic synthetic capture instead of the platform backend.
    pub fake_capture: bool,
    /// Start with the window hidden (autostart path).
    pub hide: bool,
    /// Headless verification mode: log window state and exit 0.
    pub smoke_test: bool,
}

impl GuiOptions {
    /// Parse process arguments, ignoring the executable name and unknown flags.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut options = Self::default();
        for arg in args {
            match arg.as_str() {
                "--fake-capture" => options.fake_capture = true,
                "--smoke-test" => options.smoke_test = true,
                "--hide" | "-hide" | "/hide" => options.hide = true,
                _ => {}
            }
        }
        options
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> GuiOptions {
        GuiOptions::parse(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn no_flags_means_a_normal_visible_run() {
        assert_eq!(parse(&[]), GuiOptions::default());
        assert_eq!(parse(&["--unknown", "value"]), GuiOptions::default());
    }

    #[test]
    fn recognizes_fake_capture_and_smoke_test() {
        let options = parse(&["--fake-capture", "--smoke-test"]);
        assert!(options.fake_capture);
        assert!(options.smoke_test);
        assert!(!options.hide);
    }

    #[test]
    fn recognizes_all_hide_spellings() {
        for spelling in ["--hide", "-hide", "/hide"] {
            assert!(parse(&[spelling]).hide, "{spelling} must hide the window");
        }
    }

    #[test]
    fn parsing_does_not_confuse_prefixes() {
        let options = parse(&["--hide-window", "--fake-capture=1"]);
        assert!(!options.hide);
        assert!(!options.fake_capture);
    }
}
