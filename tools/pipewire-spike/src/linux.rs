//! Linux PipeWire implementation of the capture spike.
//!
//! Uses the safe `pipewire` crate (0.10.1, feature `v1_0_0`). All PipeWire
//! objects are created and dropped on a dedicated thread; PCM crosses the
//! realtime callback boundary through a lock-free SPSC ring, and control
//! (stop) crosses the other way through `pipewire::channel`.

use std::cell::RefCell;
use std::fs::File;
use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use pipewire as pw;
use pw::spa::param::audio::{AudioFormat, AudioInfoRaw};
use pw::spa::param::format::{MediaSubtype, MediaType};
use pw::spa::param::format_utils;
use pw::spa::param::ParamType;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Object, Pod, Value};
use pw::spa::utils::{Direction, SpaTypes};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

use crate::cli::{CaptureArgs, Cli, Command, MeasureArgs};
use pipewire_spike::{
    frames_in_bytes, pick_default, summarize_intervals, summarize_values, validate_audio_params,
    EndpointInfo, IntervalStats, PcmEncoding, ValueStats,
};

const FORMAT_TIMEOUT: Duration = Duration::from_secs(5);
const CYCLE_FORMAT_TIMEOUT: Duration = Duration::from_secs(3);
const PCM_RING_BYTES: usize = 4 * 1024 * 1024;
const SAMPLE_RING_ENTRIES: usize = 8192;

// SPA chunk flags (spa/buffer/buffer.h). libspa only models CORRUPTED, so read
// the raw flags word for the discontinuity/empty bits too.
const CHUNK_FLAG_CORRUPTED: u32 = 1 << 0;
const CHUNK_FLAG_DISCONT: u32 = 1 << 1;
const CHUNK_FLAG_EMPTY: u32 = 1 << 2;

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::List => list(),
        Command::Capture(args) => capture(args),
        Command::Measure(args) => measure(args),
    }
}

// ---------------------------------------------------------------------------
// Endpoint enumeration
// ---------------------------------------------------------------------------

fn enumerate_endpoints() -> Result<Vec<EndpointInfo>> {
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None).context("creating PipeWire main loop")?;
    let context =
        pw::context::ContextRc::new(&main_loop, None).context("creating PipeWire context")?;
    let core = context.connect_rc(None).context(
        "connecting to the PipeWire daemon (is pipewire running with the expected \
         XDG_RUNTIME_DIR/PIPEWIRE_RUNTIME_DIR?)",
    )?;
    let registry = core
        .get_registry_rc()
        .context("getting PipeWire registry")?;

    let endpoints: Rc<RefCell<Vec<EndpointInfo>>> = Rc::new(RefCell::new(Vec::new()));
    let pending = core.sync(0).context("requesting PipeWire core sync")?;

    let main_loop_weak = main_loop.downgrade();
    let _core_listener = core
        .add_listener_local()
        .done(move |id, seq| {
            if id == pw::core::PW_ID_CORE && seq == pending {
                if let Some(main_loop) = main_loop_weak.upgrade() {
                    main_loop.quit();
                }
            }
        })
        .register();

    let _registry_listener = registry
        .add_listener_local()
        .global({
            let endpoints = endpoints.clone();
            move |object| {
                if object.type_ != pw::types::ObjectType::Node {
                    return;
                }
                let Some(props) = object.props else {
                    return;
                };
                if props.get(*pw::keys::MEDIA_CLASS) != Some("Audio/Sink") {
                    return;
                }
                let Some(serial) = props.get(*pw::keys::OBJECT_SERIAL) else {
                    return;
                };
                let priority_session = props
                    .get(*pw::keys::PRIORITY_SESSION)
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or(-1);
                endpoints.borrow_mut().push(EndpointInfo {
                    serial: serial.to_owned(),
                    node_name: props.get(*pw::keys::NODE_NAME).map(str::to_owned),
                    node_nick: props.get(*pw::keys::NODE_NICK).map(str::to_owned),
                    node_description: props.get(*pw::keys::NODE_DESCRIPTION).map(str::to_owned),
                    priority_session,
                });
            }
        })
        .register();

    // Registry globals are delivered before our sync roundtrip completes.
    main_loop.run();
    let result = endpoints.borrow().clone();
    Ok(result)
}

fn resolve_target(requested: &str, endpoints: &[EndpointInfo]) -> Result<String> {
    let requested = requested.trim();
    if requested.is_empty() || requested.eq_ignore_ascii_case("default") {
        let default = pick_default(endpoints).ok_or_else(|| {
            anyhow!(
                "no default Audio/Sink: every sink has priority.session <= -1; \
                 pass --endpoint <object.serial>"
            )
        })?;
        return Ok(default.serial.clone());
    }
    if let Some(endpoint) = endpoints.iter().find(|item| item.serial == requested) {
        return Ok(endpoint.serial.clone());
    }
    let available = endpoints
        .iter()
        .map(|item| format!("{} ({})", item.serial, item.display_name()))
        .collect::<Vec<_>>()
        .join(", ");
    bail!("endpoint {requested:?} is not an Audio/Sink object.serial; available: [{available}]");
}

fn list() -> Result<()> {
    let endpoints = enumerate_endpoints()?;
    if endpoints.is_empty() {
        println!("no Audio/Sink endpoints found");
        return Ok(());
    }
    let default = pick_default(&endpoints);
    for endpoint in &endpoints {
        let marker = default
            .map(|candidate| std::ptr::eq(candidate, endpoint))
            .unwrap_or(false);
        println!(
            "{} serial={} priority.session={} name={} nick={} description={} display={:?}",
            if marker { '*' } else { ' ' },
            endpoint.serial,
            endpoint.priority_session,
            endpoint.node_name.as_deref().unwrap_or("-"),
            endpoint.node_nick.as_deref().unwrap_or("-"),
            endpoint.node_description.as_deref().unwrap_or("-"),
            endpoint.display_name(),
        );
    }
    match default {
        Some(endpoint) => println!(
            "total: {}; default: serial={} ({})",
            endpoints.len(),
            endpoint.serial,
            endpoint.display_name()
        ),
        None => println!(
            "total: {}; default: none (no sink has priority.session > -1)",
            endpoints.len()
        ),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Capture session
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NegotiatedFormat {
    format: AudioFormat,
    rate: u32,
    channels: u32,
}

impl NegotiatedFormat {
    fn pcm_encoding(self) -> Option<PcmEncoding> {
        match self.format {
            AudioFormat::F32LE => Some(PcmEncoding::F32),
            AudioFormat::S8 | AudioFormat::U8 => Some(PcmEncoding::S8),
            AudioFormat::S16LE => Some(PcmEncoding::S16),
            AudioFormat::S24LE => Some(PcmEncoding::S24),
            AudioFormat::S32LE => Some(PcmEncoding::S32),
            _ => None,
        }
    }

    fn block_align(self) -> u32 {
        self.pcm_encoding()
            .map_or(0, |encoding| encoding.block_align(self.channels))
    }
}

#[derive(Debug, Clone, Copy)]
struct CallbackSample {
    timestamp_ns: u64,
    bytes: u32,
}

#[derive(Default)]
struct SharedStats {
    format: Mutex<Option<NegotiatedFormat>>,
    states: Mutex<Vec<String>>,
    error: Mutex<Option<String>>,
    callbacks: AtomicU64,
    bytes: AtomicU64,
    zero_size: AtomicU64,
    null_buffer: AtomicU64,
    corrupted: AtomicU64,
    discont: AtomicU64,
    empty: AtomicU64,
    overflow_bytes: AtomicU64,
    stream_node_id: AtomicU32,
}

struct UserData {
    stats: Arc<SharedStats>,
    pcm: HeapProd<u8>,
    samples: HeapProd<CallbackSample>,
    started: Instant,
}

struct SessionOptions {
    target: String,
    encoding: PcmEncoding,
    channels: u32,
    sample_rate: u32,
    seconds: f64,
    pcm_out: Option<PathBuf>,
    format_timeout: Duration,
}

struct SessionReport {
    format: NegotiatedFormat,
    states: Vec<String>,
    callbacks: u64,
    bytes: u64,
    pcm_bytes: u64,
    zero_size: u64,
    null_buffer: u64,
    corrupted: u64,
    discont: u64,
    empty: u64,
    overflow_bytes: u64,
    callback_intervals: Option<IntervalStats>,
    quantum_frames: Option<ValueStats>,
    elapsed: Duration,
    cpu_user: Duration,
    cpu_system: Duration,
}

fn requested_spa_format(encoding: PcmEncoding) -> AudioFormat {
    match encoding {
        PcmEncoding::F32 => AudioFormat::F32LE,
        // The old server and Android treat ENCODING_PCM_8BIT as unsigned.
        PcmEncoding::S8 => AudioFormat::U8,
        PcmEncoding::S16 => AudioFormat::S16LE,
        PcmEncoding::S24 => AudioFormat::S24LE,
        PcmEncoding::S32 => AudioFormat::S32LE,
    }
}

fn capture(args: CaptureArgs) -> Result<()> {
    validate_audio_params(args.channels, args.sample_rate).map_err(anyhow::Error::msg)?;
    if !(args.seconds.is_finite() && args.seconds > 0.0) {
        bail!("--seconds must be a positive number");
    }

    let endpoints = enumerate_endpoints()?;
    let target = resolve_target(&args.endpoint, &endpoints)?;

    let report = run_session(&SessionOptions {
        target: target.clone(),
        encoding: args.encoding,
        channels: args.channels,
        sample_rate: args.sample_rate,
        seconds: args.seconds,
        pcm_out: Some(args.out.clone()),
        format_timeout: FORMAT_TIMEOUT,
    })?;

    print_session_summary("capture", &target, &report);
    println!(
        "pcm_file={} pcm_bytes={}",
        args.out.display(),
        report.pcm_bytes
    );
    Ok(())
}

fn measure(args: MeasureArgs) -> Result<()> {
    validate_audio_params(args.channels, args.sample_rate).map_err(anyhow::Error::msg)?;
    if !(args.seconds.is_finite() && args.seconds > 0.0) {
        bail!("--seconds must be a positive number");
    }
    if !(args.cycle_seconds.is_finite() && args.cycle_seconds > 0.0) {
        bail!("--cycle-seconds must be a positive number");
    }

    let endpoints = enumerate_endpoints()?;
    let target = resolve_target(&args.endpoint, &endpoints)?;
    println!(
        "measure: target=serial:{target} requested={} channels={} sample_rate={} seconds={}",
        args.encoding, args.channels, args.sample_rate, args.seconds
    );

    let total_start = Instant::now();
    let report = run_session(&SessionOptions {
        target: target.clone(),
        encoding: args.encoding,
        channels: args.channels,
        sample_rate: args.sample_rate,
        seconds: args.seconds,
        pcm_out: None,
        format_timeout: FORMAT_TIMEOUT,
    })?;

    print_session_summary("measure", &target, &report);
    if let Some(stats) = report.callback_intervals {
        println!(
            "callback_interval_ms: count={} min={:.3} mean={:.3} p50={:.3} p95={:.3} max={:.3}",
            stats.count,
            stats.min.as_secs_f64() * 1000.0,
            stats.mean.as_secs_f64() * 1000.0,
            stats.p50.as_secs_f64() * 1000.0,
            stats.p95.as_secs_f64() * 1000.0,
            stats.max.as_secs_f64() * 1000.0,
        );
    }
    if let Some(stats) = report.quantum_frames {
        println!(
            "quantum_frames: count={} min={} mean={:.1} p50={} p95={} max={}",
            stats.count, stats.min, stats.mean, stats.p50, stats.p95, stats.max
        );
    }
    println!(
        "glitches: zero_size={} null_buffer={} corrupted={} discont={} empty={} ring_overflow_bytes={}",
        report.zero_size,
        report.null_buffer,
        report.corrupted,
        report.discont,
        report.empty,
        report.overflow_bytes,
    );
    let cpu_total = report.cpu_user + report.cpu_system;
    let cpu_percent = if report.elapsed.as_secs_f64() > 0.0 {
        cpu_total.as_secs_f64() / report.elapsed.as_secs_f64() * 100.0
    } else {
        0.0
    };
    println!(
        "cpu: user={:.3}s sys={:.3}s total={:.3}s wall={:.3}s percent={:.2}%",
        report.cpu_user.as_secs_f64(),
        report.cpu_system.as_secs_f64(),
        cpu_total.as_secs_f64(),
        report.elapsed.as_secs_f64(),
        cpu_percent,
    );

    let mut succeeded = 0usize;
    for cycle in 0..args.cycles {
        match run_session(&SessionOptions {
            target: target.clone(),
            encoding: args.encoding,
            channels: args.channels,
            sample_rate: args.sample_rate,
            seconds: args.cycle_seconds,
            pcm_out: None,
            format_timeout: CYCLE_FORMAT_TIMEOUT,
        }) {
            Ok(cycle_report) => {
                succeeded += 1;
                println!(
                    "cycle[{cycle}]: ok states={} callbacks={} bytes={}",
                    cycle_report.states.join("->"),
                    cycle_report.callbacks,
                    cycle_report.bytes,
                );
            }
            Err(error) => println!("cycle[{cycle}]: FAILED {error:#}"),
        }
    }
    println!(
        "cycles: {succeeded}/{} ok ({}s each)",
        args.cycles, args.cycle_seconds
    );
    println!(
        "measure total wall: {:.3}s",
        total_start.elapsed().as_secs_f64()
    );

    if succeeded != args.cycles {
        bail!(
            "{} of {} start/stop cycles failed",
            args.cycles - succeeded,
            args.cycles
        );
    }
    Ok(())
}

fn print_session_summary(label: &str, target: &str, report: &SessionReport) {
    let format = report.format;
    println!("{label}: target=serial:{target}");
    println!(
        "negotiated: format={:?} rate={} channels={} block_align={}",
        format.format,
        format.rate,
        format.channels,
        format.block_align(),
    );
    println!(
        "frames={} bytes={} callbacks={} states={}",
        frames_in_bytes(report.bytes, format.block_align()),
        report.bytes,
        report.callbacks,
        report.states.join("->"),
    );
}

fn run_session(options: &SessionOptions) -> Result<SessionReport> {
    let pcm_ring = HeapRb::<u8>::new(PCM_RING_BYTES);
    let (pcm_producer, mut pcm_consumer) = pcm_ring.split();
    let sample_ring = HeapRb::<CallbackSample>::new(SAMPLE_RING_ENTRIES);
    let (sample_producer, mut sample_consumer) = sample_ring.split();

    let stats = Arc::new(SharedStats::default());
    let (stop_tx, stop_rx) = pw::channel::channel::<()>();
    let (result_tx, result_rx) = mpsc::channel::<Result<(), String>>();

    let worker_stats = stats.clone();
    let target = options.target.clone();
    let encoding = options.encoding;
    let channels = options.channels;
    let sample_rate = options.sample_rate;
    let handle = thread::spawn(move || {
        let outcome = stream_worker(
            &target,
            encoding,
            channels,
            sample_rate,
            worker_stats,
            pcm_producer,
            sample_producer,
            stop_rx,
        );
        let _ = result_tx.send(outcome);
    });

    let mut output = match options.pcm_out.as_ref() {
        Some(path) => Some(
            File::create(path)
                .with_context(|| format!("creating output file {}", path.display()))?,
        ),
        None => None,
    };
    let cpu_start = cpu_times();
    let started = Instant::now();
    let mut buffer = vec![0u8; 1 << 16];
    let mut pcm_bytes = 0u64;
    let mut samples: Vec<CallbackSample> = Vec::new();
    let mut format = None;
    let mut capture_deadline: Option<Instant> = None;
    let mut worker_error: Option<String> = None;
    let mut worker_done = false;

    loop {
        drain_pcm(&mut pcm_consumer, &mut buffer, &mut output, &mut pcm_bytes)?;
        while let Some(sample) = sample_consumer.try_pop() {
            samples.push(sample);
        }
        if format.is_none() {
            format = *stats.format.lock().unwrap();
            if format.is_some() {
                capture_deadline = Some(Instant::now() + Duration::from_secs_f64(options.seconds));
            }
        }
        if worker_error.is_none() {
            if let Some(error) = stats.error.lock().unwrap().clone() {
                worker_error = Some(format!("stream error: {error}"));
            }
        }
        if !worker_done {
            match result_rx.try_recv() {
                Ok(Ok(())) => worker_done = true,
                Ok(Err(error)) => {
                    worker_error = Some(error);
                    worker_done = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    worker_error = Some("capture worker exited unexpectedly".to_owned());
                    worker_done = true;
                }
            }
        }
        if worker_done {
            break;
        }
        if let Some(deadline) = capture_deadline {
            if Instant::now() >= deadline {
                break;
            }
        } else if started.elapsed() >= options.format_timeout {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let _ = stop_tx.send(());
    if !worker_done {
        match result_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => worker_error = Some(error),
            Err(error) => worker_error = Some(format!("capture worker did not stop: {error}")),
        }
    }
    let _ = handle.join();

    // Final drain after the producer has been dropped by the worker.
    drain_pcm(&mut pcm_consumer, &mut buffer, &mut output, &mut pcm_bytes)?;
    while let Some(sample) = sample_consumer.try_pop() {
        samples.push(sample);
    }
    if let Some(file) = output.as_mut() {
        file.flush().context("flushing PCM output")?;
    }

    let elapsed = started.elapsed();
    let (cpu_end_user, cpu_end_system) = cpu_times();
    let cpu_user = cpu_end_user.saturating_sub(cpu_start.0);
    let cpu_system = cpu_end_system.saturating_sub(cpu_start.1);

    if let Some(error) = worker_error {
        bail!("PipeWire capture failed: {error}");
    }

    let format = match format.or_else(|| *stats.format.lock().unwrap()) {
        Some(format) => format,
        None => bail!(
            "no AudioFormat negotiated within {}s; is a PipeWire session manager \
             (wireplumber) running and is the target sink present?",
            options.format_timeout.as_secs()
        ),
    };
    if format.pcm_encoding().is_none() {
        bail!(
            "PipeWire negotiated unsupported format {:?} (rate={}, channels={})",
            format.format,
            format.rate,
            format.channels,
        );
    }

    let block_align = format.block_align();
    let mut timestamps = Vec::with_capacity(samples.len());
    let mut frames = Vec::with_capacity(samples.len());
    for sample in &samples {
        timestamps.push(sample.timestamp_ns);
        frames.push(frames_in_bytes(u64::from(sample.bytes), block_align));
    }
    let callback_intervals = if timestamps.len() >= 2 {
        let intervals: Vec<Duration> = timestamps
            .windows(2)
            .map(|pair| Duration::from_nanos(pair[1].saturating_sub(pair[0])))
            .collect();
        summarize_intervals(&intervals)
    } else {
        None
    };

    let states = stats.states.lock().unwrap().clone();
    let report = SessionReport {
        format,
        states,
        callbacks: stats.callbacks.load(Ordering::Relaxed),
        bytes: stats.bytes.load(Ordering::Relaxed),
        pcm_bytes,
        zero_size: stats.zero_size.load(Ordering::Relaxed),
        null_buffer: stats.null_buffer.load(Ordering::Relaxed),
        corrupted: stats.corrupted.load(Ordering::Relaxed),
        discont: stats.discont.load(Ordering::Relaxed),
        empty: stats.empty.load(Ordering::Relaxed),
        overflow_bytes: stats.overflow_bytes.load(Ordering::Relaxed),
        callback_intervals,
        quantum_frames: summarize_values(&frames),
        elapsed,
        cpu_user,
        cpu_system,
    };
    Ok(report)
}

fn drain_pcm(
    consumer: &mut HeapCons<u8>,
    buffer: &mut [u8],
    output: &mut Option<File>,
    written: &mut u64,
) -> Result<()> {
    loop {
        let count = consumer.pop_slice(buffer);
        if count == 0 {
            return Ok(());
        }
        *written += count as u64;
        if let Some(file) = output.as_mut() {
            file.write_all(&buffer[..count])
                .context("writing PCM output")?;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn stream_worker(
    target: &str,
    encoding: PcmEncoding,
    channels: u32,
    sample_rate: u32,
    stats: Arc<SharedStats>,
    pcm: HeapProd<u8>,
    samples: HeapProd<CallbackSample>,
    stop_rx: pw::channel::Receiver<()>,
) -> Result<(), String> {
    stream_worker_inner(
        target,
        encoding,
        channels,
        sample_rate,
        stats,
        pcm,
        samples,
        stop_rx,
    )
    .map_err(|error| format!("{error:#}"))
}

#[allow(clippy::too_many_arguments)]
fn stream_worker_inner(
    target: &str,
    encoding: PcmEncoding,
    channels: u32,
    sample_rate: u32,
    stats: Arc<SharedStats>,
    pcm: HeapProd<u8>,
    samples: HeapProd<CallbackSample>,
    stop_rx: pw::channel::Receiver<()>,
) -> Result<()> {
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None).context("creating PipeWire main loop")?;
    let context =
        pw::context::ContextRc::new(&main_loop, None).context("creating PipeWire context")?;
    let core = context
        .connect_rc(None)
        .context("connecting capture stream to the PipeWire daemon")?;

    let props = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::APP_NAME => "Audio Share PipeWire Spike",
        *pw::keys::NODE_NAME => "audio-share-pipewire-spike",
        *pw::keys::TARGET_OBJECT => target,
        *pw::keys::STREAM_CAPTURE_SINK => "true",
    };
    let stream = pw::stream::StreamRc::new(core.clone(), "audio-share-pipewire-spike", props)
        .context("creating PipeWire capture stream")?;

    let user_data = UserData {
        stats: stats.clone(),
        pcm,
        samples,
        started: Instant::now(),
    };

    let stream_listener = stream
        .add_local_listener_with_user_data(user_data)
        .state_changed(|stream, user_data, old, new| {
            if new == pw::stream::StreamState::Streaming {
                user_data
                    .stats
                    .stream_node_id
                    .store(stream.node_id(), Ordering::Relaxed);
            }
            if let pw::stream::StreamState::Error(message) = &new {
                *user_data.stats.error.lock().unwrap() = Some(message.clone());
            }
            user_data
                .stats
                .states
                .lock()
                .unwrap()
                .push(format!("{old:?}->{new:?}"));
        })
        .param_changed(|_stream, user_data, id, param| {
            if id != ParamType::Format.as_raw() {
                return;
            }
            let Some(param) = param else {
                return;
            };
            let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
                return;
            };
            if media_type != MediaType::Audio || media_subtype != MediaSubtype::Raw {
                return;
            }
            let mut info = AudioInfoRaw::new();
            if info.parse(param).is_err() {
                return;
            }
            let negotiated = NegotiatedFormat {
                format: info.format(),
                rate: info.rate(),
                channels: info.channels(),
            };
            *user_data.stats.format.lock().unwrap() = Some(negotiated);
        })
        .process(|stream, user_data| {
            user_data.stats.callbacks.fetch_add(1, Ordering::Relaxed);
            let Some(mut buffer) = stream.dequeue_buffer() else {
                user_data.stats.null_buffer.fetch_add(1, Ordering::Relaxed);
                return;
            };
            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }
            let data = &mut datas[0];
            // Copy the chunk metadata out before borrowing the data slice.
            let offset = data.chunk().offset() as usize;
            let size = data.chunk().size() as usize;
            let flags = data.chunk().as_raw().flags as u32;
            let Some(slice) = data.data() else {
                user_data.stats.null_buffer.fetch_add(1, Ordering::Relaxed);
                return;
            };
            if offset + size > slice.len() {
                user_data
                    .stats
                    .overflow_bytes
                    .fetch_add(size as u64, Ordering::Relaxed);
                return;
            }
            if size == 0 {
                user_data.stats.zero_size.fetch_add(1, Ordering::Relaxed);
            }
            if flags & CHUNK_FLAG_CORRUPTED != 0 {
                user_data.stats.corrupted.fetch_add(1, Ordering::Relaxed);
            }
            if flags & CHUNK_FLAG_DISCONT != 0 {
                user_data.stats.discont.fetch_add(1, Ordering::Relaxed);
            }
            if flags & CHUNK_FLAG_EMPTY != 0 {
                user_data.stats.empty.fetch_add(1, Ordering::Relaxed);
            }

            let pcm_slice = &slice[offset..offset + size];
            user_data
                .stats
                .bytes
                .fetch_add(size as u64, Ordering::Relaxed);
            let pushed = user_data.pcm.push_slice(pcm_slice);
            if pushed < pcm_slice.len() {
                user_data
                    .stats
                    .overflow_bytes
                    .fetch_add((pcm_slice.len() - pushed) as u64, Ordering::Relaxed);
            }
            let _ = user_data.samples.try_push(CallbackSample {
                timestamp_ns: user_data.started.elapsed().as_nanos() as u64,
                bytes: size as u32,
            });
        })
        .register()
        .context("registering PipeWire stream listener")?;

    let mut audio_info = AudioInfoRaw::new();
    audio_info.set_format(requested_spa_format(encoding));
    if channels > 0 {
        audio_info.set_channels(channels);
    }
    if sample_rate > 0 {
        audio_info.set_rate(sample_rate);
    }
    let object = Object {
        type_: SpaTypes::ObjectParamFormat.as_raw(),
        id: ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let serialized = PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(object))
        .context("serializing EnumFormat pod")?
        .0
        .into_inner();
    let mut params = [Pod::from_bytes(&serialized).expect("valid EnumFormat pod")];

    stream
        .connect(
            Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut params,
        )
        .context("connecting PipeWire capture stream")?;

    let main_loop_weak = main_loop.downgrade();
    let stream_control = stream.clone();
    let _stop_attached = stop_rx.attach(main_loop.loop_(), move |_| {
        let _ = stream_control.disconnect();
        if let Some(main_loop) = main_loop_weak.upgrade() {
            main_loop.quit();
        }
    });

    main_loop.run();
    drop(stream_listener);
    Ok(())
}

fn cpu_times() -> (Duration, Duration) {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if status != 0 {
        return (Duration::ZERO, Duration::ZERO);
    }
    let usage = unsafe { usage.assume_init() };
    let to_duration = |time: libc::timeval| {
        Duration::new(
            time.tv_sec.max(0) as u64,
            (time.tv_usec.max(0) as u32) * 1_000,
        )
    };
    (to_duration(usage.ru_utime), to_duration(usage.ru_stime))
}
