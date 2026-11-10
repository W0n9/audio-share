//! Synthesized fixture sets for the Windows v0.3.4 cases.
//!
//! These fixtures use the frozen v0.3.4 encoding but were not recorded on a
//! Windows machine, so every manifest says `synthesized-v0.3.4-semantics`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use audio_share_core::protocol::{
    encode_command, encode_get_format_response, encode_heartbeat, encode_start_play_response,
    encode_udp_bind, segment_audio, AudioFormat, Command, Encoding,
};

use crate::fixture::{write_fixture_set, ControlBytes, FixturePlan};

/// Provenance string used by every synthesized fixture.
pub const PROVENANCE: &str = "synthesized-v0.3.4-semantics";

/// One documented Windows baseline case.
#[derive(Debug, Clone, Copy)]
pub struct Case {
    pub name: &'static str,
    pub encoding: Encoding,
    pub channels: i32,
    pub sample_rate: i32,
}

/// The Windows v0.3.4 cases named in ticket 01.
pub const WINDOWS_CASES: [Case; 4] = [
    Case {
        name: "f32-48k-stereo",
        encoding: Encoding::PcmFloat,
        channels: 2,
        sample_rate: 48_000,
    },
    Case {
        name: "s16-44k1-stereo",
        encoding: Encoding::Pcm16Bit,
        channels: 2,
        sample_rate: 44_100,
    },
    Case {
        name: "s16-48k-mono",
        encoding: Encoding::Pcm16Bit,
        channels: 1,
        sample_rate: 48_000,
    },
    Case {
        name: "s24-48k-stereo",
        encoding: Encoding::Pcm24Bit,
        channels: 2,
        sample_rate: 48_000,
    },
];

/// `synthesize` command arguments.
#[derive(Debug, clap::Args)]
pub struct SynthesizeArgs {
    /// Directory to write the four Windows case fixture sets into.
    #[arg(long)]
    pub out: PathBuf,
}

/// Write every synthesized Windows fixture set under `out`.
pub fn run(args: &SynthesizeArgs) -> Result<()> {
    for case in WINDOWS_CASES {
        let dir = args.out.join(case.name);
        build_case(&case)
            .and_then(|plan| write_fixture_set(&dir, &plan))
            .with_context(|| format!("synthesize {}", case.name))?;
        println!("wrote {}", dir.display());
    }
    Ok(())
}

/// Build the fixture plan for one synthesized case.
pub fn build_case(case: &Case) -> Result<FixturePlan> {
    let format = AudioFormat::new(case.encoding, case.channels, case.sample_rate);
    let block_align = format.block_align();
    if block_align == 0 {
        anyhow::bail!("{} has an unusable block_align", case.name);
    }

    // Eight deterministic 10 ms buffers, then segment them exactly like v0.3.4.
    let frames_per_chunk = (case.sample_rate as usize / 100).max(1);
    let mut stream = Vec::new();
    for chunk in 0..8u64 {
        stream.extend_from_slice(&synth_chunk(
            case.encoding,
            case.channels as usize,
            frames_per_chunk,
            chunk * frames_per_chunk as u64,
        ));
    }
    let pcm_segments: Vec<Vec<u8>> = segment_audio(&stream, block_align)
        .into_iter()
        .map(<[u8]>::to_vec)
        .collect();

    let control = ControlBytes {
        get_format_request: encode_command(Command::GetFormat).to_vec(),
        get_format_response: encode_get_format_response(&format),
        start_play_request: encode_command(Command::StartPlay).to_vec(),
        start_play_response: encode_start_play_response(1).to_vec(),
        heartbeat_server: encode_heartbeat().to_vec(),
        heartbeat_client: encode_heartbeat().to_vec(),
        udp_bind: encode_udp_bind(1).to_vec(),
        start_play_id: 1,
        heartbeat_count: 1,
    };

    Ok(FixturePlan {
        provenance: PROVENANCE.to_owned(),
        platform: "windows".to_owned(),
        source: "synthesized from frozen v0.3.4 semantics".to_owned(),
        format,
        control,
        datagrams_received: pcm_segments.len(),
        pcm_segments,
        heartbeats_received: 0,
        notes: vec![
            "Synthesized from the frozen v0.3.4 Wire Protocol; not recorded on a Windows machine."
                .to_owned(),
            "PCM is a deterministic integer pattern used only to exercise framing and MTU segmentation."
                .to_owned(),
        ],
    })
}

/// Deterministic interleaved PCM for a synthetic fixture or the fake server.
///
/// Samples follow a 32-frame sawtooth using integer arithmetic only, so the
/// bytes are identical on every platform.
pub fn synth_chunk(
    encoding: Encoding,
    channels: usize,
    frames: usize,
    start_frame: u64,
) -> Vec<u8> {
    let bytes_per_sample = encoding.bytes_per_sample().unwrap_or(0);
    let mut out = Vec::with_capacity(
        frames
            .saturating_mul(channels)
            .saturating_mul(bytes_per_sample),
    );
    for frame in 0..frames {
        let saw = ((start_frame + frame as u64) % 32) as i32 - 16;
        for _ in 0..channels {
            match encoding {
                Encoding::Invalid => {}
                Encoding::PcmFloat => {
                    out.extend_from_slice(&(saw as f32 / 32.0).to_le_bytes());
                }
                Encoding::Pcm8Bit => {
                    out.push(((saw * 4) as i8) as u8);
                }
                Encoding::Pcm16Bit => {
                    out.extend_from_slice(&((saw * 1024) as i16).to_le_bytes());
                }
                Encoding::Pcm24Bit => {
                    let value = saw * 65_536;
                    out.extend_from_slice(&value.to_le_bytes()[..3]);
                }
                Encoding::Pcm32Bit => {
                    out.extend_from_slice(&(saw * 16_777_216).to_le_bytes());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_pcm_is_deterministic_for_every_encoding() {
        for encoding in Encoding::ALL {
            let first = synth_chunk(encoding, 2, 48, 0);
            let second = synth_chunk(encoding, 2, 48, 0);
            assert_eq!(first, second, "{encoding:?}");
            let expected = match encoding {
                Encoding::Invalid => 0,
                other => 48 * 2 * other.bytes_per_sample().unwrap(),
            };
            assert_eq!(first.len(), expected, "{encoding:?}");
        }
    }

    #[test]
    fn windows_cases_cover_the_requested_formats() {
        let names: Vec<_> = WINDOWS_CASES.iter().map(|case| case.name).collect();
        assert_eq!(
            names,
            vec![
                "f32-48k-stereo",
                "s16-44k1-stereo",
                "s16-48k-mono",
                "s24-48k-stereo"
            ]
        );
        for case in WINDOWS_CASES {
            let format = AudioFormat::new(case.encoding, case.channels, case.sample_rate);
            assert!(format.block_align() > 0, "{}", case.name);
        }
    }

    #[test]
    fn synthesized_segments_are_aligned_and_full() {
        for case in WINDOWS_CASES {
            let plan = build_case(&case).unwrap();
            let block_align = plan.format.block_align();
            let max = audio_share_core::protocol::max_segment_payload(block_align);
            assert!(plan.pcm_segments.iter().any(|segment| segment.len() == max));
            for segment in &plan.pcm_segments {
                assert!(segment.len() <= max);
                assert_eq!(segment.len() % block_align, 0);
            }
            plan.validate().unwrap();
        }
    }
}
