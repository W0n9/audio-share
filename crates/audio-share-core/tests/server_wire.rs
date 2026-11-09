//! Wire Protocol behavior at the public `audio_share_core::server` seam.
//!
//! Every test drives a real TCP client and a real UDP socket against a Server
//! listening on a real localhost port, with [`SyntheticCapture`] supplying PCM.
//! Only externally observable protocol behavior is asserted.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use audio_share_core::capture::{CaptureConfig, SyntheticCapture};
use audio_share_core::protocol::{
    decode_get_format_response, decode_heartbeat, decode_start_play_response, encode_command,
    encode_heartbeat, encode_udp_bind, max_segment_payload, AudioFormat, Command, Encoding,
    MAX_PAYLOAD_BYTES,
};
use audio_share_core::server::{Server, ServerConfig, ServerHandle};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::timeout;

const FRAME_INTERVAL: Duration = Duration::from_millis(5);

fn f32_stereo_48k() -> AudioFormat {
    AudioFormat::new(Encoding::PcmFloat, 2, 48_000)
}

struct TestServer {
    handle: ServerHandle,
    addr: SocketAddr,
    format: AudioFormat,
}

impl TestServer {
    async fn start(format: AudioFormat, frame_bytes: usize) -> Self {
        Self::start_with_timing(
            format,
            frame_bytes,
            Duration::from_secs(3),
            Duration::from_secs(5),
        )
        .await
    }

    async fn start_with_timing(
        format: AudioFormat,
        frame_bytes: usize,
        heartbeat_interval: Duration,
        heartbeat_timeout: Duration,
    ) -> Self {
        Self::start_on_port(
            format,
            frame_bytes,
            0,
            heartbeat_interval,
            heartbeat_timeout,
        )
        .await
    }

    async fn start_on_port(
        format: AudioFormat,
        frame_bytes: usize,
        port: u16,
        heartbeat_interval: Duration,
        heartbeat_timeout: Duration,
    ) -> Self {
        Self::try_start_on_port(
            format,
            frame_bytes,
            port,
            heartbeat_interval,
            heartbeat_timeout,
        )
        .await
        .expect("server start")
    }

    /// Like [`Self::start_on_port`], but lets the caller retry another port when
    /// the requested one is unavailable.
    async fn try_start_on_port(
        format: AudioFormat,
        frame_bytes: usize,
        port: u16,
        heartbeat_interval: Duration,
        heartbeat_timeout: Duration,
    ) -> Option<Self> {
        let capture = SyntheticCapture::new(format.clone(), frame_bytes, FRAME_INTERVAL).unwrap();
        let handle = Server::start(
            ServerConfig {
                host: "127.0.0.1".to_owned(),
                port,
                heartbeat_interval,
                heartbeat_timeout,
            },
            Arc::new(capture),
            CaptureConfig::default(),
        )
        .await
        .ok()?;
        assert_eq!(handle.format(), format);
        let addr = handle.local_addr();
        Some(Self {
            handle,
            addr,
            format,
        })
    }

    async fn stop(self) {
        timeout(Duration::from_secs(5), self.handle.stop())
            .await
            .expect("server stop timed out")
            .expect("server stop failed");
    }
}

/// Start a test server on a port outside the OS ephemeral ranges (Linux
/// 32768-60999, Windows 49152-65535).
///
/// The listener-release test must not share the ephemeral pool: with parallel
/// tests, a server started by another test can be handed the just-freed port
/// and answer the "must be refused" reconnect, which made this assertion flaky
/// on Windows CI.
async fn start_on_non_ephemeral_port(format: AudioFormat, frame_bytes: usize) -> TestServer {
    const CANDIDATES: [u16; 4] = [31337, 31338, 31339, 31340];
    for port in CANDIDATES {
        if let Some(server) = TestServer::try_start_on_port(
            format.clone(),
            frame_bytes,
            port,
            Duration::from_secs(3),
            Duration::from_secs(5),
        )
        .await
        {
            return server;
        }
    }
    panic!("no non-ephemeral test port available: {CANDIDATES:?}");
}

async fn connect(addr: SocketAddr) -> TcpStream {
    timeout(Duration::from_secs(5), TcpStream::connect(addr))
        .await
        .expect("connect timed out")
        .expect("connect failed")
}

async fn send_command(stream: &mut TcpStream, command: Command) {
    stream
        .write_all(&encode_command(command))
        .await
        .expect("send command");
}

async fn read_get_format_response(stream: &mut TcpStream) -> (AudioFormat, Vec<u8>) {
    let mut header = [0u8; 8];
    timeout(Duration::from_secs(5), stream.read_exact(&mut header))
        .await
        .expect("GET_FORMAT header timed out")
        .expect("GET_FORMAT header read failed");
    let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    assert!(size <= 1024, "implausible AudioFormat size {size}");
    let mut payload = vec![0u8; size];
    timeout(Duration::from_secs(5), stream.read_exact(&mut payload))
        .await
        .expect("GET_FORMAT payload timed out")
        .expect("GET_FORMAT payload read failed");
    let mut response = header.to_vec();
    response.extend_from_slice(&payload);
    let (format, consumed) = decode_get_format_response(&response).expect("decode GET_FORMAT");
    assert_eq!(consumed, response.len());
    (format, response)
}

async fn read_start_play_response(stream: &mut TcpStream) -> (i32, Vec<u8>) {
    let mut response = [0u8; 8];
    timeout(Duration::from_secs(5), stream.read_exact(&mut response))
        .await
        .expect("START_PLAY response timed out")
        .expect("START_PLAY response read failed");
    let (id, consumed) = decode_start_play_response(&response).expect("decode START_PLAY");
    assert_eq!(consumed, response.len());
    (id, response.to_vec())
}

async fn bind_udp(server: SocketAddr, id: i32) -> UdpSocket {
    let udp = UdpSocket::bind("127.0.0.1:0").await.expect("bind UDP");
    udp.send_to(&encode_udp_bind(id), server)
        .await
        .expect("send UDP bind");
    udp
}

async fn recv_datagram(udp: &UdpSocket, wait: Duration) -> Vec<u8> {
    try_recv_datagram(udp, wait)
        .await
        .expect("UDP datagram timed out")
}

async fn try_recv_datagram(udp: &UdpSocket, wait: Duration) -> Option<Vec<u8>> {
    let mut buffer = vec![0u8; 65_535];
    match timeout(wait, udp.recv_from(&mut buffer)).await {
        Ok(Ok((length, _))) => {
            buffer.truncate(length);
            Some(buffer)
        }
        Ok(Err(_)) | Err(_) => None,
    }
}

/// Read whatever the server sends until it closes the connection.
async fn read_until_closed(stream: &mut TcpStream, wait: Duration) -> Vec<u8> {
    let deadline = tokio::time::Instant::now() + wait;
    let mut collected = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let mut buffer = [0u8; 4];
        match timeout(remaining, stream.read(&mut buffer)).await {
            Err(_) => panic!("server did not close the connection within {wait:?}"),
            Ok(Ok(0)) | Ok(Err(_)) => return collected,
            Ok(Ok(read)) => collected.extend_from_slice(&buffer[..read]),
        }
    }
}

#[tokio::test]
async fn get_format_returns_the_exact_v034_response_bytes() {
    let server = TestServer::start(f32_stereo_48k(), 1024).await;
    let mut stream = connect(server.addr).await;

    send_command(&mut stream, Command::GetFormat).await;
    let (format, response) = read_get_format_response(&mut stream).await;

    assert_eq!(format, f32_stereo_48k());
    assert_eq!(
        response,
        vec![
            0x01, 0x00, 0x00, 0x00, // GET_FORMAT
            0x08, 0x00, 0x00, 0x00, // size = 8
            0x08, 0x01, 0x10, 0x02, 0x18, 0x80, 0xf7, 0x02, // AudioFormat
        ]
    );

    // GET_FORMAT may be repeated on one connection.
    send_command(&mut stream, Command::GetFormat).await;
    let (format, response_again) = read_get_format_response(&mut stream).await;
    assert_eq!(format, f32_stereo_48k());
    assert_eq!(response_again, response);

    server.stop().await;
}

#[tokio::test]
async fn start_play_returns_an_id_and_udp_bind_delivers_aligned_pcm() {
    let server = TestServer::start(f32_stereo_48k(), 8192).await;
    let mut stream = connect(server.addr).await;

    send_command(&mut stream, Command::StartPlay).await;
    let (id, response) = read_start_play_response(&mut stream).await;
    assert!(id > 0, "ids must be positive, got {id}");
    let mut expected = vec![0x02, 0x00, 0x00, 0x00];
    expected.extend_from_slice(&id.to_le_bytes());
    assert_eq!(response, expected);

    let udp = bind_udp(server.addr, id).await;
    let datagram = recv_datagram(&udp, Duration::from_secs(3)).await;
    assert!(!datagram.is_empty());
    assert!(datagram.len() <= MAX_PAYLOAD_BYTES);
    assert_eq!(
        datagram.len() % server.format.block_align(),
        0,
        "datagram must stay block aligned"
    );

    server.stop().await;
}

#[tokio::test]
async fn play_ids_increment_across_connections_on_one_server() {
    let server = TestServer::start(f32_stereo_48k(), 1024).await;

    let mut first = connect(server.addr).await;
    send_command(&mut first, Command::StartPlay).await;
    let (first_id, _) = read_start_play_response(&mut first).await;

    let mut second = connect(server.addr).await;
    send_command(&mut second, Command::StartPlay).await;
    let (second_id, _) = read_start_play_response(&mut second).await;

    assert!(first_id > 0, "ids must be positive, got {first_id}");
    assert!(
        second_id > first_id,
        "ids must increase on one server: {first_id} then {second_id}"
    );

    server.stop().await;
}

#[tokio::test]
async fn play_ids_are_process_wide_and_do_not_reset_with_a_new_server() {
    // v0.3.4 used a function-static counter. A process that stops one Server
    // and starts another (the GUI path) must keep allocating increasing ids.
    let first_server = TestServer::start(f32_stereo_48k(), 1024).await;
    let mut first_stream = connect(first_server.addr).await;
    send_command(&mut first_stream, Command::StartPlay).await;
    let (first_id, _) = read_start_play_response(&mut first_stream).await;
    assert!(first_id > 0, "ids must be positive, got {first_id}");
    first_server.stop().await;

    let second_server = TestServer::start(f32_stereo_48k(), 1024).await;
    let mut second_stream = connect(second_server.addr).await;
    send_command(&mut second_stream, Command::StartPlay).await;
    let (second_id, _) = read_start_play_response(&mut second_stream).await;

    assert!(
        second_id > first_id,
        "a new Server must continue the process-wide id sequence: \
         {first_id} then {second_id}"
    );
    second_server.stop().await;
}

#[tokio::test]
async fn heartbeats_are_sent_on_the_configured_interval_after_start_play() {
    let server = TestServer::start_with_timing(
        f32_stereo_48k(),
        1024,
        Duration::from_millis(100),
        Duration::from_secs(2),
    )
    .await;
    let mut stream = connect(server.addr).await;
    send_command(&mut stream, Command::StartPlay).await;
    read_start_play_response(&mut stream).await;

    let started = tokio::time::Instant::now();
    for _ in 0..3 {
        let mut heartbeat = [0u8; 4];
        timeout(Duration::from_secs(2), stream.read_exact(&mut heartbeat))
            .await
            .expect("server heartbeat timed out")
            .expect("server heartbeat read failed");
        assert_eq!(decode_heartbeat(&heartbeat).unwrap(), 4);
        // Answering keeps the session active, like the Android Client.
        stream
            .write_all(&encode_heartbeat())
            .await
            .expect("send client heartbeat");
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(150),
        "three heartbeats at 100ms arrived too fast: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "heartbeats were too slow: {elapsed:?}"
    );

    server.stop().await;
}

#[tokio::test]
async fn a_silent_playing_client_is_closed_by_the_timeout_checked_on_a_tick() {
    let timeout_after = Duration::from_millis(120);
    let server = TestServer::start_with_timing(
        f32_stereo_48k(),
        1024,
        Duration::from_millis(50),
        timeout_after,
    )
    .await;
    let mut stream = connect(server.addr).await;
    send_command(&mut stream, Command::StartPlay).await;
    read_start_play_response(&mut stream).await;

    let started = tokio::time::Instant::now();
    let received = read_until_closed(&mut stream, Duration::from_secs(3)).await;
    let elapsed = started.elapsed();

    assert!(
        elapsed >= timeout_after,
        "session closed before the timeout elapsed: {elapsed:?}"
    );
    assert!(
        elapsed <= Duration::from_millis(1_500),
        "session closed far later than the next tick: {elapsed:?}"
    );
    assert!(
        !received.is_empty() && received.len() % 4 == 0,
        "expected at least one server heartbeat before close, got {} bytes",
        received.len()
    );
    for chunk in received.as_chunks::<4>().0 {
        assert_eq!(decode_heartbeat(chunk).unwrap(), 4);
    }

    server.stop().await;
}

#[tokio::test]
async fn pre_start_connections_are_not_timed_out_and_heartbeats_are_ignored() {
    let server = TestServer::start_with_timing(
        f32_stereo_48k(),
        1024,
        Duration::from_millis(30),
        Duration::from_millis(60),
    )
    .await;
    let mut stream = connect(server.addr).await;

    // A heartbeat before START_PLAY is ignored with no reply.
    send_command(&mut stream, Command::Heartbeat).await;
    let mut unsolicited = [0u8; 4];
    assert!(
        timeout(Duration::from_millis(250), stream.read(&mut unsolicited))
            .await
            .is_err(),
        "an idle pre-START_PLAY connection must not receive heartbeats"
    );

    // Well past the timeout, the connection must still be usable.
    tokio::time::sleep(Duration::from_millis(250)).await;
    send_command(&mut stream, Command::GetFormat).await;
    let (format, _) = read_get_format_response(&mut stream).await;
    assert_eq!(format, f32_stereo_48k());

    server.stop().await;
}

#[tokio::test]
async fn duplicate_start_play_closes_without_a_second_response() {
    let server = TestServer::start(f32_stereo_48k(), 1024).await;
    let mut stream = connect(server.addr).await;
    send_command(&mut stream, Command::StartPlay).await;
    let (id, _) = read_start_play_response(&mut stream).await;
    assert!(id > 0, "ids must be positive, got {id}");

    send_command(&mut stream, Command::StartPlay).await;
    let received = read_until_closed(&mut stream, Duration::from_secs(2)).await;
    assert!(
        received.is_empty(),
        "duplicate START_PLAY must not produce a response, got {received:?}"
    );

    server.stop().await;
}

#[tokio::test]
async fn unknown_commands_close_without_a_response() {
    for raw_command in [0u32, 4, u32::MAX] {
        let server = TestServer::start(f32_stereo_48k(), 1024).await;
        let mut stream = connect(server.addr).await;
        stream
            .write_all(&raw_command.to_le_bytes())
            .await
            .expect("send unknown command");
        let received = read_until_closed(&mut stream, Duration::from_secs(2)).await;
        assert!(
            received.is_empty(),
            "command {raw_command} must not produce a response, got {received:?}"
        );
        server.stop().await;
    }
}

#[tokio::test]
async fn mtu_segments_align_to_block_align_and_split_large_capture_frames() {
    let cases = [
        // (format, frame_bytes, expected aligned payload)
        (
            AudioFormat::new(Encoding::PcmFloat, 1, 48_000), // block_align 4
            8192,
            1464,
        ),
        (
            AudioFormat::new(Encoding::Pcm24Bit, 2, 48_000), // block_align 6
            8196,
            1464,
        ),
        (
            AudioFormat::new(Encoding::PcmFloat, 8, 48_000), // block_align 32
            8192,
            1440,
        ),
    ];

    for (format, frame_bytes, expected_payload) in cases {
        let block_align = format.block_align();
        assert_eq!(max_segment_payload(block_align), expected_payload);
        let server = TestServer::start(format.clone(), frame_bytes).await;
        let mut stream = connect(server.addr).await;
        send_command(&mut stream, Command::StartPlay).await;
        let (id, _) = read_start_play_response(&mut stream).await;
        let udp = bind_udp(server.addr, id).await;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut lengths = Vec::new();
        while lengths.len() < 8 || !lengths.contains(&expected_payload) {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out before a full-size datagram for block_align {block_align}: {lengths:?}"
            );
            let datagram = recv_datagram(&udp, remaining).await;
            assert!(!datagram.is_empty());
            assert!(
                datagram.len() <= expected_payload,
                "block_align {block_align}: datagram {} exceeds {expected_payload}",
                datagram.len()
            );
            assert_eq!(
                datagram.len() % block_align,
                0,
                "block_align {block_align}: datagram {} is not sample aligned",
                datagram.len()
            );
            lengths.push(datagram.len());
        }

        server.stop().await;
    }
}

#[tokio::test]
async fn oversized_block_align_emits_no_datagrams_and_keeps_the_server_alive() {
    // 400 channels of 4-byte float PCM: block_align 1600 > the 1464-byte MTU
    // payload. No valid segment fits one sample frame, so the server must send
    // nothing rather than an oversized datagram, and must keep serving.
    let format = AudioFormat::new(Encoding::PcmFloat, 400, 48_000);
    let block_align = format.block_align();
    assert!(
        block_align > MAX_PAYLOAD_BYTES,
        "test format is not oversized"
    );
    let server = TestServer::start(format.clone(), block_align).await;

    let mut stream = connect(server.addr).await;
    send_command(&mut stream, Command::StartPlay).await;
    let (id, _) = read_start_play_response(&mut stream).await;
    let udp = bind_udp(server.addr, id).await;
    assert!(
        try_recv_datagram(&udp, Duration::from_millis(400))
            .await
            .is_none(),
        "an oversized block_align must not produce UDP datagrams"
    );

    // The server is still alive and serving control traffic.
    let mut control = connect(server.addr).await;
    send_command(&mut control, Command::GetFormat).await;
    let (reported, _) = read_get_format_response(&mut control).await;
    assert_eq!(reported, format);

    server.stop().await;
}

#[tokio::test]
async fn multiple_clients_receive_the_same_datagram_stream() {
    let server = TestServer::start(f32_stereo_48k(), 64).await; // one datagram per frame
    let mut first = connect(server.addr).await;
    send_command(&mut first, Command::StartPlay).await;
    let (first_id, _) = read_start_play_response(&mut first).await;
    let first_udp = bind_udp(server.addr, first_id).await;

    let mut second = connect(server.addr).await;
    send_command(&mut second, Command::StartPlay).await;
    let (second_id, _) = read_start_play_response(&mut second).await;
    let second_udp = bind_udp(server.addr, second_id).await;
    assert_ne!(first_id, second_id);

    // Let both peers be fully bound before collecting, then compare overlapping
    // windows: scheduling may start one collector a frame earlier than the other.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let collect = |udp: UdpSocket| async move {
        let mut datagrams = Vec::new();
        for _ in 0..12 {
            datagrams.push(recv_datagram(&udp, Duration::from_secs(3)).await);
        }
        datagrams
    };
    let (first_datagrams, second_datagrams) = tokio::join!(collect(first_udp), collect(second_udp));

    let common = first_datagrams
        .windows(5)
        .any(|window| second_datagrams.windows(5).any(|other| window == other));
    assert!(
        common,
        "clients did not receive a common datagram sequence;\nfirst: {first_datagrams:?}\nsecond: {second_datagrams:?}"
    );

    server.stop().await;
}

#[tokio::test]
async fn udp_bind_last_wins_and_unknown_ids_are_ignored() {
    let server = TestServer::start(f32_stereo_48k(), 1024).await;
    let mut stream = connect(server.addr).await;
    send_command(&mut stream, Command::StartPlay).await;
    let (id, _) = read_start_play_response(&mut stream).await;

    // A UDP socket that never bound gets no PCM.
    let unbound = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    assert!(
        try_recv_datagram(&unbound, Duration::from_millis(100))
            .await
            .is_none(),
        "PCM must not be sent before the client binds its id"
    );

    // Unknown ids are ignored.
    unbound
        .send_to(&encode_udp_bind(id + 999), server.addr)
        .await
        .unwrap();
    assert!(
        try_recv_datagram(&unbound, Duration::from_millis(100))
            .await
            .is_none(),
        "an unknown UDP id must not receive PCM"
    );

    let first_udp = bind_udp(server.addr, id).await;
    let _ = recv_datagram(&first_udp, Duration::from_secs(3)).await;

    let second_udp = bind_udp(server.addr, id).await;
    // Drain datagrams already queued for the first address.
    while try_recv_datagram(&first_udp, Duration::from_millis(30))
        .await
        .is_some()
    {}
    // The rebind must redirect audio to the second address.
    let redirected = recv_datagram(&second_udp, Duration::from_secs(3)).await;
    assert!(!redirected.is_empty());
    assert!(
        try_recv_datagram(&first_udp, Duration::from_millis(100))
            .await
            .is_none(),
        "the first bound address must stop receiving after a rebind"
    );

    server.stop().await;
}

#[tokio::test]
async fn stop_closes_playing_and_idle_sessions_and_releases_the_listener() {
    // A non-ephemeral port keeps a parallel test from being assigned this port
    // right after the server releases it.
    let server = start_on_non_ephemeral_port(f32_stereo_48k(), 1024).await;
    let addr = server.addr;

    let mut idle = connect(addr).await;
    send_command(&mut idle, Command::GetFormat).await;
    read_get_format_response(&mut idle).await;

    let mut playing = connect(addr).await;
    send_command(&mut playing, Command::StartPlay).await;
    read_start_play_response(&mut playing).await;

    timeout(Duration::from_secs(5), server.handle.stop())
        .await
        .expect("stop itself hung")
        .expect("stop failed");

    // Check the listener immediately after stop, before any longer waits give
    // another test a chance to bind this address. A connect may be refused
    // (Linux) or time out (Windows can drop the SYN while the just-closed
    // sessions still occupy the port); either proves the server is no longer
    // accepting. Only an established connection is a failure.
    let reconnect = timeout(Duration::from_secs(2), TcpStream::connect(addr)).await;
    let accepted = matches!(reconnect, Ok(Ok(_)));
    assert!(
        !accepted,
        "a stopped server must not accept new connections (addr={addr}, result={reconnect:?})"
    );

    // Both TCP sessions observe EOF/close, whether or not they completed
    // START_PLAY.
    let _ = read_until_closed(&mut idle, Duration::from_secs(3)).await;
    let _ = read_until_closed(&mut playing, Duration::from_secs(3)).await;
}

#[tokio::test]
async fn server_can_be_started_and_stopped_repeatedly_on_the_same_port() {
    let first = TestServer::start(f32_stereo_48k(), 1024).await;
    let port = first.addr.port();
    first.stop().await;

    for _ in 0..3 {
        let server = TestServer::start_on_port(
            f32_stereo_48k(),
            1024,
            port,
            Duration::from_secs(3),
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(server.addr.port(), port);
        let mut stream = connect(server.addr).await;
        send_command(&mut stream, Command::GetFormat).await;
        let (format, _) = read_get_format_response(&mut stream).await;
        assert_eq!(format, f32_stereo_48k());
        server.stop().await;
    }
}

#[tokio::test]
async fn port_zero_reports_the_os_chosen_port_for_both_transports() {
    let server = TestServer::start(f32_stereo_48k(), 1024).await;
    assert_ne!(server.addr.port(), 0);

    let mut stream = connect(server.addr).await;
    send_command(&mut stream, Command::StartPlay).await;
    let (id, _) = read_start_play_response(&mut stream).await;
    // The UDP socket must be reachable on the same reported port.
    let udp = bind_udp(server.addr, id).await;
    let datagram = recv_datagram(&udp, Duration::from_secs(3)).await;
    assert!(!datagram.is_empty());

    server.stop().await;
}

#[tokio::test]
async fn a_client_can_reconnect_after_its_session_times_out() {
    let server = TestServer::start_with_timing(
        f32_stereo_48k(),
        1024,
        Duration::from_millis(30),
        Duration::from_millis(80),
    )
    .await;

    let mut first = connect(server.addr).await;
    send_command(&mut first, Command::StartPlay).await;
    let (first_id, _) = read_start_play_response(&mut first).await;
    let _ = read_until_closed(&mut first, Duration::from_secs(3)).await;

    // The server must still accept a fresh connection and issue a new id.
    let mut second = connect(server.addr).await;
    send_command(&mut second, Command::GetFormat).await;
    let (format, _) = read_get_format_response(&mut second).await;
    assert_eq!(format, f32_stereo_48k());
    send_command(&mut second, Command::StartPlay).await;
    let (second_id, _) = read_start_play_response(&mut second).await;
    assert!(second_id > first_id);
    let udp = bind_udp(server.addr, second_id).await;
    assert!(!recv_datagram(&udp, Duration::from_secs(3)).await.is_empty());

    server.stop().await;
}
