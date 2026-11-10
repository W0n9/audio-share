//! Config and log file locations shared by `as-cmd` and the Server GUI.
//!
//! Resolution order (see `docs/server-rewrite/README.md`):
//!
//! 1. `AUDIO_SHARE_CONFIG` names the config file directly.
//! 2. `config.ini` next to the executable, when that directory is writable
//!    (portable Windows exe, `as-cmd` tarball; preserves v0.3.4 behavior).
//! 3. `$XDG_CONFIG_HOME/audio-share-server/config.ini`, defaulting to
//!    `~/.config/audio-share-server/config.ini`.
//!
//! `server.log` always lives in the directory that holds the resolved config
//! file, so a packaged Linux install never tries to write next to the binary.

use std::path::{Path, PathBuf};

/// Environment variable that overrides the config file path.
pub const CONFIG_ENV: &str = "AUDIO_SHARE_CONFIG";

/// Config file name.
pub const CONFIG_FILE_NAME: &str = "config.ini";

/// Log file name; shares the config directory.
pub const LOG_FILE_NAME: &str = "server.log";

/// Per-application directory under `$XDG_CONFIG_HOME`.
pub const APP_DIR_NAME: &str = "audio-share-server";

/// The resolved config and log file paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// `config.ini` used by the Server GUI (and reserved for `as-cmd`).
    pub config: PathBuf,
    /// `server.log` next to the config file.
    pub log: PathBuf,
    /// An existing v0.3.4 executable-directory config that must be migrated
    /// into the writable [`Self::config`] location because the executable
    /// directory is read-only (Linux package/AppImage).
    pub legacy_config: Option<PathBuf>,
}

impl Paths {
    fn in_dir(dir: &Path) -> Self {
        Self {
            config: dir.join(CONFIG_FILE_NAME),
            log: dir.join(LOG_FILE_NAME),
            legacy_config: None,
        }
    }

    fn in_legacy_migration_dir(dir: &Path, legacy_config: Option<PathBuf>) -> Self {
        Self {
            config: dir.join(CONFIG_FILE_NAME),
            log: dir.join(LOG_FILE_NAME),
            legacy_config,
        }
    }
}

/// Resolve the config/log paths from explicit inputs.
///
/// [`paths`] supplies these from the process environment and the running
/// executable. Keeping resolution pure makes the writability fallbacks testable
/// without mutating global state.
pub fn resolve_paths(
    config_override: Option<&Path>,
    exe_dir: Option<&Path>,
    xdg_config_home: Option<&Path>,
) -> Paths {
    if let Some(config) = config_override {
        let directory = config
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        return Paths {
            config: config.to_path_buf(),
            log: directory.join(LOG_FILE_NAME),
            legacy_config: None,
        };
    }

    if let Some(exe_dir) = exe_dir {
        if is_dir_writable(exe_dir) {
            return Paths::in_dir(exe_dir);
        }
    }

    if let Some(xdg_config_home) = xdg_config_home {
        let directory = xdg_config_home.join(APP_DIR_NAME);
        return Paths::in_legacy_migration_dir(
            &directory,
            legacy_config(exe_dir, &directory.join(CONFIG_FILE_NAME)),
        );
    }

    // Best effort when neither location is usable: prefer the executable
    // directory (the caller reports a log-open failure), else the CWD.
    match exe_dir {
        Some(exe_dir) => Paths::in_dir(exe_dir),
        None => Paths::in_dir(Path::new(".")),
    }
}

/// The executable-directory config when it exists and is not already the
/// resolved target (the path a read-only install must migrate from).
fn legacy_config(exe_dir: Option<&Path>, target: &Path) -> Option<PathBuf> {
    let candidate = exe_dir?.join(CONFIG_FILE_NAME);
    (candidate != target && candidate.is_file()).then_some(candidate)
}

/// Resolve the config/log paths from the current process.
///
/// `AUDIO_SHARE_CONFIG` wins. Otherwise the executable directory is used when
/// it is writable, falling back to the XDG config directory.
pub fn paths() -> Paths {
    let config_override = std::env::var_os(CONFIG_ENV).map(PathBuf::from);
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let xdg_config_home = xdg_config_home();
    resolve_paths(
        config_override.as_deref(),
        exe_dir.as_deref(),
        xdg_config_home.as_deref(),
    )
}

fn xdg_config_home() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("XDG_CONFIG_HOME") {
        let path = PathBuf::from(value);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if !home.as_os_str().is_empty() {
            return Some(home.join(".config"));
        }
    }
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let appdata = PathBuf::from(appdata);
            if !appdata.as_os_str().is_empty() {
                return Some(appdata);
            }
        }
    }
    None
}

/// Probe writability by creating and removing a uniquely named file.
///
/// Metadata permission bits are not authoritative (ACLs, read-only mounts,
/// root), so an actual write is the only reliable check.
fn is_dir_writable(directory: &Path) -> bool {
    if !directory.is_dir() {
        return false;
    }
    let probe = directory.join(format!(".audio-share-write-probe-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}
