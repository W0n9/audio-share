//! M5-06 §11.2 impairment matrix — Server/host half.
//!
//! Rows 6 (congestion), 7 (path MTU), 9 (idle) and 10 (connection kill) at the
//! layers the host can actually reach: the pure packetizer/pump/queue with an
//! **injected clock**, and the pure §5.2 state machine. No real network, no
//! sleeps, no DPLPMTUD. Every result is labelled in the evidence matrix with
//! the layer actually exercised; real-network/device parts are `NOT RUN`.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use v2_transport::auth_store::AuthorizationStore;
use v2_transport::codes::ErrorCode;
use v2_transport::media::{MediaHeader, HEADER_SIZE};
use v2_transport::media_pump::{MediaPump, MediaPumpError};
use v2_transport::media_source::{MediaChunk, StreamFormat, StreamState};
use v2_transport::send_queue::{MediaSendQueue, MAX_QUEUED_DATAGRAMS};
use v2_transport::state_machine::{
    CaptureSnapshot, Event, ServerConfig, ServerState, StateMachine,
};
use v2_transport::{IDLE_TIMEOUT, KEEP_ALIVE_INTERVAL};

const SAMPLE_RATE: u32 = 48_000;
const FRAME_BYTES: usize = 8; // F32LE stereo

fn stream() -> StreamState {
    StreamState {
        stream_instance_id: 1,
        config_generation: 1,
        next_frame: 0,
        format: StreamFormat {
            sample_rate: SAMPLE_RATE,
            channels: 2,
            bytes_per_sample: 4,
        },
    }
}

fn chunk(start_frame: u64, frames: u64) -> MediaChunk {
    MediaChunk {
        pcm: vec![0x5a; frames as usize * FRAME_BYTES],
        start_frame,
        end_frame: start_frame + frames,
        stream_instance_id: 1,
        config_generation: 1,
    }
}

fn machine() -> StateMachine {
    let store = Arc::new(AuthorizationStore::in_memory());
    StateMachine::new(
        ServerConfig {
            server_public_key: vec![1u8; 32],
            protocol_minor: 0,
            capture: CaptureSnapshot::absent(),
            max_auth_failures_per_connection: 5,
            server_random_override: Some(vec![0u8; 32]),
            auth_exporter: Some(vec![0u8; 32]),
            session_id_base: 1,
            pairing: None,
            pairing_exporter: None,
            pairing_server_random_override: None,
        },
        &store,
    )
}

// ---------------------------------------------------------------------------
// Row 6 — 300 kbit/s, queue 50
// ---------------------------------------------------------------------------

/// Matrix row 6, host-simulated at the app layer: capture produces 48 kHz
/// F32 stereo (~3 Mbit/s) into the production `MediaPump` + `MediaSendQueue`,
/// while a simulated 300 kbit/s link with a 50-datagram queue drains it on an
/// injected clock.
///
/// Gate: bounded backlog (the production 256-datagram queue, drop-newest),
/// capture never blocked (`accept` always returns without I/O), and full-rate
/// recovery once the link is no longer the bottleneck.
#[test]
fn row6_congestion_bounded_backlog_and_full_rate_recovery() {
    const LINK_QUEUE_DATAGRAMS: usize = 50;
    const LINK_BYTES_PER_SEC: u64 = 37_500; // 300 kbit/s
    const TICK_MS: u64 = 10;
    let tick = Duration::from_millis(TICK_MS);

    let mut pump = MediaPump::new(stream());
    let mut queue = MediaSendQueue::new();
    let mut link: VecDeque<u64> = VecDeque::new(); // datagram lengths

    let start = Instant::now();
    let mut biggest_pending = 0u64;
    let mut start_frame = 0u64;

    // One second of capture at full rate against the slow link.
    for index in 0..100u64 {
        let now = start + tick * index as u32;
        // Capture never blocks or fails, however congested the Client is.
        pump.accept(chunk(start_frame, 480))
            .expect("capture accepts");
        start_frame += 480;
        biggest_pending = biggest_pending.max(pump.pending_frames());

        let datagrams = pump
            .take_sendable(now, 1_192, 64)
            .expect("packetizer keeps producing");
        queue.enqueue(datagrams);

        // Move what fits into the link's bounded queue (never blocks).
        while link.len() < LINK_QUEUE_DATAGRAMS {
            let Some(front) = queue.front() else {
                break;
            };
            let bytes = front.len() as u64;
            let _ = bytes;
            let datagram = queue.pop_front().expect("front exists");
            link.push_back(datagram.len() as u64);
        }

        // Transmit at the link rate: 375 bytes per 10 ms.
        let mut budget = LINK_BYTES_PER_SEC * TICK_MS / 1_000;
        while let Some(&bytes) = link.front() {
            if bytes > budget {
                break;
            }
            budget -= bytes;
            link.pop_front();
        }

        assert!(
            pump.pending_frames() <= SAMPLE_RATE as u64 * 500 / 1_000,
            "the pump's 500 ms pending bound is never exceeded"
        );
        assert!(
            queue.len() <= MAX_QUEUED_DATAGRAMS,
            "the per-Client send queue is bounded"
        );
    }

    assert!(
        queue.dropped_newest() > 0,
        "congestion must overflow the bounded send queue (drop-newest), not memory"
    );
    assert_eq!(
        queue.len(),
        MAX_QUEUED_DATAGRAMS,
        "the backlog sits at the bound while congested"
    );
    assert_eq!(
        link.len(),
        LINK_QUEUE_DATAGRAMS,
        "the simulated link queue is full"
    );
    assert!(biggest_pending <= SAMPLE_RATE as u64 * 500 / 1_000);

    let drops_at_congestion = queue.dropped_newest();

    // Recovery: the path is healthy again. Drain the backlog at full rate.
    let recovered_at = start + Duration::from_secs(2);
    let mut drain_tick = 0u32;
    while !queue.is_empty() || !link.is_empty() {
        // Free every queued datagram and transmit the link at a rate that is
        // effectively instantaneous relative to the tick.
        while link.len() < LINK_QUEUE_DATAGRAMS {
            match queue.pop_front() {
                Some(datagram) => link.push_back(datagram.len() as u64),
                None => break,
            }
        }
        link.clear(); // the recovered link drains immediately
        drain_tick += 1;
        assert!(drain_tick < 1_000, "the bounded backlog must drain");
    }
    let now = recovered_at + tick * drain_tick;
    assert!(queue.is_empty(), "backlog drained");
    assert!(pump.idle(), "pump has no buffered audio");

    // Full-rate production after recovery: no further queue drops.
    for index in 0..20u64 {
        let at = now + tick * index as u32;
        pump.accept(chunk(start_frame, 480))
            .expect("capture accepts");
        start_frame += 480;
        let datagrams = pump.take_sendable(at, 1_192, 64).expect("send");
        queue.enqueue(datagrams);
        while queue.pop_front().is_some() {
            // a recovered link accepts every datagram
        }
    }
    assert_eq!(
        queue.dropped_newest(),
        drops_at_congestion,
        "a recovered link produces no new drops"
    );
}

// ---------------------------------------------------------------------------
// Row 7 — path MTU 1280
// ---------------------------------------------------------------------------

fn drain_at_limit(pump: &mut MediaPump, limit: usize, start: Instant) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for index in 0..2_000u64 {
        let now = start + Duration::from_millis(10 * index);
        let batch = pump
            .take_sendable(now, limit, 4_096)
            .expect("send at the path limit");
        out.extend(batch);
    }
    assert!(pump.idle(), "the whole accepted buffer is sent");
    out
}

/// Matrix row 7, host half: at a 1280-byte path (and a realistic 1200-byte
/// QUIC-DATAGRAM limit after overhead), every produced datagram fits the new
/// limit, stays frame-aligned, and its header round-trips. The chunker never
/// relies on IP fragmentation: one payload chunk becomes one DATAGRAM.
///
/// DPLPMTUD re-probing over a real path is `NOT RUN`.
#[test]
fn row7_mtu_1280_chunker_fits_the_limit_without_fragmentation() {
    let frames_total = 24_000u64; // 500 ms at 48 kHz
    for limit in [1_280usize, 1_200] {
        let mut pump = MediaPump::new(stream());
        pump.accept(chunk(0, frames_total)).expect("accept");
        let datagrams = drain_at_limit(&mut pump, limit, Instant::now());

        let mut payload_bytes = 0usize;
        let mut expected_sequence = 0u32;
        let mut expected_timestamp = 0u64;
        for datagram in &datagrams {
            assert!(
                datagram.len() <= limit,
                "datagram {} exceeds the {limit}-byte path limit",
                datagram.len()
            );
            let (header, payload) = MediaHeader::parse(datagram).expect("valid 24-byte header");
            assert_eq!(
                header.sequence, expected_sequence,
                "sequence +1 per datagram"
            );
            assert_eq!(
                header.timestamp, expected_timestamp,
                "capture clock is honest"
            );
            assert!(
                !payload.is_empty() && payload.len() % FRAME_BYTES == 0,
                "whole sample frames only (no fragment-sized tails)"
            );
            assert!(
                payload.len() <= limit - HEADER_SIZE,
                "payload fits the path after the header"
            );
            expected_sequence = expected_sequence.wrapping_add(1);
            expected_timestamp += (payload.len() / FRAME_BYTES) as u64;
            payload_bytes += payload.len();
        }
        assert_eq!(
            payload_bytes,
            frames_total as usize * FRAME_BYTES,
            "every accepted PCM byte leaves exactly once (no re-chunking loss)"
        );
        assert_eq!(expected_timestamp, frames_total);
    }

    // A limit that cannot carry the header plus one sample frame is an
    // explicit 602-class failure, not fragmentation.
    let mut pump = MediaPump::new(stream());
    pump.accept(chunk(0, 960)).expect("accept");
    assert_eq!(
        pump.take_sendable(Instant::now(), HEADER_SIZE, 8),
        Err(MediaPumpError::DatagramUnsupported)
    );
}

// ---------------------------------------------------------------------------
// Row 9 — idle 25 s survive; > 30 s silence → 601
// ---------------------------------------------------------------------------

/// Matrix row 9, injected-clock/state-machine half: QUIC keep-alive is 10 s
/// with a 30 s idle timeout, so 25 s of silence is covered by two keep-alives;
/// only the idle-timeout event maps to the §6 `601` transport close.
///
/// A real 25 s network survival is `NOT RUN`.
#[test]
fn row9_idle_timeout_is_601_and_keepalive_covers_25_seconds() {
    assert_eq!(IDLE_TIMEOUT, Duration::from_secs(30));
    assert_eq!(KEEP_ALIVE_INTERVAL, Duration::from_secs(10));
    assert!(
        KEEP_ALIVE_INTERVAL * 2 <= Duration::from_secs(25)
            && KEEP_ALIVE_INTERVAL * 3 > Duration::from_secs(25),
        "keep-alives at 10 s and 20 s cover a 25 s quiet period"
    );
    assert!(
        KEEP_ALIVE_INTERVAL < IDLE_TIMEOUT,
        "the peer must refresh the idle timer before it expires"
    );

    // Time passing is not an event: only the driver's idle-timeout event has
    // an effect, and it is the deterministic 601 classification.
    let mut machine = machine();
    assert_eq!(machine.state(), ServerState::Connecting);
    let outcomes = machine.handle(Event::IdleTimeout);
    assert_eq!(outcomes.len(), 1, "exactly one transport close");
    assert!(outcomes[0].frame.is_none(), "no control frame for 601");
    assert_eq!(
        outcomes[0].transport_close_code,
        Some(ErrorCode::NETWORK_FAILURE),
        "idle timeout → §6 601"
    );
    assert_eq!(machine.state(), ServerState::Closed);
    assert!(
        machine.handle(Event::IdleTimeout).is_empty(),
        "CLOSED absorbs later events"
    );
}

// ---------------------------------------------------------------------------
// Row 10 — connection kill: teardown (+ reconnect NOT RUN)
// ---------------------------------------------------------------------------

/// Matrix row 10, server half: a transport loss tears the session down
/// deterministically (no control frame, CLOSED absorbs everything). The
/// Client-side backoff/reconnect promise (§6.3) is owned by Kotlin's
/// `ReconnectPolicy` outside this ticket's test paths, and a live QUIC kill is
/// not a stable in-process seam — both are `NOT RUN`.
#[test]
fn row10_connection_kill_tears_the_session_down() {
    let mut first = machine();
    let outcomes = first.handle(Event::TransportLost);
    assert!(outcomes.is_empty(), "transport loss emits no control frame");
    assert_eq!(first.state(), ServerState::Closed, "teardown is terminal");
    assert!(first.handle(Event::TransportLost).is_empty());

    // A Close received first moves through CLOSING, then transport loss
    // completes the teardown.
    let mut second = machine();
    let outcomes = second.handle(Event::CloseReceived);
    assert!(outcomes.is_empty(), "Close is one-way, never echoed");
    assert_eq!(second.state(), ServerState::Closing);
    assert!(second.handle(Event::TransportLost).is_empty());
    assert_eq!(second.state(), ServerState::Closed);
}
