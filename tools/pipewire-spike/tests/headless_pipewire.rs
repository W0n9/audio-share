#![cfg(target_os = "linux")]
//! End-to-end check of the spike against a disposable headless PipeWire
//! session (`scripts/pw-test-session.sh`): list/default selection, F32LE and
//! non-F32 monitor capture with a 440 Hz tone, and repeated start/stop.
//!
//! Requires pipewire, wireplumber, pw-cli, pw-dump, pw-play and
//! dbus-run-session. Ignored by default because it starts real daemons:
//!
//! ```text
//! cargo test -p pipewire-spike --test headless_pipewire -- --ignored --nocapture
//! ```

use std::f64::consts::PI;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pipewire_spike::{decode_mono_f32, dominant_frequency, goertzel_amplitude, PcmEncoding};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("tools/pipewire-spike lives under the repo root")
        .to_path_buf()
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
            * (2.0 * PI * 440.0 * f64::from(index) / f64::from(SAMPLE_RATE)).sin()
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

fn run_session(tone: &Path, inner_script: &Path, out_dir: &Path) -> Output {
    let script = repo_root().join("scripts/pw-test-session.sh");
    let output = Command::new("bash")
        .arg(script)
        .arg("--tone-wav")
        .arg(tone)
        .arg("--")
        .arg("bash")
        .arg(inner_script)
        .env("SPIKE_BIN", env!("CARGO_BIN_EXE_pipewire-spike"))
        .env("OUT_DIR", out_dir)
        .output()
        .expect("run headless PipeWire driver");
    assert!(
        output.status.success(),
        "headless session failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn verify_tone(path: &Path, encoding: PcmEncoding, channels: u32, block_align: u32) {
    const SAMPLE_RATE: u32 = 48_000;
    let bytes = fs::read(path).expect("read captured PCM");
    assert!(!bytes.is_empty(), "captured PCM is empty");
    assert_eq!(
        bytes.len() as u32 % block_align,
        0,
        "PCM length {} is not a multiple of block_align {block_align}",
        bytes.len()
    );
    let frames = bytes.len() as u64 / u64::from(block_align);
    assert!(
        frames > u64::from(SAMPLE_RATE),
        "only {frames} frames captured; expected more than one second"
    );

    let mono = decode_mono_f32(&bytes, encoding, channels);
    let detected = dominant_frequency(&mono, SAMPLE_RATE, 300.0, 600.0)
        .expect("dominant frequency from captured PCM");
    assert!(
        (detected - 440.0).abs() <= 3.0,
        "detected {detected} Hz instead of 440 Hz"
    );
    let at_440 = goertzel_amplitude(&mono, SAMPLE_RATE, 440.0);
    let at_1000 = goertzel_amplitude(&mono, SAMPLE_RATE, 1000.0);
    assert!(at_440 > 0.01, "440 Hz amplitude too low: {at_440}");
    assert!(
        at_1000 < at_440 / 10.0,
        "1000 Hz amplitude {at_1000} is not far below 440 Hz amplitude {at_440}"
    );
}

#[test]
#[ignore = "starts real headless PipeWire/wireplumber daemons"]
fn headless_pipewire_spike_end_to_end() {
    let temp = tempfile::tempdir().expect("temp dir");
    let tone = temp.path().join("tone.wav");
    write_tone_wav(&tone);

    // All commands run inside one headless session so the dynamically
    // allocated object.serial stays valid across invocations.
    let inner = temp.path().join("run.sh");
    fs::write(
        &inner,
        r#"set -euo pipefail
"$SPIKE_BIN" list
"$SPIKE_BIN" capture --endpoint default --encoding f32 --channels 2 --sample-rate 48000 --seconds 3 --out "$OUT_DIR/f32.pcm"
"$SPIKE_BIN" capture --endpoint "$PW_TEST_ENDPOINT_SERIAL" --encoding s16 --channels 2 --sample-rate 48000 --seconds 2 --out "$OUT_DIR/s16.pcm"
"$SPIKE_BIN" capture --endpoint default --encoding s24 --channels 2 --sample-rate 48000 --seconds 2 --out "$OUT_DIR/s24.pcm"
"$SPIKE_BIN" measure --endpoint default --encoding f32 --channels 2 --sample-rate 48000 --seconds 1 --cycles 3 --cycle-seconds 0.2
"#,
    )
    .expect("write inner session script");
    let output = stdout_of(&run_session(&tone, &inner, temp.path()));

    // 1. list: TestSink is present, priority.session is read, and it is default.
    assert!(
        output.lines().any(|line| line.starts_with('*')
            && line.contains("display=\"TestSink\"")
            && line.contains("priority.session=1000")),
        "list did not mark TestSink as default:\n{output}"
    );
    assert!(output.contains("name=TestSink"), "list output:\n{output}");

    // 2. F32LE monitor capture through the default endpoint.
    assert!(
        output
            .contains("negotiated: format=AudioFormat::F32LE rate=48000 channels=2 block_align=8"),
        "unexpected F32 negotiation:\n{output}"
    );
    verify_tone(&temp.path().join("f32.pcm"), PcmEncoding::F32, 2, 8);

    // 3. S16LE through an explicit object.serial (validated against the registry).
    assert!(
        output
            .contains("negotiated: format=AudioFormat::S16LE rate=48000 channels=2 block_align=4"),
        "unexpected S16 negotiation:\n{output}"
    );
    verify_tone(&temp.path().join("s16.pcm"), PcmEncoding::S16, 2, 4);

    // 4. Packed S24LE (3 bytes/sample) negotiated block_align 6.
    assert!(
        output
            .contains("negotiated: format=AudioFormat::S24LE rate=48000 channels=2 block_align=6"),
        "unexpected S24 negotiation:\n{output}"
    );
    verify_tone(&temp.path().join("s24.pcm"), PcmEncoding::S24, 2, 6);

    // 5. Measure plus repeated start/stop (fewer cycles here for speed; the
    //    default is 10 and is exercised by the documented manual command).
    assert!(output.contains("callback_interval_ms:"), "{output}");
    assert!(output.contains("quantum_frames:"), "{output}");
    assert!(output.contains("glitches:"), "{output}");
    assert!(output.contains("cycles: 3/3 ok"), "{output}");
}
