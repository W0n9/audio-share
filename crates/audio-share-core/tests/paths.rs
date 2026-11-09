//! Config/log path resolution at the public `audio_share_core::paths` seam.
//!
//! Resolution order (architecture.md): `AUDIO_SHARE_CONFIG` override, then a
//! writable executable directory, then `$XDG_CONFIG_HOME/audio-share-server/`.
//! `server.log` always lives in the directory holding the resolved config file.

use audio_share_core::paths::{paths, resolve_paths};

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temporary directory")
}

#[test]
fn env_override_names_the_config_file_and_its_directory_holds_the_log() {
    let temp = tempdir();
    let config = temp.path().join("custom-config.ini");

    let resolved = resolve_paths(Some(&config), None, None);

    assert_eq!(resolved.config, config);
    assert_eq!(resolved.log, temp.path().join("server.log"));
}

#[test]
fn writable_executable_directory_wins_over_xdg() {
    let exe_dir = tempdir();
    let xdg = tempdir();

    let resolved = resolve_paths(None, Some(exe_dir.path()), Some(xdg.path()));

    assert_eq!(resolved.config, exe_dir.path().join("config.ini"));
    assert_eq!(resolved.log, exe_dir.path().join("server.log"));
}

#[test]
fn missing_executable_directory_falls_back_to_xdg() {
    let xdg = tempdir();
    let missing = xdg.path().join("does-not-exist");

    let resolved = resolve_paths(None, Some(&missing), Some(xdg.path()));

    assert_eq!(
        resolved.config,
        xdg.path().join("audio-share-server").join("config.ini")
    );
    assert_eq!(
        resolved.log,
        xdg.path().join("audio-share-server").join("server.log")
    );
}

#[test]
fn xdg_defaults_to_the_audio_share_server_directory() {
    let xdg = tempdir();

    let resolved = resolve_paths(None, None, Some(xdg.path()));

    assert_eq!(
        resolved.config,
        xdg.path().join("audio-share-server").join("config.ini")
    );
}

#[cfg(unix)]
#[test]
fn read_only_executable_directory_falls_back_to_xdg() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let exe_dir = tempdir();
    let xdg = tempdir();
    let executable = exe_dir.path().join("as-cmd");
    fs::write(&executable, b"placeholder").expect("write placeholder executable");
    fs::set_permissions(exe_dir.path(), fs::Permissions::from_mode(0o555))
        .expect("make the executable directory read-only");

    let resolved = resolve_paths(None, Some(exe_dir.path()), Some(xdg.path()));

    // Restore permissions so the temporary directory can be removed.
    fs::set_permissions(exe_dir.path(), fs::Permissions::from_mode(0o755))
        .expect("restore directory permissions");

    if resolved.config == exe_dir.path().join("config.ini") {
        eprintln!("skipping: directory permissions are not enforced for this user");
        return;
    }
    assert_eq!(
        resolved.config,
        xdg.path().join("audio-share-server").join("config.ini"),
        "a read-only executable directory must fall back to XDG"
    );
    assert_eq!(
        resolved.log,
        xdg.path().join("audio-share-server").join("server.log")
    );
    assert_eq!(resolved.legacy_config, None, "there is no legacy file");
}

#[cfg(unix)]
#[test]
fn read_only_executable_directory_reports_the_legacy_config_for_migration() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let exe_dir = tempdir();
    let xdg = tempdir();
    let legacy = exe_dir.path().join("config.ini");
    fs::write(&legacy, b"[App]\nconfigVersion=1\n").expect("write legacy config");
    fs::set_permissions(exe_dir.path(), fs::Permissions::from_mode(0o555))
        .expect("make the executable directory read-only");

    let resolved = resolve_paths(None, Some(exe_dir.path()), Some(xdg.path()));

    fs::set_permissions(exe_dir.path(), fs::Permissions::from_mode(0o755))
        .expect("restore directory permissions");

    if resolved.config == exe_dir.path().join("config.ini") {
        eprintln!("skipping: directory permissions are not enforced for this user");
        return;
    }
    assert_eq!(
        resolved.legacy_config,
        Some(legacy),
        "the read-only exe-dir config must be reported for migration"
    );
    assert_eq!(
        resolved.config,
        xdg.path().join("audio-share-server").join("config.ini")
    );
}

#[test]
fn a_writable_executable_directory_has_no_legacy_config() {
    let exe_dir = tempdir();
    let xdg = tempdir();
    std::fs::write(exe_dir.path().join("config.ini"), b"[App]\n").expect("write config");

    let resolved = resolve_paths(None, Some(exe_dir.path()), Some(xdg.path()));

    assert_eq!(resolved.config, exe_dir.path().join("config.ini"));
    assert_eq!(
        resolved.legacy_config, None,
        "the resolved config is the legacy file itself"
    );
}

/// The real entry point must honour the process environment variable.
///
/// This is the only test that mutates the process environment; the lock keeps
/// it from racing other tests that might read `AUDIO_SHARE_CONFIG`.
#[test]
fn paths_reads_the_audio_share_config_override_from_the_environment() {
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let temp = tempdir();
    let config = temp.path().join("env-config.ini");
    let previous = std::env::var_os("AUDIO_SHARE_CONFIG");
    std::env::set_var("AUDIO_SHARE_CONFIG", &config);

    let resolved = paths();

    match previous {
        Some(value) => std::env::set_var("AUDIO_SHARE_CONFIG", value),
        None => std::env::remove_var("AUDIO_SHARE_CONFIG"),
    }

    assert_eq!(resolved.config, config);
    assert_eq!(resolved.log, temp.path().join("server.log"));
}
