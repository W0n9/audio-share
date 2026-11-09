//! Linux PipeWire capture backend.
//!
//! Enumeration and capture use the safe `pipewire` crate. All PipeWire objects
//! are created, run, and dropped on the dedicated thread that serves one
//! operation or one capture session; nothing `!Send` crosses a thread
//! boundary. PCM leaves the realtime process callback through a lock-free
//! single-producer/single-consumer ring, and a forwarder thread feeds the
//! bounded core [`FrameSink`] without ever blocking the realtime callback.
//!
//! Stop works in three steps: [`CaptureSession::stop`] sets the core stop
//! signal, a small watcher thread sees it and sends a `pipewire::channel`
//! message, and the PipeWire thread disconnects the stream, quits its main
//! loop, and joins its helper threads. The watcher also exits on its own when
//! the session ends without a stop request, so no thread leaks.

use std::cell::RefCell;
use std::io::Cursor;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::error::TrySendError;

use pipewire as pw;
use pw::spa::param::audio::{AudioFormat as SpaFormat, AudioInfoRaw};
use pw::spa::param::format::{MediaSubtype, MediaType};
use pw::spa::param::format_utils;
use pw::spa::param::ParamType;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Object, Pod, Value};
use pw::spa::utils::{Direction, SpaTypes};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

use crate::capture::{
    CaptureBackend, CaptureConfig, CaptureSession, EncodingPreference, Endpoint, FrameSink,
    StopSignal,
};
use crate::error::{Error, Result};
use crate::protocol::{AudioFormat, Encoding};

/// How long [`PipeWireCapture::start`] waits for format negotiation.
const FORMAT_TIMEOUT: Duration = Duration::from_secs(5);

/// No process callback for this long after streaming started means the target
/// sink disappeared (PipeWire does not report it through the stream state).
const STALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Lock-free handoff between the realtime callback and the forwarder thread.
const PCM_RING_BYTES: usize = 4 * 1024 * 1024;

/// Maximum size of one `FrameSink` item produced by the forwarder.
const FORWARD_CHUNK_BYTES: usize = 64 * 1024;

/// How often the forwarder checks the ring when no PCM is available.
const FORWARD_IDLE_SLEEP: Duration = Duration::from_millis(1);

/// How often the liveness watchdog samples the callback clock.
const WATCHDOG_INTERVAL: Duration = Duration::from_millis(200);

/// How often the stop watcher checks the core stop signal.
const STOP_WATCH_INTERVAL: Duration = Duration::from_millis(100);

/// A captured `Audio/Sink` discovered through the PipeWire registry.
///
/// `registry_id` is the registry/global object id, used only to notice
/// `global_remove`; the Wire Protocol endpoint id is `object.serial`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SinkInfo {
    registry_id: u32,
    serial: String,
    node_name: Option<String>,
    node_nick: Option<String>,
    node_description: Option<String>,
    priority_session: i64,
}

impl SinkInfo {
    /// Old-server display order: `node.nick`, `node.description`, `node.name`,
    /// then the serial.
    fn display_name(&self) -> &str {
        self.node_nick
            .as_deref()
            .or(self.node_description.as_deref())
            .or(self.node_name.as_deref())
            .unwrap_or(&self.serial)
    }

    fn endpoint(&self) -> Endpoint {
        Endpoint {
            id: self.serial.clone(),
            name: self.display_name().to_owned(),
        }
    }
}

/// The PipeWire format requested for a protocol encoding preference.
fn requested_spa_format(preference: EncodingPreference) -> SpaFormat {
    match preference {
        EncodingPreference::Default | EncodingPreference::F32 => SpaFormat::F32LE,
        EncodingPreference::S8 => SpaFormat::U8,
        EncodingPreference::S16 => SpaFormat::S16LE,
        EncodingPreference::S24 => SpaFormat::S24LE,
        EncodingPreference::S32 => SpaFormat::S32LE,
    }
}

/// Map a negotiated PipeWire format back to its Wire Protocol encoding.
///
/// Only the exact little-endian formats the backend requests are accepted;
/// `S24LE` is packed 24-bit (three bytes), never the 32-bit `S24_32LE`
/// container.
fn protocol_encoding(format: SpaFormat) -> Option<Encoding> {
    match format {
        SpaFormat::F32LE => Some(Encoding::PcmFloat),
        SpaFormat::U8 => Some(Encoding::Pcm8Bit),
        SpaFormat::S16LE => Some(Encoding::Pcm16Bit),
        SpaFormat::S24LE => Some(Encoding::Pcm24Bit),
        SpaFormat::S32LE => Some(Encoding::Pcm32Bit),
        _ => None,
    }
}

/// Build the immutable protocol format from the negotiated values.
///
/// Returns `None` for an unsupported SPA format, a zero rate, or zero
/// channels. `AudioFormat::block_align` then derives the sample-frame size
/// (for example packed S24 stereo is 6 bytes).
fn negotiated_audio_format(format: SpaFormat, rate: u32, channels: u32) -> Option<AudioFormat> {
    if rate == 0 || channels == 0 {
        return None;
    }
    Some(AudioFormat::new(
        protocol_encoding(format)?,
        i32::try_from(channels).ok()?,
        i32::try_from(rate).ok()?,
    ))
}

/// Pick the default sink exactly like the old C++ server: start at priority
/// -1 and only replace on a strictly greater `priority.session`; the first
/// sink wins ties. No sink with priority greater than -1 means no default.
fn pick_default(sinks: &[SinkInfo]) -> Option<&SinkInfo> {
    let mut best: Option<&SinkInfo> = None;
    for sink in sinks {
        let best_priority = best.map_or(-1, |candidate| candidate.priority_session);
        if sink.priority_session > best_priority {
            best = Some(sink);
        }
    }
    best
}

/// Resolve a requested endpoint id against the live registry.
///
/// Empty and `"default"` select the priority-based default. Anything else
/// must equal an `object.serial`; PipeWire silently falls back to another node
/// for unknown targets, so this validation is mandatory.
fn select_sink<'a>(requested: &str, sinks: &'a [SinkInfo]) -> Result<&'a SinkInfo> {
    let requested = requested.trim();
    if requested.is_empty() || requested.eq_ignore_ascii_case("default") {
        return pick_default(sinks).ok_or_else(|| {
            Error::Capture(
                "no default Audio/Sink: every sink has priority.session <= -1".to_owned(),
            )
        });
    }
    sinks
        .iter()
        .find(|sink| sink.serial == requested)
        .ok_or_else(|| {
            let available = sinks
                .iter()
                .map(|sink| format!("{} ({})", sink.serial, sink.display_name()))
                .collect::<Vec<_>>()
                .join(", ");
            Error::InvalidConfig(format!(
                "endpoint {requested:?} is not an Audio/Sink object.serial; available: [{available}]"
            ))
        })
}

/// Format negotiated for a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NegotiatedFormat {
    format: SpaFormat,
    rate: u32,
    channels: u32,
}

impl NegotiatedFormat {
    fn protocol_format(self) -> Option<AudioFormat> {
        negotiated_audio_format(self.format, self.rate, self.channels)
    }
}

/// State shared by the PipeWire loop callbacks, the forwarder, and the
/// watchdog. Only the realtime process callback runs on the RT data thread;
/// its fields are atomics and the ring. Mutexes are used only on loop threads
/// and the forwarder/watchdog.
#[derive(Default)]
struct SharedState {
    format: Mutex<Option<NegotiatedFormat>>,
    stop_reason: Mutex<Option<String>>,
    streaming: AtomicBool,
    last_callback_ns: AtomicU64,
    block_align: AtomicUsize,
    dropped_bytes: AtomicU64,
}

impl SharedState {
    fn set_stop_reason(&self, reason: impl Into<String>) {
        let mut slot = self
            .stop_reason
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_none() {
            *slot = Some(reason.into());
        }
    }

    fn take_stop_reason(&self) -> Option<String> {
        self.stop_reason
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

struct CaptureUserData {
    state: Arc<SharedState>,
    producer: HeapProd<u8>,
    started: Instant,
}

/// Sets the finished flag on every exit path, including early errors.
struct FinishGuard(Arc<AtomicBool>);

impl Drop for FinishGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

fn pipewire_error(operation: &str, error: impl std::fmt::Display) -> Error {
    Error::Capture(format!("{operation}: {error}"))
}

fn connect_error(error: impl std::fmt::Display) -> Error {
    Error::Capture(format!(
        "connect to the PipeWire daemon: {error} (is pipewire running with \
         XDG_RUNTIME_DIR/PIPEWIRE_RUNTIME_DIR set?)"
    ))
}

/// The Linux PipeWire capture backend.
#[derive(Clone, Copy, Debug, Default)]
pub struct PipeWireCapture;

impl PipeWireCapture {
    /// Create the backend. No PipeWire objects are created until an operation
    /// runs.
    pub fn new() -> Self {
        Self
    }
}

impl CaptureBackend for PipeWireCapture {
    fn endpoints(&self) -> Result<Vec<Endpoint>> {
        Ok(enumerate_sinks()?.iter().map(SinkInfo::endpoint).collect())
    }

    fn default_endpoint(&self) -> Result<Option<Endpoint>> {
        let sinks = enumerate_sinks()?;
        Ok(pick_default(&sinks).map(SinkInfo::endpoint))
    }

    fn start(&self, config: &CaptureConfig, sink: FrameSink) -> Result<CaptureSession> {
        start_capture(config, sink)
    }
}

/// Enumerate sinks on a dedicated thread. All PipeWire objects live and die
/// there; the caller only receives plain data.
fn enumerate_sinks() -> Result<Vec<SinkInfo>> {
    let handle = thread::Builder::new()
        .name("audio-share-pipewire-enumerate".to_owned())
        .spawn(enumerate_sinks_thread)
        .map_err(|error| pipewire_error("spawn PipeWire enumeration thread", error))?;
    match handle.join() {
        Ok(result) => result,
        Err(_) => Err(Error::Capture(
            "PipeWire enumeration thread panicked".to_owned(),
        )),
    }
}

fn enumerate_sinks_thread() -> Result<Vec<SinkInfo>> {
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None)
        .map_err(|error| pipewire_error("create PipeWire main loop", error))?;
    let context = pw::context::ContextRc::new(&main_loop, None)
        .map_err(|error| pipewire_error("create PipeWire context", error))?;
    let core = context.connect_rc(None).map_err(connect_error)?;
    let registry = core
        .get_registry_rc()
        .map_err(|error| pipewire_error("get PipeWire registry", error))?;

    let sinks: Rc<RefCell<Vec<SinkInfo>>> = Rc::new(RefCell::new(Vec::new()));
    let pending = core
        .sync(0)
        .map_err(|error| pipewire_error("request PipeWire core sync", error))?;

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
            let sinks = Rc::clone(&sinks);
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
                sinks.borrow_mut().push(SinkInfo {
                    registry_id: object.id,
                    serial: serial.to_owned(),
                    node_name: props.get(*pw::keys::NODE_NAME).map(str::to_owned),
                    node_nick: props.get(*pw::keys::NODE_NICK).map(str::to_owned),
                    node_description: props.get(*pw::keys::NODE_DESCRIPTION).map(str::to_owned),
                    priority_session,
                });
            }
        })
        .register();

    // Registry globals arrive before the sync roundtrip completes.
    main_loop.run();
    let result = sinks.borrow().clone();
    Ok(result)
}

/// Start one capture session and wait (bounded) for format negotiation.
fn start_capture(config: &CaptureConfig, sink: FrameSink) -> Result<CaptureSession> {
    let signal = Arc::new(StopSignal::default());
    let finished = Arc::new(AtomicBool::new(false));
    let (stop_tx, stop_rx) = pw::channel::channel::<()>();
    let (ready_tx, ready_rx) =
        mpsc::sync_channel::<std::result::Result<NegotiatedFormat, String>>(1);

    let config = config.clone();
    let worker_abort = stop_tx.clone();
    let worker_finished = Arc::clone(&finished);
    let capture_thread = thread::Builder::new()
        .name("audio-share-pipewire-capture".to_owned())
        .spawn(move || {
            let error_tx = ready_tx.clone();
            let result = capture_worker_inner(
                &config,
                sink,
                stop_rx,
                worker_abort,
                ready_tx,
                worker_finished,
            );
            if let Err(error) = &result {
                let _ = error_tx.try_send(Err(error.to_string()));
                eprintln!("audio-share: PipeWire capture failed: {error}");
            }
        })
        .map_err(|error| pipewire_error("spawn PipeWire capture thread", error))?;

    // The core `CaptureSession` stop signal is a condvar, not a PipeWire
    // wake-up. A tiny watcher turns it into the channel message the loop
    // thread listens for.
    let watch_signal = Arc::clone(&signal);
    let watch_finished = Arc::clone(&finished);
    let fallback_stop = stop_tx.clone();
    let watcher = thread::Builder::new()
        .name("audio-share-pipewire-stop".to_owned())
        .spawn(move || loop {
            if watch_finished.load(Ordering::Relaxed) {
                return;
            }
            if !watch_signal.wait(STOP_WATCH_INTERVAL) {
                let _ = stop_tx.send(());
                return;
            }
        });
    if let Err(error) = watcher {
        let _ = fallback_stop.send(());
        let _ = capture_thread.join();
        return Err(pipewire_error("spawn PipeWire stop watcher", error));
    }

    match ready_rx.recv_timeout(FORMAT_TIMEOUT) {
        Ok(Ok(negotiated)) => {
            let Some(format) = negotiated.protocol_format() else {
                signal.stop();
                let _ = capture_thread.join();
                return Err(Error::Capture(format!(
                    "PipeWire negotiated an unsupported AudioFormat {:?} (rate={}, channels={})",
                    negotiated.format, negotiated.rate, negotiated.channels
                )));
            };
            Ok(CaptureSession {
                format,
                signal,
                thread: Some(capture_thread),
            })
        }
        Ok(Err(message)) => {
            signal.stop();
            let _ = capture_thread.join();
            Err(Error::Capture(message))
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            signal.stop();
            let _ = capture_thread.join();
            Err(Error::Capture(format!(
                "PipeWire did not negotiate an AudioFormat within {}s; is a session manager \
                 (wireplumber) running and the target sink present?",
                FORMAT_TIMEOUT.as_secs()
            )))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            signal.stop();
            let _ = capture_thread.join();
            Err(Error::Capture(
                "PipeWire capture thread exited before reporting a format".to_owned(),
            ))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn capture_worker_inner(
    config: &CaptureConfig,
    sink: FrameSink,
    stop_rx: pw::channel::Receiver<()>,
    abort_tx: pw::channel::Sender<()>,
    ready_tx: mpsc::SyncSender<std::result::Result<NegotiatedFormat, String>>,
    finished: Arc<AtomicBool>,
) -> Result<()> {
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None)
        .map_err(|error| pipewire_error("create PipeWire main loop", error))?;
    let context = pw::context::ContextRc::new(&main_loop, None)
        .map_err(|error| pipewire_error("create PipeWire context", error))?;
    let core = context.connect_rc(None).map_err(connect_error)?;
    let registry = core
        .get_registry_rc()
        .map_err(|error| pipewire_error("get PipeWire registry", error))?;

    // Enumerate on this connection so the target is validated against the
    // same registry view that will serve the stream.
    let sinks: Rc<RefCell<Vec<SinkInfo>>> = Rc::new(RefCell::new(Vec::new()));
    let pending = core
        .sync(0)
        .map_err(|error| pipewire_error("request PipeWire core sync", error))?;
    let main_loop_weak = main_loop.downgrade();
    let enumerate_done = core
        .add_listener_local()
        .done(move |id, seq| {
            if id == pw::core::PW_ID_CORE && seq == pending {
                if let Some(main_loop) = main_loop_weak.upgrade() {
                    main_loop.quit();
                }
            }
        })
        .register();
    let enumerate_registry = registry
        .add_listener_local()
        .global({
            let sinks = Rc::clone(&sinks);
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
                sinks.borrow_mut().push(SinkInfo {
                    registry_id: object.id,
                    serial: serial.to_owned(),
                    node_name: props.get(*pw::keys::NODE_NAME).map(str::to_owned),
                    node_nick: props.get(*pw::keys::NODE_NICK).map(str::to_owned),
                    node_description: props.get(*pw::keys::NODE_DESCRIPTION).map(str::to_owned),
                    priority_session,
                });
            }
        })
        .register();
    main_loop.run();
    let sinks = sinks.borrow().clone();
    drop(enumerate_done);
    drop(enumerate_registry);

    let target = select_sink(&config.endpoint_id, &sinks)?;
    let target_serial = target.serial.clone();
    let target_registry_id = target.registry_id;

    let shared = Arc::new(SharedState::default());
    let started = Instant::now();
    let finish_guard = FinishGuard(Arc::clone(&finished));

    let ring = HeapRb::<u8>::new(PCM_RING_BYTES);
    let (producer, consumer) = ring.split();

    let props = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::APP_NAME => "Audio Share Server",
        *pw::keys::NODE_NAME => "audio-share-server",
        *pw::keys::TARGET_OBJECT => target_serial.as_str(),
        // Capturing an Audio/Sink's monitor; harmless when TARGET_OBJECT
        // already names the sink.
        *pw::keys::STREAM_CAPTURE_SINK => "true",
    };
    let stream = pw::stream::StreamRc::new(core.clone(), "audio-share-server", props)
        .map_err(|error| pipewire_error("create PipeWire capture stream", error))?;

    let forwarder_state = Arc::clone(&shared);
    let forwarder_finished = Arc::clone(&finished);
    let forwarder = thread::Builder::new()
        .name("audio-share-pipewire-forward".to_owned())
        .spawn(move || forward_pcm(consumer, sink, forwarder_state, forwarder_finished))
        .map_err(|error| pipewire_error("spawn PipeWire PCM forwarder", error))?;

    let watchdog_state = Arc::clone(&shared);
    let watchdog_finished = Arc::clone(&finished);
    let watchdog_abort = abort_tx.clone();
    let watchdog = thread::Builder::new()
        .name("audio-share-pipewire-watchdog".to_owned())
        .spawn(move || {
            loop {
                if watchdog_finished.load(Ordering::Relaxed) {
                    return;
                }
                if watchdog_state.streaming.load(Ordering::Relaxed) {
                    let last = watchdog_state.last_callback_ns.load(Ordering::Relaxed);
                    // A sink that never produced a callback (idle graph) is
                    // not treated as lost; only a stream that stalls after
                    // delivering audio is.
                    if last != 0 {
                        let now = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
                        if now.saturating_sub(last) > STALL_TIMEOUT.as_nanos() as u64 {
                            watchdog_state.set_stop_reason(format!(
                                "no audio callbacks for {}s; the capture target may have disappeared",
                                STALL_TIMEOUT.as_secs()
                            ));
                            let _ = watchdog_abort.send(());
                            return;
                        }
                    }
                }
                thread::sleep(WATCHDOG_INTERVAL);
            }
        })
        .map_err(|error| pipewire_error("spawn PipeWire liveness watchdog", error))?;

    let abort_for_state = abort_tx.clone();
    let abort_for_params = abort_tx.clone();
    let ready_for_format = ready_tx.clone();
    let user_data = CaptureUserData {
        state: Arc::clone(&shared),
        producer,
        started,
    };

    let stream_listener = stream
        .add_local_listener_with_user_data(user_data)
        .state_changed(move |_stream, user_data, _old, new| match &new {
            pw::stream::StreamState::Streaming => {
                user_data.state.streaming.store(true, Ordering::Relaxed);
            }
            pw::stream::StreamState::Error(message) => {
                user_data
                    .state
                    .set_stop_reason(format!("PipeWire stream error: {message}"));
                let _ = abort_for_state.send(());
            }
            _ => {}
        })
        .param_changed(move |_stream, user_data, id, param| {
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
            let Some(format) = negotiated.protocol_format() else {
                let _ = ready_for_format.try_send(Err(format!(
                    "PipeWire negotiated an unsupported format {:?} (rate={}, channels={})",
                    negotiated.format, negotiated.rate, negotiated.channels
                )));
                user_data
                    .state
                    .set_stop_reason("unsupported PipeWire capture format");
                let _ = abort_for_params.send(());
                return;
            };

            let mut slot = user_data
                .state
                .format
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match *slot {
                None => {
                    user_data
                        .state
                        .block_align
                        .store(format.block_align(), Ordering::Relaxed);
                    *slot = Some(negotiated);
                    drop(slot);
                    let _ = ready_for_format.try_send(Ok(negotiated));
                }
                Some(previous) if previous != negotiated => {
                    drop(slot);
                    user_data
                        .state
                        .set_stop_reason("PipeWire changed the capture format mid-session");
                    let _ = abort_for_params.send(());
                }
                Some(_) => {}
            }
        })
        .process(move |stream, user_data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            let Some(data) = datas.first_mut() else {
                return;
            };
            let offset = data.chunk().offset() as usize;
            let size = data.chunk().size() as usize;
            let Some(slice) = data.data() else {
                return;
            };
            // Update liveness even for an empty chunk: the graph is alive.
            user_data.state.last_callback_ns.store(
                user_data
                    .started
                    .elapsed()
                    .as_nanos()
                    .min(u128::from(u64::MAX)) as u64,
                Ordering::Relaxed,
            );
            if size == 0 || offset.saturating_add(size) > slice.len() {
                return;
            }
            let pcm = &slice[offset..offset + size];
            let pushed = user_data.producer.push_slice(pcm);
            if pushed < pcm.len() {
                user_data
                    .state
                    .dropped_bytes
                    .fetch_add((pcm.len() - pushed) as u64, Ordering::Relaxed);
            }
        })
        .register()
        .map_err(|error| pipewire_error("register PipeWire stream listener", error))?;

    // Stop when the target Audio/Sink disappears. The stream state does not
    // report this, which is why the registry is watched as well.
    let abort_for_remove = abort_tx.clone();
    let state_for_remove = Arc::clone(&shared);
    let remove_listener = registry
        .add_listener_local()
        .global_remove(move |id| {
            if id == target_registry_id {
                state_for_remove.set_stop_reason("the capture target Audio/Sink disappeared");
                let _ = abort_for_remove.send(());
            }
        })
        .register();

    let mut audio_info = AudioInfoRaw::new();
    audio_info.set_format(requested_spa_format(config.encoding));
    if config.channels > 0 {
        audio_info.set_channels(u32::from(config.channels));
    }
    if config.sample_rate > 0 {
        audio_info.set_rate(config.sample_rate);
    }
    let object = Object {
        type_: SpaTypes::ObjectParamFormat.as_raw(),
        id: ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let serialized = PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(object))
        .map_err(|error| pipewire_error("serialize PipeWire EnumFormat pod", error))?
        .0
        .into_inner();
    let pod = Pod::from_bytes(&serialized)
        .ok_or_else(|| Error::Capture("PipeWire produced an invalid EnumFormat pod".to_owned()))?;
    let mut params = [pod];

    stream
        .connect(
            Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut params,
        )
        .map_err(|error| pipewire_error("connect the PipeWire capture stream", error))?;

    let main_loop_weak = main_loop.downgrade();
    let stream_control = stream.clone();
    let stop_attached = stop_rx.attach(main_loop.loop_(), move |_| {
        let _ = stream_control.disconnect();
        if let Some(main_loop) = main_loop_weak.upgrade() {
            main_loop.quit();
        }
    });

    main_loop.run();

    // Drop the listener first: its user data owns the ring producer, which
    // tells the forwarder that no more PCM can arrive.
    drop(stream_listener);
    drop(stop_attached);
    drop(stream);
    drop(remove_listener);
    finished.store(true, Ordering::Relaxed);
    let _ = forwarder.join();
    let _ = watchdog.join();
    drop(finish_guard);

    let dropped = shared.dropped_bytes.load(Ordering::Relaxed);
    if dropped > 0 {
        eprintln!(
            "audio-share: dropped {dropped} PCM bytes because the frame sink was not draining"
        );
    }
    if let Some(reason) = shared.take_stop_reason() {
        return Err(Error::Capture(reason));
    }
    Ok(())
}

/// Drain the lock-free ring into the bounded [`FrameSink`].
///
/// This is the only thread that calls `try_send`, so the realtime callback
/// never touches a blocking channel. Full channels drop whole aligned chunks
/// and account for their bytes; a closed channel ends the forwarder.
fn forward_pcm(
    mut consumer: HeapCons<u8>,
    sink: FrameSink,
    state: Arc<SharedState>,
    finished: Arc<AtomicBool>,
) {
    let mut buffer = vec![0u8; FORWARD_CHUNK_BYTES];
    let mut pending: Vec<u8> = Vec::with_capacity(FORWARD_CHUNK_BYTES * 2);
    loop {
        let block_align = state.block_align.load(Ordering::Relaxed);
        if block_align == 0 {
            if finished.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(FORWARD_IDLE_SLEEP);
            continue;
        }

        loop {
            let count = consumer.pop_slice(&mut buffer);
            if count == 0 {
                break;
            }
            pending.extend_from_slice(&buffer[..count]);
        }

        let sendable = pending.len() - pending.len() % block_align;
        if sendable > 0 {
            let remainder = pending.split_off(sendable);
            let chunk = std::mem::replace(&mut pending, remainder);
            match sink.try_send(chunk) {
                Ok(()) => {}
                Err(TrySendError::Full(chunk)) => {
                    state
                        .dropped_bytes
                        .fetch_add(chunk.len() as u64, Ordering::Relaxed);
                }
                Err(TrySendError::Closed(_)) => return,
            }
        }

        if finished.load(Ordering::Relaxed) && consumer.is_empty() {
            return;
        }
        thread::sleep(FORWARD_IDLE_SLEEP);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink(
        serial: &str,
        nick: Option<&str>,
        description: Option<&str>,
        name: Option<&str>,
        priority: i64,
    ) -> SinkInfo {
        SinkInfo {
            registry_id: 1,
            serial: serial.to_owned(),
            node_name: name.map(str::to_owned),
            node_nick: nick.map(str::to_owned),
            node_description: description.map(str::to_owned),
            priority_session: priority,
        }
    }

    #[test]
    fn encoding_preferences_map_to_pipewire_formats() {
        assert_eq!(
            requested_spa_format(EncodingPreference::Default),
            SpaFormat::F32LE
        );
        assert_eq!(
            requested_spa_format(EncodingPreference::F32),
            SpaFormat::F32LE
        );
        // The old server and Android treat PCM_8BIT as unsigned.
        assert_eq!(requested_spa_format(EncodingPreference::S8), SpaFormat::U8);
        assert_eq!(
            requested_spa_format(EncodingPreference::S16),
            SpaFormat::S16LE
        );
        assert_eq!(
            requested_spa_format(EncodingPreference::S24),
            SpaFormat::S24LE
        );
        assert_eq!(
            requested_spa_format(EncodingPreference::S32),
            SpaFormat::S32LE
        );
    }

    #[test]
    fn negotiated_formats_map_back_with_protocol_block_alignment() {
        for (spa, encoding, channels, block_align) in [
            (SpaFormat::F32LE, Encoding::PcmFloat, 2, 8),
            (SpaFormat::U8, Encoding::Pcm8Bit, 1, 1),
            (SpaFormat::S16LE, Encoding::Pcm16Bit, 2, 4),
            // Packed 24-bit: three bytes per sample, not S24_32LE.
            (SpaFormat::S24LE, Encoding::Pcm24Bit, 2, 6),
            (SpaFormat::S32LE, Encoding::Pcm32Bit, 1, 4),
        ] {
            let format = negotiated_audio_format(spa, 48_000, channels)
                .unwrap_or_else(|| panic!("{spa:?} must map to a protocol format"));
            assert_eq!(
                format,
                AudioFormat::new(encoding, channels as i32, 48_000),
                "{spa:?}"
            );
            assert_eq!(format.block_align(), block_align, "{spa:?}");
        }

        assert_eq!(
            negotiated_audio_format(SpaFormat::S24_32LE, 48_000, 2),
            None,
            "S24_32LE is a 32-bit container and must not be advertised as packed PCM24"
        );
        assert_eq!(negotiated_audio_format(SpaFormat::S8, 48_000, 2), None);
        assert_eq!(negotiated_audio_format(SpaFormat::F32LE, 0, 2), None);
        assert_eq!(negotiated_audio_format(SpaFormat::F32LE, 48_000, 0), None);
    }

    #[test]
    fn default_selection_matches_old_server_priority_rules() {
        let sinks = [
            sink("10", None, None, Some("low"), 500),
            sink("11", None, None, Some("high"), 2_000),
            sink("12", None, None, Some("mid"), 1_000),
        ];
        assert_eq!(pick_default(&sinks).unwrap().serial, "11");

        // First wins ties: only a strictly greater priority replaces.
        let ties = [
            sink("30", None, None, Some("first"), 1_000),
            sink("31", None, None, Some("second"), 1_000),
        ];
        assert_eq!(pick_default(&ties).unwrap().serial, "30");

        // The default priority starts at -1, so no sink strictly above it
        // means no default at all.
        let no_default = [
            sink("20", None, None, Some("missing"), -1),
            sink("21", None, None, Some("negative"), -5),
        ];
        assert!(pick_default(&no_default).is_none());
        assert!(pick_default(&[]).is_none());

        let zero_wins = [
            sink("40", None, None, Some("missing"), -1),
            sink("41", None, None, Some("zero"), 0),
        ];
        assert_eq!(pick_default(&zero_wins).unwrap().serial, "41");
    }

    #[test]
    fn select_sink_validates_targets_against_the_registry() {
        let sinks = [
            sink("50", None, Some("Desk"), Some("desk"), 1_000),
            sink("51", None, None, Some("headset"), 500),
        ];

        assert_eq!(select_sink("default", &sinks).unwrap().serial, "50");
        assert_eq!(select_sink("", &sinks).unwrap().serial, "50");
        assert_eq!(select_sink("51", &sinks).unwrap().serial, "51");
        assert_eq!(select_sink(" default ", &sinks).unwrap().serial, "50");

        let error = select_sink("999", &sinks).unwrap_err();
        assert!(matches!(error, Error::InvalidConfig(_)), "{error}");
        assert!(error.to_string().contains("50 (Desk)"), "{error}");
        assert!(error.to_string().contains("51 (headset)"), "{error}");
    }

    #[test]
    fn display_name_falls_back_in_old_server_order() {
        let with_nick = sink("1", Some("Nick"), Some("Desc"), Some("Name"), 1);
        assert_eq!(with_nick.display_name(), "Nick");

        let without_nick = sink("2", None, Some("Desc"), Some("Name"), 1);
        assert_eq!(without_nick.display_name(), "Desc");

        let only_name = sink("3", None, None, Some("Name"), 1);
        assert_eq!(only_name.display_name(), "Name");

        let bare = sink("4", None, None, None, 1);
        assert_eq!(bare.display_name(), "4");
    }
}
