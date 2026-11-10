//! TCP/UDP Wire Protocol server.
//!
//! The server owns one capture session and one TCP listener/UDP socket. TCP
//! carries the frozen v0.3.4 control protocol (`GET_FORMAT`, `START_PLAY`,
//! `HEARTBEAT`); UDP carries raw PCM after a client binds its play id to its
//! source address.
//!
//! Startup order matters: capture starts first so the actual [`AudioFormat`] is
//! known and immutable before the sockets accept clients. `GET_FORMAT` can
//! therefore never observe a format race.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{lookup_host, TcpListener, TcpSocket, TcpStream, UdpSocket};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;

use crate::capture::{CaptureBackend, CaptureConfig, CaptureSession};
use crate::error::{Error, Result};
use crate::protocol::{
    decode_command, encode_get_format_response, encode_heartbeat, encode_start_play_response,
    segment_audio, AudioFormat, Command, MAX_PAYLOAD_BYTES,
};

/// Bound on how long [`ServerHandle::stop`] waits for its tasks to finish.
const STOP_JOIN_TIMEOUT: Duration = Duration::from_secs(5);

/// How many capture buffers may queue between capture and UDP broadcast.
const FRAME_CHANNEL_CAPACITY: usize = 64;

/// Process-wide `START_PLAY` id allocator, matching v0.3.4's function-static
/// counter. Ids are never reset by stopping a Server or by starting another
/// Server in the same process (the GUI Stop/Start path), so the released
/// Android Client always receives a fresh positive id.
static NEXT_PLAY_ID: AtomicI32 = AtomicI32::new(1);

/// Server listen configuration.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// Host name or IP address to bind.
    pub host: String,
    /// TCP and UDP port; `0` asks the OS for an ephemeral port.
    pub port: u16,
    /// How often a playing session is ticked: heartbeat send and timeout check.
    pub heartbeat_interval: Duration,
    /// A playing session with no client heartbeat for longer than this is closed.
    pub heartbeat_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host()
                .map(|address| address.to_string())
                .unwrap_or_default(),
            port: crate::config::DEFAULT_PORT,
            heartbeat_interval: crate::protocol::HEARTBEAT_INTERVAL,
            heartbeat_timeout: crate::protocol::SESSION_TIMEOUT,
        }
    }
}

/// The default bind host: the first address matching v0.3.4's private-network
/// masks, else the first non-loopback IPv4 address; `None` when the machine
/// has no usable IPv4 address.
///
/// v0.3.4 compared `(address & mask) == mask` with the masks below rather than
/// using RFC 1918 ranges. That also matches some addresses outside the private
/// ranges (for example `192.169`-`192.175` and `11.x`); 0.4.0 keeps the exact
/// comparison so the default Host does not change on those networks.
pub fn default_host() -> Option<std::net::Ipv4Addr> {
    let interfaces = if_addrs::get_if_addrs().ok()?;
    select_default_address(
        interfaces
            .into_iter()
            .filter_map(|interface| match interface.addr {
                if_addrs::IfAddr::V4(address) if !address.ip.is_loopback() => Some(address.ip),
                _ => None,
            }),
    )
}

/// The three masks v0.3.4 used to select its default address.
const LEGACY_PRIVATE_MASKS: [u32; 3] = [0x0a00_0000, 0xac10_0000, 0xc0a8_0000];

/// Whether `address` matches one of v0.3.4's private-network masks.
fn is_legacy_private(address: std::net::Ipv4Addr) -> bool {
    let address = u32::from(address);
    LEGACY_PRIVATE_MASKS
        .iter()
        .any(|mask| address & mask == *mask)
}

/// Pick the first address matching v0.3.4's private masks, falling back to the
/// first enumerated address.
pub fn select_default_address(
    addresses: impl IntoIterator<Item = std::net::Ipv4Addr>,
) -> Option<std::net::Ipv4Addr> {
    let addresses: Vec<std::net::Ipv4Addr> = addresses.into_iter().collect();
    addresses
        .iter()
        .find(|address| is_legacy_private(**address))
        .copied()
        .or_else(|| addresses.first().copied())
}

/// The Wire Protocol server.
pub struct Server;

impl Server {
    /// Start capture, then bind TCP and UDP, and begin serving.
    ///
    /// Capture is started and its format negotiated first. All sockets use the
    /// same port; with `port = 0` the TCP port chosen by the OS is reused for
    /// UDP. The returned handle owns the capture session: dropping it stops
    /// capture and cancels all tasks.
    pub async fn start(
        config: ServerConfig,
        capture: Arc<dyn CaptureBackend>,
        request: CaptureConfig,
    ) -> Result<ServerHandle> {
        if config.heartbeat_interval.is_zero() || config.heartbeat_timeout.is_zero() {
            return Err(Error::InvalidConfig(
                "heartbeat interval and timeout must be greater than zero".to_owned(),
            ));
        }

        let (frame_sink, frame_source) = mpsc::channel::<Vec<u8>>(FRAME_CHANNEL_CAPACITY);
        let session = capture.start(&request, frame_sink)?;
        let format = Arc::new(session.format.clone());
        if format.block_align() == 0 {
            return Err(Error::InvalidConfig(format!(
                "capture session reported an unusable AudioFormat {format:?}"
            )));
        }

        let bind_addr = resolve_bind_address(&config.host, config.port).await?;
        let listener = bind_tcp(bind_addr).await?;
        let local_addr = listener
            .local_addr()
            .map_err(|error| Error::io("read TCP local address", error))?;
        let udp_addr = SocketAddr::new(local_addr.ip(), local_addr.port());
        let udp = Arc::new(
            UdpSocket::bind(udp_addr)
                .await
                .map_err(|error| Error::io("bind UDP socket", error))?,
        );

        let (shutdown, _) = watch::channel(false);
        let context = Arc::new(ServerContext {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            format: Arc::clone(&format),
            tasks: Arc::new(Mutex::new(JoinSet::new())),
            shutdown,
            heartbeat_interval: config.heartbeat_interval,
            heartbeat_timeout: config.heartbeat_timeout,
        });
        let handle_tasks = Arc::clone(&context.tasks);
        let handle_shutdown = context.shutdown.clone();

        {
            let mut set = lock(&context.tasks);
            set.spawn(accept_loop(listener, Arc::clone(&context)));
            set.spawn(udp_bind_loop(Arc::clone(&udp), Arc::clone(&context)));
            set.spawn(broadcast_loop(udp, Arc::clone(&context), frame_source));
        }

        Ok(ServerHandle {
            local_addr,
            format,
            shutdown: handle_shutdown,
            capture: Some(session),
            tasks: handle_tasks,
        })
    }
}

/// A running server.
///
/// `local_addr`, `format`, and `stop` are the only operations the GUI and CLI
/// need. Dropping the handle also cancels the server. Play ids are
/// process-wide and increase monotonically for the life of the process; they
/// are never reset per Server instance or per session.
pub struct ServerHandle {
    local_addr: SocketAddr,
    format: Arc<AudioFormat>,
    shutdown: watch::Sender<bool>,
    capture: Option<CaptureSession>,
    tasks: Arc<Mutex<JoinSet<()>>>,
}

impl ServerHandle {
    /// The actual TCP/UDP listen address (with the resolved ephemeral port).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The immutable capture format served by this server.
    pub fn format(&self) -> AudioFormat {
        (*self.format).clone()
    }

    /// Stop capture, close every TCP session and the UDP socket, and join all
    /// tasks. Cancellation-safe: the join is bounded and remaining tasks are
    /// aborted on timeout.
    pub async fn stop(mut self) -> Result<()> {
        let capture_result = if let Some(capture) = self.capture.take() {
            tokio::task::spawn_blocking(move || capture.stop())
                .await
                .map_err(|error| Error::Capture(format!("capture stop task failed: {error}")))
                .and_then(|result| result)
        } else {
            Ok(())
        };
        // Always cancel the network side, even when capture shutdown failed.
        let _ = self.shutdown.send(true);
        let mut tasks = std::mem::take(&mut *lock(&self.tasks));
        let joined = async { while tasks.join_next().await.is_some() {} };
        if tokio::time::timeout(STOP_JOIN_TIMEOUT, joined)
            .await
            .is_err()
        {
            tasks.abort_all();
        }
        capture_result
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        // Dropping a JoinSet aborts its tasks, so a dropped handle never leaks
        // network tasks; the capture session's Drop joins its thread.
        let _ = std::mem::take(&mut *lock(&self.tasks));
    }
}

/// Per-playing-session state shared by the connection, heartbeat, and UDP
/// broadcast paths.
struct Peer {
    udp_peer: Mutex<Option<SocketAddr>>,
    last_activity: Mutex<tokio::time::Instant>,
    cancel: watch::Sender<bool>,
}

impl Peer {
    fn new() -> Self {
        let (cancel, _) = watch::channel(false);
        Self {
            udp_peer: Mutex::new(None),
            last_activity: Mutex::new(tokio::time::Instant::now()),
            cancel,
        }
    }

    fn udp_peer(&self) -> Option<SocketAddr> {
        *lock(&self.udp_peer)
    }

    fn bind_udp(&self, source: SocketAddr) {
        *lock(&self.udp_peer) = Some(source);
    }

    /// Refresh the activity clock on any client heartbeat.
    fn touch(&self) {
        *lock(&self.last_activity) = tokio::time::Instant::now();
    }

    fn idle(&self) -> Duration {
        lock(&self.last_activity).elapsed()
    }

    fn subscribe(&self) -> watch::Receiver<bool> {
        self.cancel.subscribe()
    }

    /// Ask the owning connection task to close.
    fn cancel(&self) {
        let _ = self.cancel.send(true);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Connection/session state shared by the accept, control, heartbeat, and UDP
/// tasks.
///
/// The shutdown sender is kept here so every task can subscribe to its own
/// receiver; a receiver cannot be shared mutably across tasks.
struct ServerContext {
    sessions: Arc<Mutex<HashMap<i32, Arc<Peer>>>>,
    format: Arc<AudioFormat>,
    tasks: Arc<Mutex<JoinSet<()>>>,
    shutdown: watch::Sender<bool>,
    heartbeat_interval: Duration,
    heartbeat_timeout: Duration,
}

impl ServerContext {
    fn shutdown(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }
}

async fn resolve_bind_address(host: &str, port: u16) -> Result<SocketAddr> {
    if host.is_empty() {
        return Err(Error::InvalidConfig(
            "bind host must not be empty".to_owned(),
        ));
    }
    let mut addresses = lookup_host((host, port))
        .await
        .map_err(|error| Error::Resolve(format!("{host}:{port}: {error}")))?;
    addresses
        .next()
        .ok_or_else(|| Error::Resolve(format!("{host}:{port} resolved to no addresses")))
}

async fn bind_tcp(addr: SocketAddr) -> Result<TcpListener> {
    let socket = if addr.is_ipv4() {
        TcpSocket::new_v4()
    } else {
        TcpSocket::new_v6()
    }
    .map_err(|error| Error::io("create TCP socket", error))?;
    socket
        .set_reuseaddr(true)
        .map_err(|error| Error::io("set TCP reuse_address", error))?;
    socket
        .bind(addr)
        .map_err(|error| Error::io("bind TCP listener", error))?;
    socket
        .listen(1024)
        .map_err(|error| Error::io("listen on TCP socket", error))
}

async fn accept_loop(listener: TcpListener, context: Arc<ServerContext>) {
    let mut shutdown = context.shutdown();
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _peer)) => {
                        // v0.3.4 sets TCP_NODELAY on accepted control sockets.
                        let _ = stream.set_nodelay(true);
                        let connection =
                            handle_connection(stream, Arc::clone(&context));
                        lock(&context.tasks).spawn(connection);
                    }
                    Err(_error) => {
                        if *shutdown.borrow() {
                            break;
                        }
                        // A transient accept failure must not spin or kill the
                        // server; back off briefly and keep listening.
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }
        }
    }
}

async fn handle_connection(stream: TcpStream, context: Arc<ServerContext>) {
    let mut shutdown = context.shutdown();
    let (mut reader, write_half) = stream.into_split();
    let writer = Arc::new(tokio::sync::Mutex::new(write_half));
    let mut play_id: Option<i32> = None;
    let mut peer: Option<Arc<Peer>> = None;
    let mut cancel: Option<watch::Receiver<bool>> = None;
    let mut heartbeat: Option<tokio::task::JoinHandle<()>> = None;
    let mut command = [0u8; 4];

    loop {
        let read = if let Some(cancel_rx) = cancel.as_mut() {
            tokio::select! {
                _ = shutdown.changed() => break,
                _ = cancel_rx.changed() => break,
                read = reader.read_exact(&mut command) => read,
            }
        } else {
            tokio::select! {
                _ = shutdown.changed() => break,
                read = reader.read_exact(&mut command) => read,
            }
        };
        match read {
            Ok(_) => {}
            Err(_) => break,
        }

        match decode_command(&command) {
            Ok(Command::GetFormat) => {
                if write_frame(&writer, &encode_get_format_response(&context.format))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(Command::StartPlay) => {
                if peer.is_some() {
                    // v0.3.4 closes on a duplicate START_PLAY without replying.
                    break;
                }
                let id = NEXT_PLAY_ID.fetch_add(1, Ordering::SeqCst);
                let new_peer = Arc::new(Peer::new());
                lock(&context.sessions).insert(id, Arc::clone(&new_peer));
                cancel = Some(new_peer.subscribe());
                play_id = Some(id);
                if write_frame(&writer, &encode_start_play_response(id))
                    .await
                    .is_err()
                {
                    break;
                }
                peer = Some(Arc::clone(&new_peer));
                // Only a session that completed START_PLAY gets a heartbeat
                // loop and a timeout, exactly like v0.3.4.
                heartbeat = Some(tokio::spawn(heartbeat_loop(
                    id,
                    new_peer,
                    Arc::clone(&writer),
                    Arc::clone(&context),
                )));
            }
            Ok(Command::Heartbeat) => {
                // Heartbeats before START_PLAY are ignored.
                if let Some(peer) = &peer {
                    peer.touch();
                }
            }
            Err(_) => break,
        }
    }

    if let Some(id) = play_id {
        lock(&context.sessions).remove(&id);
    }
    if let Some(heartbeat) = heartbeat.take() {
        heartbeat.abort();
        let _ = heartbeat.await;
    }
    let _ = writer.lock().await.shutdown().await;
}

async fn heartbeat_loop(
    id: i32,
    peer: Arc<Peer>,
    writer: Arc<tokio::sync::Mutex<OwnedWriteHalf>>,
    context: Arc<ServerContext>,
) {
    let mut shutdown = context.shutdown();
    let mut ticker = tokio::time::interval(context.heartbeat_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = ticker.tick() => {}
        }
        if !lock(&context.sessions).contains_key(&id) {
            // The connection already ended; nothing left to do.
            break;
        }
        // The timeout is only checked on a tick, so a silent client is closed
        // on the first tick strictly after the timeout (~6s with 3s/5s).
        if peer.idle() > context.heartbeat_timeout {
            lock(&context.sessions).remove(&id);
            peer.cancel();
            break;
        }
        if write_frame(&writer, &encode_heartbeat()).await.is_err() {
            lock(&context.sessions).remove(&id);
            peer.cancel();
            break;
        }
    }
}

async fn write_frame(
    writer: &tokio::sync::Mutex<OwnedWriteHalf>,
    frame: &[u8],
) -> std::io::Result<()> {
    writer.lock().await.write_all(frame).await
}

async fn udp_bind_loop(udp: Arc<UdpSocket>, context: Arc<ServerContext>) {
    let mut shutdown = context.shutdown();
    let mut buffer = [0u8; 4];
    loop {
        let received = tokio::select! {
            _ = shutdown.changed() => break,
            received = udp.recv_from(&mut buffer) => received,
        };
        match received {
            Ok((length, source)) => {
                if length == 0 {
                    continue;
                }
                // v0.3.4 receives into a zero-initialized 4-byte int: a short
                // datagram binds the id with zero high bytes, and a long one is
                // truncated to its first four bytes.
                let mut id_bytes = [0u8; 4];
                let copied = length.min(4);
                id_bytes[..copied].copy_from_slice(&buffer[..copied]);
                let id = i32::from_le_bytes(id_bytes);
                if let Some(peer) = lock(&context.sessions).get(&id) {
                    peer.bind_udp(source);
                }
            }
            Err(_) => break,
        }
    }
}

async fn broadcast_loop(
    udp: Arc<UdpSocket>,
    context: Arc<ServerContext>,
    mut frames: mpsc::Receiver<Vec<u8>>,
) {
    let block_align = context.format.block_align();
    let mut shutdown = context.shutdown();
    let mut reported_oversized_format = false;
    let mut reported_send_stall = false;
    loop {
        let frame = tokio::select! {
            _ = shutdown.changed() => break,
            frame = frames.recv() => match frame {
                Some(frame) => frame,
                None => break,
            },
        };
        if block_align == 0 || block_align > MAX_PAYLOAD_BYTES {
            // No MTU-sized datagram can carry one sample frame. v0.3.4 could
            // hang or crash here; 0.4.0 sends no PCM (never an oversized
            // datagram) and reports the condition once per session.
            if block_align > MAX_PAYLOAD_BYTES && !reported_oversized_format {
                reported_oversized_format = true;
                eprintln!(
                    "audio-share: block_align {block_align} exceeds the {MAX_PAYLOAD_BYTES}-byte \
                     UDP payload limit; no PCM will be sent for this capture format"
                );
            }
            continue;
        }
        let peers: Vec<SocketAddr> = lock(&context.sessions)
            .values()
            .filter_map(|peer| peer.udp_peer())
            .collect();
        if peers.is_empty() {
            continue;
        }

        for segment in segment_audio(&frame, block_align) {
            for peer in &peers {
                // Never await a UDP send here. A Wi-Fi peer that stops draining
                // (screen off, power save, roaming) can fill the socket send
                // buffer; `send_to(...).await` would then block the broadcast
                // loop forever, the bounded frame channel would fill, and the
                // capture thread would block on `blocking_send` — every client
                // goes silent while TCP heartbeats keep the session alive.
                // Real-time audio should drop for the slow peer instead.
                match udp.try_send_to(segment, *peer) {
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if !reported_send_stall {
                            reported_send_stall = true;
                            eprintln!(
                                "audio-share: UDP send buffer full; dropping datagrams instead \
                                 of stalling capture (peer {peer})"
                            );
                        }
                    }
                    // UDP send failures are expected (unreachable peer) and
                    // must not stop the stream to other clients.
                    Err(_) => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn server_config_defaults_match_v034_timing_and_port() {
        let config = ServerConfig::default();
        assert_eq!(config.port, crate::config::DEFAULT_PORT);
        assert_eq!(config.heartbeat_interval, Duration::from_secs(3));
        assert_eq!(config.heartbeat_timeout, Duration::from_secs(5));
    }

    #[test]
    fn select_default_address_prefers_private_ipv4_in_enumeration_order() {
        let public = Ipv4Addr::new(8, 8, 8, 8);
        let private_a = Ipv4Addr::new(10, 1, 2, 3);
        let private_b = Ipv4Addr::new(192, 168, 1, 2);
        let other_private = Ipv4Addr::new(172, 20, 0, 1);

        assert_eq!(
            select_default_address([public, private_a, private_b]),
            Some(private_a)
        );
        assert_eq!(
            select_default_address([other_private, private_b]),
            Some(other_private)
        );
        assert_eq!(select_default_address([public]), Some(public));
        assert_eq!(select_default_address([]), None);
    }

    #[test]
    fn private_address_detection_replicates_the_v034_mask_exactly() {
        // v0.3.4 tested `(addr & mask) == mask` with the masks below. The
        // comparison is intentionally looser than RFC 1918 (for example it
        // also matches 192.169-192.175 and 11.x), and parity requires keeping
        // that behavior so the default Host does not change on such networks.
        for address in [
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(10, 255, 255, 254),
            Ipv4Addr::new(11, 1, 2, 3),
            Ipv4Addr::new(172, 16, 0, 1),
            Ipv4Addr::new(172, 31, 255, 254),
            Ipv4Addr::new(173, 20, 1, 2),
            Ipv4Addr::new(192, 168, 1, 2),
            Ipv4Addr::new(192, 169, 1, 2),
            Ipv4Addr::new(192, 175, 255, 254),
        ] {
            assert!(
                is_legacy_private(address),
                "{address} matches the v0.3.4 mask"
            );
        }
        for address in [
            Ipv4Addr::new(8, 8, 8, 8),
            Ipv4Addr::new(9, 255, 255, 255),
            Ipv4Addr::new(172, 15, 255, 255),
            Ipv4Addr::new(172, 32, 0, 1),
            Ipv4Addr::new(173, 1, 2, 3),
            Ipv4Addr::new(192, 167, 255, 255),
            Ipv4Addr::new(192, 176, 0, 1),
        ] {
            assert!(
                !is_legacy_private(address),
                "{address} does not match the v0.3.4 mask"
            );
        }
        // The mask comparison alone also matches 127.x; `default_host` filters
        // loopback before it reaches the selector, exactly like v0.3.4.
        assert!(is_legacy_private(Ipv4Addr::new(127, 0, 0, 1)));

        // The loose mask participates in default selection like any match.
        assert_eq!(
            select_default_address([Ipv4Addr::new(8, 8, 8, 8), Ipv4Addr::new(192, 169, 1, 2)]),
            Some(Ipv4Addr::new(192, 169, 1, 2))
        );
    }
}
