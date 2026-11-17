package dev.audioshare.v2

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * The Service-owned replayable store. These assertions are the ticket 01
 * invariants: synchronous replay, one Ended per generation, duplicate Live
 * suppression, and stale-generation rejection.
 */
class SessionOutcomeStoreTest {

    private val connecting = SessionOutcome.Live(LivePhase.CONNECTING)
    private val ready = SessionOutcome.Live(LivePhase.READY)

    @Test
    fun snapshot_is_null_before_any_session_has_published() {
        assertNull(SessionOutcomeStore().snapshot())
    }

    @Test
    fun registering_a_listener_synchronously_replays_the_latest_outcome() {
        val store = SessionOutcomeStore()
        store.beginGeneration().publish(ready)
        val seen = mutableListOf<SessionOutcome>()
        store.setListener { seen += it }
        assertEquals(listOf<SessionOutcome>(ready), seen)
    }

    @Test
    fun registering_before_any_publish_replays_nothing() {
        val store = SessionOutcomeStore()
        val seen = mutableListOf<SessionOutcome>()
        store.setListener { seen += it }
        assertTrue(seen.isEmpty())
    }

    @Test
    fun identical_live_phases_are_not_republished() {
        val store = SessionOutcomeStore()
        val sink = store.beginGeneration()
        val seen = mutableListOf<SessionOutcome>()
        store.setListener { seen += it }
        sink.publish(connecting)
        sink.publish(connecting)
        sink.publish(ready)
        assertEquals(listOf<SessionOutcome>(connecting, ready), seen)
    }

    @Test
    fun a_reconnecting_outcome_is_replaced_by_the_next_live_phase() {
        val store = SessionOutcomeStore()
        val sink = store.beginGeneration()
        sink.publish(SessionOutcome.Reconnecting(attempt = 2, lastCloseCode = 601))
        sink.publish(ready)
        assertEquals(ready, store.snapshot())
    }

    @Test
    fun each_generation_produces_exactly_one_ended_and_drops_later_events() {
        val store = SessionOutcomeStore()
        val sink = store.beginGeneration()
        val seen = mutableListOf<SessionOutcome>()
        store.setListener { seen += it }
        sink.publish(connecting)
        val ended = SessionOutcome.Ended(EndReason.UserStop)
        sink.publish(ended)
        sink.publish(ended)
        sink.publish(ready)
        sink.publish(SessionOutcome.Reconnecting(1, 601))
        assertEquals(listOf<SessionOutcome>(connecting, ended), seen)
        assertEquals(ended, store.snapshot())
    }

    @Test
    fun late_events_from_an_old_generation_are_dropped() {
        val store = SessionOutcomeStore()
        val old = store.beginGeneration()
        val current = store.beginGeneration()
        val seen = mutableListOf<SessionOutcome>()
        store.setListener { seen += it }
        current.publish(connecting)
        old.publish(ready)
        old.publish(SessionOutcome.Ended(EndReason.Surfaced(601)))
        assertEquals(connecting, store.snapshot())
        assertEquals(listOf<SessionOutcome>(connecting), seen)

        current.publish(ready)
        assertEquals(ready, store.snapshot())
    }

    @Test
    fun invalidate_orphans_the_current_generation_without_publishing() {
        val store = SessionOutcomeStore()
        val sink = store.beginGeneration()
        val seen = mutableListOf<SessionOutcome>()
        sink.publish(connecting)
        store.setListener { seen += it }
        store.invalidate()
        sink.publish(SessionOutcome.Ended(EndReason.Surfaced(204)))
        assertNull(store.snapshot(), "an internal teardown leaves nothing replayable")
        assertEquals(
            listOf<SessionOutcome>(connecting),
            seen,
            "invalidation is not a synthetic outcome for the current listener",
        )

        val replacement = store.beginGeneration()
        replacement.publish(ready)
        assertEquals(ready, store.snapshot())
    }

    @Test
    fun invalidate_clears_the_latest_outcome_so_a_late_listener_replays_nothing() {
        val store = SessionOutcomeStore()
        val sink = store.beginGeneration()
        sink.publish(SessionOutcome.Live(LivePhase.PLAYING))

        store.invalidate()

        assertNull(store.snapshot(), "a torn-down session has no replayable outcome")
        val late = mutableListOf<SessionOutcome>()
        store.setListener { late += it }
        assertTrue(late.isEmpty(), "a listener registered after the teardown sees no stale value")
    }

    @Test
    fun a_new_generation_can_publish_the_same_live_phase_after_invalidate() {
        val store = SessionOutcomeStore()
        store.beginGeneration().publish(ready)
        val seen = mutableListOf<SessionOutcome>()
        store.setListener { seen += it }

        store.invalidate()
        val next = store.beginGeneration()
        next.publish(ready)

        assertEquals(
            listOf<SessionOutcome>(ready, ready),
            seen,
            "dedupe is per store value, not a global ban on repeating a Live phase",
        )
        assertEquals(ready, store.snapshot())
    }

    @Test
    fun a_new_generation_can_end_after_the_previous_one_ended() {
        val store = SessionOutcomeStore()
        store.beginGeneration().publish(SessionOutcome.Ended(EndReason.UserStop))
        val next = store.beginGeneration()
        next.publish(connecting)
        val ended = SessionOutcome.Ended(EndReason.Surfaced(204))
        next.publish(ended)
        assertEquals(ended, store.snapshot())
    }

    @Test
    fun clearing_the_listener_stops_further_delivery() {
        val store = SessionOutcomeStore()
        val sink = store.beginGeneration()
        val seen = mutableListOf<SessionOutcome>()
        store.setListener { seen += it }
        store.setListener(null)
        sink.publish(ready)
        assertTrue(seen.isEmpty())
        assertEquals(ready, store.snapshot())
    }
}
