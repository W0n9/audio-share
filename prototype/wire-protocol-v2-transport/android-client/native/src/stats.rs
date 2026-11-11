//! Stats JSON contract for the JNI `getStats` call (low-frequency).
//!
//! Kotlin-side parser relies on: fixed field order below, no spaces, all
//! numeric fields unsigned decimal, all string values escaped JSON strings.
//! This is a prototype contract only.

use crate::control::{ClientState, CloseOrigin, CloseReason, MediaConfig};

/// One consistent snapshot of everything Kotlin displays.
#[derive(Debug, Clone, PartialEq)]
pub struct StatsSnapshot {
    pub state: ClientState,
    pub connected: bool,
    pub control_errors: u64,
    pub last_control_error: String,
    /// None until MEDIA_START has been seen.
    pub media_config: Option<MediaConfig>,
    pub media: MediaCounters,
    pub buffer: BufferStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MediaCounters {
    pub datagrams_observed: u64,
    pub highest_seq: Option<u32>,
    pub gaps: u64,
    pub duplicates: u64,
    pub reordered: u64,
    pub instance_changes: u64,
    pub byte_errors: u64,
    pub malformed_payloads: u64,
    pub config_mismatches: u64,
    pub bytes: u64,
    pub frames: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BufferStats {
    pub capacity_bytes: u64,
    /// Monotonic total bytes ever written into the ring.
    pub write_pos: u64,
    /// write_pos % capacity — where the next payload goes.
    pub write_offset: u64,
    /// Prefetch target (bytes of audio) before the client treats itself as playing.
    pub target_bytes: u64,
    pub dropped_bytes: u64,
    pub drops: u64,
}

fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn close_reason_name(reason: &CloseReason) -> &'static str {
    match reason {
        CloseReason::Normal => "NORMAL",
        CloseReason::ServerStop => "SERVER_STOP",
        CloseReason::ProtocolError => "PROTOCOL_ERROR",
        CloseReason::AuthFailed => "AUTH_FAILED",
        CloseReason::IdleTimeout => "IDLE_TIMEOUT",
        CloseReason::InternalError => "INTERNAL_ERROR",
        CloseReason::Unknown(_) => "UNKNOWN",
    }
}

fn close_origin_name(origin: &CloseOrigin) -> &'static str {
    match origin {
        CloseOrigin::Remote => "REMOTE",
        CloseOrigin::Local => "LOCAL",
    }
}

fn sample_format_name(f: u8) -> &'static str {
    match f {
        1 => "f32le",
        _ => "unknown",
    }
}

/// Render the snapshot. Deterministic; field order is part of the contract.
pub fn render(s: &StatsSnapshot) -> String {
    let mut out = String::with_capacity(512);
    let close = match &s.state {
        ClientState::Closed(info) => Some(info),
        _ => None,
    };
    out.push_str("{\"state\":\"");
    out.push_str(s.state.state_name());
    out.push_str("\",\"connected\":");
    out.push_str(if s.connected { "true" } else { "false" });
    out.push_str(",\"control_errors\":");
    out.push_str(&s.control_errors.to_string());
    out.push_str(",\"last_control_error\":\"");
    out.push_str(&escape_json(&s.last_control_error));
    out.push_str("\",\"media\":");
    match &s.media_config {
        None => out.push_str("null"),
        Some(cfg) => {
            out.push_str("{\"stream_instance_id\":");
            out.push_str(&cfg.stream_instance_id.to_string());
            out.push_str(",\"config_generation\":");
            out.push_str(&cfg.config_generation.to_string());
            out.push_str(",\"sample_rate\":");
            out.push_str(&cfg.sample_rate.to_string());
            out.push_str(",\"channels\":");
            out.push_str(&cfg.channels.to_string());
            out.push_str(",\"sample_format\":\"");
            out.push_str(sample_format_name(cfg.sample_format));
            out.push_str("\",\"datagrams_observed\":");
            out.push_str(&s.media.datagrams_observed.to_string());
            out.push_str(",\"highest_seq\":");
            match s.media.highest_seq {
                Some(v) => out.push_str(&v.to_string()),
                None => out.push_str("null"),
            }
            out.push_str(",\"gaps\":");
            out.push_str(&s.media.gaps.to_string());
            out.push_str(",\"duplicates\":");
            out.push_str(&s.media.duplicates.to_string());
            out.push_str(",\"reordered\":");
            out.push_str(&s.media.reordered.to_string());
            out.push_str(",\"instance_changes\":");
            out.push_str(&s.media.instance_changes.to_string());
            out.push_str(",\"byte_errors\":");
            out.push_str(&s.media.byte_errors.to_string());
            out.push_str(",\"malformed_payloads\":");
            out.push_str(&s.media.malformed_payloads.to_string());
            out.push_str(",\"config_mismatches\":");
            out.push_str(&s.media.config_mismatches.to_string());
            out.push_str(",\"bytes\":");
            out.push_str(&s.media.bytes.to_string());
            out.push_str(",\"frames\":");
            out.push_str(&s.media.frames.to_string());
            out.push('}');
        }
    }
    out.push_str(",\"buffer\":{\"capacity_bytes\":");
    out.push_str(&s.buffer.capacity_bytes.to_string());
    out.push_str(",\"write_pos\":");
    out.push_str(&s.buffer.write_pos.to_string());
    out.push_str(",\"write_offset\":");
    out.push_str(&s.buffer.write_offset.to_string());
    out.push_str(",\"target_bytes\":");
    out.push_str(&s.buffer.target_bytes.to_string());
    out.push_str(",\"dropped_bytes\":");
    out.push_str(&s.buffer.dropped_bytes.to_string());
    out.push_str(",\"drops\":");
    out.push_str(&s.buffer.drops.to_string());
    out.push('}');
    out.push_str(",\"close\":");
    match close {
        None => out.push_str("null"),
        Some(info) => {
            out.push_str("{\"reason\":\"");
            out.push_str(close_reason_name(&info.reason));
            out.push_str("\",\"origin\":\"");
            out.push_str(close_origin_name(&info.origin));
            out.push_str("\"}");
        }
    }
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> MediaConfig {
        MediaConfig {
            stream_instance_id: 1,
            config_generation: 1,
            sample_rate: 48000,
            channels: 2,
            sample_format: 1,
        }
    }

    fn playing_snapshot() -> StatsSnapshot {
        StatsSnapshot {
            state: ClientState::Playing(config()),
            connected: true,
            control_errors: 0,
            last_control_error: String::new(),
            media_config: Some(config()),
            media: MediaCounters {
                datagrams_observed: 100,
                highest_seq: Some(99),
                gaps: 1,
                duplicates: 2,
                reordered: 3,
                instance_changes: 0,
                byte_errors: 0,
                malformed_payloads: 0,
                config_mismatches: 0,
                bytes: 24000 * 8,
                frames: 24000,
            },
            buffer: BufferStats {
                capacity_bytes: 4_194_304,
                write_pos: 192_000,
                write_offset: 192_000,
                target_bytes: 9_600,
                dropped_bytes: 0,
                drops: 0,
            },
        }
    }

    #[test]
    fn golden_playing_json() {
        let j = render(&playing_snapshot());
        assert_eq!(
            j,
            "{\"state\":\"PLAYING\",\"connected\":true,\"control_errors\":0,\
             \"last_control_error\":\"\",\
             \"media\":{\"stream_instance_id\":1,\"config_generation\":1,\
             \"sample_rate\":48000,\"channels\":2,\"sample_format\":\"f32le\",\
             \"datagrams_observed\":100,\"highest_seq\":99,\"gaps\":1,\
             \"duplicates\":2,\"reordered\":3,\"instance_changes\":0,\
             \"byte_errors\":0,\"malformed_payloads\":0,\"config_mismatches\":0,\
             \"bytes\":192000,\"frames\":24000},\
             \"buffer\":{\"capacity_bytes\":4194304,\"write_pos\":192000,\
             \"write_offset\":192000,\"target_bytes\":9600,\"dropped_bytes\":0,\
             \"drops\":0},\"close\":null}"
        );
    }

    #[test]
    fn idle_media_null_close_null() {
        let s = StatsSnapshot {
            state: ClientState::Idle,
            connected: false,
            control_errors: 0,
            last_control_error: String::new(),
            media_config: None,
            media: MediaCounters::default(),
            buffer: BufferStats::default(),
        };
        let j = render(&s);
        assert!(j.contains("\"state\":\"IDLE\""));
        assert!(j.contains("\"connected\":false"));
        assert!(j.contains("\"media\":null"));
        assert!(j.ends_with(",\"close\":null}"));
    }

    #[test]
    fn closed_state_carries_close_object() {
        let mut s = playing_snapshot();
        s.state = ClientState::Closed(crate::control::CloseInfo {
            reason: CloseReason::ServerStop,
            origin: CloseOrigin::Remote,
        });
        let j = render(&s);
        assert!(j.contains("\"state\":\"CLOSED\""));
        assert!(j.contains("\"close\":{\"reason\":\"SERVER_STOP\",\"origin\":\"REMOTE\"}"));
    }

    #[test]
    fn error_strings_are_escaped() {
        let mut s = playing_snapshot();
        s.control_errors = 1;
        s.last_control_error = "received \"MEDIA_START\" in\nREADY".to_string();
        let j = render(&s);
        assert!(j.contains(
            "\"last_control_error\":\"received \\\"MEDIA_START\\\" in\\nREADY\""
        ));
    }

    #[test]
    fn highest_seq_null_before_first_datagram() {
        let mut s = playing_snapshot();
        s.media.highest_seq = None;
        assert!(render(&s).contains("\"highest_seq\":null"));
    }
}
