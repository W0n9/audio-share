//! Ticket 01 e2e tests (e) and (f): control-plane state-machine violations
//! and malformed/unknown messages must produce explicit errors and closes,
//! never silent ignores.
//!
//! (e) SUBSCRIBE without AUTH → AUTH_ERR(NOT_AUTHENTICATED) + close;
//! (f) malformed / unknown msg_type → CLOSE(PROTOCOL_ERROR).

mod common;

use common::*;
use proto_core::frame::{AuthErrCode, CloseReason};

fn frame_bytes(msg_type: u8, payload: &[u8]) -> Vec<u8> {
    let len = (1 + payload.len()) as u32;
    let mut out = len.to_be_bytes().to_vec();
    out.push(msg_type);
    out.extend_from_slice(payload);
    out
}

/// Reads frames until a CLOSE arrives, asserting any prelude matches.
async fn read_close(
    conn: &mut proto_client::ClientConn,
) -> (Vec<String>, CloseReason, (u64, String)) {
    let mut seen = Vec::new();
    loop {
        match conn.read_frame().await {
            Ok(proto_core::frame::Frame::Close(reason)) => {
                let transport = conn
                    .wait_transport_closed(std::time::Duration::from_secs(2))
                    .await
                    .expect("transport closes after the CLOSE frame");
                return (seen, reason, transport);
            }
            Ok(frame) => seen.push(format!("{frame:?}")),
            Err(e) => panic!("expected a CLOSE frame, got error: {e}"),
        }
    }
}

#[tokio::test]
async fn subscribe_without_auth_gets_not_authenticated_then_protocol_error() {
    init_test_logging();
    let server = spawn_server().await;
    let cfg = client_config(&server);
    let mut conn = proto_client::connect(&cfg).await.expect("handshake ok");

    conn.send_frame(&proto_core::frame::Frame::Subscribe)
        .await
        .expect("SUBSCRIBE sent");

    let (seen, reason, transport) = read_close(&mut conn).await;
    assert_eq!(
        seen,
        vec![format!(
            "{:?}",
            proto_core::frame::Frame::AuthErr(AuthErrCode::NotAuthenticated)
        )],
        "exactly one AUTH_ERR(NOT_AUTHENTICATED) prelude"
    );
    assert_eq!(reason, CloseReason::ProtocolError);
    assert_eq!(transport.0, 2, "transport application code mirrors the reason");
    assert_eq!(transport.1, "protocol-error");

    let stats = server.session_stats();
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].auth_err_not_authenticated, 1);
    assert_eq!(stats[0].protocol_errors_out, 1);
    assert_eq!(stats[0].frames_subscribe_in, 1);
    assert_eq!(stats[0].session_id, 0, "never authenticated");
    assert_eq!(stats[0].media_start_out, 0, "no media ever started");
    server.stop().await;
}

#[tokio::test]
async fn unknown_msg_type_gets_protocol_error_close() {
    init_test_logging();
    let server = spawn_server().await;
    let cfg = client_config(&server);
    let mut conn = proto_client::connect(&cfg).await.expect("handshake ok");
    conn.auth(TEST_CREDENTIAL).await.expect("auth ok");

    conn.send_raw(&frame_bytes(0x99, &[]))
        .await
        .expect("unknown frame sent");
    let (seen, reason, transport) = read_close(&mut conn).await;
    assert!(seen.is_empty(), "no prelude for unknown types, got {seen:?}");
    assert_eq!(reason, CloseReason::ProtocolError);
    assert_eq!(transport.0, 2);
    assert_eq!(server.session_stats()[0].protocol_errors_out, 1);
    server.stop().await;
}

#[tokio::test]
async fn oversized_frame_length_gets_protocol_error_close() {
    init_test_logging();
    let server = spawn_server().await;
    let cfg = client_config(&server);
    let mut conn = proto_client::connect(&cfg).await.expect("handshake ok");
    conn.auth(TEST_CREDENTIAL).await.expect("auth ok");

    // 70000 > the 64 KiB cap; the server must reject on the length prefix.
    let mut bytes = 70000u32.to_be_bytes().to_vec();
    bytes.push(0x01);
    conn.send_raw(&bytes).await.expect("oversized length sent");
    let (_, reason, _) = read_close(&mut conn).await;
    assert_eq!(reason, CloseReason::ProtocolError);
    server.stop().await;
}

#[tokio::test]
async fn bad_payload_length_for_known_type_gets_protocol_error_close() {
    init_test_logging();
    let server = spawn_server().await;
    let cfg = client_config(&server);
    let mut conn = proto_client::connect(&cfg).await.expect("handshake ok");
    conn.auth(TEST_CREDENTIAL).await.expect("auth ok");

    // SUBSCRIBE must be empty; a one-byte payload is a protocol violation.
    conn.send_raw(&frame_bytes(0x04, &[0x00]))
        .await
        .expect("malformed SUBSCRIBE sent");
    let (_, reason, _) = read_close(&mut conn).await;
    assert_eq!(reason, CloseReason::ProtocolError);
    server.stop().await;
}

#[tokio::test]
async fn unknown_close_reason_gets_protocol_error_close() {
    init_test_logging();
    let server = spawn_server().await;
    let cfg = client_config(&server);
    let mut conn = proto_client::connect(&cfg).await.expect("handshake ok");
    conn.auth(TEST_CREDENTIAL).await.expect("auth ok");

    conn.send_raw(&frame_bytes(0x07, &[0x09]))
        .await
        .expect("unknown close reason sent");
    let (_, reason, _) = read_close(&mut conn).await;
    assert_eq!(reason, CloseReason::ProtocolError);
    server.stop().await;
}

#[tokio::test]
async fn non_utf8_auth_payload_gets_malformed_auth_err_then_auth_failed() {
    init_test_logging();
    let server = spawn_server().await;
    let cfg = client_config(&server);
    let mut conn = proto_client::connect(&cfg).await.expect("handshake ok");

    conn.send_raw(&frame_bytes(0x01, &[0xff, 0xfe]))
        .await
        .expect("non-utf8 AUTH sent");
    let (seen, reason, _) = read_close(&mut conn).await;
    assert_eq!(
        seen,
        vec![format!(
            "{:?}",
            proto_core::frame::Frame::AuthErr(AuthErrCode::Malformed)
        )],
        "non-utf8 AUTH is malformed, not a plain bad credential"
    );
    assert_eq!(reason, CloseReason::AuthFailed);
    let stats = server.session_stats();
    assert_eq!(stats[0].auth_err_malformed, 1);
    assert_eq!(stats[0].close_sent_reason.as_deref(), Some("AuthFailed"));
    server.stop().await;
}

#[tokio::test]
async fn duplicate_auth_and_unsubscribe_in_ready_are_protocol_errors() {
    init_test_logging();
    let server = spawn_server().await;
    let cfg = client_config(&server);
    let mut conn = proto_client::connect(&cfg).await.expect("handshake ok");
    conn.auth(TEST_CREDENTIAL).await.expect("auth ok");

    conn.send_frame(&proto_core::frame::Frame::Auth { token: TEST_CREDENTIAL.into() })
        .await
        .expect("duplicate AUTH sent");
    let (_, reason, _) = read_close(&mut conn).await;
    assert_eq!(reason, CloseReason::ProtocolError, "AUTH outside CONNECTING");

    let mut conn2 = proto_client::connect(&cfg).await.expect("second handshake ok");
    conn2.auth(TEST_CREDENTIAL).await.expect("auth ok");
    conn2.send_frame(&proto_core::frame::Frame::Unsubscribe)
        .await
        .expect("UNSUBSCRIBE sent");
    let (_, reason, _) = read_close(&mut conn2).await;
    assert_eq!(reason, CloseReason::ProtocolError, "UNSUBSCRIBE outside PLAYING");
    server.stop().await;
}

#[tokio::test]
async fn resubscribe_without_unsubscribe_is_a_protocol_error() {
    init_test_logging();
    let server = spawn_server().await;
    let cfg = client_config(&server);
    let mut conn = proto_client::connect(&cfg).await.expect("handshake ok");
    conn.auth(TEST_CREDENTIAL).await.expect("auth ok");
    let first = conn.subscribe().await.expect("first subscribe");
    assert_eq!(first.stream_instance_id, 1);

    // SUBSCRIBE is only legal in READY; in PLAYING it is a violation.
    conn.send_frame(&proto_core::frame::Frame::Subscribe)
        .await
        .expect("duplicate SUBSCRIBE sent");
    let (_, reason, _) = read_close(&mut conn).await;
    assert_eq!(reason, CloseReason::ProtocolError);
    server.stop().await;
}

#[tokio::test]
async fn client_claiming_server_only_close_reasons_is_a_protocol_error() {
    init_test_logging();
    let server = spawn_server().await;
    let cfg = client_config(&server);
    let mut conn = proto_client::connect(&cfg).await.expect("handshake ok");
    conn.auth(TEST_CREDENTIAL).await.expect("auth ok");

    conn.send_frame(&proto_core::frame::Frame::Close(CloseReason::ServerStop))
        .await
        .expect("client claiming SERVER_STOP sent");
    let (_, reason, _) = read_close(&mut conn).await;
    assert_eq!(reason, CloseReason::ProtocolError);
    server.stop().await;
}
