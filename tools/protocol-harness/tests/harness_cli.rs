//! CLI-level tests for the protocol harness.
//!
//! These exercise the documented commands that CI and the docs rely on.

use std::fs;
use std::process::Command;

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_protocol-harness"))
}

#[test]
fn synthesize_then_replay_offline_validates_fixture_sets() {
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("windows");

    let status = harness()
        .args(["synthesize", "--out"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success(), "synthesize failed");

    for case in [
        "f32-48k-stereo",
        "s16-44k1-stereo",
        "s16-48k-mono",
        "s24-48k-stereo",
    ] {
        let dir = out.join(case);
        assert!(
            dir.join("manifest.json").is_file(),
            "missing {case} manifest"
        );

        let output = harness()
            .args(["replay", "--fixture"])
            .arg(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "replay {case} failed; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("f32-48k-stereo/manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["provenance"], "synthesized-v0.3.4-semantics");
    assert_eq!(manifest["platform"], "windows");
    assert_eq!(manifest["protocol_version"], "v0.3.4");
    assert_eq!(manifest["audio_format"]["protobuf_hex"], "080110021880f702");
    assert_eq!(manifest["audio_format"]["block_align"], 8);
}

#[test]
fn record_drives_a_server_and_writes_a_replayable_fixture_set() {
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("recorded");
    let exe = env!("CARGO_BIN_EXE_protocol-harness");

    let output = Command::new(exe)
        .args([
            "record",
            "--server-cmd",
            exe,
            "--server-arg",
            "serve-fake",
            "--server-arg",
            "--bind",
            "--server-arg",
            "127.0.0.1:{port}",
            "--server-arg",
            "--duration-secs",
            "--server-arg",
            "30",
            "--server-arg",
            "--heartbeat-interval-ms",
            "--server-arg",
            "300",
            "--server-arg",
            "--timeout-ms",
            "--server-arg",
            "2000",
            "--server-arg",
            "--chunk-interval-ms",
            "--server-arg",
            "5",
            "--out",
        ])
        .arg(&out)
        .args([
            "--heartbeats",
            "2",
            "--datagrams",
            "10",
            "--timeout-secs",
            "10",
            "--provenance",
            "synthesized-v0.3.4-semantics",
            "--platform",
            "test",
            "--server-label",
            "serve-fake",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "record failed; stdout: {}; stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let replay = Command::new(exe)
        .args(["replay", "--fixture"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        replay.status.success(),
        "recorded fixture failed offline replay; stderr: {}",
        String::from_utf8_lossy(&replay.stderr)
    );

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["provenance"], "synthesized-v0.3.4-semantics");
    assert_eq!(manifest["platform"], "test");
    assert_eq!(manifest["capture"]["heartbeats_received"], 2);
    assert_eq!(manifest["capture"]["datagrams_received"], 10);
}
