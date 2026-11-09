//! Hardware-free smoke path: fake v0.3.4 server + live replay.

use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_protocol-harness"))
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn wait_for_tcp(addr: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if TcpStream::connect(addr).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("fake server did not listen on {addr}");
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn serve_fake_and_live_replay_complete_without_audio_devices() {
    let temp = tempfile::tempdir().unwrap();
    let fixtures = temp.path().join("fixtures");
    let status = harness()
        .args(["synthesize", "--out"])
        .arg(&fixtures)
        .status()
        .unwrap();
    assert!(status.success());

    let fixture = fixtures.join("f32-48k-stereo");
    let port = free_port();
    let addr = format!("127.0.0.1:{port}");
    let child = harness()
        .args(["serve-fake", "--bind", &addr, "--fixture"])
        .arg(&fixture)
        .args([
            "--duration-secs",
            "30",
            "--heartbeat-interval-ms",
            "500",
            "--timeout-ms",
            "3000",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _guard = KillOnDrop(child);
    wait_for_tcp(&addr);

    let output = harness()
        .args(["replay", "--fixture"])
        .arg(&fixture)
        .args([
            "--connect",
            &addr,
            "--heartbeats",
            "2",
            "--datagrams",
            "8",
            "--timeout-secs",
            "10",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "live replay failed; stdout: {}; stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("replay: ok (live)"), "{stdout}");
    assert!(stdout.contains("format_bytes_match=true"), "{stdout}");
}
