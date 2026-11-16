//! Ticket 15 / §9: entering CLOSING emits one best-effort final diagnostics
//! report, never waits indefinitely, and never duplicates itself.

use audio_share_client_v2_native::control::ClientState;
use audio_share_client_v2_native::media_control::{
    MediaAction, MediaControl, DIAGNOSTICS_INTERVAL,
};

/// §5.3/§9: CLOSING is a report trigger even when the 5 s cadence has not
/// elapsed. READY/PLAYING/CLOSED are not.
#[test]
fn closing_requests_one_final_report() {
    let mut control = MediaControl::new();
    let now = std::time::Instant::now();
    assert_eq!(
        control.on_tick(&ClientState::Closing, true, now),
        MediaAction::ReportDiagnostics,
        "CLOSING must request the best-effort final §9 snapshot"
    );
    // Best-effort means one attempt only: repeated CLOSING ticks and CLOSED
    // must not queue another final snapshot.
    assert_eq!(
        control.on_tick(&ClientState::Closing, true, now),
        MediaAction::None
    );
    assert_eq!(
        control.on_tick(&ClientState::Closed, true, now),
        MediaAction::None
    );
}

/// A pending in-flight report does not suppress the closing snapshot, but the
/// report is still emitted at most once (§9: statistics never block teardown).
#[test]
fn a_pending_periodic_report_does_not_block_the_final_one() {
    let mut control = MediaControl::new();
    let now = std::time::Instant::now();
    assert_eq!(
        control.on_tick(&ClientState::Playing, true, now),
        MediaAction::ReportDiagnostics
    );
    // Periodic send still in flight: CLOSING asks for the final snapshot once.
    assert_eq!(
        control.on_tick(&ClientState::Closing, true, now + DIAGNOSTICS_INTERVAL),
        MediaAction::ReportDiagnostics
    );
    assert_eq!(
        control.on_tick(&ClientState::Closing, true, now),
        MediaAction::None
    );
}
