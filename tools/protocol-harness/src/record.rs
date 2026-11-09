//! `record`: drive an external v0.3.4 server and capture a fixture set.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use audio_share_core::protocol::{
    decode_get_format_response, decode_heartbeat, decode_start_play_response, encode_command,
    encode_heartbeat, encode_udp_bind, max_segment_payload, AudioFormat, Command,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::Instant;

use crate::fixture::{write_fixture_set, ControlBytes, FixturePlan};
use crate::process;

/// `record` command arguments.
#[derive(Debug, clap::Args)]
pub struct RecordArgs {
    /// External server executable to launch (for example the old `as-cmd`).
    #[arg(long)]
    pub server_cmd: PathBuf,
    /// Server argument; `{host}`, `{port}`, and `{bind}` are substituted.
    #[arg(long, allow_hyphen_values = true)]
    pub server_arg: Vec<String>,
    /// Directory that receives the fixture set.
    #[arg(long)]
    pub out: PathBuf,
    /// Host used to connect and for `{host}`/`{bind}`.
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    /// Port used for `{port}`/`{bind}`; 0 picks a free port.
    #[arg(long, default_value_t = 0)]
    pub port: u16,
    /// Number of server heartbeats to capture.
    #[arg(long, default_value_t = 2)]
    pub heartbeats: usize,
    /// Number of UDP datagrams to capture.
    #[arg(long, default_value_t = 32)]
    pub datagrams: usize,
    /// Overall record timeout in seconds.
    #[arg(long, default_value_t = 20)]
    pub timeout_secs: u64,
    /// Provenance string stored in the manifest.
    #[arg(long, default_value = "recorded-v0.3.4")]
    pub provenance: String,
    /// Platform string; defaults to the current OS.
    #[arg(long)]
    pub platform: Option<String>,
    /// Human-readable server label stored in the manifest (never a path).
    #[arg(long, default_value = "external-server")]
    pub server_label: String,
}

/// Record one fixture set from a live external server.
pub async fn run(args: &RecordArgs) -> Result<()> {
    let port = if args.port == 0 {
        process::pick_free_port(&args.host)?
    } else {
        args.port
    };
    let expanded = process::expand_args(&args.server_arg, &args.host, port);
    let mut child = process::spawn_server(&args.server_cmd.to_string_lossy(), &expanded)?;

    let result = capture(args, port).await;

    // Always stop the server, including on every error path.
    let _ = child.start_kill();
    let _ = child.wait().await;
    result
}

async fn capture(args: &RecordArgs, port: u16) -> Result<()> {
    let address: SocketAddr = process::resolve_address(&format!("{}:{port}", args.host)).await?;
    let mut stream = process::connect_with_retry(address, Duration::from_secs(10))
        .await
        .with_context(|| format!("connect to server {address}"))?;

    // GET_FORMAT
    stream
        .write_all(&encode_command(Command::GetFormat))
        .await
        .context("send GET_FORMAT")?;
    let (format, get_format_response) = read_get_format_response(&mut stream).await?;
    let block_align = format.block_align();
    if block_align == 0 {
        bail!("server advertised an unusable format ({format:?})");
    }

    // START_PLAY
    stream
        .write_all(&encode_command(Command::StartPlay))
        .await
        .context("send START_PLAY")?;
    let mut start_response = [0u8; 8];
    stream
        .read_exact(&mut start_response)
        .await
        .context("read START_PLAY response")?;
    let (start_play_id, consumed) = decode_start_play_response(&start_response)?;
    if consumed != start_response.len() || start_play_id <= 0 {
        bail!("server returned invalid START_PLAY id {start_play_id}");
    }

    // UDP id bind
    let udp = UdpSocket::bind(SocketAddr::new(address.ip(), 0))
        .await
        .context("bind recorder UDP socket")?;
    udp.send_to(&encode_udp_bind(start_play_id), address)
        .await
        .context("send UDP bind id")?;

    let wanted_heartbeats = args.heartbeats.max(1);
    let wanted_datagrams = args.datagrams.max(1);
    let aligned_payload = max_segment_payload(block_align);
    let deadline = Instant::now() + Duration::from_secs(args.timeout_secs.max(1));
    let mut tcp_buffer = Vec::new();
    let mut udp_buffer = vec![0u8; 65_535];
    let mut server_heartbeat: Option<Vec<u8>> = None;
    let mut heartbeats = 0usize;
    let mut segments: Vec<Vec<u8>> = Vec::new();

    loop {
        if heartbeats >= wanted_heartbeats && segments.len() >= wanted_datagrams {
            break;
        }
        if Instant::now() >= deadline {
            bail!(
                "record timed out after {}s: heartbeats {heartbeats}/{wanted_heartbeats}, datagrams {}/{}",
                args.timeout_secs,
                segments.len(),
                wanted_datagrams
            );
        }
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => continue,
            read = stream.read_buf(&mut tcp_buffer) => {
                let read = read.context("read TCP control channel")?;
                if read == 0 {
                    bail!("server closed TCP before recording completed");
                }
                while tcp_buffer.len() >= 4 {
                    let consumed = decode_heartbeat(&tcp_buffer)?;
                    let bytes: Vec<u8> = tcp_buffer.drain(..consumed).collect();
                    if server_heartbeat.is_none() {
                        server_heartbeat = Some(bytes);
                    }
                    heartbeats += 1;
                    stream
                        .write_all(&encode_heartbeat())
                        .await
                        .context("send heartbeat response")?;
                }
            }
            received = udp.recv_from(&mut udp_buffer) => {
                let (length, _peer) = received.context("receive UDP PCM")?;
                if length == 0 {
                    continue;
                }
                if length > aligned_payload {
                    bail!(
                        "UDP datagram of {length} bytes exceeds the aligned MTU payload {aligned_payload}"
                    );
                }
                if length % block_align != 0 {
                    bail!(
                        "UDP datagram of {length} bytes is not aligned to block_align {block_align}"
                    );
                }
                if segments.len() < wanted_datagrams {
                    segments.push(udp_buffer[..length].to_vec());
                }
            }
        }
    }

    let server_heartbeat =
        server_heartbeat.context("record finished without observing a server heartbeat")?;
    let control = ControlBytes {
        get_format_request: encode_command(Command::GetFormat).to_vec(),
        get_format_response,
        start_play_request: encode_command(Command::StartPlay).to_vec(),
        start_play_response: start_response.to_vec(),
        heartbeat_server: server_heartbeat,
        heartbeat_client: encode_heartbeat().to_vec(),
        udp_bind: encode_udp_bind(start_play_id).to_vec(),
        start_play_id,
        heartbeat_count: heartbeats,
    };
    let datagrams_received = segments.len();
    let plan = FixturePlan {
        provenance: args.provenance.clone(),
        platform: args
            .platform
            .clone()
            .unwrap_or_else(|| std::env::consts::OS.to_owned()),
        source: args.server_label.clone(),
        format,
        control,
        pcm_segments: segments,
        heartbeats_received: heartbeats,
        datagrams_received,
        notes: vec![
            format!(
                "Recorded by protocol-harness from {} over the v0.3.4 Wire Protocol.",
                args.server_label
            ),
            "PCM content is a live capture snapshot; re-recording produces different PCM bytes but the same framing.".to_owned(),
            "No timestamps or machine paths are stored in this fixture set.".to_owned(),
        ],
    };
    write_fixture_set(&args.out, &plan)?;
    println!("record: ok");
    println!("fixture: {}", args.out.display());
    println!(
        "format: {:?} channels={} sample_rate={} block_align={}",
        plan.format.encoding_kind(),
        plan.format.channels,
        plan.format.sample_rate,
        block_align
    );
    println!("heartbeats: {heartbeats}");
    println!("datagrams: {datagrams_received}");
    Ok(())
}

async fn read_get_format_response(stream: &mut TcpStream) -> Result<(AudioFormat, Vec<u8>)> {
    let mut header = [0u8; 8];
    stream
        .read_exact(&mut header)
        .await
        .context("read GET_FORMAT header")?;
    if header[..4] != encode_command(Command::GetFormat) {
        bail!("server did not reply with GET_FORMAT");
    }
    let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    if size > 1_048_576 {
        bail!("GET_FORMAT response declares an implausible size {size}");
    }
    let mut payload = vec![0u8; size];
    stream
        .read_exact(&mut payload)
        .await
        .context("read GET_FORMAT payload")?;
    let mut frame = header.to_vec();
    frame.extend_from_slice(&payload);
    let (format, consumed) = decode_get_format_response(&frame)?;
    if consumed != frame.len() {
        bail!("GET_FORMAT response has trailing bytes");
    }
    Ok((format, frame))
}
