//! M5-03 bounded jitter buffer contract (RED → GREEN).
//!
//! Expected values come from `docs/wire-protocol-v2/spec.md` §8/§9, never from
//! the implementation's own bookkeeping. Deterministic: datagrams are built
//! with `MediaHeader::write_be` and the session clock is injected via
//! `on_datagram(now_ms, ..)`.

use audio_share_client_v2_native::media::{MediaHeader, FLAG_STREAM_START};
use audio_share_client_v2_native::media_session::{
    ConcealmentKind, DatagramOutcome, JitterConfig, MediaSession, PlaybackReport,
};
use audio_share_client_v2_native::proto::{Configuration, PcmParameters};
use audio_share_client_v2_native::ring::MediaRing;

const RATE: u32 = 48_000;
const FRAME_BYTES: usize = 8; // F32LE stereo
const FRAMES_PER_DATAGRAM: usize = 64;

fn config(generation: u32) -> Configuration {
    Configuration {
        config_generation: generation,
        codec: 1,
        pcm: Some(PcmParameters {
            sample_rate: RATE,
            channels: 2,
            sample_format: 1, // F32LE
        }),
    }
}

fn datagram_filled(sequence: u32, timestamp: u64, byte: u8) -> Vec<u8> {
    let header = MediaHeader {
        stream_instance_id: 1,
        config_generation: 1,
        sequence,
        timestamp,
        flags: if sequence == 0 { FLAG_STREAM_START } else { 0 },
    };
    let mut out = Vec::new();
    header.write_be(&mut out);
    out.extend(std::iter::repeat_n(byte, FRAMES_PER_DATAGRAM * FRAME_BYTES));
    out
}

/// A datagram for an explicit generation and payload size (used by the
/// configuration-rebuild test, whose geometry differs from the fixture).
fn datagram_with(
    generation: u32,
    sequence: u32,
    timestamp: u64,
    payload_len: usize,
    byte: u8,
) -> Vec<u8> {
    let header = MediaHeader {
        stream_instance_id: 1,
        config_generation: generation,
        sequence,
        timestamp,
        flags: if sequence == 0 { FLAG_STREAM_START } else { 0 },
    };
    let mut out = Vec::new();
    header.write_be(&mut out);
    out.extend(std::iter::repeat_n(byte, payload_len));
    out
}

fn session_with(backing: &mut Vec<u8>) -> MediaSession {
    let ring = unsafe { MediaRing::attach(backing.as_mut_ptr(), backing.len()) };
    MediaSession::start(1, 1, 0, &config(1), Some(ring)).expect("F32LE stereo")
}

// (a) target/hard-limit config defaults are the normative §8 values.
#[test]
fn jitter_config_defaults_are_normative_values() {
    let cfg = JitterConfig::default();
    assert_eq!(cfg.target_ms, 60, "D12: target 60 ms");
    assert_eq!(cfg.hard_limit_ms, 200, "§8: hard limit 200 ms");
    assert_eq!(cfg.target_frames(RATE), 2_880, "60 ms @ 48 kHz");
    assert_eq!(cfg.hard_limit_frames(RATE), 9_600, "200 ms @ 48 kHz");
    // A configured override must not disturb the defaults.
    let custom = JitterConfig {
        target_ms: 40,
        hard_limit_ms: 120,
    };
    assert_eq!(custom.target_frames(RATE), 1_920);
    assert_eq!(custom.hard_limit_frames(RATE), 5_760);
    assert_eq!(JitterConfig::default().target_ms, 60);
}

// (b) network loss (sequence gap) is counted as lost, never as a late drop.
#[test]
fn network_loss_is_not_a_late_drop() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    // seq 0 commits; seq 2 leaves a one-datagram hole (seq 1 lost).
    assert_eq!(
        media.on_datagram(0, &datagram_filled(0, 0, 0x7f)),
        DatagramOutcome::Delivered
    );
    assert_eq!(
        media.on_datagram(0, &datagram_filled(2, 128, 0x7f)),
        DatagramOutcome::Delivered
    );
    let d = media.diagnostics(60);
    assert_eq!(d.cumulative_lost, 1, "one datagram lost on the wire");
    assert_eq!(d.cumulative_late_drops, 0, "a wire gap is not a late drop");
}

// (b) a datagram that arrives after the playout frontier passed its timestamp
//     is a late drop, counted separately from loss and never written to ring.
#[test]
fn a_datagram_after_its_playout_deadline_is_a_late_drop() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    // seq 0 at ts 0 commits (frontier 64).
    assert_eq!(
        media.on_datagram(0, &datagram_filled(0, 0, 0x7f)),
        DatagramOutcome::Delivered
    );
    // A far-ahead run fills the 60 ms target, forcing the hole (ts 64..128)
    // to be concealment-filled and the frontier to race ahead.
    for i in 0..46u32 {
        let seq = 2 + i;
        let ts = 128 + u64::from(i) * 64;
        assert_eq!(
            media.on_datagram(0, &datagram_filled(seq, ts, 0x7f)),
            DatagramOutcome::Delivered
        );
    }
    // Now seq 1 (ts 64) arrives long after its slot was passed.
    assert_eq!(
        media.on_datagram(0, &datagram_filled(1, 64, 0x7f)),
        DatagramOutcome::LateDropped
    );
    let d = media.diagnostics(60);
    assert_eq!(d.cumulative_late_drops, 1);
    assert_eq!(d.interval_late_drops, 1);
}

// Held datagrams released by a config change must go through the SAME jitter
// frontier as live media. Otherwise their sequences are written directly to
// the ring while the frontier still waits at the old generation's position,
// and the next live datagram triggers a bogus gap fill + stall.
#[test]
fn held_datagrams_released_on_config_change_keep_the_frontier_in_sync() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    // A gen-1 live frame commits the frontier at seq 1 (expects seq 1 next).
    assert_eq!(
        media.on_datagram(0, &datagram_filled(0, 0, 0x7f)),
        DatagramOutcome::Delivered
    );
    // Gen-2 media at seq 1 arrives before its configuration: held, not decoded.
    let mut held = datagram_filled(1, 64, 0x11);
    held[4..8].copy_from_slice(&2u32.to_be_bytes());
    assert_eq!(
        media.on_datagram(0, &held),
        DatagramOutcome::HeldUnknownConfig
    );
    assert_eq!(
        media.delivered_frames(),
        64,
        "held media is not decoded early"
    );

    // The reliable config releases the held frame. It must advance the jitter
    // frontier to seq 2; otherwise the frontier still expects seq 1.
    media.apply_configuration(&config(2));

    // Push a contiguous gen-2 run far enough to exceed the 60 ms target span.
    // If the held frame bypassed the frontier, seq 1 looks like a hole that was
    // never filled → a bogus 64-frame silence gap for media already in the ring.
    for i in 0..46u32 {
        let seq = 2 + i;
        let mut d = datagram_filled(seq, 128 + u64::from(i) * 64, 0x22);
        d[4..8].copy_from_slice(&2u32.to_be_bytes());
        assert_eq!(media.on_datagram(0, &d), DatagramOutcome::Delivered);
    }
    let d = media.diagnostics(60);
    assert_eq!(
        d.cumulative_concealments, 0,
        "no duplicate silence at config change"
    );
    assert_eq!(d.concealed_frames, 0);
    assert_eq!(d.cumulative_late_drops, 0);
    // One gen-1 + one held + 46 live datagrams, each written exactly once.
    assert_eq!(media.delivered_frames(), 48 * 64);
}

// M5-05 handoff: server-side pre-packetize / MediaTap-full drops re-anchor
// the packetizer timestamp to chunk.start_frame, so `timestamp` jumps while
// `sequence` stays contiguous. The client must NOT report these as network
// loss (no sequence gap), late drops, or concealment — it simply re-bases its
// expected capture-clock position on the actual timestamp.
#[test]
fn a_server_side_drop_is_a_contiguous_sequence_with_jumped_timestamp() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    // seq 0 at ts 0, then seq 1 jumps to ts 1064 (a 1000-frame server drop).
    assert_eq!(
        media.on_datagram(0, &datagram_filled(0, 0, 0x7f)),
        DatagramOutcome::Delivered
    );
    assert_eq!(
        media.on_datagram(0, &datagram_filled(1, 1064, 0x7f)),
        DatagramOutcome::Delivered
    );
    let d = media.diagnostics(60);
    assert_eq!(
        d.cumulative_lost, 0,
        "sequence is contiguous — not network loss"
    );
    assert_eq!(d.cumulative_late_drops, 0, "no playout deadline was missed");
    assert_eq!(
        d.cumulative_concealments, 0,
        "no silence inserted for a server drop"
    );
    assert_eq!(d.concealed_frames, 0);
}

// (c)+(d) a timestamp gap is filled with all-zero PCM (active format) and
//          counted as one concealment with concealed_frames = inserted.
#[test]
fn a_gap_is_concealed_with_silence_and_counted() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    assert_eq!(
        media.on_datagram(0, &datagram_filled(0, 0, 0x7f)),
        DatagramOutcome::Delivered
    );
    assert_eq!(
        media.on_datagram(0, &datagram_filled(2, 128, 0x7f)),
        DatagramOutcome::Delivered
    );
    // Push the buffer past the 60 ms target so the hole is given up on.
    for i in 0..45u32 {
        let seq = 3 + i;
        let ts = 192 + u64::from(i) * 64;
        assert_eq!(
            media.on_datagram(0, &datagram_filled(seq, ts, 0x7f)),
            DatagramOutcome::Delivered
        );
    }
    let d = media.diagnostics(60);
    assert_eq!(d.cumulative_concealments, 1, "one gap-fill intervention");
    assert_eq!(d.interval_concealments, 1);
    assert_eq!(d.concealed_frames, 64, "64 frames of silence inserted");
    // The gap region (frames 64..128 → bytes 512..1024) must be all zeros,
    // while real datagrams carry 0x7f.
    assert_eq!(
        &backing[512..1024],
        &vec![0u8; 512],
        "gap filled with silence"
    );
}

// (d) underrun fill counts as its own intervention through the M5-04-ready hook.
#[test]
fn underrun_and_drift_concealments_share_one_counter_api() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    media.record_concealment(ConcealmentKind::Underrun, 10);
    media.record_concealment(ConcealmentKind::Drift, -3);
    let d = media.diagnostics(60);
    assert_eq!(
        d.cumulative_concealments, 2,
        "each intervention counts once"
    );
    assert_eq!(d.concealed_frames, 7, "inserted − deleted = 10 − 3");
}

// M5-04: a hard-limit re-anchor trim is counted as ReAnchor concealment while
// the trim diagnostics stay separate (frames crossed / events).
#[test]
fn a_hard_limit_re_anchor_trim_is_reanchor_concealment() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    media.record_trim(1_000);
    let d = media.diagnostics(60);
    assert_eq!(d.cumulative_concealments, 1, "§8 counts the deletion");
    assert_eq!(d.concealed_frames, -1_000, "deleted frames are negative");
    assert_eq!(media.trimmed_frames(), 1_000);
    assert_eq!(media.trim_events(), 1);
}

// (e) jitter is measured (RFC 3550 A.8) at the published rate, not hard-coded 0.
#[test]
fn jitter_estimate_is_measured_at_the_published_sample_rate() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    // Arrivals drift away from the timestamp cadence → non-zero interarrival J.
    // Each datagram is 64 frames = 64/48000 s ≈ 1.333 ms of media, but arrives
    // 5 ms apart in session time.
    for seq in 0..12u32 {
        let now_ms = u64::from(seq) * 5;
        let ts = u64::from(seq) * 64;
        assert_eq!(
            media.on_datagram(now_ms, &datagram_filled(seq, ts, 0x7f)),
            DatagramOutcome::Delivered
        );
    }
    let d = media.diagnostics(60);
    assert!(
        d.jitter_microseconds > 0,
        "a real drift must produce jitter > 0"
    );
}

// (e) diagnostics returns measured values and resets only interval counters.
#[test]
fn diagnostics_reports_real_values_and_resets_intervals_only() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    media.record_concealment(ConcealmentKind::Underrun, 5);
    media.on_datagram(0, &datagram_filled(0, 0, 0x7f));
    media.record_playback(PlaybackReport {
        submitted_bytes: 0,
        underruns: 0,
        buffer_current_ms: 25,
    });
    let first = media.diagnostics(60);
    assert_eq!(first.cumulative_concealments, 1);
    assert_eq!(first.interval_concealments, 1);
    assert_eq!(
        first.submitted_position, 0,
        "no platform bytes, no progress"
    );
    assert_eq!(first.buffer_target_ms, 60);
    let second = media.diagnostics(60);
    assert_eq!(
        second.interval_concealments, 0,
        "interval reset after report"
    );
    assert_eq!(
        second.cumulative_concealments, 1,
        "cumulative survives report"
    );
}

// (b, deadline) A sender that pauses leaves the frontier parked. The next
// datagram's slot is long past by wall clock, so it must be a counted late
// drop even though the sequence is contiguous with the frontier.
#[test]
fn a_late_arrival_after_a_sender_pause_is_late_dropped() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    assert_eq!(
        media.on_datagram(0, &datagram_filled(0, 0, 0x7f)),
        DatagramOutcome::Delivered
    );
    let before = media.delivered_frames();
    assert_eq!(
        media.on_datagram(5_000, &datagram_filled(1, 64, 0x7f)),
        DatagramOutcome::LateDropped,
        "5 s of silence with an unchanged capture clock is a late arrival"
    );
    assert_eq!(
        media.delivered_frames(),
        before,
        "late media is never written"
    );
    let d = media.diagnostics(60);
    assert_eq!(d.cumulative_late_drops, 1);
    assert_eq!(d.cumulative_lost, 0, "a late arrival is not network loss");
}

// (b, reorder) A reorder that stays inside the 60 ms target still plays: the
// playout deadline is target-relative, not a strict sequence order.
#[test]
fn a_timely_reorder_still_plays_under_the_deadline_rule() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    assert_eq!(
        media.on_datagram(0, &datagram_filled(0, 0, 0x7f)),
        DatagramOutcome::Delivered
    );
    assert_eq!(
        media.on_datagram(40, &datagram_filled(2, 128, 0x7f)),
        DatagramOutcome::Delivered
    );
    assert_eq!(
        media.on_datagram(50, &datagram_filled(1, 64, 0x7f)),
        DatagramOutcome::Delivered,
        "seq 1 arrives well before its 60 ms deadline"
    );
    assert_eq!(media.delivered_frames(), 3 * 64);
    let d = media.diagnostics(60);
    assert_eq!(d.cumulative_late_drops, 0);
    assert_eq!(d.cumulative_concealments, 0);
}

// A.8 must not count a held datagram's release as a new arrival (its old
// arrival instant would inject a bogus step into the estimate).
#[test]
fn releasing_held_media_does_not_inflate_the_jitter_estimate() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    assert_eq!(
        media.on_datagram(0, &datagram_filled(0, 0, 0x7f)),
        DatagramOutcome::Delivered
    );
    let mut held = datagram_filled(1, 64, 0x11);
    held[4..8].copy_from_slice(&2u32.to_be_bytes());
    assert_eq!(
        media.on_datagram(0, &held),
        DatagramOutcome::HeldUnknownConfig
    );
    media.apply_configuration(&config(2));
    let d = media.diagnostics(60);
    assert_eq!(d.jitter_microseconds, 0, "release is not an arrival");
    assert_eq!(media.delivered_frames(), 2 * 64);
}

// A timestamp re-base (server-side drop) is a position step, not interarrival
// jitter: the A.8 window must be discarded.
#[test]
fn a_timestamp_rebase_does_not_inflate_the_jitter_estimate() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    assert_eq!(
        media.on_datagram(0, &datagram_filled(0, 0, 0x7f)),
        DatagramOutcome::Delivered
    );
    assert_eq!(
        media.on_datagram(1, &datagram_filled(1, 1_064, 0x7f)),
        DatagramOutcome::Delivered,
        "contiguous sequence with a jumped timestamp"
    );
    let d = media.diagnostics(60);
    assert_eq!(d.jitter_microseconds, 0, "the re-base resets A.8");
    assert_eq!(d.cumulative_lost, 0);
    assert_eq!(d.cumulative_concealments, 0);
}

// §8 underrun fill: the playback loop fed silence because no data was ready.
// It is a concealment intervention, separate from the platform `underruns`
// counter, and inserted frames carry no capture position.
#[test]
fn an_underrun_fill_is_counted_as_its_own_intervention() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    media.record_underrun(96);
    let d = media.diagnostics(60);
    assert_eq!(d.cumulative_concealments, 1);
    assert_eq!(d.concealed_frames, 96, "inserted silence is positive");
    assert_eq!(d.underruns, 0, "the platform field is not double-counted");
    assert_eq!(media.concealment_counts().underrun, 1);
    assert_eq!(media.concealment_counts().gap, 0);

    // Those 96 frames were silence with no capture position: reporting the
    // platform-accepted bytes must not advance `submitted_position`.
    media.record_playback(PlaybackReport {
        submitted_bytes: 96 * FRAME_BYTES as u64,
        underruns: 0,
        buffer_current_ms: 60,
    });
    assert_eq!(media.submitted_position(), 0);
}

#[test]
fn concealment_kinds_are_kept_as_a_breakdown() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    media.record_concealment(ConcealmentKind::Gap, 64);
    media.record_concealment(ConcealmentKind::Underrun, 10);
    media.record_concealment(ConcealmentKind::Drift, -3);
    media.record_concealment(ConcealmentKind::ReAnchor, -5);
    let counts = media.concealment_counts();
    assert_eq!(counts.gap, 1);
    assert_eq!(counts.underrun, 1);
    assert_eq!(counts.drift, 1);
    assert_eq!(counts.reanchor, 1);
    assert_eq!(counts.total(), 4);
    let d = media.diagnostics(60);
    assert_eq!(d.cumulative_concealments, 4, "the §9 total is unchanged");
}

// M5-03: a soft trim above the target is normal buffer steering, counted
// separately and NOT as concealment; the playout cursor still advances over
// the skipped capture positions.
#[test]
fn a_soft_target_trim_is_not_concealment() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    media.record_soft_trim(240);
    let d = media.diagnostics(60);
    assert_eq!(
        d.cumulative_concealments, 0,
        "soft trim is not an intervention"
    );
    assert_eq!(d.concealed_frames, 0);
    assert_eq!(media.soft_trimmed_frames(), 240);
    assert_eq!(media.soft_trim_events(), 1);
    assert_eq!(
        media.trimmed_frames(),
        0,
        "not the hard-limit counter either"
    );
    assert_eq!(
        media.submitted_position(),
        240,
        "the cursor advanced over it"
    );
}

// Finding 6: Kotlin reads the active bounds from native instead of
// duplicating 60/200.
#[test]
fn jitter_bounds_are_exposed_from_the_active_configuration() {
    let mut backing = vec![0u8; 64 * 1024];
    let ring = unsafe { MediaRing::attach(backing.as_mut_ptr(), backing.len()) };
    let media = MediaSession::start_with_config(
        1,
        1,
        0,
        &config(1),
        Some(ring),
        JitterConfig {
            target_ms: 40,
            hard_limit_ms: 120,
        },
    )
    .expect("geometry");
    let active = media.jitter_config();
    assert_eq!(active.target_ms, 40);
    assert_eq!(active.hard_limit_ms, 120);
}

// Deadline-wedge regression: a late datagram used to return before refreshing
// the playout anchor, so with a stalled capture clock every later datagram was
// judged against the stale instant and stayed late forever. The anchor now
// refreshes on any live arrival; lateness is still judged against the
// pre-update anchor, so the first post-stall datagram is still a late drop.
#[test]
fn a_late_datagram_does_not_wedge_the_deadline_anchor() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    assert_eq!(
        media.on_datagram(0, &datagram_filled(0, 0, 0x7f)),
        DatagramOutcome::Delivered
    );
    assert_eq!(
        media.on_datagram(5_000, &datagram_filled(1, 64, 0x7f)),
        DatagramOutcome::LateDropped,
        "the first arrival after the stall is genuinely late"
    );
    // The capture clock is stalled, but the anchor refreshed at 5 s. With the
    // old behaviour every one of these was LateDropped.
    for i in 0..46u32 {
        let sequence = 2 + i;
        let timestamp = 128 + u64::from(i) * 64;
        let now_ms = 5_010 + u64::from(i) * 10;
        assert_eq!(
            media.on_datagram(now_ms, &datagram_filled(sequence, timestamp, 0x7f)),
            DatagramOutcome::Delivered,
            "datagram {sequence} must be accepted after the anchor refresh"
        );
    }
    let d = media.diagnostics(60);
    assert_eq!(
        d.cumulative_late_drops, 1,
        "only the wedged arrival was late"
    );
    assert_eq!(
        d.cumulative_concealments, 1,
        "the seq-1 hole is filled once"
    );
    // seq-0 + 46 live datagrams + one 64-frame silence fill.
    assert_eq!(media.delivered_frames(), 48 * 64);
}

// A platform `underruns` count is §9 evidence, not a concealment. Only the
// player's actual silence fill is an intervention, so reporting the same
// platform count repeatedly must not inflate `cumulative_concealments`.
#[test]
fn reporting_a_platform_underrun_is_not_a_concealment_by_itself() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    for _ in 0..3 {
        media.record_playback_at(
            0,
            PlaybackReport {
                submitted_bytes: 0,
                underruns: 2,
                buffer_current_ms: 5,
            },
        );
    }
    let d = media.diagnostics(60);
    assert_eq!(d.underruns, 2, "the platform field is reported as reported");
    assert_eq!(
        d.cumulative_concealments, 0,
        "no fill was performed, so no intervention is counted"
    );
}

// A decode-affecting configuration change rebuilds the jitter buffer. The
// target-frame / silence-fill / deadline math must use the NEW sample_rate and
// frame_bytes; with the previous ones the 60 ms target is wrong (and the
// silence-fill byte count is wrong by the frame-byte ratio).
#[test]
fn a_decode_affecting_config_change_rebuilds_the_jitter_buffer() {
    let mut backing = vec![0u8; 64 * 1024];
    let mut media = session_with(&mut backing);
    assert_eq!(
        media.on_datagram(0, &datagram_filled(0, 0, 0x7f)),
        DatagramOutcome::Delivered
    );

    // Generation 2: 24 kHz mono S16LE → 2 bytes/frame, 60 ms = 1440 frames.
    let mut next = config(2);
    {
        let pcm = next.pcm.as_mut().expect("pcm");
        pcm.sample_rate = 24_000;
        pcm.channels = 1;
        pcm.sample_format = 3; // S16LE
    }
    media.apply_configuration(&next);
    assert_eq!(media.geometry().sample_rate, 24_000);
    assert_eq!(media.geometry().frame_bytes(), 2);
    assert_eq!(media.jitter_config().target_ms, 60, "config is preserved");

    // New frontier at seq 1; 128 B = 64 frames at the new geometry.
    assert_eq!(
        media.on_datagram(0, &datagram_with(2, 1, 64, 128, 0x22)),
        DatagramOutcome::Delivered
    );
    // Hole at seq 2. 23 later datagrams give a 1536-frame span: above the NEW
    // 1440-frame target, below the stale 48000 Hz target (2880).
    for i in 0..23u32 {
        let sequence = 3 + i;
        let timestamp = 192 + u64::from(i) * 64;
        assert_eq!(
            media.on_datagram(0, &datagram_with(2, sequence, timestamp, 128, 0x22)),
            DatagramOutcome::Delivered
        );
    }
    let d = media.diagnostics(60);
    assert_eq!(
        d.cumulative_concealments, 1,
        "the gap is filled at the new 24 kHz target"
    );
    assert_eq!(d.concealed_frames, 64);
    // Ring layout: old seq-0 512 B, new seq-1 128 B, then the 64-frame gap.
    // The fill must be 64 × NEW frame_bytes (2) = 128 B, never 64 × 8.
    assert_eq!(&backing[512..640], &vec![0x22u8; 128]);
    assert_eq!(&backing[640..768], &vec![0u8; 128], "silence at 2 B/frame");
    assert_eq!(&backing[768..896], &vec![0x22u8; 128]);
}
