//! Non-waiting bounded send queue with honest application-level drop
//! accounting (spec §7.4; ADR-0004 consequence: quinn's internal queue evicts
//! oldest silently, so application queues are mandatory for honest drops).
//!
//! On overflow the **newest** datagram is dropped and counted locally (the
//! prototype-proven behavior, D10). A stalled or congested peer never blocks
//! the sender.

use std::collections::VecDeque;

/// A datagram queued for a non-waiting send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedDatagram {
    /// The application payload: the 24-byte BE header + PCM (spec §7.1).
    pub payload: Vec<u8>,
}

/// Application-level bounded queue for per-Client media datagrams (TM-8).
///
/// `send` never waits and never blocks the caller: a full queue drops the
/// incoming (newest) datagram and counts it. Drops are observable through
/// [`SendQueue::dropped`] for honest accounting.
#[derive(Debug)]
pub struct SendQueue {
    capacity: usize,
    inner: VecDeque<Vec<u8>>,
    dropped_newest: u64,
    dropped_total: u64,
    sent_total: u64,
    /// The QUIC datagram size ceiling; sends above it are re-queried at send
    /// time (D2) — a payload larger than the current limit fails the send and
    /// is counted dropped (spec §7.3 "oversized send" row).
    max_datagram_size: usize,
}

/// The result of a non-waiting [`SendQueue::send`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    /// The datagram was queued.
    Queued,
    /// The queue was full: the newest datagram (this one) was dropped and
    /// counted (spec §7.4).
    DroppedNewest,
    /// The payload exceeds the current datagram limit: dropped and counted
    /// (spec §7.3 "oversized send" row; media is drop-eligible).
    DroppedOversize,
}

impl SendQueue {
    /// Create a queue holding `capacity` datagrams with this send-time
    /// datagram limit.
    pub fn new(capacity: usize, max_datagram_size: usize) -> Self {
        Self {
            capacity,
            inner: VecDeque::with_capacity(capacity),
            dropped_newest: 0,
            dropped_total: 0,
            sent_total: 0,
            max_datagram_size,
        }
    }

    /// Queue one datagram without waiting (spec §7.4).
    pub fn send(&mut self, payload: Vec<u8>) -> SendOutcome {
        if payload.len() > self.max_datagram_size {
            self.dropped_newest += 1;
            self.dropped_total += 1;
            return SendOutcome::DroppedOversize;
        }
        if self.inner.len() >= self.capacity {
            // Drop the NEWEST: the incoming datagram is discarded, the queued
            // backlog keeps playing (prototype-proven, D10).
            self.dropped_newest += 1;
            self.dropped_total += 1;
            return SendOutcome::DroppedNewest;
        }
        self.inner.push_back(payload);
        self.sent_total += 1;
        SendOutcome::Queued
    }

    /// Pop the oldest queued datagram for the actual transport send.
    pub fn pop(&mut self) -> Option<Vec<u8>> {
        self.inner.pop_front()
    }

    /// Datagrams currently queued.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Whether the queue is at capacity.
    pub fn is_full(&self) -> bool {
        self.inner.len() >= self.capacity
    }

    /// Newest-drop count since the last reset — the honest application-level
    /// drop accounting ADR-0004 requires.
    pub fn dropped_newest(&self) -> u64 {
        self.dropped_newest
    }

    /// All drops (newest + oversize) since creation.
    pub fn dropped_total(&self) -> u64 {
        self.dropped_total
    }

    /// Datagrams accepted into the queue since creation.
    pub fn sent_total(&self) -> u64 {
        self.sent_total
    }

    /// The send-time datagram size limit (re-queried per send, D2).
    pub fn max_datagram_size(&self) -> usize {
        self.max_datagram_size
    }

    /// Reset the accounting counters (per stream-instance change, spec §9).
    pub fn reset_accounting(&mut self) {
        self.dropped_newest = 0;
        self.sent_total = 0;
    }

    /// Drop everything queued (session teardown).
    pub fn clear(&mut self) {
        self.inner.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(tag: u8) -> Vec<u8> {
        vec![tag; 8]
    }

    #[test]
    fn queue_holds_up_to_capacity() {
        let mut queue = SendQueue::new(4, 1200);
        for tag in 0..4 {
            assert_eq!(queue.send(payload(tag)), SendOutcome::Queued);
        }
        assert_eq!(queue.len(), 4);
        assert!(queue.is_full());
    }

    #[test]
    fn overflow_drops_newest_not_oldest() {
        // Spec §7.4: "on overflow the newest is dropped and counted locally".
        let mut queue = SendQueue::new(2, 1200);
        assert_eq!(queue.send(payload(1)), SendOutcome::Queued);
        assert_eq!(queue.send(payload(2)), SendOutcome::Queued);
        assert_eq!(queue.send(payload(3)), SendOutcome::DroppedNewest);
        // The backlog is still the OLD datagrams, in order.
        assert_eq!(queue.pop(), Some(payload(1)));
        assert_eq!(queue.pop(), Some(payload(2)));
        assert_eq!(queue.pop(), None);
        assert_eq!(queue.dropped_newest(), 1);
    }

    #[test]
    fn oversize_payload_is_dropped_and_counted() {
        // Spec §7.3 "Oversized send": fail the send, count the drop.
        let mut queue = SendQueue::new(4, 1200);
        assert_eq!(queue.send(vec![0; 1201]), SendOutcome::DroppedOversize);
        assert_eq!(queue.dropped_total(), 1);
        assert!(queue.is_empty());
        // Exactly at the limit is fine.
        assert_eq!(queue.send(vec![0; 1200]), SendOutcome::Queued);
    }

    #[test]
    fn send_never_blocks_and_counts_are_honest() {
        // The property ADR-0004 requires: application-level queues with
        // honest drop accounting (quinn evicts oldest silently; we never do).
        let mut queue = SendQueue::new(8, 1200);
        for tag in 0..100u8 {
            let _ = queue.send(payload(tag));
        }
        assert_eq!(queue.len(), 8);
        assert_eq!(queue.sent_total(), 8);
        assert_eq!(queue.dropped_newest(), 92);
        assert_eq!(queue.dropped_total(), 92);
    }

    #[test]
    fn pop_is_fifo() {
        let mut queue = SendQueue::new(3, 1200);
        for tag in 1..=3u8 {
            queue.send(payload(tag));
        }
        assert_eq!(queue.pop(), Some(payload(1)));
        assert_eq!(queue.pop(), Some(payload(2)));
        assert_eq!(queue.pop(), Some(payload(3)));
    }

    #[test]
    fn reset_accounting_clears_counters_only() {
        let mut queue = SendQueue::new(2, 1200);
        queue.send(payload(1));
        queue.send(payload(2));
        queue.send(payload(3)); // dropped newest
        queue.reset_accounting();
        assert_eq!(queue.dropped_newest(), 0);
        assert_eq!(queue.sent_total(), 0);
        // Queued datagrams survive the accounting reset.
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.dropped_total(), 1, "lifetime drops are not reset");
    }

    #[test]
    fn clear_empties_backlog() {
        let mut queue = SendQueue::new(4, 1200);
        queue.send(payload(1));
        queue.send(payload(2));
        queue.clear();
        assert!(queue.is_empty());
        assert_eq!(queue.pop(), None);
    }
}
