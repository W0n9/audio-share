//! Cross-platform login autostart entries.
//!
//! `tauri-plugin-autostart` delegates to `auto-launch`, which joins the
//! executable path and arguments without quoting. A portable Windows exe under
//! `C:\Users\...\Audio Share\...` then produces a broken Run value, and the
//! same applies to Linux desktop entries. This module writes the entries
//! directly so paths with spaces are always quoted.
//!
//! `config.ini` `[App Settings] AutoRun` is the source of truth: the startup
//! path reconciles the OS entry to match it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// HKCU Run value name shared with v0.3.4.
pub const WINDOWS_VALUE_NAME: &str = "AudioShareServer";

/// XDG autostart file name.
pub const DESKTOP_FILE_NAME: &str = "AudioShareServer.desktop";

/// Run registry key.
pub const WINDOWS_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Arguments that make the GUI start hidden in the tray.
pub const HIDE_ARG: &str = "--hide";

/// Windows `Run` value data: `"<exe>" /hide`.
///
/// v0.3.4 wrote the path unquoted; quoting is the intentional fix required for
/// portable installs under paths with spaces.
pub fn windows_run_data(exe: &Path) -> String {
    format!("\"{}\" /hide", exe.display())
}

/// Linux XDG desktop entry contents.
pub fn linux_desktop_entry(exe: &Path) -> String {
    let exe = exe.to_string_lossy();
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Audio Share Server\n\
         Exec={} {HIDE_ARG}\n\
         TryExec={exe}\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n",
        desktop_quote(&exe)
    )
}

/// Resolve `$XDG_CONFIG_HOME/autostart/AudioShareServer.desktop`, defaulting
/// the base directory to `$HOME/.config`.
pub fn linux_entry_path(xdg_config_home: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let base = xdg_config_home
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or_else(|| {
            home.filter(|path| !path.as_os_str().is_empty())
                .map(|home| home.join(".config"))
        })?;
    Some(base.join("autostart").join(DESKTOP_FILE_NAME))
}

/// The executable login should launch: `$APPIMAGE` when set, else the running
/// executable.
pub fn launch_executable(appimage: Option<&Path>, current_exe: &Path) -> PathBuf {
    appimage
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| current_exe.to_path_buf())
}

/// Create or remove the platform autostart entry, with readable errors.
pub fn configure(enabled: bool) -> Result<(), String> {
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("cannot locate the application executable: {error}"))?;
    let appimage = std::env::var_os("APPIMAGE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let exe = launch_executable(appimage.as_deref(), &current_exe);

    #[cfg(target_os = "linux")]
    {
        let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let home = std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let entry_path =
            linux_entry_path(xdg_config_home.as_deref(), home.as_deref()).ok_or_else(|| {
                "cannot locate the autostart directory: neither XDG_CONFIG_HOME nor HOME is set"
                    .to_owned()
            })?;
        apply_linux_autostart(&entry_path, &exe, enabled)
    }

    #[cfg(windows)]
    {
        apply_windows_autostart(&exe, enabled)
    }

    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (&exe, enabled);
        Err("login autostart is not supported on this platform".to_owned())
    }
}

/// Write/remove a Linux desktop entry. Exposed for tests and package tooling.
pub fn apply_linux_autostart(entry_path: &Path, exe: &Path, enabled: bool) -> Result<(), String> {
    if enabled {
        if let Some(parent) = entry_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "cannot create the autostart directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        fs::write(entry_path, linux_desktop_entry(exe)).map_err(|error| {
            format!(
                "cannot write the autostart entry {}: {error}",
                entry_path.display()
            )
        })
    } else {
        match fs::remove_file(entry_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "cannot remove the autostart entry {}: {error}",
                entry_path.display()
            )),
        }
    }
}

/// Quote one `Exec` argument per the Desktop Entry specification.
fn desktop_quote(value: &str) -> String {
    let mut quoted = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '`' => quoted.push_str("\\`"),
            '$' => quoted.push_str("\\$"),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(windows)]
fn apply_windows_autostart(exe: &Path, enabled: bool) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let (run_key, _) = current_user
        .create_subkey(WINDOWS_RUN_KEY)
        .map_err(|error| format!("cannot open the login startup registry key: {error}"))?;
    if enabled {
        run_key
            .set_value(WINDOWS_VALUE_NAME, &windows_run_data(exe))
            .map_err(|error| format!("cannot write the login startup registry value: {error}"))
    } else {
        match run_key.delete_value(WINDOWS_VALUE_NAME) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "cannot remove the login startup registry value: {error}"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_run_value_quotes_the_executable_with_spaces() {
        assert_eq!(
            windows_run_data(Path::new(r"C:\Users\Foo\Audio Share\AudioShareServer.exe")),
            r#""C:\Users\Foo\Audio Share\AudioShareServer.exe" /hide"#
        );
        assert_eq!(
            windows_run_data(Path::new(r"C:\AudioShareServer.exe")),
            r#""C:\AudioShareServer.exe" /hide"#
        );
    }

    #[test]
    fn linux_desktop_entry_has_the_required_fields_and_quoted_exec() {
        let entry = linux_desktop_entry(Path::new("/home/foo/Audio Share/AudioShareServer"));
        assert_eq!(
            entry,
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Audio Share Server\n\
             Exec=\"/home/foo/Audio Share/AudioShareServer\" --hide\n\
             TryExec=/home/foo/Audio Share/AudioShareServer\n\
             Terminal=false\n\
             X-GNOME-Autostart-enabled=true\n"
        );
    }

    #[test]
    fn linux_exec_escapes_desktop_entry_reserved_characters() {
        let entry = linux_desktop_entry(Path::new("/tmp/we ird/$HOME/\"quoted\"/back\\slash"));
        let exec = entry
            .lines()
            .find(|line| line.starts_with("Exec="))
            .expect("Exec line");
        assert_eq!(
            exec,
            r#"Exec="/tmp/we ird/\$HOME/\"quoted\"/back\\slash" --hide"#
        );
    }

    #[test]
    fn xdg_config_home_wins_and_home_is_the_default() {
        let xdg = Path::new("/custom/xdg");
        assert_eq!(
            linux_entry_path(Some(xdg), Some(Path::new("/home/foo"))),
            Some(PathBuf::from(
                "/custom/xdg/autostart/AudioShareServer.desktop"
            ))
        );
        assert_eq!(
            linux_entry_path(None, Some(Path::new("/home/foo"))),
            Some(PathBuf::from(
                "/home/foo/.config/autostart/AudioShareServer.desktop"
            ))
        );
        assert_eq!(linux_entry_path(None, None), None);
    }

    #[test]
    fn appimage_path_is_used_when_present() {
        assert_eq!(
            launch_executable(
                Some(Path::new(
                    "/opt/Audio Share/AudioShareServer_0.4.0.AppImage"
                )),
                Path::new("/tmp/.mount/usr/bin/AudioShareServer")
            ),
            PathBuf::from("/opt/Audio Share/AudioShareServer_0.4.0.AppImage")
        );
        assert_eq!(
            launch_executable(None, Path::new("/usr/bin/AudioShareServer")),
            PathBuf::from("/usr/bin/AudioShareServer")
        );
    }

    #[test]
    fn enabling_writes_the_entry_file_and_disabling_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let entry_path = dir.path().join("autostart").join(DESKTOP_FILE_NAME);
        let exe = Path::new("/home/foo/Audio Share/AudioShareServer");

        apply_linux_autostart(&entry_path, exe, true).expect("enable");
        assert_eq!(
            std::fs::read_to_string(&entry_path).unwrap(),
            linux_desktop_entry(exe)
        );
        // Re-applying is idempotent and does not fail on an existing file.
        apply_linux_autostart(&entry_path, exe, true).expect("re-enable");

        apply_linux_autostart(&entry_path, exe, false).expect("disable");
        assert!(!entry_path.exists());
        // Removing an entry that is already gone is not an error.
        apply_linux_autostart(&entry_path, exe, false).expect("disable again");
    }

    #[test]
    fn linux_write_errors_are_readable() {
        let dir = tempfile::tempdir().unwrap();
        let not_a_file = dir.path().join("autostart");
        std::fs::create_dir_all(&not_a_file).unwrap();
        let error = apply_linux_autostart(&not_a_file, Path::new("/bin/app"), true)
            .expect_err("writing over a directory must fail");
        assert!(
            error.to_lowercase().contains("autostart"),
            "error was {error}"
        );
    }
}
