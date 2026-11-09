#![cfg(target_os = "linux")]
//! Real headless PipeWire capture through the public `CaptureBackend` seam.
//!
//! The ignored test starts a disposable PipeWire + WirePlumber session with a
//! null `Audio/Sink`, loops a deterministic 440 Hz tone into it, then runs this
//! same test binary inside that session to exercise enumeration, default
//! selection, F32/s16/s24/s32 monitor capture, repeated start/stop, sink
//! switching, and invalid-target rejection.
//!
//! ```text
//! cargo test -p audio-share-core --test capture_linux -- --ignored --nocapture
//! ```
//!
//! CI runs without a PipeWire daemon, so the test is `#[ignore]`d and the
//! outer driver also skips itself with a message when PipeWire is not
//! installed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use audio_share_core::capture::{
    platform_backend, CaptureBackend, CaptureConfig, EncodingPreference,
};
use audio_share_core::protocol::{AudioFormat, Encoding};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;

/// Set for the inner invocation, which runs inside the headless session.
const HEADLESS_MARKER: &str = "AUDIO_SHARE_PW_HEADLESS_TEST";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

/// Deterministic 16-bit stereo 48 kHz 440 Hz WAV, looped by the driver script.
fn write_tone_wav(path: &Path) {
    const SAMPLE_RATE: u32 = 48_000;
    const CHANNELS: u16 = 2;
    const SECONDS: u32 = 30;
    let frames = SAMPLE_RATE * SECONDS;
    let mut pcm = Vec::with_capacity(frames as usize * 2 * 2);
    for index in 0..frames {
        let value = (0.25
            * (2.0 * std::f64::consts::PI * 440.0 * f64::from(index) / f64::from(SAMPLE_RATE))
                .sin()
            * 32_767.0) as i16;
        for _ in 0..CHANNELS {
            pcm.extend_from_slice(&value.to_le_bytes());
        }
    }

    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&CHANNELS.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * u32::from(CHANNELS) * 2).to_le_bytes());
    wav.extend_from_slice(&(CHANNELS * 2).to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(&pcm);
    fs::write(path, wav).expect("write tone WAV");
}

fn run_headless_session() {
    if Command::new("pipewire").arg("--version").output().is_err() {
        eprintln!("skipping: pipewire is not installed");
        return;
    }
    if Command::new("dbus-run-session")
        .arg("--help")
        .output()
        .is_err()
    {
        eprintln!("skipping: dbus-run-session is not installed");
        return;
    }

    let temp = tempfile::tempdir().expect("temp dir");
    let tone = temp.path().join("tone.wav");
    write_tone_wav(&tone);

    // A second, lower-priority sink proves that switching between sinks works
    // while the default selection stays on TestSink (priority 1000 > 500).
    let inner = temp.path().join("run.sh");
    fs::write(
        &inner,
        r#"set -euo pipefail
pw-cli create-node adapter '{ factory.name=support.null-audio-sink node.name=TestSinkLow node.description=TestSinkLow media.class=Audio/Sink object.linger=1 audio.position=[FL,FR] priority.session=500 }' >/dev/null
pw-cli create-node adapter '{ factory.name=support.null-audio-sink node.name=TestSinkGone node.description=TestSinkGone media.class=Audio/Sink object.linger=1 audio.position=[FL,FR] priority.session=100 }' >/dev/null
for _ in $(seq 1 100); do
    pw-cli ls Node 2>/dev/null | grep -q 'node.name = "TestSinkLow"' && break
    sleep 0.05
done
exec timeout 180s "$CAPTURE_TEST_EXE" --ignored --exact linux_pipewire_capture_end_to_end --nocapture
"#,
    )
    .expect("write inner session script");

    let output = Command::new("bash")
        .arg(repo_root().join("scripts/pw-test-session.sh"))
        .arg("--tone-wav")
        .arg(&tone)
        .arg("--")
        .arg("bash")
        .arg(&inner)
        .env(HEADLESS_MARKER, "1")
        .env(
            "CAPTURE_TEST_EXE",
            std::env::current_exe().expect("test exe"),
        )
        .output()
        .expect("run headless PipeWire driver");
    assert!(
        output.status.success(),
        "headless PipeWire capture test failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Everything the integration test observes from one capture session.
#[derive(Debug)]
struct Captured {
    format: AudioFormat,
    bytes: Vec<u8>,
    chunks: usize,
}

fn capture(
    backend: &Arc<dyn CaptureBackend>,
    endpoint: &str,
    encoding: EncodingPreference,
    duration: Duration,
) -> Captured {
    let (sink, mut frames) = mpsc::channel(64);
    let config = CaptureConfig {
        endpoint_id: endpoint.to_owned(),
        encoding,
        channels: 2,
        sample_rate: 48_000,
    };
    let session = backend
        .start(&config, sink)
        .unwrap_or_else(|error| panic!("start {encoding:?} capture from {endpoint}: {error}"));
    let format = session.format.clone();

    let deadline = Instant::now() + duration;
    let mut bytes = Vec::new();
    let mut chunks = 0usize;
    while Instant::now() < deadline {
        match frames.try_recv() {
            Ok(frame) => {
                bytes.extend_from_slice(&frame);
                chunks += 1;
            }
            Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(5)),
            Err(TryRecvError::Disconnected) => break,
        }
    }
    session.stop().expect("stop capture session");
    Captured {
        format,
        bytes,
        chunks,
    }
}

fn start_stop_cycles(backend: &Arc<dyn CaptureBackend>, endpoint: &str, cycles: usize) {
    for cycle in 0..cycles {
        let (sink, mut frames) = mpsc::channel(8);
        let config = CaptureConfig {
            endpoint_id: endpoint.to_owned(),
            encoding: EncodingPreference::F32,
            channels: 2,
            sample_rate: 48_000,
        };
        let session = backend
            .start(&config, sink)
            .unwrap_or_else(|error| panic!("cycle {cycle}: start failed: {error}"));
        assert_eq!(
            session.format,
            AudioFormat::new(Encoding::PcmFloat, 2, 48_000),
            "cycle {cycle}"
        );
        if cycle % 2 == 0 {
            // Wait for at least one frame, then stop while streaming.
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut got_frame = false;
            while Instant::now() < deadline {
                match frames.try_recv() {
                    Ok(_) => {
                        got_frame = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(5)),
                    Err(TryRecvError::Disconnected) => break,
                }
            }
            assert!(got_frame, "cycle {cycle}: no frame before stop");
        }
        // Odd cycles stop immediately (during connect / while streaming).
        session
            .stop()
            .unwrap_or_else(|error| panic!("cycle {cycle}: stop failed: {error}"));
    }
}

/// Find the registry/global id of a node by `node.name` (`pw-dump` JSON).
fn node_registry_id(name: &str) -> u64 {
    let output = Command::new("pw-dump").output().expect("run pw-dump");
    let objects: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).expect("parse pw-dump JSON");
    objects
        .iter()
        .find(|object| object["info"]["props"]["node.name"].as_str() == Some(name))
        .and_then(|object| object["id"].as_u64())
        .unwrap_or_else(|| panic!("node {name} not found in pw-dump"))
}

/// Destroying the target sink mid-stream must end the capture session.
///
/// PipeWire does not report target loss through `StreamState`; the registry
/// `global_remove` watch (with the callback-liveness watchdog as a fallback)
/// must close the frame channel instead of leaving a silent session running.
fn target_disappearance_stops_session(backend: &Arc<dyn CaptureBackend>) {
    let endpoints = backend.endpoints().expect("enumerate before removal test");
    let gone = endpoints
        .iter()
        .find(|endpoint| endpoint.name == "TestSinkGone")
        .unwrap_or_else(|| panic!("TestSinkGone not enumerated: {endpoints:?}"))
        .clone();
    let tone = std::env::var("PW_TEST_TONE_WAV").expect("PW_TEST_TONE_WAV");

    // Drive the sink so its monitor is scheduled and produces callbacks.
    let mut player = Command::new("pw-play")
        .args(["--target", "TestSinkGone", &tone])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn pw-play for TestSinkGone");
    std::thread::sleep(Duration::from_millis(500));

    let (sink, mut frames) = mpsc::channel(64);
    let session = backend
        .start(
            &CaptureConfig {
                endpoint_id: gone.id.clone(),
                encoding: EncodingPreference::Default,
                channels: 2,
                sample_rate: 48_000,
            },
            sink,
        )
        .expect("start capture on TestSinkGone");
    assert_eq!(
        session.format,
        AudioFormat::new(Encoding::PcmFloat, 2, 48_000)
    );

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut got_frame = false;
    while Instant::now() < deadline {
        match frames.try_recv() {
            Ok(_) => {
                got_frame = true;
                break;
            }
            Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(5)),
            Err(TryRecvError::Disconnected) => break,
        }
    }
    assert!(got_frame, "no frames from TestSinkGone before removal");

    let id = node_registry_id("TestSinkGone");
    let status = Command::new("pw-cli")
        .args(["destroy".to_owned(), id.to_string()])
        .status()
        .expect("run pw-cli destroy");
    assert!(status.success(), "pw-cli destroy failed");

    let deadline = Instant::now() + Duration::from_secs(8);
    let mut ended = false;
    while Instant::now() < deadline {
        match frames.try_recv() {
            Ok(_) => {}
            Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(20)),
            Err(TryRecvError::Disconnected) => {
                ended = true;
                break;
            }
        }
    }
    session.stop().expect("stop after target removal");
    let _ = player.kill();
    let _ = player.wait();
    assert!(
        ended,
        "capture session did not end after the target Audio/Sink disappeared"
    );
}

fn decode_mono_f32(bytes: &[u8], encoding: Encoding, channels: usize) -> Vec<f32> {
    let channels = channels.max(1);
    let bytes_per_sample = encoding.bytes_per_sample().expect("known encoding");
    let frame_size = bytes_per_sample * channels;
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

fn goertzel_amplitude(samples: &[f32], sample_rate: u32, frequency: f32) -> f32 {
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

fn dominant_frequency(samples: &[f32], sample_rate: u32, min_hz: f32, max_hz: f32) -> f32 {
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

/// Assert non-empty, block-aligned PCM containing the 440 Hz tone.
fn assert_tone(captured: &Captured, expected: Encoding, expected_block_align: usize) {
    assert_eq!(
        captured.format.encoding_kind(),
        Some(expected),
        "{captured:?}"
    );
    assert_eq!(captured.format.channels, 2);
    assert_eq!(captured.format.sample_rate, 48_000);
    assert_eq!(captured.format.block_align(), expected_block_align);
    assert!(captured.chunks > 0, "{expected:?}: no frames arrived");
    assert!(!captured.bytes.is_empty(), "{expected:?}: PCM is empty");
    assert_eq!(
        captured.bytes.len() % expected_block_align,
        0,
        "{expected:?}: PCM length {} is not block aligned",
        captured.bytes.len()
    );
    assert!(
        captured.bytes.len() > 48_000 * expected_block_align / 2,
        "{expected:?}: only {} bytes captured, expected more than half a second",
        captured.bytes.len()
    );

    let mono = decode_mono_f32(&captured.bytes, expected, 2);
    let detected = dominant_frequency(&mono, 48_000, 300.0, 600.0);
    assert!(
        (detected - 440.0).abs() <= 3.0,
        "{expected:?}: detected {detected} Hz instead of 440 Hz"
    );
    let at_440 = goertzel_amplitude(&mono, 48_000, 440.0);
    let at_1000 = goertzel_amplitude(&mono, 48_000, 1_000.0);
    assert!(at_440 > 0.01, "{expected:?}: 440 Hz amplitude {at_440}");
    assert!(
        at_1000 < at_440 / 10.0,
        "{expected:?}: 1000 Hz amplitude {at_1000} is not far below {at_440}"
    );
}

fn run_against_headless_session() {
    let backend = platform_backend().expect("create the platform capture backend");
    let endpoints = backend.endpoints().expect("enumerate PipeWire sinks");
    let test_sink = endpoints
        .iter()
        .find(|endpoint| endpoint.name == "TestSink")
        .unwrap_or_else(|| panic!("TestSink not enumerated: {endpoints:?}"))
        .clone();
    let low_sink = endpoints
        .iter()
        .find(|endpoint| endpoint.name == "TestSinkLow")
        .unwrap_or_else(|| panic!("TestSinkLow not enumerated: {endpoints:?}"))
        .clone();

    let serial = std::env::var("PW_TEST_ENDPOINT_SERIAL").expect("PW_TEST_ENDPOINT_SERIAL");
    assert_eq!(
        test_sink.id, serial,
        "the endpoint id must be object.serial, not the registry id"
    );

    let default = backend
        .default_endpoint()
        .expect("read the default endpoint")
        .expect("TestSink has priority.session 1000");
    assert_eq!(
        default.id, test_sink.id,
        "default must be the highest priority"
    );
    assert!(
        endpoints.len() >= 2,
        "expected both test sinks: {endpoints:?}"
    );

    // An invalid target must be rejected instead of silently falling back.
    let (sink, _frames) = mpsc::channel(1);
    let invalid = backend.start(
        &CaptureConfig {
            endpoint_id: "not-a-real-sink".to_owned(),
            ..CaptureConfig::default()
        },
        sink,
    );
    assert!(
        invalid.is_err(),
        "an unknown endpoint must not start a fallback capture"
    );

    // F32 is the default encoding.
    let f32_captured = capture(
        &backend,
        &test_sink.id,
        EncodingPreference::Default,
        Duration::from_secs(2),
    );
    assert_tone(&f32_captured, Encoding::PcmFloat, 8);

    let s16_captured = capture(
        &backend,
        &test_sink.id,
        EncodingPreference::S16,
        Duration::from_millis(1_500),
    );
    assert_tone(&s16_captured, Encoding::Pcm16Bit, 4);

    let s24_captured = capture(
        &backend,
        &test_sink.id,
        EncodingPreference::S24,
        Duration::from_millis(1_500),
    );
    assert_tone(&s24_captured, Encoding::Pcm24Bit, 6);

    let s32_captured = capture(
        &backend,
        &test_sink.id,
        EncodingPreference::S32,
        Duration::from_millis(1_500),
    );
    assert_tone(&s32_captured, Encoding::Pcm32Bit, 8);

    // s8 maps to unsigned U8; its signedness lives in the negotiated format.
    let s8_captured = capture(
        &backend,
        &test_sink.id,
        EncodingPreference::S8,
        Duration::from_millis(1_500),
    );
    assert_tone(&s8_captured, Encoding::Pcm8Bit, 2);

    // Repeated start/stop, including stops immediately after start.
    start_stop_cycles(&backend, &test_sink.id, 10);

    // Switch sinks and switch back; the tone only plays into TestSink.
    let before_switch = capture(
        &backend,
        &test_sink.id,
        EncodingPreference::Default,
        Duration::from_secs(1),
    );
    assert_eq!(
        before_switch.format,
        AudioFormat::new(Encoding::PcmFloat, 2, 48_000)
    );
    let low = capture(
        &backend,
        &low_sink.id,
        EncodingPreference::S16,
        Duration::from_secs(1),
    );
    assert_eq!(low.format, AudioFormat::new(Encoding::Pcm16Bit, 2, 48_000));
    let after_switch = capture(
        &backend,
        &test_sink.id,
        EncodingPreference::Default,
        Duration::from_secs(1),
    );
    assert_tone(&after_switch, Encoding::PcmFloat, 8);

    // Destroying the target mid-stream must stop the session promptly.
    target_disappearance_stops_session(&backend);
}

#[test]
#[ignore = "starts real headless PipeWire/wireplumber daemons"]
fn linux_pipewire_capture_end_to_end() {
    if std::env::var_os(HEADLESS_MARKER).is_some() {
        run_against_headless_session();
    } else {
        run_headless_session();
    }
}
