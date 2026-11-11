//! Ticket 01 e2e test (i) plus the idle-timeout behavior: both an orderly
//! client goodbye and a server stop must deliver the expected close reason to
//! the client.

mod common;

use common::*;
use proto_client::MediaVerifier;
use proto_core::frame::CloseReason;
use std::time::Duration;

/// (i) Normal close: UNSUBSCRIBE then CLOSE(NORMAL) gets CLOSE(NORMAL) back
/// and the transport closes with the matching application code.
#[tokio::test]
async fn normal_close_delivers_close_normal_to_the_client() {
    init_test_logging();
    let server = spawn_server().await;
    let cfg = client_config(&server);
    let mut conn = proto_client::connect(&cfg).await.expect("handshake ok");
    conn.auth(TEST_CREDENTIAL).await.expect("auth ok");
    conn.subscribe().await.expect("subscribe");

    // Drain a little media so the UNSUBSCRIBE path stops a running pump.
    let mds = conn.max_datagram_size().expect("datagrams supported");
    let mut verifier = MediaVerifier::new(2);
    for _ in 0..10 {
        let dg = conn.read_datagram().await.expect("datagram");
        verifier.check(&dg, mds);
    }

    conn.unsubscribe().await.expect("unsubscribe");
    conn.send_close_normal().await.expect("close normal sent");
    let reason = conn
        .expect_close(Duration::from_secs(2))
        .await
        .expect("server acknowledges the goodbye");
    assert_eq!(reason, CloseReason::Normal);
    let (code, text) = conn
        .wait_transport_closed(Duration::from_secs(2))
        .await
        .expect("transport closes");
    assert_eq!(code, 0, "normal close uses application code 0");
    assert_eq!(text, "normal");

    let stats = server.session_stats();
    assert_eq!(stats[0].close_sent_reason.as_deref(), Some("Normal"));
    assert_eq!(stats[0].close_received_reason.as_deref(), Some("Normal"));
    server.stop().await;
}

/// (i) Server stop: a connected, subscribed client receives
/// CLOSE(SERVER_STOP) before the transport goes away.
#[tokio::test]
async fn server_stop_delivers_server_stop_reason_to_the_client() {
    init_test_logging();
    let server = spawn_server().await;
    let cfg = client_config(&server);
    let mut conn = proto_client::connect(&cfg).await.expect("handshake ok");
    conn.auth(TEST_CREDENTIAL).await.expect("auth ok");
    conn.subscribe().await.expect("subscribe");
    let mds = conn.max_datagram_size().expect("datagrams supported");
    let dg = conn.read_datagram().await.expect("at least one datagram");
    assert!(dg.len() > proto_core::media::MEDIA_HEADER_LEN);

    server.stop().await;

    let reason = conn
        .expect_close(Duration::from_secs(3))
        .await
        .expect("server sends CLOSE before stopping");
    assert_eq!(reason, CloseReason::ServerStop);
    let (code, text) = conn
        .wait_transport_closed(Duration::from_secs(2))
        .await
        .expect("transport closes after server stop");
    assert_eq!(code, 1, "server stop uses application code 1");
    assert_eq!(text, "server-stop");

    let stats = server.session_stats();
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].close_sent_reason.as_deref(), Some("ServerStop"));
    assert_eq!(stats[0].state, "CLOSED");
}

/// An authenticated-but-idle session (READY, no media) is closed with
/// CLOSE(IDLE_TIMEOUT); a subscribed session is exempt while media flows.
#[tokio::test]
async fn idle_ready_session_gets_idle_timeout_close_but_playing_is_exempt() {
    init_test_logging();
    let server = spawn_server_with(|cfg| {
        cfg.idle_timeout = Some(Duration::from_millis(300));
    })
    .await;
    let cfg = client_config(&server);

    let mut idle = proto_client::connect(&cfg).await.expect("handshake ok");
    idle.auth(TEST_CREDENTIAL).await.expect("auth ok");
    let reason = idle
        .expect_close(Duration::from_secs(3))
        .await
        .expect("idle READY session closed by server");
    assert_eq!(reason, CloseReason::IdleTimeout);
    let (code, text) = idle
        .wait_transport_closed(Duration::from_secs(2))
        .await
        .expect("transport closes on idle timeout");
    assert_eq!(code, 4);
    assert_eq!(text, "idle-timeout");

    // PLAYING is exempt: media flow counts as activity.
    let mut playing = proto_client::connect(&cfg).await.expect("second handshake ok");
    playing.auth(TEST_CREDENTIAL).await.expect("auth ok");
    playing.subscribe().await.expect("subscribe");
    let mds = playing.max_datagram_size().expect("datagrams supported");
    let mut verifier = MediaVerifier::new(2);
    let deadline = std::time::Instant::now() + Duration::from_millis(900);
    while std::time::Instant::now() < deadline {
        let dg = tokio::time::timeout(Duration::from_millis(400), playing.read_datagram())
            .await
            .expect("media still flowing well past the idle timeout")
            .expect("datagram");
        verifier.check(&dg, mds);
    }
    assert!(verifier.datagrams > 100, "media continuously flowing");

    // Once unsubscribed (READY, silent), the idle timer applies again.
    playing.unsubscribe().await.expect("unsubscribe");
    let reason = playing
        .expect_close(Duration::from_secs(3))
        .await
        .expect("idle READY session after unsubscribe closed");
    assert_eq!(reason, CloseReason::IdleTimeout);
    server.stop().await;
}
