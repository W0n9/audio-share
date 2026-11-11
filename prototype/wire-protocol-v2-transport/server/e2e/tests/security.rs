//! Ticket 01 e2e tests (b), (c), (d): security and version assertions.
//!
//! (b) an incompatible ALPN must fail the handshake before any app data;
//! (c) a server certificate whose SPKI does not match the pin must fail
//!     verification, while the correct pin succeeds;
//! (d) a wrong credential gets AUTH_ERR(BAD_CREDENTIAL) followed by
//!     CLOSE(AUTH_FAILED), while the correct credential succeeds — encrypted
//!     transport alone is never authorization.

mod common;

use common::*;
use proto_client::ClientError;
use proto_core::frame::{AuthErrCode, CloseReason};

#[tokio::test]
async fn wrong_alpn_fails_handshake_before_app_data() {
    init_test_logging();
    let server = spawn_server().await;
    let mut cfg = client_config(&server);
    cfg.alpn = "audio-share/v9".to_string();

    let result = proto_client::connect(&cfg).await;
    let error = result.expect_err("wrong ALPN must fail the handshake");
    let ClientError::Handshake(message) = error else {
        panic!("expected a handshake failure, got {error:?}");
    };
    assert!(
        message.contains("known protocol") || message.contains("application protocol"),
        "handshake failure should be the rustls no-application-protocol alert (120), got: {message}"
    );

    // No session ever existed, so no application data could have flowed.
    let endpoint = server.endpoint_stats();
    assert_eq!(endpoint.accepted, 0, "no connection accepted with wrong ALPN");
    assert_eq!(server.session_stats().len(), 0);
    assert!(
        endpoint.handshake_failures >= 1,
        "handshake failure recorded server-side"
    );
    server.stop().await;
}

#[tokio::test]
async fn wrong_pinned_cert_fails_verification() {
    init_test_logging();
    let server = spawn_server().await;
    let mut cfg = client_config(&server);
    cfg.pinned_spki = rogue_pin();

    let result = proto_client::connect(&cfg).await;
    let error = result.expect_err("unpinned server certificate must fail verification");
    let ClientError::Handshake(message) = error else {
        panic!("expected a handshake failure, got {error:?}");
    };
    assert!(
        message.to_lowercase().contains("pinned value"),
        "failure should be the SPKI pin mismatch, got: {message}"
    );

    let endpoint = server.endpoint_stats();
    assert_eq!(endpoint.accepted, 0, "no connection accepted with a wrong pin");
    server.stop().await;
}

#[tokio::test]
async fn correct_pin_and_wrong_credential_gets_auth_err_then_auth_failed_close() {
    init_test_logging();
    let server = spawn_server().await;
    let cfg = client_config(&server);

    // The correct pin handshakes fine — only the credential is wrong.
    let mut conn = proto_client::connect(&cfg)
        .await
        .expect("correct pin must complete the handshake");
    let error = conn
        .auth("not-the-credential")
        .await
        .expect_err("wrong credential must be rejected");
    let ClientError::AuthRejected { code, close_after } = error else {
        panic!("expected an auth rejection, got {error:?}");
    };
    assert_eq!(code, AuthErrCode::BadCredential);
    assert_eq!(close_after, Some(CloseReason::AuthFailed));

    let stats = server.session_stats();
    assert_eq!(stats.len(), 1);
    let s = &stats[0];
    assert_eq!(s.auth_err_out, 1);
    assert_eq!(s.auth_err_bad_credential, 1);
    assert_eq!(s.auth_ok_out, 0);
    assert_eq!(s.close_sent_reason.as_deref(), Some("AuthFailed"));
    assert_eq!(s.protocol_errors_out, 0, "auth failure is not a protocol error");
    server.stop().await;
}

#[tokio::test]
async fn empty_credential_is_bad_credential_not_protocol_error() {
    init_test_logging();
    let server = spawn_server().await;
    let mut cfg = client_config(&server);
    cfg.credential = String::new();
    let mut conn = proto_client::connect(&cfg).await.expect("handshake ok");
    let error = conn.auth("").await.expect_err("empty credential rejected");
    let ClientError::AuthRejected { code, close_after } = error else {
        panic!("expected an auth rejection, got {error:?}");
    };
    assert_eq!(code, AuthErrCode::BadCredential);
    assert_eq!(close_after, Some(CloseReason::AuthFailed));
    server.stop().await;
}
