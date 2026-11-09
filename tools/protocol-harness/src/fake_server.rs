//! A v0.3.4-compatible fake Server for hardware-free tests.
//!
//! The fake server speaks the same Wire Protocol as the released Server:
//! `GET_FORMAT`, `START_PLAY`, 3-second heartbeats with a 5-second timeout,
//! UDP id binding, and MTU-segmented PCM. Its PCM is synthetic, so no audio
//! device is required.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use audio_share_core::protocol::{
    decode_command, encode_get_format_response, encode_heartbeat, encode_start_play_response,
    segment_audio, AudioFormat, Command, Encoding,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::time::{interval, Instant, MissedTickBehavior};

use crate::fixture::load_fixture;
use crate::synth::synth_chunk;

/// `serve-fake` command arguments.
#[derive(Debug, clap::Args)]
pub struct ServeFakeArgs {
    /// TCP and UDP bind address. Port 0 asks the OS for a free port.
    #[arg(long, default_value = "127.0.0.1:65530")]
    pub bind: String,
    /// Fixture set whose AudioFormat the fake server should advertise.
    #[arg(long)]
    pub fixture: Option<std::path::PathBuf>,
    /// Encoding when no fixture is supplied.
    #[arg(long, value_parser = parse_encoding, default_value = "f32")]
    pub encoding: Encoding,
    /// Channel count when no fixture is supplied.
    #[arg(long, default_value_t = 2)]
    pub channels: i32,
    /// Sample rate when no fixture is supplied.
    #[arg(long, default_value_t = 48_000)]
    pub sample_rate: i32,
    /// Stop after this many seconds; 0 runs until killed.
    #[arg(long, default_value_t = 0)]
    pub duration_secs: u64,
    /// Heartbeat interval in milliseconds (v0.3.4 uses 3000).
    #[arg(long, default_value_t = 3_000)]
    pub heartbeat_interval_ms: u64,
    /// Session timeout in milliseconds (v0.3.4 uses 5000).
    #[arg(long, default_value_t = 5_000)]
    pub timeout_ms: u64,
    /// PCM chunk interval in milliseconds.
    #[arg(long, default_value_t = 10)]
    pub chunk_interval_ms: u64,
}

fn parse_encoding(value: &str) -> Result<Encoding, String> {
    match value.to_ascii_lowercase().as_str() {
        "f32" | "float" => Ok(Encoding::PcmFloat),
        "s8" | "u8" => Ok(Encoding::Pcm8Bit),
        "s16" => Ok(Encoding::Pcm16Bit),
        "s24" => Ok(Encoding::Pcm24Bit),
        "s32" => Ok(Encoding::Pcm32Bit),
        other => Err(format!(
            "unknown encoding {other:?}; expected f32, s8, s16, s24, or s32"
        )),
    }
}

#[derive(Debug)]
struct Session {
    last_tick: Instant,
    udp_peer: Option<SocketAddr>,
}

type Sessions = Arc<Mutex<HashMap<i32, Session>>>;

/// Run the fake server until the duration elapses or the process is killed.
pub async fn run(args: &ServeFakeArgs) -> Result<()> {
    let format = match &args.fixture {
        Some(dir) => {
            let fixture = load_fixture(dir)
                .with_context(|| format!("load fake-server fixture {}", dir.display()))?;
            fixture.decoded_format()?
        }
        None => AudioFormat::new(args.encoding, args.channels, args.sample_rate),
    };
    let block_align = format.block_align();
    if block_align == 0 {
        bail!("fake server AudioFormat has an unusable block_align");
    }

    let requested: SocketAddr = args
        .bind
        .parse()
        .with_context(|| format!("parse bind address {:?}", args.bind))?;
    let listener = TcpListener::bind(requested)
        .await
        .with_context(|| format!("bind TCP {}", args.bind))?;
    let local = listener.local_addr()?;
    let udp = Arc::new(
        UdpSocket::bind(local)
            .await
            .with_context(|| format!("bind UDP {local}"))?,
    );
    println!(
        "serve-fake: listening on {local} encoding={:?} channels={} sample_rate={} block_align={}",
        format.encoding_kind(),
        format.channels,
        format.sample_rate,
        block_align
    );

    let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
    let next_id = Arc::new(AtomicI32::new(0));

    spawn_udp_binder(Arc::clone(&udp), Arc::clone(&sessions));
    spawn_pcm_sender(
        Arc::clone(&udp),
        Arc::clone(&sessions),
        format.clone(),
        block_align,
        args.chunk_interval_ms.max(1),
    );

    let heartbeat_interval = Duration::from_millis(args.heartbeat_interval_ms.max(1));
    let timeout = Duration::from_millis(args.timeout_ms.max(1));

    let accept = async {
        loop {
            match listener.accept().await {
                Ok((socket, _peer)) => {
                    let sessions = Arc::clone(&sessions);
                    let next_id = Arc::clone(&next_id);
                    let format = format.clone();
                    tokio::spawn(async move {
                        if let Err(error) = handle_session(
                            socket,
                            sessions,
                            next_id,
                            format,
                            heartbeat_interval,
                            timeout,
                        )
                        .await
                        {
                            eprintln!("serve-fake: session ended: {error:#}");
                        }
                    });
                }
                Err(error) => {
                    eprintln!("serve-fake: accept failed: {error}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    };

    if args.duration_secs == 0 {
        accept.await;
    } else {
        tokio::select! {
            _ = accept => {}
            _ = tokio::time::sleep(Duration::from_secs(args.duration_secs)) => {
                println!("serve-fake: duration elapsed");
            }
        }
    }
    Ok(())
}

fn spawn_udp_binder(udp: Arc<UdpSocket>, sessions: Sessions) {
    tokio::spawn(async move {
        let mut buffer = [0u8; 4];
        loop {
            match udp.recv_from(&mut buffer).await {
                Ok((4, peer)) => {
                    let id = i32::from_le_bytes(buffer);
                    if let Some(session) = sessions.lock().unwrap().get_mut(&id) {
                        session.udp_peer = Some(peer);
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
}

fn spawn_pcm_sender(
    udp: Arc<UdpSocket>,
    sessions: Sessions,
    format: AudioFormat,
    block_align: usize,
    chunk_interval_ms: u64,
) {
    tokio::spawn(async move {
        let encoding = match format.encoding_kind() {
            Some(encoding) => encoding,
            None => return,
        };
        let channels = format.channels.max(1) as usize;
        let frames_per_chunk =
            ((format.sample_rate.max(1) as u64 * chunk_interval_ms) / 1_000).max(1) as usize;
        let mut ticker = interval(Duration::from_millis(chunk_interval_ms));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut start_frame = 0u64;
        loop {
            ticker.tick().await;
            let chunk = synth_chunk(encoding, channels, frames_per_chunk, start_frame);
            start_frame += frames_per_chunk as u64;
            let peers: Vec<SocketAddr> = sessions
                .lock()
                .unwrap()
                .values()
                .filter_map(|session| session.udp_peer)
                .collect();
            for peer in peers {
                for segment in segment_audio(&chunk, block_align) {
                    if udp.send_to(segment, peer).await.is_err() {
                        break;
                    }
                }
            }
        }
    });
}

async fn handle_session(
    socket: TcpStream,
    sessions: Sessions,
    next_id: Arc<AtomicI32>,
    format: AudioFormat,
    heartbeat_interval: Duration,
    timeout: Duration,
) -> Result<()> {
    let (mut reader, writer) = socket.into_split();
    let writer = Arc::new(tokio::sync::Mutex::new(writer));
    let mut play_id: Option<i32> = None;
    let mut command = [0u8; 4];

    loop {
        match reader.read_exact(&mut command).await {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error).context("read TCP command"),
        }

        match decode_command(&command)? {
            Command::GetFormat => {
                writer
                    .lock()
                    .await
                    .write_all(&encode_get_format_response(&format))
                    .await?;
            }
            Command::StartPlay => {
                if play_id.is_some() {
                    // v0.3.4 rejects a second START_PLAY on the same connection.
                    break;
                }
                let id = next_id.fetch_add(1, Ordering::SeqCst) + 1;
                sessions.lock().unwrap().insert(
                    id,
                    Session {
                        last_tick: Instant::now(),
                        udp_peer: None,
                    },
                );
                play_id = Some(id);
                writer
                    .lock()
                    .await
                    .write_all(&encode_start_play_response(id))
                    .await?;
                spawn_heartbeat(
                    id,
                    Arc::clone(&sessions),
                    Arc::clone(&writer),
                    heartbeat_interval,
                    timeout,
                );
            }
            Command::Heartbeat => {
                if let Some(id) = play_id {
                    if let Some(session) = sessions.lock().unwrap().get_mut(&id) {
                        session.last_tick = Instant::now();
                    }
                }
            }
        }
    }

    if let Some(id) = play_id {
        sessions.lock().unwrap().remove(&id);
    }
    let _ = writer.lock().await.shutdown().await;
    Ok(())
}

fn spawn_heartbeat(
    id: i32,
    sessions: Sessions,
    writer: Arc<tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    heartbeat_interval: Duration,
    timeout: Duration,
) {
    tokio::spawn(async move {
        let mut ticker = interval(heartbeat_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        ticker.tick().await; // consume the immediate first tick
        loop {
            ticker.tick().await;
            let expired = {
                let mut map = sessions.lock().unwrap();
                match map.get_mut(&id) {
                    Some(session) if session.last_tick.elapsed() > timeout => {
                        map.remove(&id);
                        true
                    }
                    Some(_) => false,
                    None => true,
                }
            };
            if expired {
                let _ = writer.lock().await.shutdown().await;
                break;
            }
            if writer
                .lock()
                .await
                .write_all(&encode_heartbeat())
                .await
                .is_err()
            {
                sessions.lock().unwrap().remove(&id);
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_parser_matches_protocol_names() {
        assert_eq!(parse_encoding("f32").unwrap(), Encoding::PcmFloat);
        assert_eq!(parse_encoding("s8").unwrap(), Encoding::Pcm8Bit);
        assert_eq!(parse_encoding("s16").unwrap(), Encoding::Pcm16Bit);
        assert_eq!(parse_encoding("s24").unwrap(), Encoding::Pcm24Bit);
        assert_eq!(parse_encoding("s32").unwrap(), Encoding::Pcm32Bit);
        assert!(parse_encoding("nope").is_err());
    }
}
