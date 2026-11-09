//! Offline and live replay of fixture sets.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use audio_share_core::protocol::{
    decode_get_format_response, decode_heartbeat, decode_start_play_response, decode_udp_bind,
    encode_command, encode_heartbeat, encode_udp_bind, max_segment_payload, AudioFormat, Command,
    IP_HEADER_BYTES, MAX_PAYLOAD_BYTES, MTU, UDP_HEADER_BYTES,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UdpSocket;
use tokio::time::Instant;

use crate::fixture::{hex, load_fixture, sha256_hex, Fixture};
use crate::process;

/// `replay` command arguments.
#[derive(Debug, clap::Args)]
pub struct ReplayArgs {
    /// Fixture set to replay or validate.
    #[arg(long)]
    pub fixture: PathBuf,
    /// Drive an already-running server at `ip:port`.
    #[arg(long)]
    pub connect: Option<String>,
    /// Launch this server executable, then drive it.
    #[arg(long)]
    pub server_cmd: Option<PathBuf>,
    /// Server argument; `{host}`, `{port}`, and `{bind}` are substituted.
    #[arg(long, allow_hyphen_values = true)]
    pub server_arg: Vec<String>,
    /// Host used for `--server-cmd` and its placeholders.
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    /// Port used for `--server-cmd`; 0 picks a free port.
    #[arg(long, default_value_t = 0)]
    pub port: u16,
    /// How many heartbeats a live replay must receive.
    #[arg(long, default_value_t = 1)]
    pub heartbeats: usize,
    /// How many UDP datagrams a live replay must receive.
    #[arg(long, default_value_t = 16)]
    pub datagrams: usize,
    /// Overall live replay timeout in seconds.
    #[arg(long, default_value_t = 20)]
    pub timeout_secs: u64,
}

/// `replay` entry point.
pub async fn run(args: &ReplayArgs) -> Result<()> {
    let fixture = load_fixture(&args.fixture)
        .with_context(|| format!("load fixture {}", args.fixture.display()))?;
    let offline = validate_offline(&fixture)?;

    match (&args.connect, &args.server_cmd) {
        (Some(_), Some(_)) => bail!("use either --connect or --server-cmd, not both"),
        (Some(address), None) => {
            let server = process::resolve_address(address).await?;
            replay_live(&fixture, &offline, server, args).await?;
        }
        (None, Some(command)) => {
            let port = if args.port == 0 {
                process::pick_free_port(&args.host)?
            } else {
                args.port
            };
            let expanded = process::expand_args(&args.server_arg, &args.host, port);
            let _child = process::spawn_server(&command.to_string_lossy(), &expanded)?;
            let server = process::resolve_address(&format!("{}:{port}", args.host)).await?;
            replay_live(&fixture, &offline, server, args).await?;
        }
        (None, None) => offline.print(&args.fixture),
    }
    Ok(())
}

/// What offline validation proved.
#[derive(Debug, Clone, PartialEq)]
pub struct OfflineReport {
    pub provenance: String,
    pub format: AudioFormat,
    pub block_align: usize,
    pub aligned_payload: usize,
    pub heartbeats: usize,
    pub start_play_id: i32,
    pub datagrams: usize,
    pub pcm_bytes: usize,
}

impl OfflineReport {
    fn print(&self, dir: &std::path::Path) {
        println!("replay: ok (offline)");
        println!("fixture: {}", dir.display());
        println!("provenance: {}", self.provenance);
        println!(
            "format: {:?} channels={} sample_rate={} block_align={}",
            self.format.encoding_kind(),
            self.format.channels,
            self.format.sample_rate,
            self.block_align
        );
        println!(
            "control: start_play_id={} heartbeats={}",
            self.start_play_id, self.heartbeats
        );
        println!(
            "pcm: {} datagrams, {} bytes, aligned payload {} (MTU {})",
            self.datagrams, self.pcm_bytes, self.aligned_payload, MTU
        );
    }
}

fn read_control(
    fixture: &Fixture,
    file: &str,
    length: usize,
    expected_hex: &str,
    expected_sha256: &str,
) -> Result<Vec<u8>> {
    let bytes = fixture.read_file(file)?;
    if bytes.len() != length {
        bail!("{file} has {} bytes, manifest says {length}", bytes.len());
    }
    if hex(&bytes) != expected_hex {
        bail!("{file} bytes disagree with the manifest hex");
    }
    if sha256_hex(&bytes) != expected_sha256 {
        bail!("{file} hash disagrees with the manifest");
    }
    Ok(bytes)
}

/// Validate one fixture set entirely from disk.
pub fn validate_offline(fixture: &Fixture) -> Result<OfflineReport> {
    let manifest = &fixture.manifest;
    let format = fixture.decoded_format()?;
    let block_align = format.block_align();
    if block_align == 0 {
        bail!("fixture format has an unusable block_align");
    }
    let Some(encoding) = format.encoding_kind() else {
        bail!("fixture format has an unknown encoding {}", format.encoding);
    };
    if manifest.audio_format.encoding_name != encoding.proto_name() {
        bail!(
            "manifest encoding_name {} is not {}",
            manifest.audio_format.encoding_name,
            encoding.proto_name()
        );
    }
    if manifest.audio_format.block_align != block_align {
        bail!(
            "manifest block_align {} is not {}",
            manifest.audio_format.block_align,
            block_align
        );
    }

    if manifest.mtu.mtu != MTU
        || manifest.mtu.ip_header_bytes != IP_HEADER_BYTES
        || manifest.mtu.udp_header_bytes != UDP_HEADER_BYTES
        || manifest.mtu.max_payload_bytes != MAX_PAYLOAD_BYTES
    {
        bail!("manifest MTU constants are not v0.3.4");
    }
    let aligned_payload = max_segment_payload(block_align);
    if manifest.mtu.aligned_payload_bytes != aligned_payload {
        bail!(
            "manifest aligned payload {} is not {aligned_payload}",
            manifest.mtu.aligned_payload_bytes
        );
    }

    // The exact AudioFormat protobuf is the canonical bytes for the format.
    let protobuf = read_control(
        fixture,
        &manifest.audio_format.protobuf_file,
        manifest.audio_format.protobuf_hex.len() / 2,
        &manifest.audio_format.protobuf_hex,
        &manifest.audio_format.protobuf_sha256,
    )?;

    let control = &manifest.control;
    let get_format_request = read_control(
        fixture,
        &control.get_format_request.file,
        control.get_format_request.length,
        &control.get_format_request.hex,
        &control.get_format_request.sha256,
    )?;
    if get_format_request != encode_command(Command::GetFormat) {
        bail!("GET_FORMAT request bytes are not the v0.3.4 command");
    }

    let get_format_response = read_control(
        fixture,
        &control.get_format_response.file,
        control.get_format_response.length,
        &control.get_format_response.hex,
        &control.get_format_response.sha256,
    )?;
    let (response_format, consumed) = decode_get_format_response(&get_format_response)?;
    if consumed != get_format_response.len() {
        bail!("GET_FORMAT response has trailing bytes");
    }
    if response_format != format {
        bail!("GET_FORMAT response carries a different AudioFormat");
    }
    if !get_format_response.ends_with(&protobuf) {
        bail!("GET_FORMAT response does not embed the audio_format.pb bytes");
    }

    let start_play_request = read_control(
        fixture,
        &control.start_play_request.file,
        control.start_play_request.length,
        &control.start_play_request.hex,
        &control.start_play_request.sha256,
    )?;
    if start_play_request != encode_command(Command::StartPlay) {
        bail!("START_PLAY request bytes are not the v0.3.4 command");
    }

    let start_play_response = read_control(
        fixture,
        &control.start_play_response.file,
        control.start_play_response.length,
        &control.start_play_response.hex,
        &control.start_play_response.sha256,
    )?;
    let (start_play_id, consumed) = decode_start_play_response(&start_play_response)?;
    if consumed != start_play_response.len() {
        bail!("START_PLAY response has trailing bytes");
    }
    if start_play_id != control.start_play_response.id || start_play_id <= 0 {
        bail!("START_PLAY response id {start_play_id} disagrees with the manifest");
    }

    let heartbeat = encode_heartbeat();
    for (label, file) in [
        ("server", &control.heartbeat_server),
        ("client", &control.heartbeat_client),
    ] {
        let bytes = read_control(fixture, &file.file, file.length, &file.hex, &file.sha256)?;
        if bytes != heartbeat {
            bail!("{label} heartbeat bytes are not the v0.3.4 command");
        }
        decode_heartbeat(&bytes)?;
        if file.count == 0 {
            bail!("{label} heartbeat count is zero");
        }
    }

    let udp_bind = read_control(
        fixture,
        &control.udp_bind.file,
        control.udp_bind.length,
        &control.udp_bind.hex,
        &control.udp_bind.sha256,
    )?;
    let bind_id = decode_udp_bind(&udp_bind)?;
    if bind_id != control.udp_bind.id || bind_id != start_play_id {
        bail!("UDP bind id {bind_id} disagrees with START_PLAY id {start_play_id}");
    }

    let pcm = &manifest.pcm;
    let stream = fixture.read_file(&pcm.file)?;
    if stream.len() != pcm.total_bytes {
        bail!(
            "{} has {} bytes, manifest says {}",
            pcm.file,
            stream.len(),
            pcm.total_bytes
        );
    }
    if sha256_hex(&stream) != pcm.sha256 {
        bail!("PCM stream hash disagrees with the manifest");
    }
    if pcm.segment_count != pcm.segments.len() || pcm.segment_lengths.len() != pcm.segments.len() {
        bail!("PCM segment counts disagree with the manifest");
    }
    if manifest.capture.datagrams_received != pcm.segments.len() {
        bail!("capture datagram count disagrees with PCM segments");
    }

    let mut rebuilt = Vec::with_capacity(stream.len());
    let mut expected_offset = 0usize;
    let mut full_segments = 0usize;
    for (index, segment) in pcm.segments.iter().enumerate() {
        if segment.offset != expected_offset {
            bail!(
                "PCM segment {index} offset {} is not {expected_offset}",
                segment.offset
            );
        }
        if pcm.segment_lengths[index] != segment.length {
            bail!("PCM segment {index} length disagrees with segment_lengths");
        }
        if segment.length == 0 || segment.length > aligned_payload {
            bail!("PCM segment {index} has invalid length {}", segment.length);
        }
        if segment.length % block_align != 0 {
            bail!(
                "PCM segment {index} length {} is not aligned to block_align {block_align}",
                segment.length
            );
        }
        if segment.length == aligned_payload {
            full_segments += 1;
        }
        let bytes = fixture.read_file(&segment.file)?;
        if bytes.len() != segment.length {
            bail!(
                "PCM segment file {} length disagrees with the manifest",
                segment.file
            );
        }
        if sha256_hex(&bytes) != segment.sha256 {
            bail!(
                "PCM segment file {} hash disagrees with the manifest",
                segment.file
            );
        }
        if stream
            .get(segment.offset..segment.offset + segment.length)
            .map(|slice| slice == bytes.as_slice())
            != Some(true)
        {
            bail!(
                "PCM segment file {} disagrees with the stream",
                segment.file
            );
        }
        expected_offset += segment.length;
        rebuilt.extend_from_slice(&bytes);
    }
    if expected_offset != stream.len() || rebuilt != stream {
        bail!("PCM segments do not reconstruct the stream");
    }
    if full_segments == 0 {
        bail!("fixture has no full-size UDP segment; MTU segmentation is unproven");
    }

    Ok(OfflineReport {
        provenance: manifest.provenance.clone(),
        format,
        block_align,
        aligned_payload,
        heartbeats: manifest.control.heartbeat_server.count,
        start_play_id,
        datagrams: pcm.segments.len(),
        pcm_bytes: stream.len(),
    })
}

/// Drive a live server and compare its control framing with the fixture.
///
/// GET_FORMAT bytes must match exactly. START_PLAY must return a positive id,
/// heartbeats must be the v0.3.4 command, and every UDP datagram must fit the
/// aligned MTU payload and stay block-aligned. PCM content is intentionally not
/// byte-compared because the live audio source differs from the recording.
async fn replay_live(
    fixture: &Fixture,
    offline: &OfflineReport,
    server: SocketAddr,
    args: &ReplayArgs,
) -> Result<()> {
    let control = &fixture.manifest.control;
    let get_format_request = fixture.read_file(&control.get_format_request.file)?;
    let expected_format_response = fixture.read_file(&control.get_format_response.file)?;
    let start_play_request = fixture.read_file(&control.start_play_request.file)?;
    let heartbeat_client = fixture.read_file(&control.heartbeat_client.file)?;

    let mut stream = process::connect_with_retry(server, Duration::from_secs(10))
        .await
        .with_context(|| format!("connect to live server {server}"))?;

    stream
        .write_all(&get_format_request)
        .await
        .context("send GET_FORMAT")?;
    let mut header = [0u8; 8];
    stream
        .read_exact(&mut header)
        .await
        .context("read GET_FORMAT header")?;
    let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    if size > 1_048_576 {
        bail!("GET_FORMAT response declares an implausible size {size}");
    }
    let mut payload = vec![0u8; size];
    stream
        .read_exact(&mut payload)
        .await
        .context("read GET_FORMAT payload")?;
    let mut response = header.to_vec();
    response.extend_from_slice(&payload);
    if response != expected_format_response {
        bail!(
            "GET_FORMAT response differs from the fixture: expected {} got {}",
            hex(&expected_format_response),
            hex(&response)
        );
    }

    stream
        .write_all(&start_play_request)
        .await
        .context("send START_PLAY")?;
    let mut start_response = [0u8; 8];
    stream
        .read_exact(&mut start_response)
        .await
        .context("read START_PLAY response")?;
    let (start_play_id, consumed) = decode_start_play_response(&start_response)?;
    if consumed != start_response.len() || start_play_id <= 0 {
        bail!("START_PLAY returned invalid id {start_play_id}");
    }

    let udp = UdpSocket::bind(SocketAddr::new(server.ip(), 0))
        .await
        .context("bind replay UDP socket")?;
    udp.send_to(&encode_udp_bind(start_play_id), server)
        .await
        .context("send UDP bind id")?;

    let wanted_heartbeats = args.heartbeats.max(1);
    let wanted_datagrams = args.datagrams.max(1);
    let deadline = Instant::now() + Duration::from_secs(args.timeout_secs.max(1));
    let mut tcp_buffer = Vec::new();
    let mut udp_buffer = vec![0u8; 65_535];
    let mut heartbeat_count = 0usize;
    let mut datagram_count = 0usize;
    let mut pcm_bytes = 0usize;
    let mut full_segments = 0usize;

    loop {
        if heartbeat_count >= wanted_heartbeats && datagram_count >= wanted_datagrams {
            break;
        }
        if Instant::now() >= deadline {
            bail!(
                "live replay timed out after {}s: heartbeats {heartbeat_count}/{wanted_heartbeats}, datagrams {datagram_count}/{wanted_datagrams}",
                args.timeout_secs
            );
        }
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => continue,
            read = stream.read_buf(&mut tcp_buffer) => {
                let read = read.context("read TCP control channel")?;
                if read == 0 {
                    bail!("server closed TCP before replay completed");
                }
                while tcp_buffer.len() >= 4 {
                    let consumed = decode_heartbeat(&tcp_buffer)?;
                    tcp_buffer.drain(..consumed);
                    heartbeat_count += 1;
                    stream
                        .write_all(&heartbeat_client)
                        .await
                        .context("send heartbeat response")?;
                }
            }
            received = udp.recv_from(&mut udp_buffer) => {
                let (length, _peer) = received.context("receive UDP PCM")?;
                if length == 0 {
                    continue;
                }
                if length > offline.aligned_payload {
                    bail!(
                        "UDP datagram of {length} bytes exceeds the aligned MTU payload {}",
                        offline.aligned_payload
                    );
                }
                if length % offline.block_align != 0 {
                    bail!(
                        "UDP datagram of {length} bytes is not aligned to block_align {}",
                        offline.block_align
                    );
                }
                if length == offline.aligned_payload {
                    full_segments += 1;
                }
                datagram_count += 1;
                pcm_bytes += length;
            }
        }
    }

    println!("replay: ok (live)");
    println!("target: {server}");
    println!("format_bytes_match=true");
    println!("start_play_id: {start_play_id}");
    println!("heartbeats: {heartbeat_count}");
    println!(
        "pcm: {datagram_count} datagrams, {pcm_bytes} bytes, {full_segments} full-size (aligned payload {})",
        offline.aligned_payload
    );
    Ok(())
}
