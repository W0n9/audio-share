//! Process-level `as-cmd pair` acceptance (ticket 19).
//!
//! Starts the real binary in its foreground pairing mode, completes the SAS
//! handshake over live QUIC with a fresh P-256 identity, and checks that the
//! Server persisted the Client after the explicit `--accept` decision.

use std::net::SocketAddr;
use std::process::Stdio;
use std::time::Duration;

use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};

use v2_transport::auth;
use v2_transport::codes::ALPN_PROTOCOL;
use v2_transport::framing::{check_length_prefix, decode_body, encode_frame, LengthCheck};
use v2_transport::pairing::{self, PAIRING_EXPORTER_LABEL};
use v2_transport::schema::{
    frame, Frame, PairingConfirm, PairingMode, PairingRequest, PairingResponse,
};

/// Serialize real-UDP process tests across test binaries: a released
/// ephemeral port must not be reused while another `as-cmd pair` child is
/// still binding it.
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

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn public_key(key: &SigningKey) -> Vec<u8> {
    key.verifying_key()
        .to_encoded_point(true)
        .as_bytes()
        .to_vec()
}

fn free_udp_port() -> u16 {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("free UDP port");
    socket.local_addr().expect("local address").port()
}

async fn connect_with_retry(address: SocketAddr) -> quinn::Connection {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let attempt = v2_transport::test_support::connect_with_alpn(address, ALPN_PROTOCOL)
            .expect("client endpoint");
        match tokio::time::timeout(Duration::from_millis(500), attempt).await {
            Ok(Ok(connection)) => return connection,
            _ if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(Err(error)) => panic!("QUIC handshake failed: {error}"),
            Err(_) => panic!("QUIC handshake timed out"),
        }
    }
}

async fn read_frame(recv: &mut quinn::RecvStream) -> Frame {
    let mut prefix = [0u8; 4];
    recv.read_exact(&mut prefix).await.expect("frame prefix");
    let body_len = match check_length_prefix(prefix) {
        LengthCheck::Accept { body_len } => body_len,
        other => panic!("invalid length prefix: {other:?}"),
    };
    let mut body = vec![0; body_len];
    recv.read_exact(&mut body).await.expect("frame body");
    decode_body(&body).expect("decodable frame")
}

async fn send_frame(send: &mut quinn::SendStream, body: frame::Body) {
    send.write_all(&encode_frame(&Frame { body: Some(body) }).expect("encode"))
        .await
        .expect("write frame");
}

async fn pair_sas(connection: &quinn::Connection, key: &SigningKey) -> PairingResponse {
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    send_frame(
        &mut send,
        frame::Body::PairingRequest(PairingRequest {
            request_id: 1,
            pairing_mode: PairingMode::Sas as i32,
            pairing_nonce: Vec::new(),
            client_random: vec![0x42; 32],
            client_public_key: public_key(key),
            protocol_major: 2,
            capabilities: Vec::new(),
            client_name: "CLI Test Phone".to_owned(),
        }),
    )
    .await;
    let response = match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResponse(response)) => response,
        other => panic!("expected PairingResponse, got {other:?}"),
    };
    let context = pairing::binding_context(
        PairingMode::Sas,
        &[],
        &response.client_random,
        &response.server_random,
        &public_key(key),
        &response.server_public_key,
        &response.capabilities,
        &response.display_name,
    )
    .expect("binding context");
    let mut binding = [0u8; 48];
    connection
        .export_keying_material(&mut binding, PAIRING_EXPORTER_LABEL, &context)
        .expect("TLS exporter");
    let proof: Signature = key.sign(&auth::build_pairing_proof_input(&binding));
    send_frame(
        &mut send,
        frame::Body::PairingConfirm(PairingConfirm {
            request_id: 1,
            binding_echo: binding.to_vec(),
            client_proof: proof.to_vec(),
        }),
    )
    .await;
    match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResult(result)) => assert_eq!(result.status.code, 0),
        other => panic!("expected PairingResult(OK), got {other:?}"),
    }
    response
}

/// Run one real `as-cmd pair --sas --accept` process and complete the SAS
/// handshake. `pair_sas` asserts that the Client observes `PairingResult(OK)`;
/// a 503/transport close fails here.
async fn run_pair_accept_once(iteration: usize) -> (tempfile::TempDir, SigningKey, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let port = free_udp_port();
    let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_as-cmd"))
        .args([
            "pair",
            "--sas",
            "--accept",
            "--timeout",
            "15",
            "--v2-port",
            &port.to_string(),
        ])
        .env("AUDIO_SHARE_CONFIG", temp.path().join("config.ini"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("iteration {iteration}: spawn as-cmd pair: {error}"));

    let address: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let connection = connect_with_retry(address).await;
    let key = SigningKey::random(&mut rand_core::OsRng);
    pair_sas(&connection, &key).await;
    connection.close(0u32.into(), b"");

    let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .unwrap_or_else(|_| panic!("iteration {iteration}: as-cmd pair did not exit"))
        .unwrap_or_else(|error| panic!("iteration {iteration}: wait for as-cmd: {error}"));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "iteration {iteration}: pair exited with {:?}\nstdout:\n{stdout}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    for expected in [
        "Pairing mode: SAS",
        "Server fingerprint:",
        "Pairing code:",
        "Paired: CLI Test Phone",
    ] {
        assert!(
            stdout.contains(expected),
            "iteration {iteration}: missing {expected:?}\nstdout:\n{stdout}"
        );
    }
    (temp, key, stdout)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn as_cmd_pair_accept_persists_the_client_and_exits() {
    let _port_lock = PortLock::acquire();
    let (temp, key, _stdout) = run_pair_accept_once(0).await;

    let store = v2_transport::auth_store::AuthorizationStore::open(temp.path())
        .expect("authorization store");
    let clients = store.clients();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].display_name, "CLI Test Phone");
    assert_eq!(clients[0].public_key, public_key(&key));
    assert!(!clients[0].created_at.is_empty());
    // Directory/file permissions from pairing.md §10.2.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = temp.path().join("pairing");
        let file = dir.join("clients.json");
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn as_cmd_pair_accept_never_races_the_success_result() {
    // Regression for the 503/transport-close race: the one-shot CLI must not
    // stop the runtime until the driver has written PairingResult(OK). Twenty
    // consecutive live pairings must each observe the success result.
    let _port_lock = PortLock::acquire();
    for iteration in 0..20 {
        let (temp, _key, _stdout) = run_pair_accept_once(iteration).await;
        drop(temp);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn as_cmd_pair_interactive_decline_stores_nothing() {
    use tokio::io::AsyncWriteExt as _;
    let _port_lock = PortLock::acquire();
    let temp = tempfile::tempdir().expect("tempdir");
    let port = free_udp_port();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_as-cmd"))
        .args([
            "pair",
            "--sas",
            "--timeout",
            "15",
            "--v2-port",
            &port.to_string(),
        ])
        .env("AUDIO_SHARE_CONFIG", temp.path().join("config.ini"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn as-cmd pair");

    let address: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let connection = connect_with_retry(address).await;
    let key = SigningKey::random(&mut rand_core::OsRng);
    let (mut send, mut recv) = connection.open_bi().await.expect("control stream");
    send_frame(
        &mut send,
        frame::Body::PairingRequest(PairingRequest {
            request_id: 1,
            pairing_mode: PairingMode::Sas as i32,
            pairing_nonce: Vec::new(),
            client_random: vec![0x42; 32],
            client_public_key: public_key(&key),
            protocol_major: 2,
            capabilities: Vec::new(),
            client_name: "CLI Test Phone".to_owned(),
        }),
    )
    .await;
    let response = match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResponse(response)) => response,
        other => panic!("expected PairingResponse, got {other:?}"),
    };
    let context = pairing::binding_context(
        PairingMode::Sas,
        &[],
        &response.client_random,
        &response.server_random,
        &public_key(&key),
        &response.server_public_key,
        &response.capabilities,
        &response.display_name,
    )
    .expect("binding context");
    let mut binding = [0u8; 48];
    connection
        .export_keying_material(&mut binding, PAIRING_EXPORTER_LABEL, &context)
        .expect("TLS exporter");
    let proof: Signature = key.sign(&auth::build_pairing_proof_input(&binding));
    send_frame(
        &mut send,
        frame::Body::PairingConfirm(PairingConfirm {
            request_id: 1,
            binding_echo: binding.to_vec(),
            client_proof: proof.to_vec(),
        }),
    )
    .await;

    // Answer the operator prompt with "n".
    let mut stdin = child.stdin.take().expect("stdin");
    stdin.write_all(b"n\n").await.expect("write decline");
    stdin.shutdown().await.ok();
    match read_frame(&mut recv).await.body {
        Some(frame::Body::PairingResult(result)) => {
            assert_eq!(result.status.code, 201);
            assert!(result.status.detail.is_empty());
        }
        other => panic!("expected PairingResult(201), got {other:?}"),
    }
    assert_eq!(client_count(temp.path()), 0);

    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    let _ = child.kill().await;
}

fn client_count(data_dir: &std::path::Path) -> usize {
    v2_transport::auth_store::AuthorizationStore::open(data_dir)
        .expect("store")
        .clients()
        .len()
}

#[test]
fn qr_payload_in_pair_mode_is_parseable() {
    // The CLI's QR URI comes from the same production encoder the GUI uses.
    let key: [u8; 32] = unhex("11b307d9bf3b84b7941e28d31283e0c813fbbed5b4ffd95ae5504120045afc9c")
        .try_into()
        .unwrap();
    let uri = pairing::encode_qr_uri("192.0.2.10:65531", &key, &[7u8; 16]);
    let parsed = pairing::parse_qr_uri(&uri).expect("parse");
    assert_eq!(parsed.addr_hint, "192.0.2.10:65531");
    assert_eq!(parsed.nonce, [7u8; 16]);
}
