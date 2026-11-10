//! Windows WASAPI shared-mode loopback capture spike.
//!
//! Every command must run on a COM-initialized thread; this module initializes
//! an MTA on the main thread and creates/destroys all COM objects there. The
//! `wasapi` crate sets `AUDCLNT_STREAMFLAGS_LOOPBACK` internally when a render
//! device's client is initialized with `Direction::Capture` in shared mode.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use wasapi::{
    deinitialize, initialize_mta, DeviceEnumerator, Direction, SampleType, ShareMode, StreamMode,
    WasapiError, WaveFormat,
};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::Media::Audio::AUDCLNT_E_UNSUPPORTED_FORMAT;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessHandleCount, GetProcessTimes,
};

use crate::logic::{
    self, AdvertisedFormat, EncodingPreference, FormatSpec, NegotiatedFormat, SampleSubtype, Stats,
};

/// Requested shared-mode buffer: 1 second, matching the old C++ server.
const BUFFER_DURATION_HNS: i64 = 10_000_000;

#[derive(Parser)]
#[command(
    name = "wasapi-spike",
    about = "Windows WASAPI shared-mode loopback capture spike",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Enumerate active render endpoints and mark the default.
    List,
    /// Capture shared-mode loopback to a raw PCM file.
    Capture(CaptureArgs),
    /// Capture, report interval/CPU/glitch statistics, then run start/stop cycles.
    Measure(MeasureArgs),
    /// Repeatedly start and stop a 1-second-buffer loopback stream.
    StartStop(StartStopArgs),
}

#[derive(Args, Clone)]
struct FormatArgs {
    /// Endpoint ID from `list`, or "default".
    #[arg(long, default_value = "default")]
    endpoint: String,
    /// Requested encoding; unsupported values fall back to the closest match.
    #[arg(long, value_enum, default_value = "default")]
    encoding: EncodingPreference,
    /// Channel count; 0 keeps the endpoint mix format.
    #[arg(long, default_value_t = 0)]
    channels: u16,
    /// Sample rate in Hz; 0 keeps the endpoint mix format.
    #[arg(long, default_value_t = 0)]
    sample_rate: u32,
}

#[derive(Args)]
struct CaptureArgs {
    #[command(flatten)]
    format: FormatArgs,
    /// Capture duration in seconds.
    #[arg(long, default_value_t = 10.0)]
    seconds: f64,
    /// Raw PCM output path.
    #[arg(long, default_value = "capture.pcm")]
    out: PathBuf,
}

#[derive(Args)]
struct MeasureArgs {
    #[command(flatten)]
    format: FormatArgs,
    /// Steady-state capture duration in seconds.
    #[arg(long, default_value_t = 60.0)]
    seconds: f64,
    /// Start/stop cycles run after the steady-state capture.
    #[arg(long, default_value_t = 10)]
    cycles: u32,
}

#[derive(Args)]
struct StartStopArgs {
    #[command(flatten)]
    format: FormatArgs,
    /// Number of start/stop iterations.
    #[arg(long, default_value_t = 10)]
    iterations: u32,
    /// Capture time per iteration in milliseconds (stops before the 1 s buffer fills).
    #[arg(long, default_value_t = 100)]
    milliseconds: u64,
}

pub fn main() {
    let cli = Cli::parse();
    if let Err(err) = run(cli) {
        eprintln!("wasapi-spike: {err:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let _com = ComGuard::initialize()?;
    match cli.command {
        Command::List => list_endpoints(),
        Command::Capture(args) => capture(args),
        Command::Measure(args) => measure(args),
        Command::StartStop(args) => start_stop(args),
    }
}

/// One `CoInitializeEx` per successful call, balanced by one `CoUninitialize`.
struct ComGuard;

impl ComGuard {
    fn initialize() -> Result<Self> {
        let hr = initialize_mta();
        if hr.is_err() {
            bail!("CoInitializeEx(COINIT_MULTITHREADED) failed: {hr:?}");
        }
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        deinitialize();
    }
}

#[derive(Clone, Debug)]
struct Endpoint {
    id: String,
    name: String,
}

fn list_endpoints() -> Result<()> {
    let endpoints = enumerate_render_endpoints()?;
    let default_id = match default_render_endpoint() {
        Ok(endpoint) => Some(endpoint.id),
        Err(err) => {
            eprintln!("warning: default render endpoint unavailable: {err:#}");
            None
        }
    };
    for endpoint in &endpoints {
        let marker = if default_id.as_deref() == Some(endpoint.id.as_str()) {
            "*"
        } else {
            " "
        };
        println!("{marker} {}\t{}", endpoint.id, endpoint.name);
    }
    println!("total: {}", endpoints.len());
    Ok(())
}

fn enumerate_render_endpoints() -> Result<Vec<Endpoint>> {
    let enumerator = DeviceEnumerator::new().context("create audio device enumerator")?;
    let collection = enumerator
        .get_device_collection(Direction::Render)
        .context("enumerate active render endpoints")?;
    let count = collection
        .get_nbr_devices()
        .context("read render endpoint count")?;
    let mut endpoints = Vec::with_capacity(count as usize);
    for index in 0..count {
        let device = collection
            .get_device_at_index(index)
            .with_context(|| format!("open render endpoint {index}"))?;
        endpoints.push(endpoint_of(&device)?);
    }
    Ok(endpoints)
}

fn default_render_endpoint() -> Result<Endpoint> {
    let enumerator = DeviceEnumerator::new().context("create audio device enumerator")?;
    let device = enumerator
        .get_default_device(Direction::Render)
        .context("get default render endpoint (eRender, eConsole)")?;
    endpoint_of(&device)
}

fn endpoint_of(device: &wasapi::Device) -> Result<Endpoint> {
    let id = device.get_id().context("read IMMDevice ID")?;
    let name = device
        .get_friendlyname()
        .unwrap_or_else(|_| "<unknown>".to_string());
    Ok(Endpoint { id, name })
}

struct SessionInfo {
    endpoint: Endpoint,
    mix: FormatSpec,
    requested: FormatSpec,
    negotiated: NegotiatedFormat,
    advertised: AdvertisedFormat,
    buffer_frames: u32,
    default_period_hns: i64,
    min_period_hns: i64,
}

struct SessionReport {
    info: SessionInfo,
    stats: Stats,
    elapsed: Duration,
}

fn capture(args: CaptureArgs) -> Result<()> {
    let duration = positive_duration(args.seconds, "--seconds")?;
    let file = std::fs::File::create(&args.out)
        .with_context(|| format!("create PCM output {}", args.out.display()))?;
    let mut writer = std::io::BufWriter::new(file);
    let report = run_session(&args.format, duration, true, &mut writer)?;
    writer
        .flush()
        .with_context(|| format!("flush PCM output {}", args.out.display()))?;
    print_flags(&report.stats);
    println!(
        "captured: {} frames, {} bytes in {:.3} s",
        report.stats.frames,
        report.stats.bytes,
        report.elapsed.as_secs_f64()
    );
    println!("wrote: {}", args.out.display());
    Ok(())
}

fn measure(args: MeasureArgs) -> Result<()> {
    let duration = positive_duration(args.seconds, "--seconds")?;
    let mut sink = std::io::sink();
    let cpu_before = process_cpu_time()?;
    let wall_start = Instant::now();
    let report = run_session(&args.format, duration, true, &mut sink)?;
    let wall = wall_start.elapsed();
    let cpu_used = process_cpu_time()?.saturating_sub(cpu_before);

    print_stats(&report.stats, report.elapsed);
    println!(
        "effective rate: {:.1} frames/s (advertised {} Hz)",
        report.stats.frames as f64 / report.elapsed.as_secs_f64().max(f64::EPSILON),
        report.info.advertised.sample_rate
    );
    match logic::cpu_percent(cpu_used, wall) {
        Some(percent) => println!(
            "cpu: {:.3} s over {:.3} s wall ({percent:.2}% of one core)",
            cpu_used.as_secs_f64(),
            wall.as_secs_f64()
        ),
        None => println!("cpu: no wall time recorded"),
    }
    let min_period = hns_to_duration(report.info.min_period_hns);
    println!(
        "glitches: {} index-gap events, {} data discontinuities, {} timestamp errors, \
         {} polls slower than 2x min device period ({})",
        report.stats.index_gap_events,
        report.stats.discontinuities,
        report.stats.timestamp_errors,
        report.stats.long_poll_count(min_period.saturating_mul(2)),
        format_duration(min_period.saturating_mul(2))
    );

    run_cycles(&args.format, args.cycles, Duration::from_millis(100))?;
    Ok(())
}

fn start_stop(args: StartStopArgs) -> Result<()> {
    let per_iteration = Duration::from_millis(args.milliseconds.max(1));
    run_cycles(&args.format, args.iterations, per_iteration)
}

fn run_cycles(format: &FormatArgs, iterations: u32, per_iteration: Duration) -> Result<()> {
    let handles_before = process_handle_count()?;
    let mut sink = std::io::sink();
    let mut failures = 0u32;
    for iteration in 1..=iterations {
        match run_session(format, per_iteration, false, &mut sink) {
            Ok(report) => println!(
                "iteration {iteration}: ok ({} frames, {} packets)",
                report.stats.frames, report.stats.packets
            ),
            Err(err) => {
                failures += 1;
                eprintln!("iteration {iteration}: failed: {err:#}");
            }
        }
    }
    let handles_after = process_handle_count()?;
    println!(
        "start-stop: {}/{} succeeded, {} failed, process survived",
        iterations - failures,
        iterations,
        failures
    );
    println!("process handles: {handles_before} before, {handles_after} after");
    if failures > 0 {
        bail!("{failures} of {iterations} start/stop iterations failed");
    }
    Ok(())
}

/// Open a fresh client, negotiate the format, capture until `duration` elapses,
/// and stop. A new enumerator/device/client per call mirrors the old C++ server
/// and avoids `AUDCLNT_E_ALREADY_INITIALIZED` and stale buffered audio.
fn run_session(
    args: &FormatArgs,
    duration: Duration,
    announce: bool,
    writer: &mut dyn Write,
) -> Result<SessionReport> {
    let enumerator = DeviceEnumerator::new().context("create audio device enumerator")?;
    let device = if args.endpoint.is_empty() || args.endpoint == "default" {
        enumerator
            .get_default_device(Direction::Render)
            .context("get default render endpoint (eRender, eConsole)")?
    } else {
        enumerator
            .get_device(&args.endpoint)
            .with_context(|| format!("open endpoint {}", args.endpoint))?
    };
    // A capture endpoint would become a silent microphone capture instead of
    // failing, unlike the old AUDCLNT_E_WRONG_ENDPOINT_TYPE behavior.
    if device.get_direction() != Direction::Render {
        bail!(
            "endpoint {} is not a render endpoint; refusing to capture (WASAPI would \
             silently create a microphone stream)",
            args.endpoint
        );
    }
    let endpoint = endpoint_of(&device)?;

    let mut client = device
        .get_iaudioclient()
        .with_context(|| format!("activate IAudioClient for {}", endpoint.id))?;
    let mix_format = client
        .get_mixformat()
        .context("get endpoint mix format (GetMixFormat)")?;
    let mix = spec_from_waveformat(&mix_format)?;
    let requested = logic::requested_format(mix, args.encoding, args.channels, args.sample_rate);
    let requested_waveformat = waveformat_from_spec(&requested)?;

    let closest = client
        .is_supported(&requested_waveformat, ShareMode::Shared)
        .map_err(map_is_supported_error)?;
    let closest_spec = closest.as_ref().map(spec_from_waveformat).transpose()?;
    let negotiated = logic::select_negotiated(requested, closest_spec);
    let advertised = logic::advertised_format(&negotiated.format)
        .context("negotiated format cannot be advertised to the v0.3.4 client")?;
    // Closest match wins when the OS returned S_FALSE.
    let negotiated_waveformat = closest.unwrap_or(requested_waveformat);
    let (default_period_hns, min_period_hns) =
        client.get_device_period().context("get device period")?;

    client
        .initialize_client(
            &negotiated_waveformat,
            // Render device + Capture direction + Shared mode == loopback.
            Direction::Capture,
            StreamMode::PollingShared {
                autoconvert: false,
                buffer_duration_hns: BUFFER_DURATION_HNS,
            },
            None,
        )
        .map_err(map_initialize_error)?;
    let buffer_frames = client
        .get_buffer_size()
        .context("get loopback buffer size")?;
    let capture_client = client
        .get_audiocaptureclient()
        .context("get IAudioCaptureClient")?;

    let info = SessionInfo {
        endpoint,
        mix,
        requested,
        negotiated,
        advertised,
        buffer_frames,
        default_period_hns,
        min_period_hns,
    };
    if announce {
        print_session_header(&info);
    }

    let block_align = info.advertised.block_align as usize;
    if block_align == 0 || buffer_frames == 0 {
        bail!(
            "invalid negotiated stream: block_align={}, buffer_frames={}",
            info.advertised.block_align,
            buffer_frames
        );
    }
    let mut buffer = vec![0u8; buffer_frames as usize * block_align];
    let poll = hns_to_duration(min_period_hns.max(1));

    client
        .start_stream()
        .context("start loopback capture stream")?;
    let started = Instant::now();
    let mut last_poll = started;
    let mut last_packet: Option<Instant> = None;
    let mut stats = Stats::default();
    let mut loop_error: Option<anyhow::Error> = None;

    while started.elapsed() < duration {
        std::thread::sleep(poll);
        let now = Instant::now();
        stats.on_poll(now.saturating_duration_since(last_poll));
        last_poll = now;

        loop {
            let frames = match capture_client.get_next_packet_size() {
                Ok(None) | Ok(Some(0)) => break,
                Ok(Some(frames)) => frames,
                Err(err) => {
                    loop_error = Some(anyhow!("GetNextPacketSize failed: {err}"));
                    break;
                }
            };
            let want = frames as usize * block_align;
            if want > buffer.len() {
                // A packet never exceeds GetBufferSize(), but stay defensive.
                buffer.resize(want, 0);
            }
            match capture_client.read_from_device(&mut buffer[..want]) {
                Ok((got, buffer_info)) => {
                    let packet_time = Instant::now();
                    let interval =
                        last_packet.map(|previous| packet_time.saturating_duration_since(previous));
                    last_packet = Some(packet_time);
                    let flags = logic::PacketFlags {
                        silent: buffer_info.flags.silent,
                        data_discontinuity: buffer_info.flags.data_discontinuity,
                        timestamp_error: buffer_info.flags.timestamp_error,
                    };
                    stats.on_packet(got, block_align as u32, flags, buffer_info.index, interval);
                    let bytes = got as usize * block_align;
                    if let Err(err) = writer.write_all(&buffer[..bytes]) {
                        loop_error = Some(anyhow!("write PCM output: {err}"));
                        break;
                    }
                }
                Err(err) => {
                    loop_error = Some(anyhow!("IAudioCaptureClient::GetBuffer failed: {err}"));
                    break;
                }
            }
        }
        if loop_error.is_some() {
            break;
        }
    }

    let elapsed = started.elapsed();
    let stop_result = client.stop_stream();
    if let Some(err) = loop_error {
        return Err(err);
    }
    stop_result.context("stop loopback capture stream")?;
    Ok(SessionReport {
        info,
        stats,
        elapsed,
    })
}

fn print_session_header(info: &SessionInfo) {
    println!("endpoint: {} ({})", info.endpoint.id, info.endpoint.name);
    println!("mix format: {}", describe_format(&info.mix));
    println!("requested format: {}", describe_format(&info.requested));
    if info.negotiated.used_closest_match {
        println!("format support: S_FALSE (closest match used)");
    } else {
        println!("format support: S_OK (requested format accepted)");
    }
    println!(
        "negotiated AudioFormat: encoding={} channels={} sample_rate={} block_align={}",
        info.advertised.encoding.proto_name(),
        info.advertised.channels,
        info.advertised.sample_rate,
        info.advertised.block_align
    );
    let buffer_ms =
        f64::from(info.buffer_frames) * 1000.0 / f64::from(info.advertised.sample_rate.max(1));
    println!(
        "buffer: {} frames ({buffer_ms:.1} ms at {} Hz)",
        info.buffer_frames, info.advertised.sample_rate
    );
    println!(
        "device period: default={} hns ({}), min={} hns ({})",
        info.default_period_hns,
        format_duration(hns_to_duration(info.default_period_hns)),
        info.min_period_hns,
        format_duration(hns_to_duration(info.min_period_hns))
    );
}

fn print_stats(stats: &Stats, elapsed: Duration) {
    println!(
        "captured: {} frames, {} bytes in {:.3} s ({} packets, max {} frames/packet)",
        stats.frames,
        stats.bytes,
        elapsed.as_secs_f64(),
        stats.packets,
        stats.max_packet_frames
    );
    print_interval_summary("packet interval", stats.packet_interval_summary());
    print_interval_summary("poll interval", stats.poll_interval_summary());
    print_flags(stats);
}

fn print_interval_summary(label: &str, summary: Option<logic::IntervalSummary>) {
    match summary {
        Some(summary) => println!(
            "{label}: min={} avg={} max={} p99={} (n={})",
            format_duration(summary.min),
            format_duration(summary.avg),
            format_duration(summary.max),
            format_duration(summary.p99),
            summary.count
        ),
        None => println!("{label}: no samples"),
    }
}

fn print_flags(stats: &Stats) {
    println!(
        "flags: silent={} data_discontinuity={} timestamp_error={}",
        stats.silent_packets, stats.discontinuities, stats.timestamp_errors
    );
    println!(
        "index gaps: {} frames across {} packets",
        stats.index_gap_frames, stats.index_gap_events
    );
}

fn describe_format(format: &FormatSpec) -> String {
    format!(
        "{:?} {} bits ({} valid) {} ch {} Hz block_align={}",
        format.subtype,
        format.container_bits,
        format.valid_bits,
        format.channels,
        format.sample_rate,
        format.block_align
    )
}

fn format_duration(duration: Duration) -> String {
    format!("{:.3} ms", duration.as_secs_f64() * 1000.0)
}

fn hns_to_duration(hns: i64) -> Duration {
    // 1 hns = 100 ns.
    Duration::from_nanos(hns.max(0) as u64 * 100)
}

fn positive_duration(seconds: f64, flag: &str) -> Result<Duration> {
    if seconds <= 0.0 {
        bail!("{flag} must be greater than zero");
    }
    Duration::try_from_secs_f64(seconds)
        .map_err(|_| anyhow!("{flag} is not a valid duration: {seconds}"))
}

fn spec_from_waveformat(format: &WaveFormat) -> Result<FormatSpec> {
    let subtype = match format.get_subformat() {
        Ok(SampleType::Float) => SampleSubtype::Float,
        Ok(SampleType::Int) => SampleSubtype::Int,
        // Keep the spec so advertised_format can produce the typed error.
        Err(WasapiError::UnsupportedSubformat(_)) => SampleSubtype::Other,
        Err(err) => return Err(anyhow!("read format subformat: {err}")),
    };
    Ok(FormatSpec {
        subtype,
        container_bits: format.get_bitspersample(),
        valid_bits: format.get_validbitspersample(),
        channels: format.get_nchannels(),
        sample_rate: format.get_samplespersec(),
        block_align: format.get_blockalign(),
        channel_mask: Some(format.get_dwchannelmask()),
    })
}

fn waveformat_from_spec(format: &FormatSpec) -> Result<WaveFormat> {
    let sample_type = match format.subtype {
        SampleSubtype::Float => SampleType::Float,
        SampleSubtype::Int => SampleType::Int,
        SampleSubtype::Other => bail!("cannot request a format with an unknown sample subtype"),
    };
    Ok(WaveFormat::new(
        usize::from(format.container_bits),
        usize::from(format.valid_bits),
        sample_type,
        format.sample_rate as usize,
        usize::from(format.channels),
        format.channel_mask,
    ))
}

fn map_is_supported_error(err: WasapiError) -> anyhow::Error {
    match &err {
        WasapiError::Windows(windows_error)
            if windows_error.code() == AUDCLNT_E_UNSUPPORTED_FORMAT =>
        {
            anyhow!(
                "requested format is not supported by the endpoint \
                 (AUDCLNT_E_UNSUPPORTED_FORMAT); try another --encoding, --channels, \
                 or --sample-rate"
            )
        }
        WasapiError::UnsupportedFormat => anyhow!(
            "requested format is not supported by the endpoint; try another --encoding, \
             --channels, or --sample-rate"
        ),
        _ => anyhow!("IsFormatSupported failed: {err}"),
    }
}

/// `IsFormatSupported` can return `S_OK` for a format that shared-mode loopback
/// still rejects at `Initialize` (observed on Realtek for mono at the mix
/// sample rate). Map the raw `0x88890008` to an actionable message.
fn map_initialize_error(err: WasapiError) -> anyhow::Error {
    match &err {
        WasapiError::Windows(windows_error)
            if windows_error.code() == AUDCLNT_E_UNSUPPORTED_FORMAT =>
        {
            anyhow!(
                "the endpoint rejected the negotiated loopback format \
                 (AUDCLNT_E_UNSUPPORTED_FORMAT); shared-mode loopback may require the \
                 endpoint's mix format — try another --encoding, --channels, or --sample-rate"
            )
        }
        _ => anyhow!("initialize shared-mode loopback (1 s buffer) failed: {err}"),
    }
}

fn process_cpu_time() -> Result<Duration> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    }
    .map_err(|err| anyhow!("GetProcessTimes failed: {err}"))?;
    let ticks = filetime_ticks(kernel).saturating_add(filetime_ticks(user));
    Ok(Duration::from_nanos(ticks.saturating_mul(100)))
}

fn filetime_ticks(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

fn process_handle_count() -> Result<u32> {
    let mut count = 0u32;
    unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) }
        .map_err(|err| anyhow!("GetProcessHandleCount failed: {err}"))?;
    Ok(count)
}
