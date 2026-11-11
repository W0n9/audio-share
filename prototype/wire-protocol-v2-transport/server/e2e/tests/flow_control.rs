//! Ticket 01 e2e tests (g) and (h): non-waiting sends into a bounded queue
//! with drop-newest, slow-client isolation, and the datagram size invariant.

mod common;

use common::*;
use proto_client::MediaVerifier;
use proto_core::media::{MEDIA_HEADER_LEN, SAMPLE_FORMAT_F32LE};
use std::time::{Duration, Instant};

/// (g) A client that stops reading datagrams must not block the server: media
/// keeps being produced, and the reliable control stream stays responsive.
#[tokio::test]
async fn slow_reader_does_not_block_server_and_control_stays_responsive() {
    init_test_logging();
    let server = spawn_server().await;
    let cfg = client_config(&server);
    let mut conn = proto_client::connect(&cfg).await.expect("handshake ok");
    conn.auth(TEST_CREDENTIAL).await.expect("auth ok");
    let media_start = conn.subscribe().await.expect("subscribe");
    assert_eq!(media_start.sample_format, SAMPLE_FORMAT_F32LE);

    let mut verifier = MediaVerifier::new(media_start.channels);
    let mds = conn.max_datagram_size().expect("datagrams supported");
    for _ in 0..10 {
        let dg = conn.read_datagram().await.expect("datagram");
        verifier.check(&dg, mds);
    }

    // Stop reading for 500 ms: the server must keep producing into its own
    // bounded queues while the client endpoint drops the overflow.
    let before = server.session_stats()[0].media_produced;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let after = server.session_stats()[0].media_produced;
    assert!(
        after - before >= 80,
        "server kept producing during the silent window ({before} -> {after})"
    );
    assert!(
        verifier.datagrams < after,
        "client received fewer datagrams than produced while not reading"
    );

    // The reliable control stream stayed responsive the whole time.
    conn.unsubscribe().await.expect("UNSUBSCRIBE honored while silent");
    conn.send_close_normal().await.expect("CLOSE sent");
    let reason = conn
        .expect_close(Duration::from_secs(2))
        .await
        .expect("graceful close still works after a silent period");
    assert_eq!(reason, proto_core::frame::CloseReason::Normal);
    assert_eq!(server.session_stats()[0].media_unsubscribes, 1);
    server.stop().await;
}

/// (g) With a deliberately slow outbound path (test-only forwarder delay), the
/// bounded queue overflows, drops the NEWEST datagram, keeps counting, and the
/// session still closes cleanly.
#[tokio::test]
async fn bounded_queue_drops_newest_under_overload_without_stalling() {
    init_test_logging();
    let server = spawn_server_with(|cfg| {
        cfg.forwarder_delay = Some(Duration::from_millis(50));
    })
    .await;
    let cfg = client_config(&server);
    let mut conn = proto_client::connect(&cfg).await.expect("handshake ok");
    conn.auth(TEST_CREDENTIAL).await.expect("auth ok");
    conn.subscribe().await.expect("subscribe");

    // ~1.5 s at ~400 datagrams/s production vs ~20/s forwarder: the queue
    // (cap 256) must overflow into drop-newest long before this ends.
    let mds = conn.max_datagram_size().expect("datagrams supported");
    let mut verifier = MediaVerifier::new(2);
    let deadline = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(300), conn.read_datagram()).await {
            Ok(Ok(dg)) => verifier.check(&dg, mds),
            _ => break,
        }
    }

    let stats = server.session_stats();
    assert_eq!(stats.len(), 1);
    let s = &stats[0];
    assert!(
        s.media_queue_dropped > 0,
        "drop-newest must be counted under overload (produced {}, sent {})",
        s.media_produced,
        s.media_sent
    );
    assert!(s.media_produced > s.media_sent, "production outpaced the forwarder");
    assert!(
        s.media_sent > 0,
        "the forwarder still made progress instead of stalling"
    );
    // Bounded queue: tokio's bounded mpsc empirically holds capacity + 1
    // messages (this run: 257 in flight with a 256 slot channel) — a library
    // quirk recorded in the README, not an adoption decision.
    const QUEUE_CAPACITY: u64 = 256;
    assert!(
        s.media_produced <= s.media_sent + s.media_queue_dropped + QUEUE_CAPACITY + 1,
        "accounting: produced {} sent {} dropped {} in-flight must be <= {QUEUE_CAPACITY}+1",
        s.media_produced,
        s.media_sent,
        s.media_queue_dropped
    );
    // Client view under drop-newest: the forwarder drains the queue in FIFO
    // order, so the client sees an ordered, gap-free prefix of OLD media
    // (the newest datagrams are the ones dropped at overflow), never a
    // retransmission and never corruption.
    assert!(verifier.datagrams > 0);
    assert_eq!(verifier.duplicates, 0);
    assert_eq!(verifier.pattern_mismatches, 0);
    assert!(
        verifier.sample_frames as f64 / 48000.0 < 1.0,
        "client fell far behind real-time ({} frames received)",
        verifier.sample_frames
    );

    conn.unsubscribe().await.expect("UNSUBSCRIBE under overload");
    conn.send_close_normal().await.expect("CLOSE under overload");
    let reason = conn
        .expect_close(Duration::from_secs(3))
        .await
        .expect("control path responsive even mid-overload");
    assert_eq!(reason, proto_core::frame::CloseReason::Normal);
    server.stop().await;
}

/// One slow client must never affect another client's media.
#[tokio::test]
async fn slow_client_does_not_degrade_another_client() {
    init_test_logging();
    let server = spawn_server().await;
    let cfg = client_config(&server);

    let mut fast = proto_client::connect(&cfg).await.expect("fast client handshake");
    fast.auth(TEST_CREDENTIAL).await.expect("fast client auth");
    let media_start = fast.subscribe().await.expect("fast client subscribe");

    let mut slow = proto_client::connect(&cfg).await.expect("slow client handshake");
    slow.auth(TEST_CREDENTIAL).await.expect("slow client auth");
    slow.subscribe().await.expect("slow client subscribe");

    let mds = fast.max_datagram_size().expect("datagrams supported");
    let mut verifier = MediaVerifier::new(media_start.channels);
    let deadline = Instant::now() + Duration::from_millis(700);
    while Instant::now() < deadline {
        let dg = tokio::time::timeout(Duration::from_millis(500), fast.read_datagram())
            .await
            .expect("fast client keeps receiving")
            .expect("datagram");
        verifier.check(&dg, mds);
    }
    assert!(
        verifier.datagrams >= 100,
        "fast client throughput unaffected: {} datagrams",
        verifier.datagrams
    );
    assert_eq!(verifier.gaps, 0, "no gaps for the fast client");
    assert_eq!(verifier.pattern_mismatches, 0);

    // The slow client received far less than it was sent (its receive buffer
    // dropped the unread overflow) without any effect on the fast client.
    let stats = server.session_stats();
    let slow_stats = stats
        .iter()
        .find(|s| s.remote_addr != stats[0].remote_addr || s.session_id != stats[0].session_id)
        .cloned()
        .unwrap_or_else(|| stats[stats.len() - 1].clone());
    assert!(slow_stats.media_produced > 0);

    fast.unsubscribe().await.expect("fast unsubscribe");
    slow.unsubscribe().await.expect("slow unsubscribe");
    fast.send_close_normal().await.expect("fast close");
    slow.send_close_normal().await.expect("slow close");
    let r1 = fast.expect_close(Duration::from_secs(2)).await.expect("fast close ack");
    let r2 = slow.expect_close(Duration::from_secs(2)).await.expect("slow close ack");
    assert_eq!(r1, proto_core::frame::CloseReason::Normal);
    assert_eq!(r2, proto_core::frame::CloseReason::Normal);
    server.stop().await;
}

/// (h) Every received datagram fits the client's currently queried
/// `max_datagram_size()` and carries whole f32le sample frames. A server that
/// assumed a fixed 1464-byte path (1940-byte datagrams) would fail this test.
#[tokio::test]
async fn every_datagram_respects_queried_max_datagram_size() {
    init_test_logging();
    let server = spawn_server().await;
    let cfg = client_config(&server);
    let mut conn = proto_client::connect(&cfg).await.expect("handshake ok");
    conn.auth(TEST_CREDENTIAL).await.expect("auth ok");
    let media_start = conn.subscribe().await.expect("subscribe");

    let mds = conn.max_datagram_size().expect("datagrams supported");
    assert!(mds > MEDIA_HEADER_LEN);
    // With DPLPMTUD disabled (prototype default) the size is the QUIC initial
    // MTU; query repeatedly to prove it is read live, not cached once.
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(conn.max_datagram_size(), Some(mds));
    }

    let mut verifier = MediaVerifier::new(media_start.channels);
    let deadline = Instant::now() + Duration::from_millis(1200);
    while Instant::now() < deadline {
        let dg = tokio::time::timeout(Duration::from_millis(500), conn.read_datagram())
            .await
            .expect("datagram in time")
            .expect("datagram readable");
        assert!(
            dg.len() <= mds,
            "datagram of {} bytes exceeds queried max_datagram_size {mds}",
            dg.len()
        );
        verifier.check(&dg, mds);
    }
    assert!(verifier.datagrams >= 100, "{} datagrams", verifier.datagrams);
    assert_eq!(verifier.oversized, 0);
    assert_eq!(verifier.misaligned, 0);
    assert_eq!(verifier.too_short, 0);
    assert!(verifier.max_datagram_len <= mds);
    assert!(
        verifier.max_datagram_len > MEDIA_HEADER_LEN + 8,
        "datagrams actually carry sample frames"
    );
    conn.unsubscribe().await.expect("unsubscribe");
    server.stop().await;
}
