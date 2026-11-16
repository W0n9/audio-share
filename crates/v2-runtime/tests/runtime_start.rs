//! Shared-runtime lifecycle: the exact assembly the GUI and `as-cmd` use.

use std::sync::Arc;

use audio_share_core::capture::{synthetic_backend, CaptureConfig, EncodingPreference};
use audio_share_core::protocol::{HEARTBEAT_INTERVAL, SESSION_TIMEOUT};
use audio_share_core::server::{Server, ServerConfig};
use v2_runtime::admin::PairingMode;
use v2_runtime::{RuntimeConfig, V2Runtime};

#[tokio::test]
async fn starts_binds_opens_pairing_and_stops() {
    let dir = tempfile::tempdir().expect("tempdir");
    let v1 = Server::start(
        ServerConfig {
            host: "127.0.0.1".to_owned(),
            port: 0,
            heartbeat_interval: HEARTBEAT_INTERVAL,
            heartbeat_timeout: SESSION_TIMEOUT,
        },
        Arc::new(synthetic_backend().expect("synthetic backend")),
        CaptureConfig {
            endpoint_id: "default".to_owned(),
            encoding: EncodingPreference::Default,
            channels: 0,
            sample_rate: 0,
        },
    )
    .await
    .expect("v1 server");

    let runtime = V2Runtime::start(
        RuntimeConfig {
            data_dir: dir.path().to_path_buf(),
            allow_loose_permissions: false,
            bind_host: "127.0.0.1".to_owned(),
            port: 0,
            addr_hint: None,
        },
        Some((v1.capture_observer(), v1.media_tap().clone())),
    )
    .await
    .expect("v2 runtime");

    assert_eq!(
        runtime.local_addr().ip(),
        "127.0.0.1".parse::<std::net::IpAddr>().unwrap()
    );
    assert_ne!(runtime.local_addr().port(), 0);
    assert!(
        runtime
            .addr_hint()
            .ends_with(&format!(":{}", runtime.local_addr().port())),
        "addr hint {} must carry the bound port",
        runtime.addr_hint()
    );
    assert_eq!(runtime.fingerprint(), runtime.admin().fingerprint());

    let invitation = runtime.admin().begin(PairingMode::Qr).expect("begin QR");
    let uri = invitation.qr_uri.expect("QR URI");
    let parsed = v2_transport::pairing::parse_qr_uri(&uri).expect("QR payload parses");
    assert_eq!(parsed.addr_hint, runtime.addr_hint());

    runtime.stop();
    runtime.stop(); // idempotent
    v1.stop().await.expect("stop v1");
}

#[tokio::test]
async fn a_corrupt_authorization_store_fails_closed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let v1 = Server::start(
        ServerConfig {
            host: "127.0.0.1".to_owned(),
            port: 0,
            heartbeat_interval: HEARTBEAT_INTERVAL,
            heartbeat_timeout: SESSION_TIMEOUT,
        },
        Arc::new(synthetic_backend().expect("synthetic backend")),
        CaptureConfig {
            endpoint_id: "default".to_owned(),
            encoding: EncodingPreference::Default,
            channels: 0,
            sample_rate: 0,
        },
    )
    .await
    .expect("v1 server");

    let pairing_dir = dir.path().join("pairing");
    std::fs::create_dir_all(&pairing_dir).expect("pairing dir");
    std::fs::write(pairing_dir.join("clients.json"), "{not json").expect("corrupt store");

    let error = V2Runtime::start(
        RuntimeConfig {
            data_dir: dir.path().to_path_buf(),
            allow_loose_permissions: false,
            bind_host: "127.0.0.1".to_owned(),
            port: 0,
            addr_hint: None,
        },
        Some((v1.capture_observer(), v1.media_tap().clone())),
    )
    .await
    .expect_err("a corrupt store must refuse v2");
    let message = error.to_string();
    assert!(
        message.contains("authorization store"),
        "error was {message}"
    );

    v1.stop().await.expect("stop v1");
}
