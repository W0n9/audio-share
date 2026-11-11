//! Process-level CLI parity for `as-cmd` v0.3.4.
//!
//! These tests assert externally observable behavior only: stdout/stderr and
//! exit codes. Output text may improve, but the flags, defaults, examples and
//! exit-code contract must stay recognizable to existing scripts.

use assert_cmd::Command;
use predicates::prelude::*;

fn as_cmd() -> Command {
    Command::cargo_bin("as-cmd").expect("as-cmd binary is built for tests")
}

#[test]
fn help_prints_the_old_examples_flags_and_defaults() {
    for flag in ["-h", "--help"] {
        as_cmd()
            .arg(flag)
            .assert()
            .success()
            .stdout(predicate::str::contains("Example:"))
            .stdout(predicate::str::contains("as-cmd -b"))
            .stdout(predicate::str::contains("as-cmd --bind="))
            .stdout(predicate::str::contains("as-cmd -l"))
            .stdout(predicate::str::contains("as-cmd --list-encoding"))
            .stdout(predicate::str::contains("--list-endpoint"))
            .stdout(predicate::str::contains("--bind"))
            .stdout(predicate::str::contains("--encoding"))
            .stdout(predicate::str::contains("--channels"))
            .stdout(predicate::str::contains("--sample-rate"))
            .stdout(predicate::str::contains("65530"))
            .stdout(predicate::str::contains("default"));
    }
}

#[test]
fn version_matches_the_old_layout_with_the_rewrite_version() {
    for flag in ["-v", "--version"] {
        as_cmd()
            .arg(flag)
            .assert()
            .success()
            .stdout(predicate::str::contains("as-cmd\n"))
            .stdout(predicate::str::contains(concat!(
                "version: ",
                env!("CARGO_PKG_VERSION")
            )))
            .stdout(predicate::str::contains(
                "url: https://github.com/mkckr0/audio-share",
            ));
    }
}

#[test]
fn list_encoding_prints_the_six_entries_in_order() {
    let expected = concat!(
        "encoding list:\n",
        "\tdefault\t\tDefault encoding\n",
        "\tf32\t\t32 bit floating-point PCM\n",
        "\ts8\t\t8 bit integer PCM\n",
        "\ts16\t\t16 bit integer PCM\n",
        "\ts24\t\t24 bit integer PCM\n",
        "\ts32\t\t32 bit integer PCM\n",
    );
    as_cmd()
        .arg("--list-encoding")
        .assert()
        .success()
        .stdout(predicate::eq(expected));
}

#[test]
fn no_arguments_prints_help_to_stderr_and_exits_zero_like_v034() {
    as_cmd()
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("Example:"))
        .stderr(predicate::str::contains("--list-endpoint"));
}

#[test]
fn options_without_bind_also_print_help_to_stderr_and_exit_zero() {
    as_cmd()
        .arg("--encoding=f32")
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("Example:"));
}

#[test]
fn unknown_flag_is_a_readable_error_with_exit_code_one() {
    as_cmd()
        .arg("--bogus")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("bogus"));
}

#[test]
fn unknown_encoding_is_rejected_at_parse_time_with_exit_code_one() {
    for args in [vec!["--encoding=bogus"], vec!["--encoding", "bogus"]] {
        as_cmd()
            .args(args)
            .assert()
            .code(1)
            .stderr(predicate::str::contains("encoding"));
    }
}

#[test]
fn invalid_bind_and_port_values_exit_one_with_readable_errors() {
    for bind in [
        "127.0.0.1:not-a-port",
        "127.0.0.1:99999",
        "127.0.0.1:-1",
        "127.0.0.1:",
    ] {
        as_cmd()
            .arg(format!("--bind={bind}"))
            .assert()
            .code(1)
            .stderr(predicate::str::contains("port"));
    }
}

#[test]
fn missing_endpoint_value_exits_one() {
    as_cmd()
        .arg("-e")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("endpoint"));
}

#[test]
fn negative_channels_and_sample_rate_exit_one_instead_of_crashing() {
    for args in [vec!["--channels=-1"], vec!["--sample-rate=-48000"]] {
        as_cmd()
            .args(args)
            .assert()
            .code(1)
            .stderr(predicate::str::contains("invalid value"));
    }
}

#[test]
fn empty_host_means_the_default_address() {
    // The empty host must be accepted as "default address" (compatibility
    // improvement over v0.3.4, which exited with `Invalid argument`). With an
    // invalid port the bind parser still reports a readable error, proving the
    // empty host parsed before the port check.
    as_cmd()
        .arg("--bind=:99999")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("port"));
}

/// v0.3.4 segfaulted (exit 139) when no PipeWire daemon was reachable. The
/// rewrite must report a readable error and exit 1.
#[cfg(target_os = "linux")]
#[test]
fn list_endpoints_without_a_pipewire_daemon_errors_instead_of_crashing() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let runtime = temp.path().join("empty-runtime");
    std::fs::create_dir_all(&runtime).expect("isolated runtime directory");

    as_cmd()
        .arg("-l")
        .env_clear()
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("PIPEWIRE_RUNTIME_DIR", &runtime)
        .env("AUDIO_SHARE_CONFIG", temp.path().join("config.ini"))
        .assert()
        .code(1)
        .stderr(predicate::str::contains("PipeWire"));
}
