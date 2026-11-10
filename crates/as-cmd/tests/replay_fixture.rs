//! End-to-end: the ticket-01 harness replayer drives a real `as-cmd` process
//! over localhost sockets, against the committed v0.3.4 fixture.

use std::path::PathBuf;

use protocol_harness::replay::{self, ReplayArgs};

#[tokio::test]
async fn harness_replay_of_the_linux_v034_fixture_passes_against_as_cmd() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest_dir.join("../../fixtures/protocol/v0.3.4/linux/f32-48k-stereo");
    assert!(fixture.is_dir(), "missing fixture {}", fixture.display());

    let args = ReplayArgs {
        fixture,
        connect: None,
        server_cmd: Some(PathBuf::from(env!("CARGO_BIN_EXE_as-cmd"))),
        server_arg: vec!["--bind={bind}".to_owned(), "--fake-capture".to_owned()],
        host: "127.0.0.1".to_owned(),
        port: 0,
        heartbeats: 1,
        datagrams: 8,
        timeout_secs: 20,
    };

    replay::run(&args)
        .await
        .expect("protocol-harness replay against as-cmd failed");
}
