//! Ticket 14 session-level media control: when to subscribe, when to report.
//!
//! The §5.3 machine decides state; the session still has to decide *when* a
//! media-side action is due — subscribing after the first configuration, and
//! reporting diagnostics every 5 s plus once on close with at most one send
//! pending. Those are clock-and-count decisions, not protocol tables, so they
//! live here as one small interface the session layer drives.

use std::time::Duration;

use crate::control::ClientState;

/// §9: the Client reports playback diagnostics every five seconds.
pub const DIAGNOSTICS_INTERVAL: Duration = Duration::from_secs(5);

/// The media-side action the session should take on this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaAction {
    /// Send `Subscribe` (the first configuration is current, not yet requested).
    Subscribe,
    /// Send one `PlaybackDiagnostics` snapshot.
    ReportDiagnostics,
    /// Nothing to do.
    None,
}

/// Tracks the two media-side cadences for one session.
#[derive(Debug, Default)]
pub struct MediaControl {
    subscribed_once: bool,
    diagnostics_pending: bool,
    last_report: Option<std::time::Instant>,
    /// §9: one best-effort final snapshot on entering CLOSING, emitted once.
    /// It is independent of `diagnostics_pending` (a periodic send may still
    /// be in flight) and of the 5 s cadence.
    closing_report_sent: bool,
}

impl MediaControl {
    /// A fresh session: nothing subscribed, nothing reported.
    pub fn new() -> Self {
        Self::default()
    }

    /// The session reached a new state; returns what to send now.
    ///
    /// - READY with a decodable configuration and no subscription yet →
    ///   `Subscribe` (M3 shipped without a play button; M4's tracer bullet
    ///   must not stall in READY forever).
    /// - PLAYING with the 5 s cadence elapsed → `ReportDiagnostics`, unless a
    ///   send is already pending (then this cycle is skipped, never queued).
    /// - CLOSING → one best-effort `ReportDiagnostics` (§9), emitted exactly
    ///   once regardless of a pending periodic send or the 5 s cadence.
    pub fn on_tick(
        &mut self,
        state: &ClientState,
        has_configuration: bool,
        now: std::time::Instant,
    ) -> MediaAction {
        match state {
            ClientState::Ready if !self.subscribed_once && has_configuration => {
                self.subscribed_once = true;
                MediaAction::Subscribe
            }
            ClientState::Playing => {
                if self.diagnostics_pending {
                    MediaAction::None
                } else {
                    match self.last_report {
                        Some(last) if now.duration_since(last) < DIAGNOSTICS_INTERVAL => {
                            MediaAction::None
                        }
                        _ => {
                            self.last_report = Some(now);
                            self.diagnostics_pending = true;
                            MediaAction::ReportDiagnostics
                        }
                    }
                }
            }
            ClientState::Closing => {
                if self.closing_report_sent {
                    MediaAction::None
                } else {
                    self.closing_report_sent = true;
                    MediaAction::ReportDiagnostics
                }
            }
            _ => MediaAction::None,
        }
    }

    /// The pending diagnostics send completed or failed; either way the next
    /// cycle may report (statistics failures never block media).
    pub fn report_finished(&mut self) {
        self.diagnostics_pending = false;
    }

    /// A stream ended or the subscription was dropped: allow a later
    /// configuration to subscribe again, but never duplicate an in-flight one.
    pub fn stream_ended(&mut self) {
        self.subscribed_once = false;
        self.last_report = None;
    }

    /// Whether a subscription request has been issued for the current session.
    pub const fn subscribed(&self) -> bool {
        self.subscribed_once
    }
}
