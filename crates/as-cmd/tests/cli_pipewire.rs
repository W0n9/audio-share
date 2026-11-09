#![cfg(target_os = "linux")]
//! Real PipeWire behavior of the `as-cmd` process.
//!
//! The ignored test is also its own driver: invoked normally (with
//! `--ignored`), it starts a disposable PipeWire + WirePlumber session with a
//! null `Audio/Sink` and re-invokes this test binary inside it. The inner run
//! asserts `-l` lists the real endpoint, drives a live v0.3.4 protocol replay
//! through the real capture backend, and verifies Ctrl-C stops cleanly.
//!
//! ```text
//! cargo test -p as-cmd --test cli_pipewire -- --ignored --nocapture
//! ```
//!
//! It skips cleanly (with a message) when PipeWire or `dbus-run-session` is not
//! installed, so CI without an audio daemon still passes.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use protocol_harness::replay::{self, ReplayArgs};

const HEADLESS_MARKER: &str = "AUDIO_SHARE_AS_CMD_HEADLESS_TEST";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn fixture() -> PathBuf {
    repo_root().join("fixtures/protocol/v0.3.4/linux/f32-48k-stereo")
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("pick a free port");
    listener.local_addr().expect("local address").port()
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

fn run_headless_session() {
    if Command::new("pipewire").arg("--version").output().is_err() {
        eprintln!("skipping: pipewire is not installed");
        return;
    }
    if Command::new("dbus-run-session")
        .arg("--help")
        .output()
        .is_err()
    {
        eprintln!("skipping: dbus-run-session is not installed");
        return;
    }
    let script = repo_root().join("scripts/pw-test-session.sh");
    if !script.is_file() {
        eprintln!("skipping: {} is missing", script.display());
        return;
    }

    let temp = tempfile::tempdir().expect("temporary directory");
    let inner = temp.path().join("run.sh");
    fs::write(
        &inner,
        r#"set -euo pipefail
exec timeout 180s "$AS_CMD_TEST_EXE" --ignored --exact pipewire_list_endpoints_and_live_replay --nocapture
"#,
    )
    .expect("write inner session script");

    let output = Command::new("bash")
        .arg(&script)
        .arg("--")
        .arg("bash")
        .arg(&inner)
        .env(HEADLESS_MARKER, "1")
        .env(
            "AS_CMD_TEST_EXE",
            std::env::current_exe().expect("test executable"),
        )
        .output()
        .expect("run headless PipeWire driver");
    assert!(
        output.status.success(),
        "headless PipeWire as-cmd test failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

async fn run_in_headless_session() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let config = temp.path().join("config.ini");
    let as_cmd = env!("CARGO_BIN_EXE_as-cmd");

    // 1. `-l` enumerates the real TestSink and marks the default.
    let list = Command::new(as_cmd)
        .arg("--list-endpoint")
        .env("AUDIO_SHARE_CONFIG", &config)
        .output()
        .expect("run as-cmd -l");
    assert!(
        list.status.success(),
        "as-cmd -l failed with {}\nstderr:\n{}",
        list.status,
        String::from_utf8_lossy(&list.stderr)
    );
    let list_stdout = String::from_utf8_lossy(&list.stdout);
    assert!(
        list_stdout.contains("endpoint list:"),
        "missing endpoint header:\n{list_stdout}"
    );
    assert!(
        list_stdout.contains("name: TestSink"),
        "the real null sink is missing:\n{list_stdout}"
    );
    assert!(
        list_stdout
            .lines()
            .any(|line| line.contains("* id:") && line.contains("name: TestSink")),
        "the default sink must carry the * marker:\n{list_stdout}"
    );
    let total = list_stdout
        .lines()
        .find_map(|line| line.strip_prefix("total: "))
        .and_then(|value| value.parse::<usize>().ok())
        .expect("total: N line");
    assert!(total >= 1, "expected at least one endpoint:\n{list_stdout}");

    // 2. Live replay through the real PipeWire backend: the harness starts
    //    as-cmd itself and checks GET_FORMAT/START_PLAY/heartbeat/UDP framing.
    std::env::set_var("AUDIO_SHARE_CONFIG", &config);
    replay::run(&ReplayArgs {
        fixture: fixture(),
        connect: None,
        server_cmd: Some(PathBuf::from(as_cmd)),
        server_arg: vec!["--bind={bind}".to_owned()],
        host: "127.0.0.1".to_owned(),
        port: 0,
        heartbeats: 1,
        datagrams: 8,
        timeout_secs: 20,
    })
    .await
    .expect("protocol-harness replay through the real PipeWire backend");

    // 3. A real-backend server stops gracefully on Ctrl-C (exit 0).
    let port = free_port();
    let log_path = temp.path().join("server.log");
    let mut child = Command::new(as_cmd)
        .arg("--bind")
        .arg(format!("127.0.0.1:{port}"))
        .env("AUDIO_SHARE_CONFIG", &config)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn as-cmd with the real backend");
    let listening = format!("tcp listen success on 127.0.0.1:{port}");
    if !wait_until(Duration::from_secs(15), || {
        fs::read_to_string(&log_path)
            .map(|text| text.contains(&listening))
            .unwrap_or(false)
    }) {
        let _ = child.kill();
        let _ = wait_child(&mut child, Duration::from_secs(5));
        let stderr = child
            .stderr
            .take()
            .map(|mut pipe| {
                let mut text = String::new();
                let _ = std::io::Read::read_to_string(&mut pipe, &mut text);
                text
            })
            .unwrap_or_default();
        panic!("real-backend as-cmd did not start within 15s\nstderr:\n{stderr}");
    }
    interrupt(&child);
    let status = wait_child(&mut child, Duration::from_secs(15))
        .unwrap_or_else(|| panic!("real-backend as-cmd did not stop after SIGINT"));
    assert!(status.success(), "Ctrl-C must exit 0, got {status:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "starts real headless PipeWire/wireplumber daemons"]
async fn pipewire_list_endpoints_and_live_replay() {
    if std::env::var_os(HEADLESS_MARKER).is_some() {
        run_in_headless_session().await;
    } else {
        run_headless_session();
    }
}
