//! Ticket 01 e2e test (a): positive end-to-end happy path.
//!
//! Handshake with pinned SPKI, AUTH, SUBSCRIBE, media datagram receipt with
//! sequence/timestamp/pattern continuity, UNSUBSCRIBE, normal close — and the
//! server-side stats agreeing with everything the client observed.

mod common;

use common::*;
use proto_client::MediaVerifier;
use proto_core::frame::CloseReason;
use proto_core::media::MEDIA_HEADER_LEN;
use std::time::{Duration, Instant};

#[tokio::test]
async fn happy_path_auth_subscribe_media_unsubscribe_close() {
    init_test_logging();
    let server = spawn_server().await;
    let cfg = client_config(&server);

    let mut conn = proto_client::connect(&cfg).await.expect("handshake with correct pin");
    let session_id = conn.auth(TEST_CREDENTIAL).await.expect("auth accepted");
    assert!(session_id >= 1, "session id monotonic from 1, got {session_id}");

    let media_start = conn.subscribe().await.expect("subscribe");
    assert_eq!(media_start.sample_rate, 48000);
    assert_eq!(media_start.channels, 2);
    assert_eq!(media_start.sample_format, proto_core::media::SAMPLE_FORMAT_F32LE);
    assert_eq!(media_start.config_generation, 1);
    assert_eq!(media_start.stream_instance_id, 1, "first subscription is instance 1");

    // Client-side invariant (h): every received datagram fits the queried limit.
    let mds = conn.max_datagram_size().expect("datagrams supported by peer");
    assert!(mds > MEDIA_HEADER_LEN, "max_datagram_size {mds} too small to carry media");

    let mut verifier = MediaVerifier::new(media_start.channels);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && verifier.datagrams < 60 {
        let dg = tokio::time::timeout(Duration::from_secs(2), conn.read_datagram())
            .await
            .expect("datagram arrives in time")
            .expect("datagram readable");
        verifier.check(&dg, mds);
    }
    assert!(verifier.datagrams >= 60, "only {} datagrams received", verifier.datagrams);
    assert_eq!(verifier.gaps, 0, "no sequence gaps on an unimpaired loopback");
    assert_eq!(verifier.duplicates, 0);
    assert_eq!(verifier.pattern_mismatches, 0, "deterministic PCM pattern verified");
    assert_eq!(verifier.timestamp_mismatches, 0);
    assert_eq!(verifier.oversized, 0, "every datagram <= queried max_datagram_size");
    assert_eq!(verifier.misaligned, 0, "payloads are complete f32le sample frames");
    assert_eq!(verifier.instance_changes, 0);
    assert!(
        verifier.max_datagram_len > MEDIA_HEADER_LEN,
        "datagrams carry PCM payload"
    );

    conn.unsubscribe().await.expect("unsubscribe");
    conn.send_close_normal().await.expect("send CLOSE(Normal)");
    let reason = conn
        .expect_close(Duration::from_secs(3))
        .await
        .expect("server acknowledges with a CLOSE frame");
    assert_eq!(reason, CloseReason::Normal);

    // Server-side agreement.
    let stats = server.session_stats();
    assert_eq!(stats.len(), 1, "exactly one session");
    let s = &stats[0];
    assert_eq!(s.frames_auth_in, 1);
    assert_eq!(s.frames_subscribe_in, 1);
    assert_eq!(s.frames_unsubscribe_in, 1);
    assert_eq!(s.frames_close_in, 1);
    assert_eq!(s.auth_ok_out, 1);
    assert_eq!(s.auth_err_out, 0);
    assert_eq!(s.media_start_out, 1);
    assert_eq!(s.protocol_errors_out, 0);
    assert_eq!(s.close_out, 1, "server replies with its own CLOSE(Normal)");
    assert!(s.media_sent >= 60, "server sent {} media datagrams", s.media_sent);
    assert_eq!(s.media_queue_dropped, 0, "no drops on a fast loopback client");
    assert_eq!(s.media_sendbuf_dropped, 0);
    assert_eq!(s.media_produced, s.media_sent, "no hidden accounting gap");
    assert!(
        s.media_bytes_sent > verifier.bytes.min(1) || verifier.bytes > 0,
        "byte counters populated"
    );

    server.stop().await;
    let after = server.session_stats();
    assert_eq!(after.len(), 1, "session record survives server stop");
}
