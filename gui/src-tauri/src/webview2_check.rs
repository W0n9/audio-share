//! WebView2 runtime detection for Windows.
//!
//! Tauri checks for WebView2 only when it creates the first webview and, on a
//! release Windows build, that failure ends in an `expect()` panic. This
//! preflight runs before `tauri::Builder`: when the runtime is missing it shows
//! a Yes/No dialog that can open the Microsoft download page, then lets `main`
//! exit cleanly.
//!
//! The decision logic below is platform-neutral so it can be unit-tested with
//! injected registry/environment values on any host; only the registry read and
//! the native dialog are `#[cfg(windows)]`.
//!
//! Detection locations (Microsoft):
//! <https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution#detect-if-a-webview2-runtime-is-already-installed>

/// EdgeUpdate client GUID for the Evergreen WebView2 Runtime.
pub const WEBVIEW2_CLIENT_GUID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

/// Machine-wide client key (the x64 target reads the 32-bit view).
pub const MACHINE_KEY_PATH: &str =
    r"SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

/// Per-user client key written by a user-scoped runtime install.
pub const USER_KEY_PATH: &str =
    r"Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

/// Microsoft's WebView2 download page, opened from the missing-runtime dialog.
pub const DOWNLOAD_URL: &str = "https://developer.microsoft.com/en-us/microsoft-edge/webview2/";

/// Environment variable that points at a fixed-version runtime folder.
pub const BROWSER_EXECUTABLE_FOLDER_ENV: &str = "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER";

/// Where an installed runtime was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeSource {
    /// `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` with `msedgewebview2.exe` present.
    FixedVersion,
    /// `HKLM\SOFTWARE\WOW6432Node\...\EdgeUpdate\Clients\{GUID}`.
    Machine,
    /// `HKCU\Software\...\EdgeUpdate\Clients\{GUID}`.
    User,
}

/// A runtime that satisfies Tauri's requirement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebView2Runtime {
    /// `pv` value from the registry, or `"fixed-version"` for the env folder.
    pub version: String,
    pub source: RuntimeSource,
}

/// A `pv` value means "installed" unless it is missing, blank, or `0.0.0.0`.
fn usable_version(value: Option<&str>) -> Option<&str> {
    let value = value?.trim();
    (!value.is_empty() && value != "0.0.0.0").then_some(value)
}

/// Pure detection decision over injected values.
///
/// Precedence mirrors how a process resolves the runtime: an explicit
/// fixed-version folder first, then the machine-wide install, then the
/// per-user install.
pub fn decide(
    machine_pv: Option<&str>,
    user_pv: Option<&str>,
    fixed_version_exe: bool,
) -> Option<WebView2Runtime> {
    if fixed_version_exe {
        return Some(WebView2Runtime {
            version: "fixed-version".to_string(),
            source: RuntimeSource::FixedVersion,
        });
    }
    let (pv, source) = [
        (machine_pv, RuntimeSource::Machine),
        (user_pv, RuntimeSource::User),
    ]
    .into_iter()
    .find_map(|(pv, source)| usable_version(pv).map(|version| (version, source)))?;
    Some(WebView2Runtime {
        version: pv.to_string(),
        source,
    })
}

#[cfg(windows)]
fn pv_at(hive: winreg::HKEY, path: &str) -> Option<String> {
    use winreg::enums::KEY_READ;

    let key = winreg::RegKey::predef(hive)
        .open_subkey_with_flags(path, KEY_READ)
        .ok()?;
    key.get_value("pv").ok()
}

/// Returns the installed WebView2 runtime, if any.
#[cfg(windows)]
pub fn installed() -> Option<WebView2Runtime> {
    let fixed_version_exe = std::env::var_os(BROWSER_EXECUTABLE_FOLDER_ENV)
        .map(|folder| {
            std::path::Path::new(&folder)
                .join("msedgewebview2.exe")
                .is_file()
        })
        .unwrap_or(false);
    let machine = pv_at(winreg::enums::HKEY_LOCAL_MACHINE, MACHINE_KEY_PATH);
    let user = pv_at(winreg::enums::HKEY_CURRENT_USER, USER_KEY_PATH);

    decide(machine.as_deref(), user.as_deref(), fixed_version_exe)
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Shows the missing-runtime dialog; Yes opens the download page.
#[cfg(windows)]
fn show_missing_dialog() {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_ICONERROR, MB_YESNO, SW_SHOWNORMAL,
    };

    let text = wide(
        "Audio Share Server needs the Microsoft Edge WebView2 Runtime.\n\n\
         Click Yes to open the download page. Install the Evergreen Runtime, \
         then start Audio Share Server again.",
    );
    let title = wide("Audio Share Server");
    let operation = wide("open");
    let url = wide(DOWNLOAD_URL);

    // SAFETY: every pointer is a valid, null-terminated UTF-16 buffer that
    // outlives the call; a null owner window is explicitly allowed.
    unsafe {
        let answer = MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_YESNO | MB_ICONERROR,
        );
        if answer == IDYES {
            let _ = ShellExecuteW(
                None,
                PCWSTR(operation.as_ptr()),
                PCWSTR(url.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        }
    }
}

/// Returns `true` when Tauri may start. On a missing runtime, shows the dialog
/// (and optionally opens the download page) and returns `false`.
#[cfg(windows)]
pub fn preflight() -> bool {
    if installed().is_some() {
        return true;
    }
    show_missing_dialog();
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const MACHINE_VERSION: &str = "120.0.2210.91";
    const USER_VERSION: &str = "119.0.2151.58";

    #[test]
    fn missing_registry_values_mean_not_installed() {
        assert_eq!(decide(None, None, false), None);
    }

    #[test]
    fn machine_key_installs_the_runtime() {
        assert_eq!(
            decide(Some(MACHINE_VERSION), None, false),
            Some(WebView2Runtime {
                version: MACHINE_VERSION.to_string(),
                source: RuntimeSource::Machine,
            })
        );
    }

    #[test]
    fn user_key_installs_the_runtime_when_machine_key_is_missing() {
        assert_eq!(
            decide(None, Some(USER_VERSION), false),
            Some(WebView2Runtime {
                version: USER_VERSION.to_string(),
                source: RuntimeSource::User,
            })
        );
    }

    #[test]
    fn machine_key_wins_over_user_key() {
        let runtime = decide(Some(MACHINE_VERSION), Some(USER_VERSION), false).unwrap();
        assert_eq!(runtime.source, RuntimeSource::Machine);
        assert_eq!(runtime.version, MACHINE_VERSION);
    }

    #[test]
    fn zero_version_is_not_installed() {
        assert_eq!(decide(Some("0.0.0.0"), Some("0.0.0.0"), false), None);
    }

    #[test]
    fn zero_machine_version_falls_back_to_the_user_key() {
        let runtime = decide(Some("0.0.0.0"), Some(USER_VERSION), false).unwrap();
        assert_eq!(runtime.source, RuntimeSource::User);
        assert_eq!(runtime.version, USER_VERSION);
    }

    #[test]
    fn empty_or_blank_versions_are_not_installed() {
        assert_eq!(decide(Some(""), Some("   "), false), None);
    }

    #[test]
    fn surrounding_whitespace_is_ignored_and_trimmed() {
        let runtime = decide(Some(" 120.0.2210.91 "), None, false).unwrap();
        assert_eq!(runtime.version, MACHINE_VERSION);
    }

    #[test]
    fn fixed_version_folder_installs_the_runtime_without_registry_entries() {
        assert_eq!(
            decide(None, None, true),
            Some(WebView2Runtime {
                version: "fixed-version".to_string(),
                source: RuntimeSource::FixedVersion,
            })
        );
    }

    #[test]
    fn fixed_version_folder_wins_over_registry_entries() {
        let runtime = decide(Some(MACHINE_VERSION), Some(USER_VERSION), true).unwrap();
        assert_eq!(runtime.source, RuntimeSource::FixedVersion);
    }

    #[test]
    fn fixed_version_folder_without_the_executable_falls_back_to_registry() {
        let runtime = decide(Some(MACHINE_VERSION), None, false).unwrap();
        assert_eq!(runtime.source, RuntimeSource::Machine);
    }
}
