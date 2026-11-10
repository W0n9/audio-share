//! WASAPI shared-mode loopback capture backend.
//!
//! `windows` COM interfaces are `!Send`/`!Sync`, so this backend stores no COM
//! handles. Endpoint enumeration, format negotiation, and capture each run on a
//! dedicated thread that initializes a multi-threaded apartment and tears it
//! down before exiting. The capture thread negotiates the format and starts the
//! stream before reporting success, so the returned [`CaptureSession`] always
//! carries the actual format the endpoint will deliver.

use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use wasapi::{
    deinitialize, initialize_mta, AudioCaptureClient, AudioClient, DeviceEnumerator, Direction,
    SampleType, ShareMode, StreamMode, WasapiError, WaveFormat,
};
use windows::Win32::Media::Audio::{
    AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_RESOURCES_INVALIDATED, AUDCLNT_E_SERVICE_NOT_RUNNING,
    AUDCLNT_E_UNSUPPORTED_FORMAT,
};

use super::format::{
    advertised_format, requested_format, select_negotiated, silence_fill, FormatSpec, SampleSubtype,
};
use crate::capture::{
    CaptureBackend, CaptureConfig, CaptureSession, Endpoint, FrameSink, StopSignal,
};
use crate::error::{Error, Result};
use crate::protocol::AudioFormat;

/// Requested shared-mode buffer: 1 second, matching the old C++ server.
const BUFFER_DURATION_HNS: i64 = 10_000_000;

/// Map an endpoint format probe or capture failure to a readable core error.
fn capture_error(operation: &str, error: WasapiError) -> Error {
    if is_invalidated(&error) {
        Error::Capture(format!(
            "{operation}: the audio endpoint was invalidated or removed ({error})"
        ))
    } else {
        Error::Capture(format!("{operation}: {error}"))
    }
}

fn is_invalidated(error: &WasapiError) -> bool {
    matches!(error, WasapiError::Windows(windows_error)
        if windows_error.code() == AUDCLNT_E_DEVICE_INVALIDATED
            || windows_error.code() == AUDCLNT_E_RESOURCES_INVALIDATED
            || windows_error.code() == AUDCLNT_E_SERVICE_NOT_RUNNING)
}

/// Balanced `CoInitializeEx`/`CoUninitialize` for one worker thread.
///
/// `S_OK` and `S_FALSE` both require one matching `CoUninitialize`, so the
/// guard is only created after a successful call.
struct ComGuard;

impl ComGuard {
    fn initialize() -> Result<Self> {
        let hr = initialize_mta();
        if hr.is_err() {
            return Err(Error::Capture(format!(
                "CoInitializeEx(COINIT_MULTITHREADED) failed: {hr:?}"
            )));
        }
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        deinitialize();
    }
}

/// Run a COM operation on a fresh MTA thread and return its owned result.
///
/// The thread is always joined so a panic is not silently detached.
fn run_com_thread<T, F>(name: &'static str, operation: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    let (sender, receiver) = mpsc::sync_channel(1);
    let thread = thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            let _com = match ComGuard::initialize() {
                Ok(com) => com,
                Err(error) => {
                    let _ = sender.send(Err(error));
                    return;
                }
            };
            let _ = sender.send(operation());
        })
        .map_err(|error| Error::io("spawn WASAPI worker thread", error))?;

    let result = receiver.recv().map_err(|_| {
        Error::Capture("WASAPI worker thread exited without reporting a result".to_owned())
    });
    let _ = thread.join();
    result?
}

fn endpoint_of(device: &wasapi::Device) -> Result<Endpoint> {
    let id = device
        .get_id()
        .map_err(|error| capture_error("read IMMDevice ID", error))?;
    let name = device
        .get_friendlyname()
        .unwrap_or_else(|_| "<unknown>".to_owned());
    Ok(Endpoint { id, name })
}

fn enumerate_render_endpoints() -> Result<Vec<Endpoint>> {
    let enumerator = DeviceEnumerator::new()
        .map_err(|error| capture_error("create audio device enumerator", error))?;
    let collection = enumerator
        .get_device_collection(Direction::Render)
        .map_err(|error| capture_error("enumerate active render endpoints", error))?;
    let count = collection
        .get_nbr_devices()
        .map_err(|error| capture_error("read render endpoint count", error))?;
    let mut endpoints = Vec::with_capacity(count as usize);
    for index in 0..count {
        let device = collection
            .get_device_at_index(index)
            .map_err(|error| capture_error("open render endpoint", error))?;
        endpoints.push(endpoint_of(&device)?);
    }
    Ok(endpoints)
}

fn default_render_endpoint() -> Result<Endpoint> {
    let enumerator = DeviceEnumerator::new()
        .map_err(|error| capture_error("create audio device enumerator", error))?;
    let device = enumerator
        .get_default_device(Direction::Render)
        .map_err(|error| capture_error("get default render endpoint (eRender, eConsole)", error))?;
    endpoint_of(&device)
}

/// WASAPI shared-mode loopback capture backend.
///
/// The struct is stateless and holds no COM handles, so it is `Send + Sync` and
/// can be shared by the Server. Each [`CaptureBackend::start`] creates a fresh
/// endpoint, client, and capture thread, which mirrors the old C++ server and
/// avoids `AUDCLNT_E_ALREADY_INITIALIZED` plus stale buffered audio across
/// repeated start/stop and endpoint switches.
#[derive(Clone, Copy, Debug, Default)]
pub struct WasapiCapture;

impl WasapiCapture {
    /// Create the Windows capture backend. Creating it does not touch COM.
    pub const fn new() -> Self {
        Self
    }
}

impl CaptureBackend for WasapiCapture {
    fn endpoints(&self) -> Result<Vec<Endpoint>> {
        run_com_thread("audio-share-wasapi-endpoints", enumerate_render_endpoints)
    }

    fn default_endpoint(&self) -> Result<Option<Endpoint>> {
        run_com_thread("audio-share-wasapi-default", || {
            default_render_endpoint().map(Some)
        })
    }

    fn start(&self, config: &CaptureConfig, sink: FrameSink) -> Result<CaptureSession> {
        let signal = Arc::new(StopSignal::default());
        let thread_signal = Arc::clone(&signal);
        let thread_config = config.clone();
        let (ready_sender, ready_receiver) =
            mpsc::sync_channel::<std::result::Result<AudioFormat, Error>>(1);

        let thread = thread::Builder::new()
            .name("audio-share-wasapi-capture".to_owned())
            .spawn(move || {
                capture_thread(&thread_config, &sink, &thread_signal, &ready_sender);
            })
            .map_err(|error| Error::io("spawn WASAPI capture thread", error))?;

        match ready_receiver.recv() {
            Ok(Ok(format)) => Ok(CaptureSession {
                format,
                signal,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(Error::Capture(
                    "WASAPI capture thread exited before reporting a format".to_owned(),
                ))
            }
        }
    }
}

/// Entry point for the capture thread: set up, report the negotiated format,
/// then run until stopped.
fn capture_thread(
    config: &CaptureConfig,
    sink: &FrameSink,
    stop: &StopSignal,
    ready: &SyncSender<std::result::Result<AudioFormat, Error>>,
) {
    let _com = match ComGuard::initialize() {
        Ok(com) => com,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };

    let mut session = match open_loopback_session(config) {
        Ok(session) => session,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };

    // The format is published only after Start() succeeded, so a returned
    // CaptureSession can never advertise a stream that failed to start.
    if ready.send(Ok(session.format.clone())).is_err() {
        // The caller disappeared (for example during shutdown); stop the stream.
        let _ = session.client.stop_stream();
        return;
    }

    let loop_result = capture_loop(&mut session, sink, stop);
    let stop_result = match session.client.stop_stream() {
        Ok(()) => Ok(()),
        // Device invalidation during teardown is expected and must not mask the
        // original capture result.
        Err(error) if is_invalidated(&error) => Ok(()),
        Err(error) => Err(capture_error("stop loopback capture stream", error)),
    };
    if let Err(error) = loop_result.and(stop_result) {
        // The session is already running; there is no error channel back to the
        // Server yet, so report the restartable failure and let the owner
        // restart capture.
        eprintln!("WASAPI capture stopped: {error}");
    }
}

/// A live shared-mode loopback stream. All COM objects live on the capture
/// thread that owns this value.
struct LoopbackSession {
    client: AudioClient,
    capture: AudioCaptureClient,
    format: AudioFormat,
    block_align: usize,
    buffer: Vec<u8>,
    poll: Duration,
}

fn open_loopback_session(config: &CaptureConfig) -> Result<LoopbackSession> {
    let enumerator = DeviceEnumerator::new()
        .map_err(|error| capture_error("create audio device enumerator", error))?;
    let device = if config.endpoint_id.is_empty() || config.endpoint_id == "default" {
        enumerator
            .get_default_device(Direction::Render)
            .map_err(|error| capture_error("open default render endpoint", error))?
    } else {
        enumerator
            .get_device(&config.endpoint_id)
            .map_err(|error| capture_error("open render endpoint", error))?
    };

    // A capture endpoint initialized with Direction::Capture would silently
    // become a microphone stream, unlike the old C++ server's
    // AUDCLNT_E_WRONG_ENDPOINT_TYPE. Refuse it explicitly.
    if device.get_direction() != Direction::Render {
        let id = device
            .get_id()
            .unwrap_or_else(|_| config.endpoint_id.clone());
        return Err(Error::Capture(format!(
            "endpoint {id:?} is not a render endpoint; refusing to capture (WASAPI \
             would silently create a microphone stream)"
        )));
    }

    let mut client = device
        .get_iaudioclient()
        .map_err(|error| capture_error("activate IAudioClient", error))?;
    let mix = client
        .get_mixformat()
        .map_err(|error| capture_error("get endpoint mix format (GetMixFormat)", error))?;
    let mix_spec = spec_from_waveformat(&mix)?;

    let requested_spec = requested_format(
        mix_spec,
        config.encoding,
        config.channels,
        config.sample_rate,
    );
    if requested_spec.channels == 0
        || requested_spec.sample_rate == 0
        || requested_spec.block_align == 0
    {
        return Err(Error::InvalidConfig(format!(
            "requested capture format {requested_spec:?} is unusable"
        )));
    }
    let requested = waveformat_from_spec(&requested_spec)?;

    let closest = client
        .is_supported(&requested, ShareMode::Shared)
        .map_err(|error| map_is_supported_error(&requested_spec, error))?;
    let closest_spec = closest.as_ref().map(spec_from_waveformat).transpose()?;
    let negotiated = select_negotiated(requested_spec, closest_spec);
    let advertised = advertised_format(&negotiated.format).map_err(|error| {
        Error::Capture(format!(
            "negotiated WASAPI format cannot be advertised to the v0.3.4 client: {error}"
        ))
    })?;

    let negotiated_waveformat = closest.unwrap_or(requested);
    let (_default_period_hns, min_period_hns) = client
        .get_device_period()
        .map_err(|error| capture_error("get device period", error))?;

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
        .map_err(|error| map_initialize_error(&negotiated.format, error))?;

    let buffer_frames = client
        .get_buffer_size()
        .map_err(|error| capture_error("get loopback buffer size", error))?;
    let capture = client
        .get_audiocaptureclient()
        .map_err(|error| capture_error("get IAudioCaptureClient", error))?;
    let block_align = negotiated.format.block_align as usize;
    if block_align == 0 || buffer_frames == 0 {
        return Err(Error::Capture(format!(
            "invalid negotiated stream: block_align={block_align}, buffer_frames={buffer_frames}"
        )));
    }
    let buffer = vec![0u8; buffer_frames as usize * block_align];

    client
        .start_stream()
        .map_err(|error| capture_error("start loopback capture stream", error))?;

    Ok(LoopbackSession {
        client,
        capture,
        format: advertised,
        block_align,
        buffer,
        poll: hns_to_duration(min_period_hns),
    })
}

fn capture_loop(session: &mut LoopbackSession, sink: &FrameSink, stop: &StopSignal) -> Result<()> {
    loop {
        // Poll at the device minimum period, exactly like the old C++ server.
        // The StopSignal wait wakes immediately when `stop()` is called.
        if !stop.wait(session.poll) {
            break;
        }
        loop {
            let frames = match session.capture.get_next_packet_size() {
                Ok(None) | Ok(Some(0)) => break,
                Ok(Some(frames)) => frames,
                Err(error) => {
                    return Err(capture_error("GetNextPacketSize failed", error));
                }
            };
            let want = (frames as usize)
                .checked_mul(session.block_align)
                .ok_or_else(|| Error::Capture("capture packet size overflowed".to_owned()))?;
            if want == 0 {
                continue;
            }
            if want > session.buffer.len() {
                // A packet never exceeds GetBufferSize(), but stay defensive.
                session.buffer.resize(want, 0);
            }
            // `read_from_device` performs GetBuffer/ReleaseBuffer, zero-fills
            // silent packets, and reports the packet flags. `data_discontinuity`
            // means frames were lost before this packet; the packet itself is
            // still valid and the frozen Wire Protocol has no counter for it, so
            // forward it unchanged.
            let (got, info) = session
                .capture
                .read_from_device(&mut session.buffer[..want])
                .map_err(|error| capture_error("IAudioCaptureClient::GetBuffer failed", error))?;
            let bytes = (got as usize)
                .checked_mul(session.block_align)
                .ok_or_else(|| Error::Capture("captured frame count overflowed".to_owned()))?;
            if bytes == 0 {
                continue;
            }
            if info.flags.silent {
                if let Some(encoding) = session.format.encoding_kind() {
                    // 8-bit PCM is unsigned; fix up the crate's zero fill.
                    session.buffer[..bytes].fill(silence_fill(encoding));
                }
            }
            // The Server's broadcast task always drains this bounded channel.
            // A send error means the receiver is gone, i.e. the Server is
            // stopping; exit quietly rather than reporting a capture failure.
            if sink
                .blocking_send(session.buffer[..bytes].to_vec())
                .is_err()
            {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn spec_from_waveformat(format: &WaveFormat) -> Result<FormatSpec> {
    let subtype = match format.get_subformat() {
        Ok(SampleType::Float) => SampleSubtype::Float,
        Ok(SampleType::Int) => SampleSubtype::Int,
        // Keep the spec so `advertised_format` can produce the typed error.
        Err(WasapiError::UnsupportedSubformat(_)) => SampleSubtype::Other,
        Err(error) => return Err(capture_error("read format subformat", error)),
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
        SampleSubtype::Other => {
            return Err(Error::Capture(
                "cannot request a format with an unknown sample subtype".to_owned(),
            ));
        }
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

fn map_is_supported_error(requested: &FormatSpec, error: WasapiError) -> Error {
    match &error {
        WasapiError::Windows(windows_error)
            if windows_error.code() == AUDCLNT_E_UNSUPPORTED_FORMAT =>
        {
            Error::Capture(format!(
                "requested format {requested:?} is not supported by the endpoint \
                 (AUDCLNT_E_UNSUPPORTED_FORMAT); try another encoding, channel count, \
                 or sample rate"
            ))
        }
        WasapiError::UnsupportedFormat => Error::Capture(format!(
            "requested format {requested:?} is not supported by the endpoint; try \
             another encoding, channel count, or sample rate"
        )),
        _ => capture_error("IsFormatSupported failed", error),
    }
}

/// Map an `IAudioClient::Initialize` failure to a readable error.
///
/// `IsFormatSupported` can return `S_OK` for a format that shared-mode
/// loopback still rejects at `Initialize` (observed on Realtek for mono at the
/// mix sample rate). Without this mapping the user only sees the raw HRESULT
/// `0x88890008`.
fn map_initialize_error(negotiated: &FormatSpec, error: WasapiError) -> Error {
    match &error {
        WasapiError::Windows(windows_error)
            if windows_error.code() == AUDCLNT_E_UNSUPPORTED_FORMAT =>
        {
            Error::Capture(format!(
                "the endpoint rejected the negotiated loopback format {negotiated:?} \
                 (AUDCLNT_E_UNSUPPORTED_FORMAT); shared-mode loopback may require the \
                 endpoint's mix format — try another encoding, channel count, or sample rate"
            ))
        }
        _ => capture_error("initialize shared-mode loopback (1 s buffer)", error),
    }
}

fn hns_to_duration(hns: i64) -> Duration {
    // 1 hns = 100 ns.
    Duration::from_nanos((hns.max(1) as u64).saturating_mul(100))
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::E_FAIL;

    fn mono_s16() -> FormatSpec {
        FormatSpec {
            subtype: SampleSubtype::Int,
            container_bits: 16,
            valid_bits: 16,
            channels: 1,
            sample_rate: 48_000,
            block_align: 2,
            channel_mask: None,
        }
    }

    #[test]
    fn initialize_unsupported_format_is_mapped_to_a_readable_error() {
        let error = map_initialize_error(
            &mono_s16(),
            WasapiError::Windows(windows::core::Error::from_hresult(
                AUDCLNT_E_UNSUPPORTED_FORMAT,
            )),
        );
        let message = error.to_string();
        assert!(
            message.contains("AUDCLNT_E_UNSUPPORTED_FORMAT"),
            "message was {message}"
        );
        assert!(message.contains("mix format"), "message was {message}");
        assert!(
            message.contains("try another encoding"),
            "message was {message}"
        );
    }

    #[test]
    fn other_initialize_errors_keep_the_operation_context() {
        let error = map_initialize_error(
            &mono_s16(),
            WasapiError::Windows(windows::core::Error::from_hresult(E_FAIL)),
        );
        let message = error.to_string();
        assert!(
            message.contains("initialize shared-mode loopback"),
            "message was {message}"
        );
    }
}
