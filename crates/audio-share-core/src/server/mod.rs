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
            port: 65_530,
            heartbeat_interval: crate::protocol::HEARTBEAT_INTERVAL,
            heartbeat_timeout: crate::protocol::SESSION_TIMEOUT,
        }
    }
}

/// The default bind host: the first private IPv4 address, else the first
/// non-loopback IPv4 address; `None` when the machine has no usable IPv4
/// address.
///
/// This mirrors v0.3.4's intent (first 10/8, 172.16/12 or 192.168/16 address,
/// otherwise the first enumerated address). The old mask comparison also
/// matched ranges outside RFC 1918 (for example `192.169.x.x`); the rewrite
/// follows the documented private ranges, per the parity recommendation.
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

/// Pick the first private IPv4 address, falling back to the first address.
pub fn select_default_address(
    addresses: impl IntoIterator<Item = std::net::Ipv4Addr>,
) -> Option<std::net::Ipv4Addr> {
    let addresses: Vec<std::net::Ipv4Addr> = addresses.into_iter().collect();
    addresses
        .iter()
        .find(|address| address.is_private())
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

        let sessions: Arc<Mutex<HashMap<i32, Arc<Peer>>>> = Arc::new(Mutex::new(HashMap::new()));
        let next_id = Arc::new(AtomicI32::new(1));
        let (shutdown, _) = watch::channel(false);
        let tasks: Arc<Mutex<JoinSet<()>>> = Arc::new(Mutex::new(JoinSet::new()));

        {
            let mut set = lock(&tasks);
            set.spawn(accept_loop(
                listener,
                Arc::clone(&sessions),
                Arc::clone(&next_id),
                Arc::clone(&format),
                shutdown.subscribe(),
                Arc::clone(&tasks),
                config.heartbeat_interval,
                config.heartbeat_timeout,
            ));
            set.spawn(udp_bind_loop(
                Arc::clone(&udp),
                Arc::clone(&sessions),
                shutdown.subscribe(),
            ));
            set.spawn(broadcast_loop(
                udp,
                Arc::clone(&sessions),
                Arc::clone(&format),
                frame_source,
                shutdown.subscribe(),
            ));
        }

        Ok(ServerHandle {
            local_addr,
            format,
            shutdown,
            capture: Some(session),
            tasks,
        })
    }
}

/// A running server.
///
/// `local_addr`, `format`, and `stop` are the only operations the GUI and CLI
/// need. Dropping the handle also cancels the server. Play ids start at 1 for
/// each server instance and increase monotonically while it runs.
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

#[allow(clippy::too_many_arguments)]
async fn accept_loop(
    listener: TcpListener,
    sessions: Arc<Mutex<HashMap<i32, Arc<Peer>>>>,
    next_id: Arc<AtomicI32>,
    format: Arc<AudioFormat>,
    mut shutdown: watch::Receiver<bool>,
    tasks: Arc<Mutex<JoinSet<()>>>,
    heartbeat_interval: Duration,
    heartbeat_timeout: Duration,
) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _peer)) => {
                        // v0.3.4 sets TCP_NODELAY on accepted control sockets.
                        let _ = stream.set_nodelay(true);
                        let connection = handle_connection(
                            stream,
                            Arc::clone(&sessions),
                            Arc::clone(&next_id),
                            Arc::clone(&format),
                            shutdown.clone(),
                            heartbeat_interval,
                            heartbeat_timeout,
                        );
                        lock(&tasks).spawn(connection);
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

async fn handle_connection(
    stream: TcpStream,
    sessions: Arc<Mutex<HashMap<i32, Arc<Peer>>>>,
    next_id: Arc<AtomicI32>,
    format: Arc<AudioFormat>,
    mut shutdown: watch::Receiver<bool>,
    heartbeat_interval: Duration,
    heartbeat_timeout: Duration,
) {
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
                if write_frame(&writer, &encode_get_format_response(&format))
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
                let id = next_id.fetch_add(1, Ordering::SeqCst);
                let new_peer = Arc::new(Peer::new());
                lock(&sessions).insert(id, Arc::clone(&new_peer));
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
                    heartbeat_interval,
                    heartbeat_timeout,
                    shutdown.clone(),
                    Arc::clone(&sessions),
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
        lock(&sessions).remove(&id);
    }
    if let Some(heartbeat) = heartbeat.take() {
        heartbeat.abort();
        let _ = heartbeat.await;
    }
    let _ = writer.lock().await.shutdown().await;
}

#[allow(clippy::too_many_arguments)]
async fn heartbeat_loop(
    id: i32,
    peer: Arc<Peer>,
    writer: Arc<tokio::sync::Mutex<OwnedWriteHalf>>,
    heartbeat_interval: Duration,
    heartbeat_timeout: Duration,
    mut shutdown: watch::Receiver<bool>,
    sessions: Arc<Mutex<HashMap<i32, Arc<Peer>>>>,
) {
    let mut ticker = tokio::time::interval(heartbeat_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = ticker.tick() => {}
        }
        if !lock(&sessions).contains_key(&id) {
            // The connection already ended; nothing left to do.
            break;
        }
        // The timeout is only checked on a tick, so a silent client is closed
        // on the first tick strictly after the timeout (~6s with 3s/5s).
        if peer.idle() > heartbeat_timeout {
            lock(&sessions).remove(&id);
            peer.cancel();
            break;
        }
        if write_frame(&writer, &encode_heartbeat()).await.is_err() {
            lock(&sessions).remove(&id);
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

async fn udp_bind_loop(
    udp: Arc<UdpSocket>,
    sessions: Arc<Mutex<HashMap<i32, Arc<Peer>>>>,
    mut shutdown: watch::Receiver<bool>,
) {
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
                if let Some(peer) = lock(&sessions).get(&id) {
                    peer.bind_udp(source);
                }
            }
            Err(_) => break,
        }
    }
}

async fn broadcast_loop(
    udp: Arc<UdpSocket>,
    sessions: Arc<Mutex<HashMap<i32, Arc<Peer>>>>,
    format: Arc<AudioFormat>,
    mut frames: mpsc::Receiver<Vec<u8>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let block_align = format.block_align();
    loop {
        let frame = tokio::select! {
            _ = shutdown.changed() => break,
            frame = frames.recv() => match frame {
                Some(frame) => frame,
                None => break,
            },
        };
        if block_align == 0 {
            continue;
        }
        let peers: Vec<SocketAddr> = lock(&sessions)
            .values()
            .filter_map(|peer| peer.udp_peer())
            .collect();
        if peers.is_empty() {
            continue;
        }

        if block_align > MAX_PAYLOAD_BYTES {
            // No valid MTU segment fits one sample frame. Never split a sample
            // and never hang: send each capture buffer whole.
            for peer in &peers {
                let _ = udp.send_to(&frame, peer).await;
            }
            continue;
        }
        for segment in segment_audio(&frame, block_align) {
            for peer in &peers {
                // UDP send failures are expected (unreachable peer) and must
                // not stop the stream to other clients.
                let _ = udp.send_to(segment, peer).await;
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
        assert_eq!(config.port, 65_530);
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
}
