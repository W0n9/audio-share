//! Resource limits (spec §10; threat-model §6, TM-1–TM-8).
//!
//! Pure guards: the accept loop and session layer call these to decide admit /
//! replace / reject. Values come from [`Limits`], defaults frozen in spec §10
//! and configurable in the documented ranges.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use crate::codes::ErrorCode;

/// The server-wide limit configuration (spec §10 / threat-model §6).
#[derive(Clone, Debug)]
pub struct Limits {
    /// Total established sessions, default 32, config range 1–256 (TM-2).
    pub max_sessions: u32,
    /// Concurrent handshakes in progress (TM-1).
    pub max_concurrent_handshakes: u32,
    /// Auth failures per connection before Close(203) (TM-4).
    pub max_auth_failures_per_connection: u32,
    /// Auth failures per source address per sliding window (TM-4): 20/60 s.
    pub auth_failures_per_window: u32,
    /// The sliding window for address-level auth failures (TM-4).
    pub auth_failure_window: Duration,
    /// Outstanding requests per session (spec §10); over → 302.
    pub max_outstanding_requests: u32,
}

impl Limits {
    /// The spec §10 defaults.
    pub const fn defaults() -> Self {
        Self {
            max_sessions: crate::codes::DEFAULT_MAX_SESSIONS,
            max_concurrent_handshakes: crate::codes::MAX_CONCURRENT_HANDSHAKES,
            max_auth_failures_per_connection: 5,
            auth_failures_per_window: 20,
            auth_failure_window: Duration::from_secs(60),
            max_outstanding_requests: crate::codes::MAX_OUTSTANDING_REQUESTS,
        }
    }

    /// Validate a configured instance against the documented ranges (§10:
    /// sessions 1–256).
    pub fn validate(&self) -> Result<(), String> {
        if self.max_sessions == 0 || self.max_sessions > 256 {
            return Err(format!(
                "max_sessions {} outside the documented range 1–256",
                self.max_sessions
            ));
        }
        if self.max_concurrent_handshakes == 0 {
            return Err("max_concurrent_handshakes must be at least 1".to_owned());
        }
        if self.max_auth_failures_per_connection == 0 {
            return Err("max_auth_failures_per_connection must be at least 1".to_owned());
        }
        if self.auth_failure_window.is_zero() {
            return Err("auth_failure_window must be greater than zero".to_owned());
        }
        Ok(())
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::defaults()
    }
}

/// The outcome of consulting the connection-capacity guards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapacityDecision {
    /// Admit the connection / establish the session.
    Admit,
    /// Reject: at the concurrent-handshake bound (TM-1) → transport close 301.
    HandshakeBound,
    /// Reject: at the established-session bound (TM-2) → transport close 301.
    SessionBound,
}

/// Tracks handshake and session counts against [`Limits`] (TM-1, TM-2).
///
/// A connection enters as a handshake and becomes a session after its control
/// handshake completes. The accept loop owns the transitions; the counters are
/// shared (`Clone`able guard) so a per-connection task can release its session
/// slot when the session ends (ticket 07).
#[derive(Clone, Debug)]
pub struct CapacityGuard {
    limits: Limits,
    counters: std::sync::Arc<std::sync::Mutex<CapacityCounters>>,
}

#[derive(Debug, Default)]
struct CapacityCounters {
    handshakes_in_progress: u32,
    established_sessions: u32,
}

impl CapacityGuard {
    /// Create a guard for these limits.
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            counters: std::sync::Arc::new(std::sync::Mutex::new(CapacityCounters::default())),
        }
    }

    fn counters(&self) -> std::sync::MutexGuard<'_, CapacityCounters> {
        self.counters.lock().expect("capacity guard lock")
    }

    /// The configured limits.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Consult whether a new incoming connection may begin its handshake.
    pub fn try_begin_handshake(&mut self) -> CapacityDecision {
        let mut counters = self.counters();
        if counters.established_sessions >= self.limits.max_sessions {
            return CapacityDecision::SessionBound;
        }
        if counters.handshakes_in_progress >= self.limits.max_concurrent_handshakes {
            return CapacityDecision::HandshakeBound;
        }
        counters.handshakes_in_progress += 1;
        CapacityDecision::Admit
    }

    /// The handshake completed: it now counts as an established session
    /// instead of an in-progress handshake.
    pub fn handshake_completed(&mut self) {
        let mut counters = self.counters();
        debug_assert!(counters.handshakes_in_progress > 0);
        counters.handshakes_in_progress = counters.handshakes_in_progress.saturating_sub(1);
        counters.established_sessions = counters.established_sessions.saturating_add(1);
    }

    /// The handshake failed or was refused; release the in-progress slot.
    pub fn handshake_abandoned(&mut self) {
        let mut counters = self.counters();
        counters.handshakes_in_progress = counters.handshakes_in_progress.saturating_sub(1);
    }

    /// A session ended; release its slot.
    pub fn session_ended(&mut self) {
        let mut counters = self.counters();
        counters.established_sessions = counters.established_sessions.saturating_sub(1);
    }

    /// Current in-progress handshake count (diagnostics).
    pub fn handshakes_in_progress(&self) -> u32 {
        self.counters().handshakes_in_progress
    }

    /// Current established-session count (diagnostics).
    pub fn established_sessions(&self) -> u32 {
        self.counters().established_sessions
    }

    /// The §6 code a [`CapacityDecision`] rejection signals (301 for both —
    /// "Server at capacity (total sessions or pre-auth sessions)").
    pub const fn decision_error_code(decision: CapacityDecision) -> ErrorCode {
        let _ = decision;
        ErrorCode::BUSY
    }
}

/// Auth-failure accounting per connection (TM-4): 5 failures then Close(203).
#[derive(Debug)]
pub struct PerConnectionAuthFailures {
    limit: u32,
    count: u32,
}

impl PerConnectionAuthFailures {
    /// Create with the limit from [`Limits::max_auth_failures_per_connection`].
    pub fn new(limit: u32) -> Self {
        Self { limit, count: 0 }
    }

    /// Record one failure; `true` when the failure exhausted the budget (the
    /// caller answers with `Close(203)`).
    pub fn record_failure(&mut self) -> bool {
        self.count += 1;
        self.count >= self.limit
    }

    /// Failures recorded so far (diagnostics).
    pub fn count(&self) -> u32 {
        self.count
    }

    /// Whether the connection is still below the failure bound.
    pub fn exhausted(&self) -> bool {
        self.count >= self.limit
    }
}

/// Sliding-window rate accounting keyed by source address (TM-4: 20 auth
/// failures per sliding 60 s per source → 302).
///
/// Entries are pruned lazily as the window slides; the map holds at most one
/// entry per address that failed recently, which is bounded by the number of
/// distinct sources seen inside the window.
#[derive(Debug)]
pub struct AddressRateGuard {
    window: Duration,
    limit: u32,
    events: HashMap<IpAddr, Vec<Instant>>,
}

impl AddressRateGuard {
    /// Create a guard with this window and per-address event limit.
    pub fn new(window: Duration, limit: u32) -> Self {
        Self { window, limit, events: HashMap::new() }
    }

    /// Record an event for the address; `true` when the address has now
    /// reached the limit within the sliding window (caller answers 302).
    pub fn record(&mut self, address: IpAddr, now: Instant) -> bool {
        let window = self.window;
        let reached = {
            let entry = self.events.entry(address).or_default();
            entry.retain(|instant| now.duration_since(*instant) < window);
            entry.push(now);
            entry.len() >= self.limit as usize
        };
        self.prune_stale(now);
        reached
    }

    /// Whether the address is currently over the limit without recording a
    /// new event (pre-check before doing per-request work).
    pub fn is_limited(&self, address: IpAddr, now: Instant) -> bool {
        self.events
            .get(&address)
            .map(|entry| {
                entry.iter().filter(|instant| now.duration_since(**instant) < self.window).count()
                    >= self.limit as usize
            })
            .unwrap_or(false)
    }

    /// Drop addresses whose newest event is entirely outside the window.
    fn prune_stale(&mut self, now: Instant) {
        self.events.retain(|_, entry| {
            entry.iter().any(|instant| now.duration_since(*instant) < self.window)
        });
    }

    /// Distinct addresses currently tracked (diagnostics).
    pub fn tracked_addresses(&self) -> usize {
        self.events.len()
    }
}

/// The §6 code for an exhausted address rate window: 302 (TM-4).
pub const RATE_LIMITED: ErrorCode = ErrorCode::RATE_LIMITED;

/// Validate an error payload bound (TM-7): post-auth errors ≤ 512 B with
/// `detail` ≤ 256 bytes. The caller passes the would-be serialized size.
pub fn error_payload_within_bounds(total_frame_bytes: usize, detail_bytes: usize) -> bool {
    detail_bytes <= crate::codes::MAX_DETAIL_BYTES && total_frame_bytes <= 512
}

/// Validate a pre-auth response bound (TM-6): general pre-auth responses
/// (framed, length prefix included) ≤ 128 B with no `detail`.
pub fn preauth_response_within_bounds(framed_bytes: usize) -> bool {
    framed_bytes <= 128
}

/// Validate the successful-PairingResponse exception (TM-6): ≤ 320 B
/// `Frame.length`.
pub fn pairing_response_within_bounds(frame_length: usize) -> bool {
    frame_length <= 320
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec_10() {
        let limits = Limits::defaults();
        assert_eq!(limits.max_sessions, 32);
        assert_eq!(limits.max_concurrent_handshakes, 16);
        assert_eq!(limits.max_auth_failures_per_connection, 5);
        assert_eq!(limits.auth_failures_per_window, 20);
        assert_eq!(limits.auth_failure_window, Duration::from_secs(60));
        assert_eq!(limits.max_outstanding_requests, 8);
        limits.validate().expect("defaults are valid");
    }

    #[test]
    fn validate_rejects_out_of_range_sessions() {
        let mut limits = Limits::defaults();
        limits.max_sessions = 0;
        assert!(limits.validate().is_err());
        limits.max_sessions = 257;
        assert!(limits.validate().is_err());
        limits.max_sessions = 256;
        assert!(limits.validate().is_ok());
        limits.max_sessions = 1;
        assert!(limits.validate().is_ok());
    }

    #[test]
    fn capacity_guard_admits_up_to_bounds() {
        let mut guard = CapacityGuard::new(Limits::defaults());
        // TM-2: 32 established sessions.
        for _ in 0..32 {
            assert_eq!(guard.try_begin_handshake(), CapacityDecision::Admit);
            guard.handshake_completed();
        }
        assert_eq!(guard.established_sessions(), 32);
        // 33rd connection: session bound → 301.
        assert_eq!(guard.try_begin_handshake(), CapacityDecision::SessionBound);
        assert_eq!(
            CapacityGuard::decision_error_code(CapacityDecision::SessionBound),
            ErrorCode::BUSY
        );
        assert_eq!(
            CapacityGuard::decision_error_code(CapacityDecision::HandshakeBound),
            ErrorCode::BUSY
        );
        guard.session_ended();
        assert_eq!(guard.try_begin_handshake(), CapacityDecision::Admit);
    }

    #[test]
    fn capacity_guard_enforces_handshake_bound() {
        let mut guard = CapacityGuard::new(Limits::defaults());
        // TM-1: 16 concurrent handshakes (sessions not yet established).
        for _ in 0..16 {
            assert_eq!(guard.try_begin_handshake(), CapacityDecision::Admit);
        }
        assert_eq!(guard.try_begin_handshake(), CapacityDecision::HandshakeBound);
        // Abandoning releases the slot.
        guard.handshake_abandoned();
        assert_eq!(guard.try_begin_handshake(), CapacityDecision::Admit);
    }

    #[test]
    fn capacity_guard_session_bound_outranks_handshake_bound() {
        let mut guard = CapacityGuard::new(Limits { max_sessions: 1, ..Limits::defaults() });
        assert_eq!(guard.try_begin_handshake(), CapacityDecision::Admit);
        guard.handshake_completed();
        // 1 established session fills the whole table: 301 regardless of the
        // handshake slots still free.
        assert_eq!(guard.try_begin_handshake(), CapacityDecision::SessionBound);
    }

    #[test]
    fn per_connection_failures_trip_at_five() {
        let mut failures = PerConnectionAuthFailures::new(5);
        for attempt in 1..=4 {
            assert!(!failures.record_failure(), "failure {attempt} must not close");
            assert!(!failures.exhausted());
        }
        assert!(failures.record_failure(), "the 5th failure closes (TM-4)");
        assert!(failures.exhausted());
        assert_eq!(failures.count(), 5);
    }

    #[test]
    fn address_rate_guard_trips_at_twenty_in_window() {
        let mut guard = AddressRateGuard::new(Duration::from_secs(60), 20);
        let address = "192.168.1.50".parse().unwrap();
        let mut now = Instant::now();
        for _ in 0..19 {
            assert!(!guard.record(address, now));
        }
        assert!(!guard.is_limited(address, now));
        now += Duration::from_millis(500);
        assert!(guard.record(address, now), "the 20th failure trips 302");
        assert!(guard.is_limited(address, now));
    }

    #[test]
    fn address_rate_guard_window_slides() {
        let mut guard = AddressRateGuard::new(Duration::from_secs(60), 20);
        let address = "192.168.1.50".parse().unwrap();
        let now = Instant::now();
        for _ in 0..20 {
            guard.record(address, now);
        }
        assert!(guard.is_limited(address, now));
        // 61 s later every event has aged out of the sliding window.
        let later = now + Duration::from_secs(61);
        assert!(!guard.is_limited(address, later));
        assert!(!guard.record(address, later), "fresh window admits again");
    }

    #[test]
    fn address_rate_guard_is_per_address() {
        let mut guard = AddressRateGuard::new(Duration::from_secs(60), 20);
        let a = "10.0.0.1".parse().unwrap();
        let b = "10.0.0.2".parse().unwrap();
        let now = Instant::now();
        for _ in 0..20 {
            guard.record(a, now);
        }
        assert!(guard.is_limited(a, now));
        assert!(!guard.is_limited(b, now), "another source is unaffected (TM-4)");
        assert_eq!(guard.tracked_addresses(), 1);
    }

    #[test]
    fn payload_bounds_match_tm_rows() {
        // TM-6: framed pre-auth ≤ 128 B.
        assert!(preauth_response_within_bounds(128));
        assert!(!preauth_response_within_bounds(129));
        // TM-6 exception: successful PairingResponse ≤ 320 B Frame.length.
        assert!(pairing_response_within_bounds(320));
        assert!(!pairing_response_within_bounds(321));
        // TM-7: post-auth error ≤ 512 B total, detail ≤ 256 B.
        assert!(error_payload_within_bounds(512, 256));
        assert!(!error_payload_within_bounds(513, 256));
        assert!(!error_payload_within_bounds(512, 257));
    }
}
