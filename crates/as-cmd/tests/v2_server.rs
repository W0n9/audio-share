#![cfg(unix)]
//! Live `as-cmd` v2 seam: `--v2-port` starts/stops the QUIC listener with
//! lifecycle logs, the Ed25519 identity key lands with pairing.md §10.2
//! permissions, `--v2-port=0` keeps the v1-only behavior byte-for-byte (no
//! v2 logs, no identity files), and loose identity-key permissions refuse v2
//! — with the Server itself unaffected — unless
//! `--allow-loose-key-permissions` overrides (pairing.md §10.2/PR-11).

use std::fs;
use std::io::Read;
use std::net::{TcpListener, UdpSocket};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

fn free_tcp_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("pick a free port");
    listener.local_addr().expect("local address").port()
}

/// A free UDP port for the v2 listener (bind + immediately release).
fn free_udp_port() -> u16 {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("pick a free UDP port");
    socket.local_addr().expect("local address").port()
}

fn wait_until(timeout: Duration, mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if check() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn wait_child(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().expect("query child status") {
            Some(status) => return Some(status),
            None if Instant::now() >= deadline => return None,
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}

fn interrupt(child: &Child) {
    let result = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) };
    assert_eq!(result, 0, "send SIGINT to as-cmd");
}

fn read_stderr(child: &mut Child) -> String {
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    stderr
}

fn spawn(temp: &Path, tcp_port: u16, v2_args: &[&str]) -> (Child, PathBuf, PathBuf) {
    let log_path = temp.join("server.log");
    let stdout_path = temp.join("stdout.log");
    let stdout_file = fs::File::create(&stdout_path).expect("stdout capture file");
    let mut command = Command::new(env!("CARGO_BIN_EXE_as-cmd"));
    command
        .arg("-V")
        .arg("-b")
        .arg(format!("127.0.0.1:{tcp_port}"))
        .arg("--fake-capture")
        .args(v2_args)
        .env("AUDIO_SHARE_CONFIG", temp.join("config.ini"))
        .stdout(Stdio::from(stdout_file.try_clone().expect("clone stdout")))
        .stderr(Stdio::piped());
    let child = command.spawn().expect("spawn as-cmd");
    (child, log_path, stdout_path)
}

fn log_text(log_path: &Path) -> String {
    fs::read_to_string(log_path).unwrap_or_default()
}

/// The identity key must sit at `<data dir>/identity/server-identity.pem`
/// (pairing.md §10.2) with 0600 file / 0700 directory permissions.
fn assert_identity_layout(config_dir: &Path) {
    let identity_dir = config_dir.join("identity");
    let key = identity_dir.join("server-identity.pem");
    assert!(key.is_file(), "identity key missing at {}", key.display());
    let file_mode = fs::metadata(&key).expect("key metadata").permissions().mode();
    assert_eq!(
        file_mode & 0o777,
        0o600,
        "identity key must be 0600, got {:o}",
        file_mode & 0o777
    );
    let dir_mode = fs::metadata(&identity_dir)
        .expect("identity dir metadata")
        .permissions()
        .mode();
    assert_eq!(
        dir_mode & 0o777,
        0o700,
        "identity dir must be 0700, got {:o}",
        dir_mode & 0o777
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_is_enabled_by_default_on_65531() {
    // The default-on rule (spec D1) is only observable when the default port
    // is actually free; CI machines may legitimately hold it.
    if UdpSocket::bind(("127.0.0.1", 65531)).is_ok() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let tcp_port = free_tcp_port();
        let (mut child, log_path, _) = spawn(temp.path(), tcp_port, &[]);
        let listening = "v2 listen success on 127.0.0.1:65531";
        if !wait_until(Duration::from_secs(10), || log_text(&log_path).contains(listening)) {
            let _ = child.kill();
            let _ = wait_child(&mut child, Duration::from_secs(5));
            panic!(
                "as-cmd without --v2-port must enable v2 on 65531\nlog:\n{}",
                log_text(&log_path)
            );
        }
        interrupt(&child);
        let status = wait_child(&mut child, Duration::from_secs(15)).unwrap_or_else(|| {
            panic!("as-cmd did not exit\nstderr: {}", read_stderr(&mut child))
        });
        assert!(status.success(), "graceful stop must exit 0, got {status:?}");
    } else {
        eprintln!("skipping: UDP 65531 is already in use on this host");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_listener_starts_with_logs_and_stops_on_sigint() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let tcp_port = free_tcp_port();
    let v2_port = free_udp_port();

    let (mut child, log_path, _) = spawn(temp.path(), tcp_port, &[&format!("--v2-port={v2_port}")]);

    let listening = format!("v2 listen success on 127.0.0.1:{v2_port}");
    if !wait_until(Duration::from_secs(10), || {
        let log = log_text(&log_path);
        log.contains(&listening) && log.contains("fingerprint=")
    }) {
        let _ = child.kill();
        let _ = wait_child(&mut child, Duration::from_secs(5));
        panic!(
            "as-cmd did not bring up the v2 listener ({listening:?})\nlog:\n{}",
            log_text(&log_path)
        );
    }

    // The QUIC endpoint holds its UDP port for the lifetime of the process.
    let rebound = UdpSocket::bind(("127.0.0.1", v2_port));
    assert!(
        rebound.is_err(),
        "v2 UDP port {v2_port} must be held by the running as-cmd"
    );

    assert_identity_layout(temp.path());
    // Ticket 07 wires the §5.2 session driver: the authorization store opens
    // (pairing.md §8.1) and admitted connections are driven, not parked.
    let log = log_text(&log_path);
    assert!(
        log.contains("v2 authorization store ready"),
        "the authorization store must open with the v2 listener:\n{log}"
    );
    assert!(
        log.contains("persistent"),
        "the store must be the on-disk `pairing/clients.json` (pairing.md §8.1):\n{log}"
    );

    interrupt(&child);
    let status = wait_child(&mut child, Duration::from_secs(15)).unwrap_or_else(|| {
        panic!("as-cmd did not exit after SIGINT\nstderr: {}", read_stderr(&mut child))
    });
    assert!(status.success(), "graceful stop must exit 0, got {status:?}");

    let log = log_text(&log_path);
    assert!(
        log.contains("v2 listener stopped"),
        "the v2 stop must be logged:\n{log}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_disabled_with_zero_port_is_v1_only() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let tcp_port = free_tcp_port();

    let (mut child, log_path, _) = spawn(temp.path(), tcp_port, &["--v2-port=0"]);

    let listening = format!("tcp listen success on 127.0.0.1:{tcp_port}");
    if !wait_until(Duration::from_secs(10), || log_text(&log_path).contains(&listening)) {
        let _ = child.kill();
        let _ = wait_child(&mut child, Duration::from_secs(5));
        panic!("as-cmd did not start the v1 server\nlog:\n{}", log_text(&log_path));
    }
    // Give the startup path a beat to (not) log v2 lines.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let log = log_text(&log_path);
    assert!(
        log.contains("v2 transport disabled"),
        "--v2-port=0 must be acknowledged in the log:\n{log}"
    );
    assert!(
        !log.contains("v2 listen success"),
        "v2 must not listen when disabled:\n{log}"
    );
    assert!(
        !temp.path().join("identity").exists(),
        "the identity key must not be created when v2 is disabled"
    );
    assert!(
        !temp.path().join("pairing").exists(),
        "the authorization store must not be created when v2 is disabled"
    );

    interrupt(&child);
    let status = wait_child(&mut child, Duration::from_secs(15))
        .unwrap_or_else(|| panic!("as-cmd did not exit\nstderr: {}", read_stderr(&mut child)));
    assert!(status.success(), "graceful stop must exit 0, got {status:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loose_permissions_refuse_v2_without_killing_v1_and_flag_overrides() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let tcp_port = free_tcp_port();
    let v2_port = free_udp_port();

    // Phase 1: mint a valid identity with default (tight) permissions.
    let (mut child, log_path, _) =
        spawn(temp.path(), tcp_port, &[&format!("--v2-port={v2_port}")]);
    let listening = format!("v2 listen success on 127.0.0.1:{v2_port}");
    assert!(
        wait_until(Duration::from_secs(10), || log_text(&log_path).contains(&listening)),
        "phase-1 v2 must start\nlog: {}",
        log_text(&log_path)
    );
    interrupt(&child);
    let status = wait_child(&mut child, Duration::from_secs(15))
        .unwrap_or_else(|| panic!("phase-1 as-cmd did not exit"));
    assert!(status.success(), "phase-1 graceful stop must exit 0");

    // Phase 2: loosen the permissions; v2 must refuse, v1 must keep running.
    let identity_dir = temp.path().join("identity");
    let key = identity_dir.join("server-identity.pem");
    fs::set_permissions(&key, fs::Permissions::from_mode(0o644)).expect("loosen key");
    fs::set_permissions(&identity_dir, fs::Permissions::from_mode(0o755)).expect("loosen dir");

    let tcp_port2 = free_tcp_port();
    let (mut child, log_path, _) = spawn(temp.path(), tcp_port2, &[&format!("--v2-port={v2_port}")]);
    let listening2 = format!("tcp listen success on 127.0.0.1:{tcp_port2}");
    assert!(
        wait_until(Duration::from_secs(10), || log_text(&log_path).contains(&listening2)),
        "phase-2 v1 must still start\nlog: {}",
        log_text(&log_path)
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let log = log_text(&log_path);
    assert!(
        log.contains("refusing to enable v2"),
        "the refusal must be logged:\n{log}"
    );
    assert!(
        log.contains("--allow-loose-key-permissions"),
        "the refusal must name the override flag:\n{log}"
    );
    assert!(
        !log.contains("v2 listen success"),
        "v2 must not listen after the permission refusal:\n{log}"
    );
    // The server is still alive and owns v1.
    assert!(child.try_wait().expect("query child").is_none(), "as-cmd must keep running");
    interrupt(&child);
    let status = wait_child(&mut child, Duration::from_secs(15))
        .unwrap_or_else(|| panic!("phase-2 as-cmd did not exit"));
    assert!(status.success(), "phase-2 graceful stop must exit 0");

    // Phase 3: the override flag enables v2 despite the loose permissions,
    // with the override itself logged (discouraged; pairing.md §10.2).
    let (mut child, log_path, _) = spawn(
        temp.path(),
        tcp_port2,
        &[
            &format!("--v2-port={v2_port}"),
            "--allow-loose-key-permissions",
        ],
    );
    let listening3 = format!("v2 listen success on 127.0.0.1:{v2_port}");
    assert!(
        wait_until(Duration::from_secs(10), || log_text(&log_path).contains(&listening3)),
        "phase-3 v2 must start under the override\nlog: {}",
        log_text(&log_path)
    );
    let log = log_text(&log_path);
    assert!(
        log.contains("loose"),
        "the override must be logged as discouraged (mentioning the loose permissions):\n{log}"
    );
    interrupt(&child);
    let status = wait_child(&mut child, Duration::from_secs(15))
        .unwrap_or_else(|| panic!("phase-3 as-cmd did not exit"));
    assert!(status.success(), "phase-3 graceful stop must exit 0");
}
