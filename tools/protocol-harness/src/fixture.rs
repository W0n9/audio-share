//! Fixture-set schema and disk IO.
//!
//! A fixture set is a directory with a deterministic `manifest.json`, control
//! message payloads under `control/`, the exact `AudioFormat` protobuf under
//! `audio_format.pb`, and captured UDP payloads under `pcm/`.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use audio_share_core::protocol::{
    decode_get_format_response, decode_start_play_response, decode_udp_bind, AudioFormat,
    IP_HEADER_BYTES, MAX_PAYLOAD_BYTES, MTU, UDP_HEADER_BYTES,
};
use prost::Message;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Current fixture schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// The frozen protocol revision these fixtures describe.
pub const PROTOCOL_VERSION: &str = "v0.3.4";

/// Everything needed to write one fixture set.
#[derive(Debug, Clone, PartialEq)]
pub struct FixturePlan {
    pub provenance: String,
    pub platform: String,
    pub source: String,
    pub format: AudioFormat,
    pub control: ControlBytes,
    pub pcm_segments: Vec<Vec<u8>>,
    pub heartbeats_received: usize,
    pub datagrams_received: usize,
    pub notes: Vec<String>,
}

/// Raw control-channel bytes captured or synthesized for a fixture set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlBytes {
    pub get_format_request: Vec<u8>,
    pub get_format_response: Vec<u8>,
    pub start_play_request: Vec<u8>,
    pub start_play_response: Vec<u8>,
    pub heartbeat_server: Vec<u8>,
    pub heartbeat_client: Vec<u8>,
    pub udp_bind: Vec<u8>,
    pub start_play_id: i32,
    pub heartbeat_count: usize,
}

/// `manifest.json` for a fixture set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub protocol_version: String,
    pub provenance: String,
    pub platform: String,
    pub source: String,
    pub audio_format: AudioFormatManifest,
    pub mtu: MtuManifest,
    pub control: ControlManifest,
    pub pcm: PcmManifest,
    pub capture: CaptureManifest,
    pub notes: Vec<String>,
}

/// AudioFormat fields plus the exact protobuf bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioFormatManifest {
    pub encoding: i32,
    pub encoding_name: String,
    pub channels: i32,
    pub sample_rate: i32,
    pub block_align: usize,
    pub protobuf_file: String,
    pub protobuf_hex: String,
    pub protobuf_sha256: String,
}

/// MTU values used for UDP segmentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MtuManifest {
    pub mtu: usize,
    pub ip_header_bytes: usize,
    pub udp_header_bytes: usize,
    pub max_payload_bytes: usize,
    pub aligned_payload_bytes: usize,
}

/// Control-message fixture metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlManifest {
    pub get_format_request: ControlFileManifest,
    pub get_format_response: ControlFileManifest,
    pub start_play_request: ControlFileManifest,
    pub start_play_response: StartPlayResponseManifest,
    pub heartbeat_server: HeartbeatManifest,
    pub heartbeat_client: HeartbeatManifest,
    pub udp_bind: UdpBindManifest,
}

/// A control payload stored as a file plus its bytes and hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlFileManifest {
    pub file: String,
    pub length: usize,
    pub hex: String,
    pub sha256: String,
}

/// The `START_PLAY` reply metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartPlayResponseManifest {
    pub file: String,
    pub length: usize,
    pub hex: String,
    pub sha256: String,
    pub id: i32,
}

/// A repeated heartbeat payload and how often it was observed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatManifest {
    pub file: String,
    pub length: usize,
    pub hex: String,
    pub sha256: String,
    pub count: usize,
}

/// The UDP bind packet and the play id it carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UdpBindManifest {
    pub file: String,
    pub length: usize,
    pub hex: String,
    pub sha256: String,
    pub id: i32,
}

/// PCM fixture metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PcmManifest {
    pub file: String,
    pub total_bytes: usize,
    pub sha256: String,
    pub segment_count: usize,
    pub segment_lengths: Vec<usize>,
    pub segments: Vec<PcmSegmentManifest>,
}

/// One captured UDP datagram.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PcmSegmentManifest {
    pub file: String,
    pub offset: usize,
    pub length: usize,
    pub sha256: String,
}

/// Counts observed while recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureManifest {
    pub heartbeats_received: usize,
    pub datagrams_received: usize,
}

/// A loaded fixture set.
#[derive(Debug, Clone)]
pub struct Fixture {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

/// Lowercase hex encoding.
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Lowercase SHA-256 hex digest.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex(&hasher.finalize())
}

impl FixturePlan {
    /// Validate cross-field consistency before writing.
    pub fn validate(&self) -> Result<()> {
        let format = self.format.encoding_kind().with_context(|| {
            format!(
                "fixture format has unknown encoding {}",
                self.format.encoding
            )
        })?;
        if self.format.channels <= 0 {
            bail!("fixture format must have positive channels");
        }
        if self.format.sample_rate <= 0 {
            bail!("fixture format must have a positive sample rate");
        }
        let block_align = self.format.block_align();
        if block_align == 0 {
            bail!("fixture format has an unusable block_align (encoding {format:?})");
        }

        let expect_command = |actual: &[u8], command| {
            let expected = audio_share_core::protocol::encode_command(command);
            if actual != expected {
                bail!(
                    "fixture control request {command:?} is {:?}, expected {:?}",
                    actual,
                    expected
                );
            }
            Ok(())
        };
        expect_command(
            &self.control.get_format_request,
            audio_share_core::protocol::Command::GetFormat,
        )?;
        expect_command(
            &self.control.start_play_request,
            audio_share_core::protocol::Command::StartPlay,
        )?;

        let (decoded_format, consumed) =
            decode_get_format_response(&self.control.get_format_response)
                .context("fixture GET_FORMAT response is malformed")?;
        if consumed != self.control.get_format_response.len() {
            bail!("fixture GET_FORMAT response has trailing bytes");
        }
        if decoded_format != self.format {
            bail!("fixture GET_FORMAT response does not carry the declared AudioFormat");
        }

        let (start_play_id, consumed) =
            decode_start_play_response(&self.control.start_play_response)
                .context("fixture START_PLAY response is malformed")?;
        if consumed != self.control.start_play_response.len() {
            bail!("fixture START_PLAY response has trailing bytes");
        }
        if start_play_id != self.control.start_play_id {
            bail!(
                "fixture START_PLAY id {start_play_id} disagrees with manifest id {}",
                self.control.start_play_id
            );
        }

        let heartbeat = audio_share_core::protocol::encode_heartbeat();
        if self.control.heartbeat_server != heartbeat {
            bail!("fixture server heartbeat bytes are not a v0.3.4 heartbeat");
        }
        if self.control.heartbeat_client != heartbeat {
            bail!("fixture client heartbeat bytes are not a v0.3.4 heartbeat");
        }
        if decode_udp_bind(&self.control.udp_bind)? != self.control.start_play_id {
            bail!("fixture UDP bind id disagrees with the START_PLAY id");
        }

        let max_segment = audio_share_core::protocol::max_segment_payload(block_align);
        if max_segment == 0 {
            bail!("fixture block_align does not fit an MTU payload");
        }
        for segment in &self.pcm_segments {
            if segment.is_empty() {
                bail!("fixture contains an empty UDP segment");
            }
            if segment.len() > max_segment {
                bail!(
                    "fixture UDP segment of {} bytes exceeds the aligned MTU payload {max_segment}",
                    segment.len()
                );
            }
            if segment.len() % block_align != 0 {
                bail!(
                    "fixture UDP segment of {} bytes is not aligned to block_align {block_align}",
                    segment.len()
                );
            }
        }
        if self.datagrams_received != self.pcm_segments.len() {
            bail!(
                "fixture datagram count {} disagrees with {} recorded segments",
                self.datagrams_received,
                self.pcm_segments.len()
            );
        }
        Ok(())
    }
}

fn write_control_file(dir: &Path, relative: &str, bytes: &[u8]) -> Result<ControlFileManifest> {
    fs::write(dir.join(relative), bytes)
        .with_context(|| format!("write control fixture {relative}"))?;
    Ok(ControlFileManifest {
        file: relative.to_owned(),
        length: bytes.len(),
        hex: hex(bytes),
        sha256: sha256_hex(bytes),
    })
}

/// Write one fixture set to `dir`, replacing any existing contents.
pub fn write_fixture_set(dir: &Path, plan: &FixturePlan) -> Result<()> {
    plan.validate()?;

    if dir.exists() {
        fs::remove_dir_all(dir)
            .with_context(|| format!("clear fixture directory {}", dir.display()))?;
    }
    fs::create_dir_all(dir.join("control"))
        .with_context(|| format!("create control directory in {}", dir.display()))?;
    fs::create_dir_all(dir.join("pcm").join("segments"))
        .with_context(|| format!("create PCM directory in {}", dir.display()))?;

    let protobuf = plan.format.encode_to_vec();
    let protobuf_file = "audio_format.pb";
    fs::write(dir.join(protobuf_file), &protobuf)?;

    let get_format_request = write_control_file(
        dir,
        "control/get_format.request.bin",
        &plan.control.get_format_request,
    )?;
    let get_format_response = write_control_file(
        dir,
        "control/get_format.response.bin",
        &plan.control.get_format_response,
    )?;
    let start_play_request = write_control_file(
        dir,
        "control/start_play.request.bin",
        &plan.control.start_play_request,
    )?;
    let start_play_response = write_control_file(
        dir,
        "control/start_play.response.bin",
        &plan.control.start_play_response,
    )?;
    let heartbeat_server = write_control_file(
        dir,
        "control/heartbeat.server.bin",
        &plan.control.heartbeat_server,
    )?;
    let heartbeat_client = write_control_file(
        dir,
        "control/heartbeat.client.bin",
        &plan.control.heartbeat_client,
    )?;
    let udp_bind = write_control_file(dir, "control/udp_bind.bin", &plan.control.udp_bind)?;

    let mut stream = Vec::new();
    let mut segments = Vec::with_capacity(plan.pcm_segments.len());
    let mut segment_lengths = Vec::with_capacity(plan.pcm_segments.len());
    for (index, segment) in plan.pcm_segments.iter().enumerate() {
        let offset = stream.len();
        let relative = format!("pcm/segments/{index:04}.bin");
        fs::write(dir.join(&relative), segment)
            .with_context(|| format!("write PCM segment {relative}"))?;
        segments.push(PcmSegmentManifest {
            file: relative,
            offset,
            length: segment.len(),
            sha256: sha256_hex(segment),
        });
        segment_lengths.push(segment.len());
        stream.extend_from_slice(segment);
    }
    let pcm_file = "pcm/stream.pcm";
    fs::write(dir.join(pcm_file), &stream).context("write concatenated PCM fixture")?;

    let encoding = plan.format.encoding_kind().expect("validated encoding");
    let manifest = Manifest {
        schema_version: SCHEMA_VERSION,
        protocol_version: PROTOCOL_VERSION.to_owned(),
        provenance: plan.provenance.clone(),
        platform: plan.platform.clone(),
        source: plan.source.clone(),
        audio_format: AudioFormatManifest {
            encoding: plan.format.encoding,
            encoding_name: encoding.proto_name().to_owned(),
            channels: plan.format.channels,
            sample_rate: plan.format.sample_rate,
            block_align: plan.format.block_align(),
            protobuf_file: protobuf_file.to_owned(),
            protobuf_hex: hex(&protobuf),
            protobuf_sha256: sha256_hex(&protobuf),
        },
        mtu: MtuManifest {
            mtu: MTU,
            ip_header_bytes: IP_HEADER_BYTES,
            udp_header_bytes: UDP_HEADER_BYTES,
            max_payload_bytes: MAX_PAYLOAD_BYTES,
            aligned_payload_bytes: audio_share_core::protocol::max_segment_payload(
                plan.format.block_align(),
            ),
        },
        control: ControlManifest {
            get_format_request,
            get_format_response,
            start_play_request,
            start_play_response: StartPlayResponseManifest {
                file: start_play_response.file,
                length: start_play_response.length,
                hex: start_play_response.hex,
                sha256: start_play_response.sha256,
                id: plan.control.start_play_id,
            },
            heartbeat_server: HeartbeatManifest {
                file: heartbeat_server.file,
                length: heartbeat_server.length,
                hex: heartbeat_server.hex,
                sha256: heartbeat_server.sha256,
                count: plan.control.heartbeat_count,
            },
            heartbeat_client: HeartbeatManifest {
                file: heartbeat_client.file,
                length: heartbeat_client.length,
                hex: heartbeat_client.hex,
                sha256: heartbeat_client.sha256,
                count: plan.control.heartbeat_count,
            },
            udp_bind: UdpBindManifest {
                file: udp_bind.file,
                length: udp_bind.length,
                hex: udp_bind.hex,
                sha256: udp_bind.sha256,
                id: plan.control.start_play_id,
            },
        },
        pcm: PcmManifest {
            file: pcm_file.to_owned(),
            total_bytes: stream.len(),
            sha256: sha256_hex(&stream),
            segment_count: segments.len(),
            segment_lengths,
            segments,
        },
        capture: CaptureManifest {
            heartbeats_received: plan.heartbeats_received,
            datagrams_received: plan.datagrams_received,
        },
        notes: plan.notes.clone(),
    };

    let mut json = serde_json::to_vec_pretty(&manifest).context("serialize fixture manifest")?;
    json.push(b'\n');
    fs::write(dir.join("manifest.json"), json).context("write fixture manifest")?;
    Ok(())
}

/// Read and deserialize `manifest.json`.
pub fn read_manifest(dir: &Path) -> Result<Manifest> {
    let path = dir.join("manifest.json");
    let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
}

/// Load a fixture set and check its schema/protocol header.
pub fn load_fixture(dir: &Path) -> Result<Fixture> {
    let manifest = read_manifest(dir)?;
    if manifest.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported fixture schema version {} (expected {SCHEMA_VERSION})",
            manifest.schema_version
        );
    }
    if manifest.protocol_version != PROTOCOL_VERSION {
        bail!(
            "unsupported fixture protocol version {} (expected {PROTOCOL_VERSION})",
            manifest.protocol_version
        );
    }
    Ok(Fixture {
        dir: dir.to_path_buf(),
        manifest,
    })
}

impl Fixture {
    /// Read a fixture-relative file, rejecting absolute paths and escapes.
    pub fn read_file(&self, relative: &str) -> Result<Vec<u8>> {
        let path = Path::new(relative);
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            bail!("fixture path {relative:?} must stay inside the fixture directory");
        }
        let full = self.dir.join(path);
        fs::read(&full).with_context(|| format!("read fixture file {}", full.display()))
    }

    /// Decode the exact `AudioFormat` protobuf stored in the fixture.
    pub fn decoded_format(&self) -> Result<AudioFormat> {
        let bytes = self.read_file(&self.manifest.audio_format.protobuf_file)?;
        let format =
            AudioFormat::decode(bytes.as_slice()).context("decode fixture AudioFormat protobuf")?;
        let declared = &self.manifest.audio_format;
        if format.encoding != declared.encoding
            || format.channels != declared.channels
            || format.sample_rate != declared.sample_rate
        {
            bail!(
                "fixture AudioFormat protobuf {format:?} disagrees with manifest {}",
                serde_json::to_string(declared).unwrap_or_default()
            );
        }
        Ok(format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_share_core::protocol::{
        encode_command, encode_get_format_response, encode_heartbeat, encode_start_play_response,
        encode_udp_bind, Command, Encoding,
    };

    fn format() -> AudioFormat {
        AudioFormat::new(Encoding::PcmFloat, 2, 48_000)
    }

    fn control() -> ControlBytes {
        ControlBytes {
            get_format_request: encode_command(Command::GetFormat).to_vec(),
            get_format_response: encode_get_format_response(&format()),
            start_play_request: encode_command(Command::StartPlay).to_vec(),
            start_play_response: encode_start_play_response(1).to_vec(),
            heartbeat_server: encode_heartbeat().to_vec(),
            heartbeat_client: encode_heartbeat().to_vec(),
            udp_bind: encode_udp_bind(1).to_vec(),
            start_play_id: 1,
            heartbeat_count: 3,
        }
    }

    fn plan() -> FixturePlan {
        FixturePlan {
            provenance: "synthesized-v0.3.4-semantics".to_owned(),
            platform: "windows".to_owned(),
            source: "test".to_owned(),
            format: format(),
            control: control(),
            pcm_segments: vec![vec![0x11; 1464], vec![0x22; 1464], vec![0x33; 72]],
            heartbeats_received: 3,
            datagrams_received: 3,
            notes: vec!["synthetic".to_owned()],
        }
    }

    #[test]
    fn write_then_load_round_trips_manifest_and_pcm() {
        let temp = tempfile::tempdir().unwrap();
        write_fixture_set(temp.path(), &plan()).unwrap();

        let fixture = load_fixture(temp.path()).unwrap();
        assert_eq!(fixture.manifest.schema_version, SCHEMA_VERSION);
        assert_eq!(fixture.manifest.protocol_version, PROTOCOL_VERSION);
        assert_eq!(fixture.manifest.audio_format.encoding, 1);
        assert_eq!(
            fixture.manifest.audio_format.encoding_name,
            "ENCODING_PCM_FLOAT"
        );
        assert_eq!(fixture.manifest.audio_format.channels, 2);
        assert_eq!(fixture.manifest.audio_format.sample_rate, 48_000);
        assert_eq!(fixture.manifest.audio_format.block_align, 8);
        assert_eq!(fixture.manifest.mtu.max_payload_bytes, 1464);
        assert_eq!(fixture.manifest.mtu.aligned_payload_bytes, 1464);
        assert_eq!(fixture.manifest.control.start_play_response.id, 1);
        assert_eq!(fixture.manifest.control.udp_bind.id, 1);
        assert_eq!(fixture.manifest.pcm.segment_count, 3);
        assert_eq!(fixture.manifest.pcm.segment_lengths, vec![1464, 1464, 72]);
        assert_eq!(fixture.manifest.pcm.total_bytes, 3000);
        assert_eq!(fixture.manifest.capture.heartbeats_received, 3);
        assert_eq!(fixture.decoded_format().unwrap(), format());

        let stream = fixture.read_file(&fixture.manifest.pcm.file).unwrap();
        assert_eq!(stream.len(), 3000);
        assert_eq!(&stream[..1464], &[0x11; 1464]);
        assert_eq!(&stream[1464..2928], &[0x22; 1464]);
        assert_eq!(&stream[2928..], &[0x33; 72]);
        assert_eq!(sha256_hex(&stream), fixture.manifest.pcm.sha256);

        let protobuf = fixture
            .read_file(&fixture.manifest.audio_format.protobuf_file)
            .unwrap();
        assert_eq!(hex(&protobuf), "080110021880f702");
    }

    #[test]
    fn manifest_is_deterministic_and_contains_no_paths_or_timestamps() {
        let temp = tempfile::tempdir().unwrap();
        write_fixture_set(temp.path(), &plan()).unwrap();
        let first = fs::read(temp.path().join("manifest.json")).unwrap();
        write_fixture_set(temp.path(), &plan()).unwrap();
        let second = fs::read(temp.path().join("manifest.json")).unwrap();
        assert_eq!(first, second);

        let text = String::from_utf8(first).unwrap();
        assert!(!text.contains(temp.path().to_str().unwrap()));
        assert!(!text.contains("timestamp"));
        assert!(text.contains("\"provenance\": \"synthesized-v0.3.4-semantics\""));
    }

    #[test]
    fn load_rejects_foreign_schema_version() {
        let temp = tempfile::tempdir().unwrap();
        write_fixture_set(temp.path(), &plan()).unwrap();
        let path = temp.path().join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["schema_version"] = serde_json::json!(99);
        fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
        let error = load_fixture(temp.path()).unwrap_err().to_string();
        assert!(error.contains("schema"), "unexpected error: {error}");
    }

    #[test]
    fn write_rejects_inconsistent_control_bytes() {
        let mut bad = plan();
        bad.control.start_play_response = encode_start_play_response(2).to_vec();
        let temp = tempfile::tempdir().unwrap();
        assert!(write_fixture_set(temp.path(), &bad).is_err());
    }

    #[test]
    fn read_file_rejects_path_escape() {
        let temp = tempfile::tempdir().unwrap();
        write_fixture_set(temp.path(), &plan()).unwrap();
        let fixture = load_fixture(temp.path()).unwrap();
        assert!(fixture.read_file("../manifest.json").is_err());
        assert!(fixture.read_file("/etc/passwd").is_err());
    }

    #[test]
    fn decoded_format_checks_manifest_agreement() {
        let temp = tempfile::tempdir().unwrap();
        write_fixture_set(temp.path(), &plan()).unwrap();
        let mut fixture = load_fixture(temp.path()).unwrap();
        fixture.manifest.audio_format.channels = 1;
        assert!(fixture.decoded_format().is_err());
    }

    #[test]
    fn decode_helpers_parse_written_control_files() {
        let temp = tempfile::tempdir().unwrap();
        write_fixture_set(temp.path(), &plan()).unwrap();
        let fixture = load_fixture(temp.path()).unwrap();
        let response = fixture
            .read_file(&fixture.manifest.control.get_format_response.file)
            .unwrap();
        assert_eq!(decode_get_format_response(&response).unwrap().0, format());
        let start = fixture
            .read_file(&fixture.manifest.control.start_play_response.file)
            .unwrap();
        assert_eq!(decode_start_play_response(&start).unwrap().0, 1);
        let bind = fixture
            .read_file(&fixture.manifest.control.udp_bind.file)
            .unwrap();
        assert_eq!(decode_udp_bind(&bind).unwrap(), 1);
    }
}
