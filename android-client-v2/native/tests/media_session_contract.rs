//! Ticket 14 media delivery contract, through the public `MediaSession` API.
//!
//! Expected values come from `docs/wire-protocol-v2/spec.md` §7.1/§7.5/§7.6/§9
//! and `fixtures/wire-protocol-v2/media/*.json` — never from the receiver's own
//! bookkeeping.

use audio_share_client_v2_native::media::{MediaHeader, FLAG_STREAM_START};
use audio_share_client_v2_native::media_session::{
    DatagramOutcome, MediaSession, PlaybackReport, UNKNOWN_CONFIG_GIVE_UP_MS,
    UNKNOWN_CONFIG_MAX_DATAGRAMS,
};
use audio_share_client_v2_native::proto::{Configuration, PcmParameters};
use audio_share_client_v2_native::ring::MediaRing;

fn config(generation: u32, sample_rate: u32, channels: u32) -> Configuration {
    Configuration {
        config_generation: generation,
        codec: 1,
        pcm: Some(PcmParameters {
            sample_rate,
            channels,
            sample_format: 1, // F32LE
        }),
    }
}

fn datagram(
    instance: u32,
    generation: u32,
    sequence: u32,
    timestamp: u64,
    frames: usize,
) -> Vec<u8> {
    let header = MediaHeader {
        stream_instance_id: instance,
        config_generation: generation,
        sequence,
        timestamp,
        flags: if sequence == 0 { FLAG_STREAM_START } else { 0 },
    };
    let mut out = Vec::new();
    header.write_be(&mut out);
    out.extend(std::iter::repeat_n(0u8, frames * 8));
    out
}

fn session() -> MediaSession {
    MediaSession::start(1, 1, 0, &config(1, 48_000, 2), None).expect("F32LE stereo")
}

/// §5.3/§6.2: a valid-but-undecodable geometry is a 403 class, never a
/// silent fallback. S24LE has no AudioTrack encoding on API 26.
#[test]
fn an_undecodable_geometry_is_refused_not_silently_played() {
    let mut s24 = config(1, 48_000, 2);
    s24.pcm.as_mut().unwrap().sample_format = 4; // S24LE
    assert!(
        MediaSession::start(1, 1, 0, &s24, None).is_none(),
        "S24LE must be refused so the session emits 403"
    );
    // F32LE and S16LE remain decodable.
    for format in [1, 3] {
        let mut cfg = config(1, 48_000, 2);
        cfg.pcm.as_mut().unwrap().sample_format = format;
        assert!(
            MediaSession::start(1, 1, 0, &cfg, None).is_some(),
            "format {format}"
        );
    }
}

/// §9: `submitted_position` is derived from what the platform actually
/// accepted, not from rings writes or received datagrams.
#[test]
fn submitted_position_follows_the_platform_playback_report() {
    let mut media = session();
    // Receive 4 datagrams of 64 frames each — delivered, but not played.
    for sequence in 0..4u32 {
        assert_eq!(
            media.on_datagram(0, &datagram(1, 1, sequence, u64::from(sequence) * 64, 64)),
            DatagramOutcome::Delivered
        );
    }
    assert_eq!(
        media.submitted_position(),
        0,
        "receiving and ring delivery are not playback evidence"
    );

    // The AudioTrack accepted 3 datagrams' worth (3 × 64 frames × 8 bytes).
    media.record_playback(PlaybackReport {
        submitted_bytes: 3 * 64 * 8,
        underruns: 0,
        buffer_current_ms: 25,
    });
    assert_eq!(media.submitted_position(), 192);
    let diagnostics = media.diagnostics(60);
    assert_eq!(diagnostics.submitted_position, 192);
    assert_eq!(diagnostics.cumulative_received, 4);
    assert_eq!(diagnostics.buffer_target_ms, 60);
}

/// §7.5: duplicates are dropped and counted; sequence gaps become
/// `cumulative_lost`.
#[test]
fn duplicates_and_gaps_are_reported_per_spec_nine() {
    let mut media = session();
    assert_eq!(
        media.on_datagram(0, &datagram(1, 1, 0, 0, 64)),
        DatagramOutcome::Delivered
    );
    assert_eq!(
        media.on_datagram(0, &datagram(1, 1, 0, 0, 64)),
        DatagramOutcome::Duplicate
    );
    // 1 is missing, 2 arrives: one lost.
    assert_eq!(
        media.on_datagram(0, &datagram(1, 1, 2, 128, 64)),
        DatagramOutcome::Delivered
    );
    let diagnostics = media.diagnostics(60);
    assert_eq!(diagnostics.cumulative_duplicates, 1);
    assert_eq!(diagnostics.cumulative_received, 2);
    assert_eq!(diagnostics.cumulative_lost, 1);
    assert_eq!(diagnostics.interval_duplicates, 1);
    assert_eq!(diagnostics.interval_lost, 1);

    // Interval counters reset after a report; cumulative ones do not.
    let next = media.diagnostics(60);
    assert_eq!(next.interval_duplicates, 0);
    assert_eq!(next.cumulative_duplicates, 1);
}

/// §7.5: a datagram older than the current generation is dropped and counted
/// in `unknown_config_drops`, never decoded with the previous configuration.
#[test]
fn a_stale_generation_is_dropped_and_counted() {
    let mut media = session();
    media.apply_configuration(&config(2, 48_000, 2));
    assert_eq!(
        media.on_datagram(0, &datagram(1, 1, 5, 320, 64)),
        DatagramOutcome::StaleGeneration
    );
    assert_eq!(media.delivered_frames(), 0);
    let diagnostics = media.diagnostics(60);
    assert_eq!(diagnostics.cumulative_unknown_config_drops, 1);
    assert_eq!(diagnostics.config_generation, 2);
}

/// §7.6: an unknown generation is held, never decoded; when the reliable
/// configuration arrives the held media is released.
#[test]
fn unknown_generation_is_held_then_released_on_configuration() {
    let mut media = session();
    assert_eq!(
        media.on_datagram(0, &datagram(1, 2, 0, 0, 64)),
        DatagramOutcome::HeldUnknownConfig
    );
    assert!(media.holding_unknown_config());
    assert_eq!(media.delivered_frames(), 0, "held media is never decoded");

    media.apply_configuration(&config(2, 48_000, 2));
    assert!(!media.holding_unknown_config());
    assert_eq!(
        media.delivered_frames(),
        64,
        "the held datagram is released"
    );
}

/// §7.6: the hold is bounded at 32 datagrams; overflow drops the oldest and
/// counts it.
#[test]
fn the_unknown_configuration_hold_is_bounded() {
    let mut media = session();
    for sequence in 0..(UNKNOWN_CONFIG_MAX_DATAGRAMS as u32 + 8) {
        assert_eq!(
            media.on_datagram(0, &datagram(1, 2, sequence, u64::from(sequence), 1)),
            DatagramOutcome::HeldUnknownConfig
        );
    }
    let diagnostics = media.diagnostics(60);
    assert_eq!(
        diagnostics.cumulative_unknown_config_drops, 8,
        "the oldest held datagrams are dropped"
    );
}

/// §7.6 step 3: still unknown after 2 s → the session must close with 403.
#[test]
fn an_unknown_generation_gives_up_after_two_seconds() {
    let mut media = session();
    media.on_datagram(0, &datagram(1, 2, 0, 0, 64));
    media.tick(UNKNOWN_CONFIG_GIVE_UP_MS - 1);
    assert!(!media.gave_up_unknown_config());
    media.tick(UNKNOWN_CONFIG_GIVE_UP_MS);
    assert!(media.gave_up_unknown_config());
}

/// §7.6: the give-up clock belongs to the HOLD, not to the session forever.
/// Once the reliable configuration releases the hold, a later tick must not
/// kill the session and a later known-generation datagram is not "unknown".
#[test]
fn a_released_hold_never_later_triggers_give_up() {
    let mut media = session();
    assert_eq!(
        media.on_datagram(0, &datagram(1, 2, 0, 0, 64)),
        DatagramOutcome::HeldUnknownConfig
    );
    media.apply_configuration(&config(2, 48_000, 2));
    assert!(!media.holding_unknown_config());
    // Long after the 2 s bound: the hold is gone, so no give-up.
    assert!(!media.tick(UNKNOWN_CONFIG_GIVE_UP_MS + 5_000));
    assert!(!media.gave_up_unknown_config());
    // A subsequent known-generation datagram is delivered, never "unknown".
    assert_eq!(
        media.on_datagram(0, &datagram(1, 2, 1, 64, 64)),
        DatagramOutcome::Delivered
    );
}

/// §7.5: malformed input (truncated header, zero instance, misaligned payload)
/// is dropped and counted, never panics.
#[test]
fn malformed_media_is_dropped() {
    let mut media = session();
    assert_eq!(media.on_datagram(0, &[0u8; 12]), DatagramOutcome::Malformed);
    let mut zero_instance = datagram(1, 1, 0, 0, 4);
    zero_instance[0..4].copy_from_slice(&0u32.to_be_bytes());
    assert_eq!(
        media.on_datagram(0, &zero_instance),
        DatagramOutcome::Malformed
    );
    let mut misaligned = datagram(1, 1, 0, 0, 4);
    misaligned.push(0);
    assert_eq!(
        media.on_datagram(0, &misaligned),
        DatagramOutcome::Malformed
    );
}

/// §7.5: a datagram of a different stream instance is not silently adopted —
/// the reliable `StreamStarted` push is the only way to start an epoch.
#[test]
fn another_instance_is_not_adopted_from_media() {
    let mut media = session();
    assert_eq!(
        media.on_datagram(0, &datagram(2, 1, 0, 0, 64)),
        DatagramOutcome::Malformed
    );
    assert_eq!(media.stream_instance_id(), 1);
    assert_eq!(media.delivered_frames(), 0);
}

/// The ring is the only PCM path: delivered frames land in the native ring and
/// appear at the JVM cursor.
#[test]
fn delivered_media_is_written_to_the_native_ring() {
    let mut backing = vec![0u8; 4096];
    let ring = unsafe { MediaRing::attach(backing.as_mut_ptr(), backing.len()) };
    let mut media =
        MediaSession::start(1, 1, 0, &config(1, 48_000, 2), Some(ring)).expect("geometry");
    assert_eq!(
        media.on_datagram(0, &datagram(1, 1, 0, 0, 64)),
        DatagramOutcome::Delivered
    );
    assert_eq!(media.delivered_frames(), 64);
}

/// The announced anchor is the instance's capture-clock origin: a late
/// Subscriber's submitted position is anchor-relative, not zero-relative.
#[test]
fn the_playback_position_is_anchor_relative() {
    let mut media =
        MediaSession::start(3, 1, 96_000, &config(1, 48_000, 2), None).expect("geometry");
    assert_eq!(media.anchor(), 96_000);
    media.record_playback(PlaybackReport {
        submitted_bytes: 8 * 128,
        underruns: 1,
        buffer_current_ms: 40,
    });
    assert_eq!(media.submitted_position(), 96_128);
    let diagnostics = media.diagnostics(60);
    assert_eq!(diagnostics.underruns, 1);
    assert_eq!(diagnostics.stream_instance_id, 3);
}
