//! `config.ini` parity at the public `audio_share_core::config` seam.
//!
//! These tests pin the v0.3.4 keys/defaults and the rewrite's explicit
//! migration behavior: recognized values survive, unsupported versions are
//! left untouched with a backup, corrupt files are recovered, and every write
//! is atomic UTF-8 without a BOM.

use std::fs;
use std::path::{Path, PathBuf};

use audio_share_core::capture::Endpoint;
use audio_share_core::config::{
    backup_path, load, save, Config, ConfigNotice, EncodingSetting, LoadOptions, WhenAppStart,
    WhenClose, CONFIG_VERSION, DEFAULT_PORT,
};
use audio_share_core::server::default_host;

const V1_CONFIG: &str = "\
[App]
configVersion=1
language=zh-CN
Running=1

[Network]
host=10.1.2.3
port=65000

[Capture]
endpoint=default
encoding=4

[App Settings]
AutoRun=1
WhenAppStart=2
WhenClose=0
AutoUpdate=1

[Unknown]
Keep=me
; a comment that must survive migration
";

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temporary directory")
}

fn config_path(dir: &Path) -> PathBuf {
    dir.join("config.ini")
}

fn write(path: &Path, text: &str) {
    fs::write(path, text).expect("write config");
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("read config as UTF-8")
}

fn default_host_string() -> String {
    default_host()
        .map(|address| address.to_string())
        .unwrap_or_else(|| "0.0.0.0".to_owned())
}

#[test]
fn fresh_config_uses_defaults_and_writes_version_two() {
    let dir = tempdir();
    let path = config_path(dir.path());

    let loaded = load(&path, LoadOptions::default()).expect("fresh load");

    assert_eq!(loaded.config.host, default_host_string());
    assert_eq!(loaded.config.port, DEFAULT_PORT);
    assert_eq!(loaded.config.port, 65_530);
    assert_eq!(loaded.config.endpoint, "default");
    assert_eq!(loaded.config.encoding, EncodingSetting::Default);
    assert!(!loaded.config.autostart);
    assert_eq!(loaded.config.when_start, WhenAppStart::Nothing);
    assert_eq!(loaded.config.when_close, WhenClose::Minimize);
    assert!(!loaded.config.auto_update);
    assert_eq!(loaded.config.language, "default");
    assert!(!loaded.config.running);
    assert!(
        loaded.notices.is_empty(),
        "a fresh config has nothing to report"
    );

    let raw = read(&path);
    assert!(raw.contains("configVersion=2"), "raw config was {raw:?}");
    assert!(raw.contains("port=65530"), "raw config was {raw:?}");
    assert!(raw.contains("WhenClose=1"), "raw config was {raw:?}");
    assert!(
        !raw.as_bytes().starts_with(&[0xef, 0xbb, 0xbf]),
        "writes must not include a UTF-8 BOM"
    );
    assert!(
        !raw.as_bytes().starts_with(&[0xff, 0xfe]),
        "writes must not include a UTF-16 BOM"
    );
}

#[test]
fn legacy_v1_values_migrate_and_unknown_keys_survive() {
    let dir = tempdir();
    let path = config_path(dir.path());
    write(&path, V1_CONFIG);

    let loaded = load(&path, LoadOptions::default()).expect("v1 migration");

    assert_eq!(loaded.config.host, "10.1.2.3");
    assert_eq!(loaded.config.port, 65_000);
    assert_eq!(loaded.config.endpoint, "default");
    assert_eq!(loaded.config.encoding, EncodingSetting::S16);
    assert!(loaded.config.autostart);
    assert_eq!(loaded.config.when_start, WhenAppStart::KeepLast);
    assert_eq!(loaded.config.when_close, WhenClose::Exit);
    assert!(loaded.config.auto_update);
    assert_eq!(loaded.config.language, "zh-CN");
    assert!(loaded.config.running);
    assert_eq!(
        loaded.notices,
        vec![ConfigNotice::Migrated { from: 1 }],
        "migration must be reported, not silently performed"
    );

    let raw = read(&path);
    assert!(raw.contains("configVersion=2"), "raw config was {raw:?}");
    assert!(raw.contains("port=65000"), "raw config was {raw:?}");
    assert!(raw.contains("encoding=4"), "raw config was {raw:?}");
    assert!(
        raw.contains("[Unknown]\nKeep=me"),
        "unknown keys must survive migration: {raw:?}"
    );
    assert!(
        raw.contains("; a comment that must survive migration"),
        "comments must survive migration: {raw:?}"
    );
}

#[test]
fn a_missing_version_migrates_from_zero() {
    let dir = tempdir();
    let path = config_path(dir.path());
    write(&path, "[Network]\nport=12345\n");

    let loaded = load(&path, LoadOptions::default()).expect("missing version migration");

    assert_eq!(loaded.config.port, 12_345);
    assert_eq!(loaded.notices, vec![ConfigNotice::Migrated { from: 0 }]);
    assert!(read(&path).contains("configVersion=2"));
}

#[test]
fn a_future_version_is_read_without_being_overwritten_and_gets_a_backup() {
    let dir = tempdir();
    let path = config_path(dir.path());
    let future = "\
[App]
configVersion=3
language=en-US

[Network]
host=10.9.9.9
port=61000
Janitor=yes
";
    write(&path, future);
    let original = fs::read(&path).unwrap();

    let loaded = load(&path, LoadOptions::default()).expect("future version load");

    assert_eq!(loaded.config.host, "10.9.9.9");
    assert_eq!(loaded.config.port, 61_000);
    assert_eq!(loaded.config.language, "en-US");
    assert_eq!(
        loaded.notices,
        vec![ConfigNotice::FutureVersion { version: 3 }]
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        original,
        "a future-version config must not be rewritten"
    );
    assert_eq!(
        fs::read(backup_path(&path)).unwrap(),
        original,
        "the original file must be kept as config.ini.bak"
    );
}

#[test]
fn a_corrupt_file_is_backed_up_and_defaults_are_restored() {
    let dir = tempdir();
    let path = config_path(dir.path());
    let corrupt = b"[App]\nconfigVersion=2\0garbage\n";
    fs::write(&path, corrupt).unwrap();

    let loaded = load(&path, LoadOptions::default()).expect("corrupt recovery");

    assert_eq!(loaded.config.port, DEFAULT_PORT);
    assert_eq!(loaded.config.endpoint, "default");
    assert_eq!(loaded.notices, vec![ConfigNotice::Recovered]);
    assert_eq!(
        fs::read(backup_path(&path)).unwrap(),
        corrupt,
        "the damaged file must be preserved as config.ini.bak"
    );
    let raw = read(&path);
    assert!(raw.contains("configVersion=2"), "raw config was {raw:?}");
    assert!(!raw.contains('\0'), "the restored file must be clean text");
}

#[test]
fn atomic_writes_replace_the_file_and_leave_no_temporary_files() {
    let dir = tempdir();
    let path = config_path(dir.path());
    let first = Config {
        port: 12_345,
        ..Config::default()
    };
    let second = Config {
        port: 12_346,
        ..Config::default()
    };

    save(&path, &first).expect("first save");
    save(&path, &second).expect("second save");

    let loaded = load(&path, LoadOptions::default()).expect("reload");
    assert_eq!(loaded.config.port, 12_346);

    let entries: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        entries,
        vec![PathBuf::from("config.ini")],
        "atomic writes must not leave temp or stray files"
    );
}

#[cfg(unix)]
#[test]
fn atomic_replacement_does_not_truncate_a_read_only_target_in_place() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir();
    let path = config_path(dir.path());
    save(
        &path,
        &Config {
            port: 12_345,
            ..Config::default()
        },
    )
    .expect("initial save");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).expect("make target read-only");

    // An in-place truncating write would fail on the read-only file; an atomic
    // same-directory rename replaces the directory entry instead.
    save(
        &path,
        &Config {
            port: 12_347,
            ..Config::default()
        },
    )
    .expect("atomic save must replace the read-only file");

    assert_eq!(
        load(&path, LoadOptions::default()).unwrap().config.port,
        12_347
    );
}

#[test]
fn invalid_values_fall_back_to_defaults_and_are_reported() {
    let dir = tempdir();
    let path = config_path(dir.path());
    write(
        &path,
        "[App]\nconfigVersion=2\n\n[Network]\nhost=127.0.0.1\nport=not-a-port\n\n\
         [Capture]\nendpoint=default\nencoding=1\n",
    );

    let loaded = load(&path, LoadOptions::default()).expect("invalid values");

    assert_eq!(loaded.config.port, DEFAULT_PORT);
    assert_eq!(loaded.config.encoding, EncodingSetting::Default);
    assert!(loaded.notices.contains(&ConfigNotice::InvalidValue {
        key: "port".to_owned(),
        value: "not-a-port".to_owned(),
    }));
    assert!(loaded.notices.contains(&ConfigNotice::InvalidValue {
        key: "encoding".to_owned(),
        value: "1".to_owned(),
    }));
    let raw = read(&path);
    assert!(raw.contains("port=65530"), "raw config was {raw:?}");
    assert!(raw.contains("encoding=0"), "raw config was {raw:?}");
}

#[test]
fn zero_and_out_of_range_ports_and_encodings_use_defaults() {
    for (port, encoding) in [("0", "7"), ("70000", "99")] {
        let dir = tempdir();
        let path = config_path(dir.path());
        write(
            &path,
            &format!(
                "[App]\nconfigVersion=2\n[Network]\nport={port}\n[Capture]\nencoding={encoding}\n"
            ),
        );

        let loaded = load(&path, LoadOptions::default()).expect("out-of-range values");

        assert_eq!(loaded.config.port, DEFAULT_PORT, "port {port}");
        assert_eq!(
            loaded.config.encoding,
            EncodingSetting::Default,
            "encoding {encoding}"
        );
        assert_eq!(loaded.notices.len(), 2, "both values must be reported");
    }
}

#[test]
fn encoding_legacy_numbers_round_trip() {
    assert_eq!(
        EncodingSetting::from_legacy(0),
        Some(EncodingSetting::Default)
    );
    assert_eq!(EncodingSetting::from_legacy(1), None, "1 is invalid/unused");
    assert_eq!(EncodingSetting::from_legacy(2), Some(EncodingSetting::F32));
    assert_eq!(EncodingSetting::from_legacy(3), Some(EncodingSetting::S8));
    assert_eq!(EncodingSetting::from_legacy(4), Some(EncodingSetting::S16));
    assert_eq!(EncodingSetting::from_legacy(5), Some(EncodingSetting::S24));
    assert_eq!(EncodingSetting::from_legacy(6), Some(EncodingSetting::S32));
    assert_eq!(EncodingSetting::from_legacy(7), None);

    for (setting, legacy, name) in [
        (EncodingSetting::Default, 0, "default"),
        (EncodingSetting::F32, 2, "f32"),
        (EncodingSetting::S8, 3, "s8"),
        (EncodingSetting::S16, 4, "s16"),
        (EncodingSetting::S24, 5, "s24"),
        (EncodingSetting::S32, 6, "s32"),
    ] {
        assert_eq!(setting.legacy_value(), legacy);
        assert_eq!(setting.name(), name);
        assert_eq!(EncodingSetting::from_name(name), Some(setting));
        assert_eq!(
            EncodingSetting::from_name(&name.to_ascii_uppercase()),
            Some(setting)
        );
    }
    assert_eq!(EncodingSetting::from_name("bogus"), None);
}

#[test]
fn reads_are_case_insensitive_and_trim_win32_style() {
    let dir = tempdir();
    let path = config_path(dir.path());
    write(
        &path,
        "\
[aPP]
ConfigVersion=2
LANGUAGE=\"zh-CN\"

[network]
 HOST = 10.2.3.4
Port = 62000

[CAPTURE]
Encoding=5

[app settings]
WhenClose=0
",
    );

    let loaded = load(&path, LoadOptions::default()).expect("case-insensitive read");

    assert_eq!(loaded.config.language, "zh-CN", "quotes must be stripped");
    assert_eq!(loaded.config.host, "10.2.3.4");
    assert_eq!(loaded.config.port, 62_000);
    assert_eq!(loaded.config.encoding, EncodingSetting::S24);
    assert_eq!(loaded.config.when_close, WhenClose::Exit);
}

#[test]
fn when_close_defaults_to_minimize_but_an_explicit_zero_keeps_exit() {
    let cases = [
        (None, WhenClose::Minimize),
        (Some(0), WhenClose::Exit),
        (Some(1), WhenClose::Minimize),
        (Some(2), WhenClose::Minimize),
    ];

    for (value, expected) in cases {
        let dir = tempdir();
        let path = config_path(dir.path());
        let settings = match value {
            Some(value) => format!("[App Settings]\nWhenClose={value}\n"),
            None => String::new(),
        };
        write(&path, &format!("[App]\nconfigVersion=2\n{settings}"));

        let loaded = load(&path, LoadOptions::default()).expect("when close read");
        assert_eq!(loaded.config.when_close, expected, "value {value:?}");
    }
}

#[test]
fn unknown_endpoints_reset_to_default_and_are_reported() {
    let dir = tempdir();
    let path = config_path(dir.path());
    write(
        &path,
        "[App]\nconfigVersion=2\n[Capture]\nendpoint=gone-sink\nencoding=2\n",
    );
    let endpoints = vec![Endpoint {
        id: "sink-1".to_owned(),
        name: "Sink One".to_owned(),
    }];

    let loaded = load(
        &path,
        LoadOptions {
            legacy: None,
            endpoints: Some(&endpoints),
        },
    )
    .expect("endpoint validation");

    assert_eq!(loaded.config.endpoint, "default");
    assert!(loaded.notices.contains(&ConfigNotice::UnknownEndpoint {
        endpoint: "gone-sink".to_owned()
    }));
    assert!(read(&path).contains("endpoint=default"));
}

#[test]
fn a_known_endpoint_is_kept_and_an_unavailable_list_skips_validation() {
    let endpoints = vec![Endpoint {
        id: "sink-1".to_owned(),
        name: "Sink One".to_owned(),
    }];

    let dir = tempdir();
    let path = config_path(dir.path());
    write(
        &path,
        "[App]\nconfigVersion=2\n[Capture]\nendpoint=sink-1\n",
    );
    let loaded = load(
        &path,
        LoadOptions {
            legacy: None,
            endpoints: Some(&endpoints),
        },
    )
    .expect("known endpoint");
    assert_eq!(loaded.config.endpoint, "sink-1");
    assert!(loaded.notices.is_empty());

    let dir = tempdir();
    let path = config_path(dir.path());
    write(
        &path,
        "[App]\nconfigVersion=2\n[Capture]\nendpoint=gone-sink\n",
    );
    let loaded = load(&path, LoadOptions::default()).expect("unknown list");
    assert_eq!(
        loaded.config.endpoint, "gone-sink",
        "without an endpoint list the configured endpoint is preserved"
    );
    assert!(loaded.notices.is_empty());
}

#[test]
fn utf16le_configs_are_read() {
    let dir = tempdir();
    let path = config_path(dir.path());
    let text = "[App]\r\nconfigVersion=2\r\nlanguage=zh-CN\r\n[Network]\r\nhost=10.7.7.7\r\nport=63000\r\n";
    let mut bytes = vec![0xff, 0xfe];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    fs::write(&path, bytes).unwrap();

    let loaded = load(&path, LoadOptions::default()).expect("UTF-16LE read");

    assert_eq!(loaded.config.host, "10.7.7.7");
    assert_eq!(loaded.config.port, 63_000);
    assert_eq!(loaded.config.language, "zh-CN");
    assert!(loaded.notices.is_empty());
}

#[test]
fn migrating_a_utf16_file_writes_utf8_without_a_bom() {
    let dir = tempdir();
    let path = config_path(dir.path());
    let text = "[App]\nconfigVersion=1\n[Network]\nhost=10.7.7.8\nport=63001\n";
    let mut bytes = vec![0xff, 0xfe];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    fs::write(&path, bytes).unwrap();

    let loaded = load(&path, LoadOptions::default()).expect("UTF-16LE migration");
    assert_eq!(loaded.config.port, 63_001);

    let raw = fs::read(&path).unwrap();
    assert!(!raw.starts_with(&[0xff, 0xfe]));
    assert!(!raw.starts_with(&[0xef, 0xbb, 0xbf]));
    assert_eq!(
        std::str::from_utf8(&raw).expect("migrated output is UTF-8"),
        String::from_utf8(raw.clone()).unwrap()
    );
    assert!(raw.starts_with(b"[App]"), "raw output was {raw:?}");
}

#[test]
fn legacy_read_only_location_values_migrate_into_the_writable_target() {
    let dir = tempdir();
    let legacy = dir.path().join("app-dir-config.ini");
    let target = dir.path().join("xdg").join("config.ini");
    write(
        &legacy,
        "[App]\nconfigVersion=1\n[Network]\nhost=10.4.4.4\nport=64000\n",
    );

    let loaded = load(
        &target,
        LoadOptions {
            legacy: Some(&legacy),
            endpoints: None,
        },
    )
    .expect("legacy location migration");

    assert_eq!(loaded.config.host, "10.4.4.4");
    assert_eq!(loaded.config.port, 64_000);
    assert!(loaded.notices.contains(&ConfigNotice::LegacyMigrated {
        from: legacy.clone()
    }));
    assert!(target.exists(), "the writable target must be created");
    assert!(read(&target).contains("configVersion=2"));
    assert_eq!(
        read(&legacy),
        "[App]\nconfigVersion=1\n[Network]\nhost=10.4.4.4\nport=64000\n",
        "the read-only legacy file must be left untouched"
    );
}

#[test]
fn running_state_round_trips() {
    let dir = tempdir();
    let path = config_path(dir.path());

    save(
        &path,
        &Config {
            running: true,
            ..Config::default()
        },
    )
    .unwrap();
    assert!(load(&path, LoadOptions::default()).unwrap().config.running);

    save(
        &path,
        &Config {
            running: false,
            ..Config::default()
        },
    )
    .unwrap();
    assert!(!load(&path, LoadOptions::default()).unwrap().config.running);
}

#[test]
fn save_rejects_unusable_server_values() {
    let dir = tempdir();
    let path = config_path(dir.path());

    let empty_host = Config {
        host: "  ".to_owned(),
        ..Config::default()
    };
    let error = save(&path, &empty_host).expect_err("empty host must be rejected");
    assert!(error.to_string().contains("host"), "error was {error}");

    let zero_port = Config {
        port: 0,
        ..Config::default()
    };
    let error = save(&path, &zero_port).expect_err("zero port must be rejected");
    assert!(error.to_string().contains("port"), "error was {error}");
}

#[test]
fn load_options_default_matches_a_plain_load() {
    // The convenience default must not validate endpoints or read a legacy file.
    let dir = tempdir();
    let path = config_path(dir.path());
    write(
        &path,
        "[App]\nconfigVersion=2\n[Capture]\nendpoint=whatever\n",
    );
    let loaded = load(&path, LoadOptions::default()).unwrap();
    assert_eq!(loaded.config.endpoint, "whatever");
    assert_eq!(CONFIG_VERSION, 2);
}
