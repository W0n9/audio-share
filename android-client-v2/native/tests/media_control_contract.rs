//! Ticket 14 media cadence through `MediaControl` (spec §5.3, §9).

use std::time::{Duration, Instant};

use audio_share_client_v2_native::control::ClientState;
use audio_share_client_v2_native::media_control::{MediaAction, MediaControl, DIAGNOSTICS_INTERVAL};

#[test]
fn ready_with_configuration_subscribes_once() {
    let mut control = MediaControl::new();
    let now = Instant::now();
    assert_eq!(
        control.on_tick(&ClientState::Ready, true, now),
        MediaAction::Subscribe
    );
    assert!(
        control.subscribed(),
        "the session records that it asked to subscribe"
    );
    assert_eq!(
        control.on_tick(&ClientState::Ready, true, now),
        MediaAction::None,
        "a second Subscribe would be a protocol error (non-fatal 402)"
    );
}

#[test]
fn ready_without_a_configuration_waits() {
    let mut control = MediaControl::new();
    assert_eq!(
        control.on_tick(&ClientState::Ready, false, Instant::now()),
        MediaAction::None,
        "never a silent media wait: Subscribe waits for the first publication"
    );
    assert!(!control.subscribed());
}

#[test]
fn playing_reports_every_five_seconds_but_not_while_one_is_pending() {
    let mut control = MediaControl::new();
    let start = Instant::now();
    assert_eq!(
        control.on_tick(&ClientState::Playing, true, start),
        MediaAction::ReportDiagnostics
    );
    // A send is pending: the next cycle is skipped, never queued.
    assert_eq!(
        control.on_tick(
            &ClientState::Playing,
            true,
            start + DIAGNOSTICS_INTERVAL
        ),
        MediaAction::None
    );
    control.report_finished();
    assert_eq!(
        control.on_tick(
            &ClientState::Playing,
            true,
            start + DIAGNOSTICS_INTERVAL + Duration::from_secs(1)
        ),
        MediaAction::ReportDiagnostics,
        "the skipped cycle reports as soon as the previous send completes"
    );
    control.report_finished();
    assert_eq!(
        control.on_tick(
            &ClientState::Playing,
            true,
            start + DIAGNOSTICS_INTERVAL + DIAGNOSTICS_INTERVAL
        ),
        MediaAction::None,
        "the 5 s cadence restarts from the last successful report"
    );
    assert_eq!(
        control.on_tick(
            &ClientState::Playing,
            true,
            start + DIAGNOSTICS_INTERVAL + DIAGNOSTICS_INTERVAL + Duration::from_secs(5)
        ),
        MediaAction::ReportDiagnostics
    );
}

#[test]
fn a_capture_end_allows_the_next_stream_to_subscribe_again() {
    let mut control = MediaControl::new();
    let now = Instant::now();
    assert_eq!(
        control.on_tick(&ClientState::Ready, true, now),
        MediaAction::Subscribe
    );
    control.stream_ended();
    assert_eq!(
        control.on_tick(&ClientState::Ready, true, now),
        MediaAction::Subscribe,
        "a new stream instance reaches the still-subscribed Client (§5.2)"
    );
}

/// Non-playing states do not run the 5 s periodic cadence. CLOSING is the
/// exception: §9 requires one best-effort final report there, covered by
/// `media_control_closing.rs`.
#[test]
fn states_other_than_playing_never_report_periodically() {
    let mut control = MediaControl::new();
    for state in [
        ClientState::Idle,
        ClientState::Authenticating,
        ClientState::Negotiating,
        ClientState::Closed,
    ] {
        assert_eq!(
            control.on_tick(&state, true, Instant::now()),
            MediaAction::None,
            "{state:?}"
        );
    }
}
