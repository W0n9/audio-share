#![cfg(unix)]
//! Live `as-cmd` process: starts the Server with synthetic capture, drives the
//! committed v0.3.4 wire fixture through the protocol harness, then stops it
//! with SIGINT. Also verifies `server.log` creation, truncation, line format,
//! and that `-V` raises the level to trace.

use std::fs;
use std::io::Read;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use protocol_harness::replay::{self, ReplayArgs};

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("pick a free port");
    listener.local_addr().expect("local address").port()
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/protocol/v0.3.4/linux/f32-48k-stereo")
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn starts_serves_the_fixture_and_stops_gracefully_on_sigint() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let config = temp.path().join("config.ini");
    let log_path = temp.path().join("server.log");
    let stdout_path = temp.path().join("stdout.log");
    let port = free_port();

    let stdout_file = fs::File::create(&stdout_path).expect("stdout capture file");
    let mut child = Command::new(env!("CARGO_BIN_EXE_as-cmd"))
        .arg("-V")
        .arg("-b")
        .arg(format!("127.0.0.1:{port}"))
        .arg("--fake-capture")
        .env("AUDIO_SHARE_CONFIG", &config)
        .stdout(Stdio::from(
            stdout_file.try_clone().expect("clone stdout file"),
        ))
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn as-cmd");

    let log_text = || fs::read_to_string(&log_path).unwrap_or_default();
    let listening = format!("tcp listen success on 127.0.0.1:{port}");
    if !wait_until(Duration::from_secs(10), || log_text().contains(&listening)) {
        let _ = child.kill();
        let status = wait_child(&mut child, Duration::from_secs(5));
        panic!(
            "as-cmd did not log {listening:?} within 10s (status {status:?})\nstderr:\n{}",
            read_stderr(&mut child)
        );
    }

    // The committed fixture expects F32 stereo 48 kHz, exactly what
    // --fake-capture produces. GET_FORMAT must match the fixture byte for byte.
    replay::run(&ReplayArgs {
        fixture: fixture(),
        connect: Some(format!("127.0.0.1:{port}")),
        server_cmd: None,
        server_arg: Vec::new(),
        host: "127.0.0.1".to_owned(),
        port,
        heartbeats: 1,
        datagrams: 8,
        timeout_secs: 20,
    })
    .await
    .expect("protocol-harness replay against a live as-cmd");

    // A connection opened before the stop must be closed by the graceful stop.
    let mut session = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect session");
    session
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set read timeout");

    interrupt(&child);
    let status = wait_child(&mut child, Duration::from_secs(15)).unwrap_or_else(|| {
        panic!(
            "as-cmd did not exit after SIGINT\nstderr:\n{}",
            read_stderr(&mut child)
        )
    });
    assert!(
        status.success(),
        "graceful Ctrl-C must exit 0, got {status:?}\nstderr:\n{}",
        read_stderr(&mut child)
    );

    let mut byte = [0u8; 1];
    match session.read(&mut byte) {
        Ok(0) => {}
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("expected the session to close after stop, got {other:?}"),
    }

    let log = log_text();
    assert!(
        log.contains("[info]"),
        "server.log must use [info] lines:\n{log}"
    );
    assert!(
        log.contains("[trace]"),
        "-V must raise the log level to trace:\n{log}"
    );
    assert!(log.contains(&listening), "missing listen line:\n{log}");
    assert!(log.contains("server stopped"), "missing stop line:\n{log}");

    let stdout = fs::read_to_string(&stdout_path).expect("read captured stdout");
    assert!(
        stdout.contains(&listening),
        "stdout must mirror the log:\n{stdout}"
    );

    // A fresh launch truncates server.log rather than appending.
    fs::OpenOptions::new()
        .append(true)
        .open(&log_path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, b"STALE-MARKER\n"))
        .expect("append stale marker");

    let second_port = free_port();
    let second_stdout = fs::File::create(temp.path().join("stdout-2.log")).expect("stdout file");
    let mut second = Command::new(env!("CARGO_BIN_EXE_as-cmd"))
        .arg("--bind")
        .arg(format!("127.0.0.1:{second_port}"))
        .arg("--fake-capture")
        .env("AUDIO_SHARE_CONFIG", &config)
        .stdout(Stdio::from(second_stdout))
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn second as-cmd");

    let second_listening = format!("tcp listen success on 127.0.0.1:{second_port}");
    let truncated = wait_until(Duration::from_secs(10), || {
        let text = log_text();
        text.contains(&second_listening) && !text.contains("STALE-MARKER")
    });
    if !truncated {
        let _ = second.kill();
        let _ = wait_child(&mut second, Duration::from_secs(5));
        panic!("server.log was not truncated on launch:\n{}", log_text());
    }

    interrupt(&second);
    let status = wait_child(&mut second, Duration::from_secs(15)).unwrap_or_else(|| {
        panic!(
            "second as-cmd did not exit\nstderr:\n{}",
            read_stderr(&mut second)
        )
    });
    assert!(
        status.success(),
        "second graceful stop must exit 0, got {status:?}\nstderr:\n{}",
        read_stderr(&mut second)
    );
}
