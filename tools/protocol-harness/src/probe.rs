//! Acceptance probe: drive a running server over the v0.3.4 Wire Protocol and
//! report the negotiated format, datagram framing, heartbeat cadence, and
//! whether the captured PCM contains the expected tone.
//!
//! The probe is deliberately client-side only. It never assumes a requested
//! format was accepted: every field in its report comes from the server's
//! `GET_FORMAT` reply and the datagrams actually received.
//!
//! It is used by `scripts/acceptance-matrix.sh` and `scripts/soak.sh`; the JSON
//! report is the machine-readable evidence those scripts aggregate.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use audio_share_core::protocol::{
    decode_get_format_response, decode_heartbeat, decode_start_play_response, encode_command,
    encode_heartbeat, encode_udp_bind, max_segment_payload, AudioFormat, Command, Encoding,
    MAX_PAYLOAD_BYTES,
};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UdpSocket;
use tokio::time::Instant;

use crate::fixture::hex;
use crate::process;

/// How much captured PCM is kept for tone analysis (8 MiB ≈ 21 s of F32
/// stereo 48 kHz). Datagram statistics cover the whole run.
const PCM_CAPTURE_CAP: usize = 8 * 1024 * 1024;

/// Datagram arrival gaps are counted for these thresholds in milliseconds.
const GAP_THRESHOLDS_MS: [u64; 3] = [50, 100, 250];

/// `probe` command arguments.
#[derive(Debug, clap::Args)]
pub struct ProbeArgs {
    /// Running server address, `host:port`.
    #[arg(long)]
    pub connect: String,
    /// Label copied into the JSON report.
    #[arg(long, default_value = "probe")]
    pub label: String,
    /// How long the probe runs, in seconds.
    #[arg(long, default_value_t = 3)]
    pub duration_secs: u64,
    /// Stop a single-session probe after this many datagrams; 0 = no limit.
    #[arg(long, default_value_t = 0)]
    pub datagrams: usize,
    /// Minimum acceptable datagram count (zero allows a silence probe).
    #[arg(long, default_value_t = 1)]
    pub min_datagrams: usize,
    /// Hard overall timeout in seconds; 0 = duration + 10.
    #[arg(long, default_value_t = 0)]
    pub timeout_secs: u64,
    /// Expected server heartbeat interval in milliseconds.
    #[arg(long, default_value_t = 3_000)]
    pub heartbeat_interval_ms: u64,
    /// A gap longer than this factor times the interval counts as a miss.
    #[arg(long, default_value_t = 2.0)]
    pub heartbeat_gap_factor: f32,
    /// Do not reply to server heartbeats (exercises the timeout).
    #[arg(long)]
    pub no_heartbeat_reply: bool,
    /// Wait for the server to close the TCP session and report when.
    #[arg(long)]
    pub expect_server_close: bool,
    /// Reconnect after this many seconds; 0 = one long session.
    #[arg(long, default_value_t = 0)]
    pub reconnect_every_secs: u64,
    /// Delay between a disconnect and the next connection.
    #[arg(long, default_value_t = 300)]
    pub reconnect_delay_ms: u64,
    /// Tone frequency to look for; 0 disables tone analysis.
    #[arg(long, default_value_t = 440.0)]
    pub tone_hz: f32,
    /// Minimum Goertzel amplitude for the tone to count as present.
    #[arg(long, default_value_t = 0.01)]
    pub tone_min_amplitude: f32,
    /// Write the JSON report to this file as well as stdout.
    #[arg(long)]
    pub json_out: Option<PathBuf>,
    /// Create this file after START_PLAY and the UDP bind (script handshake).
    #[arg(long)]
    pub ready_file: Option<PathBuf>,
}

/// The negotiated capture format, as reported by `GET_FORMAT`.
#[derive(Clone, Debug, Serialize)]
pub struct FormatReport {
    pub encoding: String,
    pub encoding_value: i32,
    pub channels: i32,
    pub sample_rate: i32,
    pub block_align: usize,
}

impl FormatReport {
    fn new(format: &AudioFormat) -> Self {
        Self {
            encoding: format
                .encoding_kind()
                .map(|encoding| match encoding {
                    Encoding::Invalid => "invalid",
                    Encoding::PcmFloat => "f32",
                    Encoding::Pcm8Bit => "s8",
                    Encoding::Pcm16Bit => "s16",
                    Encoding::Pcm24Bit => "s24",
                    Encoding::Pcm32Bit => "s32",
                })
                .unwrap_or("unknown")
                .to_owned(),
            encoding_value: format.encoding,
            channels: format.channels,
            sample_rate: format.sample_rate,
            block_align: format.block_align(),
        }
    }
}

/// Result of looking for the test tone in captured PCM.
#[derive(Clone, Debug, Serialize)]
pub struct ToneReport {
    /// Frequency the probe looked for.
    pub expected_hz: f32,
    /// Strongest frequency found in the 300-600 Hz scan.
    pub detected_hz: f32,
    /// Goertzel amplitude at the expected frequency.
    pub amplitude: f32,
    /// Control frequency used to reject broadband noise or a wrong tone.
    pub reference_hz: f32,
    /// Goertzel amplitude at the control frequency.
    pub reference_amplitude: f32,
    /// `true` when `detected_hz` is within 5 Hz of the expected tone and the
    /// expected amplitude clears `min_amplitude`.
    pub present: bool,
}

/// One probe run's machine-readable result.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ProbeReport {
    pub label: String,
    pub ok: bool,
    pub sessions: u64,
    pub reconnects: u64,
    pub format: Option<FormatReport>,
    pub format_response_hex: String,
    pub start_play_id: i32,
    pub start_play_ids: Vec<i32>,
    pub udp_bound: bool,
    pub aligned_payload_bytes: usize,
    pub datagrams: u64,
    pub pcm_bytes: u64,
    pub min_datagram_bytes: usize,
    pub max_datagram_bytes: usize,
    pub mean_datagram_bytes: f64,
    pub full_datagrams: u64,
    pub partial_datagrams: u64,
    pub alignment_ok: bool,
    pub max_datagram_ok: bool,
    pub heartbeat_count: u64,
    pub heartbeat_interval_ms: u64,
    pub heartbeat_gaps_ms: Vec<u64>,
    pub heartbeat_misses: u64,
    pub server_closed: bool,
    pub closed_after_ms: Option<u64>,
    pub expect_server_close: bool,
    pub tone: Option<ToneReport>,
    pub datagram_gaps_over_50ms: u64,
    pub datagram_gaps_over_100ms: u64,
    pub datagram_gaps_over_250ms: u64,
    pub max_datagram_gap_ms: u64,
    pub duration_elapsed_ms: u64,
    pub errors: Vec<String>,
}

/// Run one probe and print its JSON report.
pub async fn run(args: &ProbeArgs) -> Result<()> {
    let report = probe(args).await;
    let json = serde_json::to_string(&report).context("serialize probe report")?;
    println!("{json}");
    if let Some(path) = &args.json_out {
        std::fs::write(path, format!("{json}\n"))
            .with_context(|| format!("write probe report {}", path.display()))?;
    }
    if !report.ok {
        if report.errors.is_empty() {
            bail!("probe acceptance checks failed for {}", args.label);
        }
        bail!(
            "probe acceptance checks failed for {}: {}",
            args.label,
            report.errors.join("; ")
        );
    }
    Ok(())
}

/// Execute the probe, converting every failure into report data.
async fn probe(args: &ProbeArgs) -> ProbeReport {
    let started = Instant::now();
    let duration = Duration::from_secs(args.duration_secs.max(1));
    let total_deadline = started + duration;
    let hard_timeout = if args.timeout_secs == 0 {
        duration + Duration::from_secs(10)
    } else {
        Duration::from_secs(args.timeout_secs)
    };
    let hard_deadline = started + hard_timeout;

    let address = match process::resolve_address(&args.connect).await {
        Ok(address) => address,
        Err(error) => {
            return failed_report(
                args,
                started,
                format!("resolve {}: {error:#}", args.connect),
            );
        }
    };
    if args.reconnect_every_secs > 0 && args.expect_server_close {
        return failed_report(
            args,
            started,
            "--reconnect-every-secs and --expect-server-close cannot be combined".to_owned(),
        );
    }

    let mut totals = Totals::new();
    loop {
        let remaining = total_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let session_limit = if args.reconnect_every_secs > 0 {
            Duration::from_secs(args.reconnect_every_secs)
                .min(remaining)
                .min(hard_deadline.saturating_duration_since(Instant::now()))
        } else {
            hard_deadline
                .saturating_duration_since(Instant::now())
                .min(remaining.max(Duration::from_millis(1)))
        };
        match run_session(
            args,
            address,
            Instant::now() + session_limit,
            totals.sessions == 0,
        )
        .await
        {
            Ok(outcome) => {
                totals.absorb(outcome);
            }
            Err(error) => {
                totals.errors.push(format!("{error:#}"));
                break;
            }
        }
        if args.reconnect_every_secs == 0 {
            break;
        }
        let delay = Duration::from_millis(args.reconnect_delay_ms);
        if Instant::now() + delay >= total_deadline {
            break;
        }
        tokio::time::sleep(delay).await;
    }

    totals.finish(args, started)
}

fn failed_report(args: &ProbeArgs, started: Instant, error: String) -> ProbeReport {
    let mut totals = Totals::new();
    totals.errors.push(error);
    totals.finish(args, started)
}

/// Aggregated statistics over one or more probe sessions.
struct Totals {
    sessions: u64,
    format: Option<AudioFormat>,
    format_response_hex: String,
    start_play_ids: Vec<i32>,
    udp_bound: bool,
    aligned_payload_bytes: usize,
    datagrams: u64,
    pcm_bytes: u64,
    min_datagram_bytes: Option<usize>,
    max_datagram_bytes: usize,
    full_datagrams: u64,
    partial_datagrams: u64,
    alignment_ok: bool,
    max_datagram_ok: bool,
    heartbeat_count: u64,
    heartbeat_gaps_ms: Vec<u64>,
    heartbeat_misses: u64,
    pcm: Vec<u8>,
    server_closed: bool,
    closed_after_ms: Option<u64>,
    datagram_gaps_over_50ms: u64,
    datagram_gaps_over_100ms: u64,
    datagram_gaps_over_250ms: u64,
    max_datagram_gap_ms: u64,
    errors: Vec<String>,
}

impl Totals {
    fn new() -> Self {
        Self {
            sessions: 0,
            format: None,
            format_response_hex: String::new(),
            start_play_ids: Vec::new(),
            udp_bound: false,
            aligned_payload_bytes: 0,
            datagrams: 0,
            pcm_bytes: 0,
            min_datagram_bytes: None,
            max_datagram_bytes: 0,
            full_datagrams: 0,
            partial_datagrams: 0,
            alignment_ok: true,
            max_datagram_ok: true,
            heartbeat_count: 0,
            heartbeat_gaps_ms: Vec::new(),
            heartbeat_misses: 0,
            pcm: Vec::new(),
            server_closed: false,
            closed_after_ms: None,
            datagram_gaps_over_50ms: 0,
            datagram_gaps_over_100ms: 0,
            datagram_gaps_over_250ms: 0,
            max_datagram_gap_ms: 0,
            errors: Vec::new(),
        }
    }

    fn absorb(&mut self, outcome: SessionOutcome) {
        self.sessions += 1;
        if let Some(format) = self.format.as_ref() {
            if *format != outcome.format {
                self.errors.push(format!(
                    "server renegotiated the capture format between sessions: {:?} -> {:?}",
                    format, outcome.format
                ));
            }
        } else {
            self.format = Some(outcome.format.clone());
            self.aligned_payload_bytes = max_segment_payload(outcome.format.block_align());
        }
        if self.format_response_hex.is_empty() {
            self.format_response_hex = outcome.format_response_hex;
        }
        self.start_play_ids.push(outcome.start_play_id);
        self.udp_bound |= outcome.udp_bound;
        self.datagrams += outcome.datagrams;
        self.pcm_bytes += outcome.pcm_bytes;
        self.min_datagram_bytes = Some(
            self.min_datagram_bytes
                .map_or(outcome.min_datagram_bytes, |current| {
                    current.min(outcome.min_datagram_bytes)
                }),
        );
        self.max_datagram_bytes = self.max_datagram_bytes.max(outcome.max_datagram_bytes);
        self.full_datagrams += outcome.full_datagrams;
        self.partial_datagrams += outcome.partial_datagrams;
        self.alignment_ok &= outcome.alignment_ok;
        self.max_datagram_ok &= outcome.max_datagram_ok;
        self.heartbeat_count += outcome.heartbeat_count;
        self.heartbeat_gaps_ms.extend(outcome.heartbeat_gaps_ms);
        self.heartbeat_misses += outcome.heartbeat_misses;
        let remaining = PCM_CAPTURE_CAP.saturating_sub(self.pcm.len());
        self.pcm
            .extend_from_slice(&outcome.pcm[..outcome.pcm.len().min(remaining)]);
        self.server_closed = outcome.server_closed;
        if outcome.closed_after_ms.is_some() {
            self.closed_after_ms = outcome.closed_after_ms;
        }
        self.datagram_gaps_over_50ms += outcome.datagram_gaps_over_50ms;
        self.datagram_gaps_over_100ms += outcome.datagram_gaps_over_100ms;
        self.datagram_gaps_over_250ms += outcome.datagram_gaps_over_250ms;
        self.max_datagram_gap_ms = self.max_datagram_gap_ms.max(outcome.max_datagram_gap_ms);
        self.errors.extend(outcome.errors);
    }

    fn finish(&mut self, args: &ProbeArgs, started: Instant) -> ProbeReport {
        let tone = match (self.format.as_ref(), args.tone_hz > 0.0) {
            (Some(format), true) => Some(analyze_tone(
                &self.pcm,
                format,
                args.tone_hz,
                args.tone_min_amplitude,
            )),
            _ => None,
        };
        let structural_ok = self.sessions > 0
            && self.udp_bound
            && self.datagrams >= args.min_datagrams as u64
            && (args.min_datagrams == 0 || self.pcm_bytes > 0)
            && self.alignment_ok
            && self.max_datagram_ok
            && self.start_play_ids.iter().all(|&id| id > 0);
        let tone_ok = tone.as_ref().is_none_or(|tone| tone.present);
        let heartbeats_ok = self.heartbeat_misses == 0;
        let close_ok = !args.expect_server_close || self.server_closed;
        let expected_format_ok = self.format.as_ref().is_some_and(|format| {
            format.encoding_kind().is_some()
                && format.block_align() > 0
                && format.channels > 0
                && format.sample_rate > 0
        });
        let ok = structural_ok
            && tone_ok
            && heartbeats_ok
            && close_ok
            && expected_format_ok
            && self.errors.is_empty();
        ProbeReport {
            label: args.label.clone(),
            ok,
            sessions: self.sessions,
            reconnects: self.sessions.saturating_sub(1),
            format: self.format.as_ref().map(FormatReport::new),
            format_response_hex: std::mem::take(&mut self.format_response_hex),
            start_play_id: self.start_play_ids.first().copied().unwrap_or(0),
            start_play_ids: std::mem::take(&mut self.start_play_ids),
            udp_bound: self.udp_bound,
            aligned_payload_bytes: self.aligned_payload_bytes,
            datagrams: self.datagrams,
            pcm_bytes: self.pcm_bytes,
            min_datagram_bytes: self.min_datagram_bytes.unwrap_or(0),
            max_datagram_bytes: self.max_datagram_bytes,
            mean_datagram_bytes: if self.datagrams == 0 {
                0.0
            } else {
                self.pcm_bytes as f64 / self.datagrams as f64
            },
            full_datagrams: self.full_datagrams,
            partial_datagrams: self.partial_datagrams,
            alignment_ok: self.alignment_ok,
            max_datagram_ok: self.max_datagram_ok,
            heartbeat_count: self.heartbeat_count,
            heartbeat_interval_ms: args.heartbeat_interval_ms,
            heartbeat_gaps_ms: std::mem::take(&mut self.heartbeat_gaps_ms),
            heartbeat_misses: self.heartbeat_misses,
            server_closed: self.server_closed,
            closed_after_ms: self.closed_after_ms,
            expect_server_close: args.expect_server_close,
            tone,
            datagram_gaps_over_50ms: self.datagram_gaps_over_50ms,
            datagram_gaps_over_100ms: self.datagram_gaps_over_100ms,
            datagram_gaps_over_250ms: self.datagram_gaps_over_250ms,
            max_datagram_gap_ms: self.max_datagram_gap_ms,
            duration_elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            errors: std::mem::take(&mut self.errors),
        }
    }
}

/// One connection's observations.
struct SessionOutcome {
    format: AudioFormat,
    format_response_hex: String,
    start_play_id: i32,
    udp_bound: bool,
    datagrams: u64,
    pcm_bytes: u64,
    min_datagram_bytes: usize,
    max_datagram_bytes: usize,
    full_datagrams: u64,
    partial_datagrams: u64,
    alignment_ok: bool,
    max_datagram_ok: bool,
    heartbeat_count: u64,
    heartbeat_gaps_ms: Vec<u64>,
    heartbeat_misses: u64,
    pcm: Vec<u8>,
    server_closed: bool,
    closed_after_ms: Option<u64>,
    datagram_gaps_over_50ms: u64,
    datagram_gaps_over_100ms: u64,
    datagram_gaps_over_250ms: u64,
    max_datagram_gap_ms: u64,
    errors: Vec<String>,
}

async fn run_session(
    args: &ProbeArgs,
    address: SocketAddr,
    session_deadline: Instant,
    write_ready_file: bool,
) -> Result<SessionOutcome> {
    let mut stream = process::connect_with_retry(address, Duration::from_secs(10))
        .await
        .with_context(|| format!("connect to server {address}"))?;

    // GET_FORMAT
    stream
        .write_all(&encode_command(Command::GetFormat))
        .await
        .context("send GET_FORMAT")?;
    let mut header = [0u8; 8];
    stream
        .read_exact(&mut header)
        .await
        .context("read GET_FORMAT header")?;
    if header[..4] != encode_command(Command::GetFormat) {
        bail!(
            "GET_FORMAT reply has command {}, expected 1",
            u32::from_le_bytes([header[0], header[1], header[2], header[3]])
        );
    }
    let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    if size > 1_048_576 {
        bail!("GET_FORMAT reply declares an implausible size {size}");
    }
    let mut payload = vec![0u8; size];
    stream
        .read_exact(&mut payload)
        .await
        .context("read GET_FORMAT payload")?;
    let mut response = header.to_vec();
    response.extend_from_slice(&payload);
    let (format, consumed) =
        decode_get_format_response(&response).context("decode GET_FORMAT reply")?;
    if consumed != response.len() {
        bail!("GET_FORMAT reply has trailing bytes");
    }
    if format.encoding_kind().is_none() || format.block_align() == 0 {
        bail!("server advertised an unusable AudioFormat {format:?}");
    }

    // START_PLAY
    let session_start = Instant::now();
    stream
        .write_all(&encode_command(Command::StartPlay))
        .await
        .context("send START_PLAY")?;
    let mut start_response = [0u8; 8];
    stream
        .read_exact(&mut start_response)
        .await
        .context("read START_PLAY response")?;
    let (start_play_id, consumed) =
        decode_start_play_response(&start_response).context("decode START_PLAY response")?;
    if consumed != start_response.len() || start_play_id <= 0 {
        bail!("server returned an invalid START_PLAY id {start_play_id}");
    }

    // UDP id bind
    let udp = UdpSocket::bind(SocketAddr::new(address.ip(), 0))
        .await
        .context("bind probe UDP socket")?;
    udp.send_to(&encode_udp_bind(start_play_id), address)
        .await
        .context("send UDP bind id")?;
    if write_ready_file {
        if let Some(path) = &args.ready_file {
            std::fs::write(path, format!("ready id={start_play_id}\n"))
                .with_context(|| format!("write ready file {}", path.display()))?;
        }
    }

    let block_align = format.block_align();
    let aligned_payload = max_segment_payload(block_align);
    let datagram_limit = if args.datagrams > 0 && args.reconnect_every_secs == 0 {
        Some(args.datagrams)
    } else {
        None
    };
    let heartbeat_gap_limit =
        (args.heartbeat_interval_ms as f32 * args.heartbeat_gap_factor.max(1.0)) as u64;

    let mut outcome = SessionOutcome {
        format: format.clone(),
        format_response_hex: hex(&response),
        start_play_id,
        udp_bound: true,
        datagrams: 0,
        pcm_bytes: 0,
        min_datagram_bytes: usize::MAX,
        max_datagram_bytes: 0,
        full_datagrams: 0,
        partial_datagrams: 0,
        alignment_ok: true,
        max_datagram_ok: true,
        heartbeat_count: 0,
        heartbeat_gaps_ms: Vec::new(),
        heartbeat_misses: 0,
        pcm: Vec::new(),
        server_closed: false,
        closed_after_ms: None,
        datagram_gaps_over_50ms: 0,
        datagram_gaps_over_100ms: 0,
        datagram_gaps_over_250ms: 0,
        max_datagram_gap_ms: 0,
        errors: Vec::new(),
    };

    let mut tcp_buffer = Vec::new();
    let mut udp_buffer = vec![0u8; 65_535];
    let mut last_heartbeat: Option<Instant> = None;
    let mut last_datagram: Option<Instant> = None;
    let mut finished_by_limit = false;

    loop {
        if datagram_limit.is_some_and(|limit| outcome.datagrams >= limit as u64) {
            finished_by_limit = true;
            break;
        }
        if Instant::now() >= session_deadline {
            finished_by_limit = true;
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep_until(session_deadline) => {
                finished_by_limit = true;
                break;
            }
            read = stream.read_buf(&mut tcp_buffer) => {
                let read = read.context("read TCP control channel")?;
                if read == 0 {
                    outcome.server_closed = true;
                    outcome.closed_after_ms = Some(
                        u64::try_from(session_start.elapsed().as_millis()).unwrap_or(u64::MAX),
                    );
                    break;
                }
                while tcp_buffer.len() >= 4 {
                    let consumed = match decode_heartbeat(&tcp_buffer) {
                        Ok(consumed) => consumed,
                        Err(error) => {
                            outcome.errors.push(format!("decode heartbeat: {error}"));
                            tcp_buffer.clear();
                            break;
                        }
                    };
                    let now = Instant::now();
                    if let Some(previous) = last_heartbeat {
                        let gap_ms = u64::try_from(now.duration_since(previous).as_millis())
                            .unwrap_or(u64::MAX);
                        outcome.heartbeat_gaps_ms.push(gap_ms);
                        if gap_ms > heartbeat_gap_limit {
                            outcome.heartbeat_misses += 1;
                        }
                    }
                    last_heartbeat = Some(now);
                    outcome.heartbeat_count += 1;
                    tcp_buffer.drain(..consumed);
                    if !args.no_heartbeat_reply {
                        stream
                            .write_all(&encode_heartbeat())
                            .await
                            .context("reply to server heartbeat")?;
                    }
                }
            }
            received = udp.recv_from(&mut udp_buffer) => {
                let (length, _peer) = received.context("receive UDP PCM")?;
                if length == 0 {
                    continue;
                }
                match validate_datagram(length, block_align) {
                    Ok(()) => {}
                    Err(error) => {
                        outcome.errors.push(error);
                        if length > MAX_PAYLOAD_BYTES {
                            outcome.max_datagram_ok = false;
                        }
                        if !length.is_multiple_of(block_align) {
                            outcome.alignment_ok = false;
                        }
                        continue;
                    }
                }
                let now = Instant::now();
                if let Some(previous) = last_datagram {
                    let gap_ms = u64::try_from(now.duration_since(previous).as_millis())
                        .unwrap_or(u64::MAX);
                    outcome.max_datagram_gap_ms = outcome.max_datagram_gap_ms.max(gap_ms);
                    for threshold in GAP_THRESHOLDS_MS {
                        if gap_ms > threshold {
                            match threshold {
                                50 => outcome.datagram_gaps_over_50ms += 1,
                                100 => outcome.datagram_gaps_over_100ms += 1,
                                250 => outcome.datagram_gaps_over_250ms += 1,
                                _ => {}
                            }
                        }
                    }
                }
                last_datagram = Some(now);
                outcome.datagrams += 1;
                outcome.pcm_bytes += length as u64;
                outcome.min_datagram_bytes = outcome.min_datagram_bytes.min(length);
                outcome.max_datagram_bytes = outcome.max_datagram_bytes.max(length);
                if length == aligned_payload {
                    outcome.full_datagrams += 1;
                } else {
                    outcome.partial_datagrams += 1;
                }
                let remaining = PCM_CAPTURE_CAP.saturating_sub(outcome.pcm.len());
                let copied = remaining.min(length);
                outcome
                    .pcm
                    .extend_from_slice(&udp_buffer[..copied]);
            }
        }
    }

    if outcome.server_closed && !finished_by_limit && !args.expect_server_close {
        outcome
            .errors
            .push("server closed the session before the probe finished".to_owned());
    }
    if args.expect_server_close && !outcome.server_closed {
        outcome
            .errors
            .push("server did not close the session within the probe duration".to_owned());
    }
    if outcome.min_datagram_bytes == usize::MAX {
        outcome.min_datagram_bytes = 0;
    }
    let _ = stream.shutdown().await;
    Ok(outcome)
}

/// Decode interleaved PCM to mono by keeping channel 0 of each sample frame.
///
/// Samples are normalized to `[-1, 1]` for the integer encodings. Unknown
/// encodings or a non-positive channel count return an empty vector.
pub fn decode_mono_f32(bytes: &[u8], encoding: Encoding, channels: usize) -> Vec<f32> {
    let channels = channels.max(1);
    let Some(bytes_per_sample) = encoding.bytes_per_sample() else {
        return Vec::new();
    };
    let frame_size = bytes_per_sample.saturating_mul(channels);
    if frame_size == 0 {
        return Vec::new();
    }
    bytes
        .chunks_exact(frame_size)
        .map(|frame| match encoding {
            Encoding::Invalid => 0.0,
            Encoding::PcmFloat => f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]),
            Encoding::Pcm8Bit => (f32::from(frame[0]) - 128.0) / 128.0,
            Encoding::Pcm16Bit => i16::from_le_bytes([frame[0], frame[1]]) as f32 / 32_768.0,
            Encoding::Pcm24Bit => {
                let high = if frame[2] & 0x80 != 0 { 0xff } else { 0x00 };
                let value = i32::from_le_bytes([frame[0], frame[1], frame[2], high]);
                value as f32 / 8_388_608.0
            }
            Encoding::Pcm32Bit => {
                i32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]) as f32
                    / 2_147_483_648.0
            }
        })
        .collect()
}

/// Approximate peak amplitude of `frequency` in `samples` (Goertzel).
///
/// Returns `0.0` for an empty input or a zero sample rate. The estimate is
/// normalized so a full-scale sine of the target frequency measures ~1.0.
pub fn goertzel_amplitude(samples: &[f32], sample_rate: u32, frequency: f32) -> f32 {
    if samples.is_empty() || sample_rate == 0 {
        return 0.0;
    }
    let omega = 2.0 * std::f32::consts::PI * frequency / sample_rate as f32;
    let coefficient = 2.0 * omega.cos();
    let mut previous = 0.0f32;
    let mut previous_previous = 0.0f32;
    for &sample in samples {
        let current = coefficient * previous - previous_previous + sample;
        previous_previous = previous;
        previous = current;
    }
    let power = previous * previous + previous_previous * previous_previous
        - coefficient * previous * previous_previous;
    power.max(0.0).sqrt() * 2.0 / samples.len() as f32
}

/// Coarse-to-fine Goertzel scan for the strongest frequency in a range.
///
/// The scan steps by 10 Hz and refines around the best coarse bin by 1 Hz.
/// Returns `min_hz` when the input is empty.
pub fn dominant_frequency(samples: &[f32], sample_rate: u32, min_hz: f32, max_hz: f32) -> f32 {
    if samples.is_empty() || sample_rate == 0 || min_hz >= max_hz {
        return min_hz;
    }
    let mut best = (min_hz, f32::NEG_INFINITY);
    let mut frequency = min_hz;
    while frequency <= max_hz {
        let amplitude = goertzel_amplitude(samples, sample_rate, frequency);
        if amplitude > best.1 {
            best = (frequency, amplitude);
        }
        frequency += 10.0;
    }
    let coarse = best.0;
    let mut frequency = (coarse - 10.0).max(min_hz);
    while frequency <= (coarse + 10.0).min(max_hz) {
        let amplitude = goertzel_amplitude(samples, sample_rate, frequency);
        if amplitude > best.1 {
            best = (frequency, amplitude);
        }
        frequency += 1.0;
    }
    best.0
}

/// Check one received UDP datagram against the v0.3.4 MTU and alignment rules.
///
/// A valid datagram is non-empty, at most `1464` bytes, at most the largest
/// multiple of `block_align` that fits the MTU, and a whole number of sample
/// frames. The error names the violated rule so the matrix can record it.
pub fn validate_datagram(length: usize, block_align: usize) -> Result<(), String> {
    if block_align == 0 {
        return Err("server advertised an unusable block_align 0".to_owned());
    }
    if length == 0 {
        return Err("received an empty UDP datagram".to_owned());
    }
    if length > MAX_PAYLOAD_BYTES {
        return Err(format!(
            "datagram of {length} bytes exceeds the {MAX_PAYLOAD_BYTES}-byte MTU payload"
        ));
    }
    let aligned_payload = max_segment_payload(block_align);
    if aligned_payload == 0 || length > aligned_payload {
        return Err(format!(
            "datagram of {length} bytes exceeds the {aligned_payload}-byte payload aligned to block_align {block_align}"
        ));
    }
    if !length.is_multiple_of(block_align) {
        return Err(format!(
            "datagram of {length} bytes is not aligned to block_align {block_align}"
        ));
    }
    Ok(())
}

/// Look for the expected tone in interleaved PCM.
///
/// The scan covers 300-600 Hz because every generated acceptance tone is
/// 440 Hz. The control frequency is 1000 Hz for tones below 750 Hz and
/// 300 Hz for higher tones; a broadband capture that clears the amplitude
/// threshold at both frequencies is not counted as a tone.
pub fn analyze_tone(
    bytes: &[u8],
    format: &AudioFormat,
    expected_hz: f32,
    min_amplitude: f32,
) -> ToneReport {
    let encoding = format.encoding_kind().unwrap_or(Encoding::Invalid);
    let channels = usize::try_from(format.channels.max(1)).unwrap_or(1);
    let sample_rate = u32::try_from(format.sample_rate.max(0)).unwrap_or(0);
    let samples = decode_mono_f32(bytes, encoding, channels);
    let detected_hz = dominant_frequency(&samples, sample_rate, 300.0, 600.0);
    let amplitude = goertzel_amplitude(&samples, sample_rate, expected_hz);
    let reference_hz = if expected_hz < 750.0 { 1_000.0 } else { 300.0 };
    let reference_amplitude = goertzel_amplitude(&samples, sample_rate, reference_hz);
    let present = !samples.is_empty()
        && (detected_hz - expected_hz).abs() <= 5.0
        && amplitude >= min_amplitude
        && amplitude > reference_amplitude * 4.0;
    ToneReport {
        expected_hz,
        detected_hz,
        amplitude,
        reference_hz,
        reference_amplitude,
        present,
    }
}
