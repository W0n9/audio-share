//! Shared-runtime lifecycle: the exact assembly the GUI and `as-cmd` use.

use std::sync::Arc;

use audio_share_core::capture::{synthetic_backend, CaptureConfig, EncodingPreference};
use audio_share_core::protocol::{HEARTBEAT_INTERVAL, SESSION_TIMEOUT};
use audio_share_core::server::{Server, ServerConfig};
use v2_runtime::admin::RequestedPairingMode;
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

    let invitation = runtime
        .admin()
        .begin(RequestedPairingMode::Qr)
        .expect("begin QR");
    let uri = invitation.qr_uri.expect("QR URI");
    let parsed = v2_transport::pairing::parse_qr_uri(&uri).expect("QR payload parses");
    assert_eq!(parsed.addr_hint, runtime.addr_hint());

    runtime.stop();
    runtime.stop(); // idempotent
    v1.stop().await.expect("stop v1");
}

#[tokio::test]
async fn a_configured_host_is_advertised_with_the_bound_port() {
    // Reserve an ephemeral UDP port (QUIC uses UDP) so the runtime binds the
    // same port the hint advertises, mirroring the GUI's fixed v2 port.
    let probe = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("reserve a UDP port");
    let port = probe.local_addr().expect("probe address").port();
    drop(probe);

    let dir = tempfile::tempdir().expect("tempdir");
    let hint = v2_runtime::qr_addr_hint("2001:db8::5", port).expect("IPv6 hint");
    let runtime = V2Runtime::start(
        RuntimeConfig {
            data_dir: dir.path().to_path_buf(),
            allow_loose_permissions: false,
            // The Host only changes the advertisement, not the bind address.
            bind_host: "127.0.0.1".to_owned(),
            port,
            addr_hint: Some(hint.clone()),
        },
        None,
    )
    .await
    .expect("v2 runtime");

    assert_eq!(runtime.local_addr().port(), port);
    assert_eq!(runtime.addr_hint(), hint);
    let invitation = runtime
        .admin()
        .begin(RequestedPairingMode::Qr)
        .expect("begin QR");
    let parsed =
        v2_transport::pairing::parse_qr_uri(&invitation.qr_uri.expect("QR URI")).expect("parses");
    assert_eq!(parsed.addr_hint, hint);

    runtime.stop();
}

#[tokio::test]
async fn a_second_core_cannot_share_the_data_dir_and_the_lock_releases_on_drop() {
    use v2_runtime::RuntimeError;

    let dir = tempfile::tempdir().expect("tempdir");
    let config = || RuntimeConfig {
        data_dir: dir.path().to_path_buf(),
        allow_loose_permissions: false,
        bind_host: "127.0.0.1".to_owned(),
        port: 0,
        addr_hint: None,
    };

    let runtime = V2Runtime::start(config(), None)
        .await
        .expect("first runtime");
    match V2Runtime::start(config(), None).await {
        Ok(second) => {
            drop(second);
            panic!("a second Core must not share the data directory");
        }
        Err(error) => {
            assert!(
                matches!(error, RuntimeError::Store(_)),
                "unexpected error: {error:?}"
            );
            assert!(
                error.to_string().contains("stop the Server first"),
                "error must tell the operator what to do: {error}"
            );
        }
    }

    // Stopping (and dropping) the runtime releases the store lock for the
    // next Core or an offline admin operation.
    runtime.stop();
    drop(runtime);
    let second = V2Runtime::start(config(), None)
        .await
        .expect("the lock is released when the Core stops");
    drop(second);
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
