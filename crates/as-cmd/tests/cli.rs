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
            // v2 flags: the default v2 port and the disable value are
            // documented in the help (spec D1; pairing.md §10.2).
            .stdout(predicate::str::contains("--v2-port"))
            .stdout(predicate::str::contains("65531"))
            .stdout(predicate::str::contains("--allow-loose-key-permissions"))
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

// ---------------------------------------------------------------------------
// Ticket 19: headless v2 pairing/administration verbs. The v0.3.4 flag paths
// above stay byte-compatible; these tests pin the additive surface.
// ---------------------------------------------------------------------------

fn isolated_config(temp: &tempfile::TempDir) -> std::path::PathBuf {
    temp.path().join("config.ini")
}

fn free_udp_port() -> u16 {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("free UDP port");
    socket.local_addr().expect("local address").port()
}

/// Serialize real-UDP process tests across test binaries (cargo runs targets
/// in parallel): a released ephemeral port must not be reused while another
/// `as-cmd pair` child is still binding it.
struct PortLock {
    _file: std::fs::File,
}

impl PortLock {
    fn acquire() -> Self {
        let path = std::env::temp_dir().join("audio-share-pair-test-port.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .expect("port lock file");
        file.lock().expect("port lock");
        Self { _file: file }
    }
}

#[test]
fn help_states_v1_is_unencrypted_and_lists_the_new_verbs() {
    as_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("UNENCRYPTED"))
        .stdout(predicate::str::contains("trusted network"))
        .stdout(predicate::str::contains("pair"))
        .stdout(predicate::str::contains("clients"))
        .stdout(predicate::str::contains("identity"));
}

#[test]
fn clients_list_on_an_empty_store_succeeds_and_says_so() {
    let temp = tempfile::tempdir().expect("tempdir");
    as_cmd()
        .args(["clients", "list"])
        .env("AUDIO_SHARE_CONFIG", isolated_config(&temp))
        .assert()
        .success()
        .stdout(predicate::str::contains("no paired clients"));
}

#[test]
fn clients_revoke_unknown_id_exits_one_with_a_readable_error() {
    let temp = tempfile::tempdir().expect("tempdir");
    as_cmd()
        .args(["clients", "revoke", "deadbeefdeadbeef"])
        .env("AUDIO_SHARE_CONFIG", isolated_config(&temp))
        .assert()
        .code(1)
        .stderr(predicate::str::contains("unknown client id"));
}

#[test]
fn offline_admin_reports_a_running_core_instead_of_blocking() {
    use std::time::Duration;
    use v2_transport::auth_store::StoreLock;

    let temp = tempfile::tempdir().expect("tempdir");
    let _held = StoreLock::acquire(temp.path()).expect("hold the running Core's lock");
    as_cmd()
        .args(["clients", "list"])
        .env("AUDIO_SHARE_CONFIG", isolated_config(&temp))
        .timeout(Duration::from_secs(10))
        .assert()
        .code(1)
        .stderr(predicate::str::contains("stop the Server first"));
}

#[test]
fn identity_reset_requires_confirmation_or_yes() {
    let temp = tempfile::tempdir().expect("tempdir");
    // EOF on stdin: the warning is printed but nothing is reset.
    as_cmd()
        .args(["identity", "reset"])
        .env("AUDIO_SHARE_CONFIG", isolated_config(&temp))
        .write_stdin("")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("WARNING"))
        .stdout(predicate::str::contains("cleared first"))
        .stderr(predicate::str::contains("cancelled"));
    assert!(!temp.path().join("identity").exists());

    as_cmd()
        .args(["identity", "reset", "--yes"])
        .env("AUDIO_SHARE_CONFIG", isolated_config(&temp))
        .assert()
        .success()
        .stdout(predicate::str::contains("identity reset complete"))
        .stdout(predicate::str::contains("Server fingerprint:"));
    assert!(temp.path().join("identity/server-identity.pem").is_file());
}

#[test]
fn pair_sas_prints_fingerprint_and_times_out_without_a_device() {
    let _port_lock = PortLock::acquire();
    let temp = tempfile::tempdir().expect("tempdir");
    let port = free_udp_port();
    as_cmd()
        .args([
            "pair",
            "--sas",
            "--timeout",
            "1",
            "--v2-port",
            &port.to_string(),
        ])
        .env("AUDIO_SHARE_CONFIG", isolated_config(&temp))
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Pairing mode: SAS"))
        .stdout(predicate::str::contains("Server fingerprint:"))
        .stdout(predicate::str::contains("Pairing code"))
        .stderr(predicate::str::contains("timed out"));
}

#[test]
fn pair_qr_prints_the_uri_and_terminal_qr() {
    let _port_lock = PortLock::acquire();
    let temp = tempfile::tempdir().expect("tempdir");
    let port = free_udp_port();
    as_cmd()
        .args([
            "pair",
            "--timeout",
            "1",
            "--addr",
            "127.0.0.1:65531",
            "--v2-port",
            &port.to_string(),
        ])
        .env("AUDIO_SHARE_CONFIG", isolated_config(&temp))
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "Pairing URI: audioshare-pair://p1?",
        ))
        .stdout(predicate::str::contains("addr=127.0.0.1:65531"))
        .stdout(predicate::str::contains("Server fingerprint:"))
        // Half-block QR art contains at least one of these glyphs.
        .stdout(predicate::str::contains("█").or(predicate::str::contains("▀")));
}

#[test]
fn clients_list_and_revoke_operate_on_the_persisted_store() {
    let temp = tempfile::tempdir().expect("tempdir");
    let pairing_dir = temp.path().join("pairing");
    std::fs::create_dir_all(&pairing_dir).expect("pairing dir");
    std::fs::write(
        pairing_dir.join("clients.json"),
        r#"{
  "version": 1,
  "clients": [{
    "client_id": "cf7940b474310246",
    "public_key": "A9ClKd50dWQawFDHkrQs_weRB8pTENp5U9JZUSFHfA9t",
    "display_name": "Test Phone",
    "state": "active",
    "created_at": "2026-11-16T10:00:00Z",
    "last_seen_at": "2026-11-16T10:05:00Z"
  }]
}"#,
    )
    .expect("seed store");

    as_cmd()
        .args(["clients", "list"])
        .env("AUDIO_SHARE_CONFIG", isolated_config(&temp))
        .assert()
        .success()
        .stdout(predicate::str::contains("cf7940b474310246"))
        .stdout(predicate::str::contains("Test Phone"))
        .stdout(predicate::str::contains("active"))
        .stdout(predicate::str::contains("2026-11-16T10:05:00Z"));

    as_cmd()
        .args(["clients", "revoke", "cf7940b474310246"])
        .env("AUDIO_SHARE_CONFIG", isolated_config(&temp))
        .assert()
        .success()
        .stdout(predicate::str::contains("revoked client cf7940b474310246"));

    let text = std::fs::read_to_string(pairing_dir.join("clients.json")).expect("store");
    assert!(text.contains("\"state\": \"revoked\""), "{text}");
    assert!(pairing_dir.join("clients.json.lock").is_file());
}
