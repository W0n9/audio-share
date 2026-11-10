//! Windows maintenance logic that is unit-tested on every host.
//!
//! Ticket 11 repairs the inbound firewall rules with an elevated PowerShell
//! script and opens the Windows Sound control panel. The script generation,
//! profile-mask conversion, and error classification are pure so they are
//! tested here on Linux; the COM/ShellExecute runtime lives in
//! `windows_maintenance.rs` behind `#[cfg(windows)]`.
//!
//! Firewall shape (v0.3.4 legacy Windows GUI parity):
//! - one application-scoped inbound allow rule per protocol, named
//!   `Audio Share Server(TCP-In)` and `Audio Share Server(UDP-In)`;
//! - profiles are the currently active profiles, with Public removed unless
//!   Public is the only active one;
//! - the old rules are deleted before the new ones are added.

use crate::i18n::Locale;
use crate::strings;

/// Inbound TCP rule name, identical to the legacy Windows GUI.
pub const TCP_RULE_NAME: &str = "Audio Share Server(TCP-In)";
/// Inbound UDP rule name, identical to the legacy Windows GUI.
pub const UDP_RULE_NAME: &str = "Audio Share Server(UDP-In)";

/// `NET_FW_PROFILE2_DOMAIN`.
pub const PROFILE_DOMAIN: i32 = 1;
/// `NET_FW_PROFILE2_PRIVATE`.
pub const PROFILE_PRIVATE: i32 = 2;
/// `NET_FW_PROFILE2_PUBLIC`.
pub const PROFILE_PUBLIC: i32 = 4;

const ALL_PROFILES: i32 = PROFILE_DOMAIN | PROFILE_PRIVATE | PROFILE_PUBLIC;

/// `HRESULT_FROM_WIN32(ERROR_CANCELLED)` (1223), returned by
/// `ShellExecuteExW` when the user dismisses the UAC prompt.
pub const HRESULT_UAC_CANCELLED: i32 = 0x8007_04C7u32 as i32;

/// Everything that can stop a Windows maintenance action, kept display-free so
/// the command layer can localize it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaintenanceError {
    /// The command exists but this build is not for Windows.
    UnsupportedPlatform,
    /// The running executable path could not be read.
    AppPathUnavailable(String),
    /// A path cannot be embedded in a script safely.
    UnsafePath(String),
    /// A profile list outside `domain,private,public` was requested.
    InvalidProfiles(String),
    /// The temporary script could not be created.
    TempScript(String),
    /// `INetFwPolicy2::CurrentProfileTypes` failed.
    ProfileQuery(String),
    /// The user dismissed the UAC prompt.
    ElevationCancelled,
    /// The elevated process could not be started.
    ElevationFailed(String),
    /// The elevated script exited non-zero (a netsh failure).
    NetshFailed { exit_code: u32 },
    /// Waiting for the elevated process failed.
    WaitFailed(String),
    /// `control.exe` could not be spawned.
    SoundPanelFailed(String),
}

impl MaintenanceError {
    /// Render the error in the UI language.
    pub fn localized(&self, locale: Locale) -> String {
        match self {
            Self::UnsupportedPlatform => strings::t_for(locale, "error.maintenance.unsupported"),
            Self::AppPathUnavailable(message) | Self::UnsafePath(message) => {
                strings::t_args_for(locale, "error.firewall.path", &[("message", message)])
            }
            Self::InvalidProfiles(profiles) => {
                strings::t_args_for(locale, "error.firewall.profiles", &[("profiles", profiles)])
            }
            Self::TempScript(message) => {
                strings::t_args_for(locale, "error.firewall.script", &[("message", message)])
            }
            Self::ProfileQuery(message) => {
                strings::t_args_for(locale, "error.firewall.profile", &[("message", message)])
            }
            Self::ElevationCancelled => strings::t_for(locale, "error.firewall.cancelled"),
            Self::ElevationFailed(message) => {
                strings::t_args_for(locale, "error.firewall.elevation", &[("message", message)])
            }
            Self::NetshFailed { exit_code } => strings::t_args_for(
                locale,
                "error.firewall.netsh",
                &[("code", &exit_code.to_string())],
            ),
            Self::WaitFailed(message) => {
                strings::t_args_for(locale, "error.firewall.wait", &[("message", message)])
            }
            Self::SoundPanelFailed(message) => {
                strings::t_args_for(locale, "error.soundPanel", &[("message", message)])
            }
        }
    }
}

/// Convert `INetFwPolicy2::CurrentProfileTypes` into netsh's `profile=` value.
///
/// The legacy Windows GUI xor-ed Public out of the mask unless Public was the
/// only active profile; an empty or unknown-only mask falls back to all
/// profiles so the rule is never silently inactive.
pub fn netsh_profiles(active_mask: i32) -> String {
    let mut mask = active_mask & ALL_PROFILES;
    // MFC parity: Public applies only when it is the only active profile.
    if mask != PROFILE_PUBLIC {
        mask &= !PROFILE_PUBLIC;
    }
    if mask == 0 {
        // No recognized active profile: a rule that matches all profiles is
        // better than one that never applies.
        mask = ALL_PROFILES;
    }
    let mut names = Vec::with_capacity(3);
    if mask & PROFILE_DOMAIN != 0 {
        names.push("domain");
    }
    if mask & PROFILE_PRIVATE != 0 {
        names.push("private");
    }
    if mask & PROFILE_PUBLIC != 0 {
        names.push("public");
    }
    names.join(",")
}

/// Quote a value as a PowerShell single-quoted literal.
fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn safe_path(value: &str) -> Result<(), MaintenanceError> {
    if value.is_empty() {
        return Err(MaintenanceError::AppPathUnavailable(
            "the path is empty".to_owned(),
        ));
    }
    if value.contains(['\0', '\r', '\n']) {
        return Err(MaintenanceError::UnsafePath(
            "the path contains a line break or NUL".to_owned(),
        ));
    }
    Ok(())
}

/// Reject anything that is not a unique comma-separated subset of
/// `domain,private,public`; the value is interpolated into the script.
fn validate_profiles(profiles: &str) -> Result<(), MaintenanceError> {
    let mut seen = [false; 3];
    for name in profiles.split(',') {
        let index = match name {
            "domain" => 0,
            "private" => 1,
            "public" => 2,
            _ => return Err(MaintenanceError::InvalidProfiles(profiles.to_owned())),
        };
        if seen[index] {
            return Err(MaintenanceError::InvalidProfiles(profiles.to_owned()));
        }
        seen[index] = true;
    }
    if seen.iter().all(|seen| !seen) {
        return Err(MaintenanceError::InvalidProfiles(profiles.to_owned()));
    }
    Ok(())
}

/// Build the elevated PowerShell repair script.
///
/// The script deletes both rules (ignoring a missing rule), then adds the TCP
/// and UDP rules; a netsh failure exits with 10 (TCP) or 11 (UDP).
pub fn powershell_script(exe_path: &str, profiles: &str) -> Result<String, MaintenanceError> {
    safe_path(exe_path)?;
    validate_profiles(profiles)?;
    let exe = ps_quote(exe_path);
    let tcp = ps_quote(TCP_RULE_NAME);
    let udp = ps_quote(UDP_RULE_NAME);
    // PowerShell single-quoted literals keep spaces, `%`, `$`, `&`, and
    // embedded double quotes literal; only `'` needs doubling (done above).
    Ok(format!(
        "# Generated by Audio Share Server: rebuild this application's inbound firewall rules.\n\
         $ErrorActionPreference = 'Continue'\n\
         & netsh advfirewall firewall delete rule name={tcp} 2>$null | Out-Null\n\
         & netsh advfirewall firewall delete rule name={udp} 2>$null | Out-Null\n\
         & netsh advfirewall firewall add rule name={tcp} description={tcp} dir=in action=allow protocol=TCP program={exe} enable=yes profile={profiles}\n\
         if ($LASTEXITCODE -ne 0) {{ exit 10 }}\n\
         & netsh advfirewall firewall add rule name={udp} description={udp} dir=in action=allow protocol=UDP program={exe} enable=yes profile={profiles}\n\
         if ($LASTEXITCODE -ne 0) {{ exit 11 }}\n\
         exit 0\n"
    ))
}

/// Command-line arguments passed to `powershell.exe` through
/// `ShellExecuteExW`. The temporary path must be quoted because the user's
/// temp directory can contain spaces.
pub fn powershell_args(script_path: &str) -> Result<String, MaintenanceError> {
    safe_path(script_path)?;
    if script_path.contains('"') {
        return Err(MaintenanceError::UnsafePath(
            "the path contains a double quote".to_owned(),
        ));
    }
    Ok(format!(
        "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"{script_path}\""
    ))
}

/// Map the HRESULT from `ShellExecuteExW` to a localized-friendly error.
pub fn classify_shell_error(code: i32, message: String) -> MaintenanceError {
    if code == HRESULT_UAC_CANCELLED {
        MaintenanceError::ElevationCancelled
    } else {
        MaintenanceError::ElevationFailed(message)
    }
}

/// Map the elevated script's exit code: zero is success, anything else is a
/// netsh failure.
pub fn classify_exit_code(code: u32) -> Result<(), MaintenanceError> {
    if code == 0 {
        Ok(())
    } else {
        Err(MaintenanceError::NetshFailed { exit_code: code })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_profiles_keep_public_only_when_it_is_the_only_profile() {
        assert_eq!(netsh_profiles(PROFILE_DOMAIN), "domain");
        assert_eq!(netsh_profiles(PROFILE_PRIVATE), "private");
        assert_eq!(netsh_profiles(PROFILE_PUBLIC), "public");
        assert_eq!(
            netsh_profiles(PROFILE_DOMAIN | PROFILE_PRIVATE),
            "domain,private"
        );
        assert_eq!(netsh_profiles(PROFILE_DOMAIN | PROFILE_PUBLIC), "domain");
        assert_eq!(netsh_profiles(PROFILE_PRIVATE | PROFILE_PUBLIC), "private");
        assert_eq!(
            netsh_profiles(PROFILE_DOMAIN | PROFILE_PRIVATE | PROFILE_PUBLIC),
            "domain,private"
        );
    }

    #[test]
    fn an_empty_or_unknown_profile_mask_falls_back_to_all_profiles() {
        assert_eq!(netsh_profiles(0), "domain,private,public");
        assert_eq!(netsh_profiles(1 << 8), "domain,private,public");
        assert_eq!(netsh_profiles(-1), "domain,private");
    }

    #[test]
    fn script_removes_old_rules_then_adds_tcp_and_udp_rules() {
        let script = powershell_script(
            r"C:\Program Files\Audio Share\AudioShareServer.exe",
            "domain,private",
        )
        .expect("a normal path must produce a script");

        let tcp_delete = script
            .find(&format!("delete rule name='{TCP_RULE_NAME}'"))
            .expect("TCP rule delete");
        let udp_delete = script
            .find(&format!("delete rule name='{UDP_RULE_NAME}'"))
            .expect("UDP rule delete");
        let tcp_add = script
            .find(&format!("add rule name='{TCP_RULE_NAME}'"))
            .expect("TCP rule add");
        let udp_add = script
            .find(&format!("add rule name='{UDP_RULE_NAME}'"))
            .expect("UDP rule add");

        assert!(tcp_delete < tcp_add, "TCP delete must precede TCP add");
        assert!(udp_delete < udp_add, "UDP delete must precede UDP add");
        assert!(
            script.contains(r"program='C:\Program Files\Audio Share\AudioShareServer.exe'"),
            "the executable must stay one quoted argument:\n{script}"
        );
        assert!(script.contains("dir=in action=allow protocol=TCP"));
        assert!(script.contains("dir=in action=allow protocol=UDP"));
        assert!(
            script.contains(&format!("description='{TCP_RULE_NAME}'")),
            "the rule description matches the legacy Windows GUI:\n{script}"
        );
        assert!(script.contains("enable=yes profile=domain,private"));
        assert!(script.trim_end().ends_with("exit 0"));
    }

    #[test]
    fn script_uses_powershell_single_quote_escaping() {
        let script = powershell_script(r#"C:\Users\O'Brien\"odd"\AudioShareServer.exe"#, "public")
            .expect("quotes in a path are representable");
        assert!(
            script.contains(r#"program='C:\Users\O''Brien\"odd"\AudioShareServer.exe'"#),
            "single quotes must be doubled and double quotes are literal:\n{script}"
        );
    }

    #[test]
    fn script_rejects_empty_multiline_or_untrusted_input() {
        assert!(matches!(
            powershell_script("", "domain"),
            Err(MaintenanceError::AppPathUnavailable(_))
        ));
        assert!(matches!(
            powershell_script("C:\\a\nb.exe", "domain"),
            Err(MaintenanceError::UnsafePath(_))
        ));
        assert!(matches!(
            powershell_script("C:\\a\rb.exe", "domain"),
            Err(MaintenanceError::UnsafePath(_))
        ));
        assert!(matches!(
            powershell_script("C:\\app.exe", "domain; Remove-Item C:\\"),
            Err(MaintenanceError::InvalidProfiles(_))
        ));
        assert!(matches!(
            powershell_script("C:\\app.exe", "public,public"),
            Err(MaintenanceError::InvalidProfiles(_))
        ));
    }

    #[test]
    fn powershell_args_quote_a_temporary_path_with_spaces() {
        assert_eq!(
            powershell_args(r"C:\Users\A B\AppData\Local\Temp\audio-share-fw.ps1").unwrap(),
            r#"-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "C:\Users\A B\AppData\Local\Temp\audio-share-fw.ps1""#
        );
        assert!(matches!(
            powershell_args("C:\\bad\"path.ps1"),
            Err(MaintenanceError::UnsafePath(_))
        ));
        assert!(matches!(
            powershell_args("C:\\bad\npath.ps1"),
            Err(MaintenanceError::UnsafePath(_))
        ));
    }

    #[test]
    fn uac_cancel_maps_to_the_cancelled_error() {
        assert_eq!(
            classify_shell_error(HRESULT_UAC_CANCELLED, "cancelled".to_owned()),
            MaintenanceError::ElevationCancelled
        );
        assert_eq!(
            classify_shell_error(0x8007_0005u32 as i32, "access denied".to_owned()),
            MaintenanceError::ElevationFailed("access denied".to_owned())
        );
    }

    #[test]
    fn netsh_exit_codes_map_to_a_failure_with_the_code() {
        assert_eq!(classify_exit_code(0), Ok(()));
        assert_eq!(
            classify_exit_code(10),
            Err(MaintenanceError::NetshFailed { exit_code: 10 })
        );
        assert_eq!(
            classify_exit_code(11),
            Err(MaintenanceError::NetshFailed { exit_code: 11 })
        );
    }

    #[test]
    fn errors_are_localized_in_both_languages() {
        assert_eq!(
            MaintenanceError::ElevationCancelled.localized(Locale::EnUs),
            "Firewall repair was cancelled because administrator permission was not granted."
        );
        assert_eq!(
            MaintenanceError::ElevationCancelled.localized(Locale::ZhCn),
            "防火墙修复已取消：未授予管理员权限。"
        );
        let netsh = MaintenanceError::NetshFailed { exit_code: 10 }.localized(Locale::EnUs);
        assert!(netsh.contains("10"), "localized netsh error was {netsh}");
        assert_eq!(
            MaintenanceError::UnsupportedPlatform.localized(Locale::ZhCn),
            "此操作仅在 Windows 上可用。"
        );
        let sound = MaintenanceError::SoundPanelFailed("boom".to_owned()).localized(Locale::EnUs);
        assert!(sound.contains("boom"), "localized sound error was {sound}");
    }
}
