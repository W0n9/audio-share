//! CLI-level acceptance-probe tests against the fake v0.3.4 server.
//!
//! The probe is exercised exactly as the acceptance and soak scripts use it:
//! as a subprocess that emits a JSON report.

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

fn harness() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_protocol-harness"))
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("pick a free port");
    listener.local_addr().expect("local address").port()
}

fn wait_for_server(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("fake server did not listen on port {port}");
}

/// Spawn `serve-fake` with short heartbeat timings; kill it on drop.
struct FakeServer {
    child: Child,
    port: u16,
}

impl FakeServer {
    fn start(heartbeat_interval_ms: u64, timeout_ms: u64) -> Self {
        let port = free_port();
        let child = Command::new(harness())
            .args([
                "serve-fake",
                "--bind",
                &format!("127.0.0.1:{port}"),
                "--duration-secs",
                "30",
                "--heartbeat-interval-ms",
                &heartbeat_interval_ms.to_string(),
                "--timeout-ms",
                &timeout_ms.to_string(),
                "--chunk-interval-ms",
                "10",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn fake server");
        wait_for_server(port);
        Self { child, port }
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn run_probe(port: u16, args: &[&str]) -> (bool, Value) {
    let output = Command::new(harness())
        .args(["probe", "--connect", &format!("127.0.0.1:{port}")])
        .args(args)
        .output()
        .expect("run probe");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|error| {
        panic!(
            "probe did not print JSON ({error}); stdout={stdout:?} stderr={:?}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.success(), json)
}

#[test]
fn probe_reports_real_format_framing_and_heartbeats() {
    let server = FakeServer::start(500, 3_000);
    let (ok, report) = run_probe(server.port, &["--duration-secs", "2", "--tone-hz", "0"]);
    assert!(ok, "probe failed: {report}");
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["format"]["encoding"], "f32", "{report}");
    assert_eq!(report["format"]["channels"], 2, "{report}");
    assert_eq!(report["format"]["sample_rate"], 48_000, "{report}");
    assert_eq!(report["format"]["block_align"], 8, "{report}");
    assert_eq!(report["aligned_payload_bytes"], 1_464, "{report}");
    assert_eq!(report["start_play_id"], 1, "{report}");
    assert_eq!(report["alignment_ok"], true, "{report}");
    assert_eq!(report["max_datagram_ok"], true, "{report}");
    assert!(
        report["datagrams"].as_u64().unwrap() > 10,
        "no PCM received: {report}"
    );
    assert!(
        report["pcm_bytes"].as_u64().unwrap() > 0,
        "empty PCM: {report}"
    );
    assert!(
        report["heartbeat_count"].as_u64().unwrap() >= 2,
        "expected two 500 ms heartbeats in two seconds: {report}"
    );
    assert_eq!(report["server_closed"], false, "{report}");
    assert_eq!(report["tone"], Value::Null, "{report}");
}

#[test]
fn probe_observes_the_heartbeat_timeout_and_server_close() {
    // The fake server uses a 100 ms heartbeat and a 500 ms timeout; a client
    // that never replies must be closed on a tick after the timeout.
    let server = FakeServer::start(100, 500);
    let (ok, report) = run_probe(
        server.port,
        &[
            "--duration-secs",
            "4",
            "--no-heartbeat-reply",
            "--expect-server-close",
            "--tone-hz",
            "0",
            "--min-datagrams",
            "0",
        ],
    );
    assert!(ok, "probe failed: {report}");
    assert_eq!(report["server_closed"], true, "{report}");
    let closed_after = report["closed_after_ms"].as_u64().expect("close time");
    assert!(
        (400..=1_500).contains(&closed_after),
        "closed after {closed_after} ms, expected ~500-600 ms: {report}"
    );
}

#[test]
fn probe_reconnects_and_gets_a_fresh_play_id() {
    let server = FakeServer::start(100, 5_000);
    let (ok, report) = run_probe(
        server.port,
        &[
            "--duration-secs",
            "3",
            "--reconnect-every-secs",
            "1",
            "--reconnect-delay-ms",
            "100",
            "--tone-hz",
            "0",
        ],
    );
    assert!(ok, "probe failed: {report}");
    assert!(
        report["reconnects"].as_u64().unwrap() >= 1,
        "no reconnect happened: {report}"
    );
    let ids: Vec<i64> = report["start_play_ids"]
        .as_array()
        .expect("start_play_ids")
        .iter()
        .map(|id| id.as_i64().expect("integer id"))
        .collect();
    assert!(ids.len() >= 2, "{report}");
    assert!(
        ids.windows(2).all(|pair| pair[0] < pair[1]),
        "play ids are not increasing: {ids:?}"
    );
}
