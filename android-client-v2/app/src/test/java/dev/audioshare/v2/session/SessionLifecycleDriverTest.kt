package dev.audioshare.v2.session

import dev.audioshare.v2.EndReason
import dev.audioshare.v2.LivePhase
import dev.audioshare.v2.OutcomeSink
import dev.audioshare.v2.SessionConfiguration
import dev.audioshare.v2.SessionOutcome
import dev.audioshare.v2.StopReason
import org.junit.Test
import java.util.PriorityQueue
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * Ticket 22: the JVM-testable session driver. These tests fix the timing and
 * ownership rules the Android adapter cannot exercise without a device:
 * bounded backoff, rebind handshake gating, STOP/create interleaving, media
 * before native teardown, and stale-generation rejection.
 *
 * The fakes share one event log so ordering assertions are literal. Virtual
 * time is injected through [VirtualScheduler]; nothing runs inline.
 */
class SessionLifecycleDriverTest {

    // ---- create / connect failure -----------------------------------------

    @Test
    fun create_failure_schedules_bounded_backoff_and_publishes_reconnecting() {
        val h = Harness()
        h.native.createResults += listOf(0L, 0L)

        h.start()
        assertEquals(listOf(1000L), h.scheduler.delayedDelays)
        assertEquals(
            SessionOutcome.Reconnecting(attempt = 1, lastCloseCode = null),
            h.sink.outcomes.last(),
        )
        assertEquals(1, h.native.createCount)
        assertTrue(h.native.stopCalls.isEmpty(), "nothing was created, so nothing is stopped")
        assertTrue(h.native.destroyCalls.isEmpty())

        h.scheduler.advanceBy(1000)
        assertEquals(2, h.native.createCount)
        assertEquals(listOf(1000L, 2000L), h.scheduler.delayedDelays)
        assertEquals(
            SessionOutcome.Reconnecting(attempt = 2, lastCloseCode = null),
            h.sink.outcomes.last(),
        )
    }

    @Test
    fun connect_rejection_stops_media_then_destroys_and_backs_off() {
        val h = Harness()
        h.native.connectResult = false

        h.start()

        assertEquals(1, h.native.createCount)
        assertEquals(listOf(42L to ""), h.native.connectCalls)
        assertEquals(
            listOf("media.attach", "media.start", "native.connect", "media.stop", "native.destroy"),
            h.events,
        )
        assertEquals(listOf(1000L), h.scheduler.delayedDelays)
        assertTrue(
            h.events.indexOf("media.stop") < h.events.indexOf("native.destroy"),
            "the playback owner stops before the native session is destroyed",
        )
        assertTrue(h.native.stopCalls.isEmpty(), "connect rejection uses destroy, not stop")
        assertEquals(listOf(42L), h.native.destroyCalls)
        assertEquals(
            SessionOutcome.Reconnecting(attempt = 1, lastCloseCode = null),
            h.sink.outcomes.last(),
        )
    }

    // ---- rebind ------------------------------------------------------------

    @Test
    fun rebind_success_keeps_the_session_alive() {
        val h = Harness()
        h.start()

        h.network.available()

        assertEquals(listOf(42L to 7), h.native.rebindCalls)
        assertTrue(h.native.stopCalls.isEmpty())
        assertTrue(h.native.destroyCalls.isEmpty())
        assertEquals(
            listOf<SessionOutcome>(SessionOutcome.Live(LivePhase.CONNECTING)),
            h.sink.outcomes,
        )
    }

    @Test
    fun rebind_failure_tears_down_media_then_native_and_backs_off() {
        val h = Harness()
        h.native.rebindResult = false
        h.start()

        h.network.available()

        assertEquals(listOf(42L to 7), h.native.rebindCalls)
        assertEquals(listOf(42L), h.native.stopCalls)
        assertEquals(listOf(42L), h.native.destroyCalls)
        val mediaStop = h.events.indexOf("media.stop")
        val nativeStop = h.events.indexOf("native.stop")
        val nativeDestroy = h.events.indexOf("native.destroy")
        assertTrue(mediaStop in 0 until nativeStop, "media stops before native stop")
        assertTrue(nativeStop < nativeDestroy, "native stop precedes destroy")
        assertEquals(listOf(1000L), h.scheduler.delayedDelays)
        assertEquals(
            SessionOutcome.Reconnecting(attempt = 1, lastCloseCode = null),
            h.sink.outcomes.last(),
        )
    }

    @Test
    fun network_available_during_the_handshake_does_not_rebind() {
        val h = Harness()
        // A handle exists, but the QUIC endpoint has not been stored yet.
        h.native.stats = """{"connected":false}"""
        h.start()

        h.network.available()

        assertTrue(h.native.rebindCalls.isEmpty(), "rebind during the handshake would kill it")
        assertTrue(h.native.stopCalls.isEmpty())
        assertTrue(h.native.destroyCalls.isEmpty())
        assertTrue(h.scheduler.delayedDelays.isEmpty())
    }

    @Test
    fun network_available_before_the_create_runs_leaves_the_in_flight_start_alone() {
        val h = Harness()
        h.driver.start(config())

        h.network.available() // handle is still 0: the queued start owns the connect

        assertTrue(h.native.rebindCalls.isEmpty())
        assertTrue(h.native.createCalls.isEmpty())
        h.scheduler.runUntilIdle()
        assertEquals(1, h.native.createCount, "exactly one attempt, no extra reconnect")
    }

    // ---- STOP interleaving -------------------------------------------------

    @Test
    fun stop_racing_create_destroys_the_handle_without_attaching_or_connecting() {
        val h = Harness()
        h.native.createResults += 42L
        h.native.onCreate = { h.driver.stop(StopReason.User) }

        h.start()

        assertEquals(1, h.native.createCount)
        assertTrue(h.native.connectCalls.isEmpty())
        assertFalse(h.events.contains("media.attach"))
        assertFalse(h.events.contains("media.start"))
        assertEquals(listOf(42L), h.native.destroyCalls)
        assertFalse(h.driver.isRunning())
        assertEquals(
            SessionOutcome.Ended(EndReason.UserStop),
            h.sink.outcomes.last(),
        )
        assertTrue(h.scheduler.delayedDelays.isEmpty(), "a stopped generation never retries")
    }

    @Test
    fun stop_stops_media_before_stopping_and_destroying_native() {
        val h = Harness()
        h.start()

        h.driver.stop(StopReason.User)

        val mediaStop = h.events.indexOf("media.stop")
        val nativeStop = h.events.indexOf("native.stop")
        val nativeDestroy = h.events.indexOf("native.destroy")
        assertTrue(mediaStop in 0 until nativeStop)
        assertTrue(nativeStop < nativeDestroy)
        assertEquals(listOf(42L), h.native.stopCalls)
        assertEquals(listOf(42L), h.native.destroyCalls)
        assertEquals(listOf(1), listOf(h.network.unregisterCount))
        assertEquals(
            SessionOutcome.Ended(EndReason.UserStop),
            h.sink.outcomes.last(),
        )
    }

    @Test
    fun an_internal_stop_publishes_no_session_outcome() {
        val h = Harness()
        h.start()

        h.driver.stop(StopReason.Superseded)

        assertEquals(
            listOf<SessionOutcome>(SessionOutcome.Live(LivePhase.CONNECTING)),
            h.sink.outcomes,
            "internal teardowns leave the outcome store to the orchestrator",
        )
    }

    // ---- stale generations -------------------------------------------------

    @Test
    fun stop_drops_a_pending_reconnect() {
        val h = Harness()
        h.native.createResults += 0L
        h.start()
        assertEquals(listOf(1000L), h.scheduler.delayedDelays)

        h.driver.stop(StopReason.User)
        h.scheduler.advanceBy(1000)

        assertEquals(1, h.native.createCount, "the old generation's retry is dropped")
        assertTrue(h.native.connectCalls.isEmpty())
        assertEquals(
            SessionOutcome.Ended(EndReason.UserStop),
            h.sink.outcomes.last(),
        )
    }

    @Test
    fun a_network_callback_from_an_old_generation_is_ignored() {
        val h = Harness()
        h.start()
        val staleListener = h.network.listener

        h.driver.stop(StopReason.User)
        h.start()
        staleListener!!.onAvailable(h.network.link)

        assertTrue(h.native.rebindCalls.isEmpty(), "the old lifetime must not migrate the new session")

        // The current generation still rebinds normally.
        h.network.available()
        assertEquals(listOf(42L to 7), h.native.rebindCalls)
    }

    @Test
    fun bind_hint_uses_the_active_network_address_only_when_enabled() {
        val h = Harness(bindHintEnabled = true)
        h.driver.start(config())
        h.network.available() // handle is still 0: records the active address
        h.scheduler.runUntilIdle()

        assertEquals(listOf(42L to "192.0.2.10"), h.native.connectCalls)
    }

    // ---- harness -----------------------------------------------------------

    private fun config(name: String = "client") =
        SessionConfiguration(
            host = "server.example",
            port = 65531,
            serverSpkiSha256Hex = "spki",
            serverPublicKeyHex = "key",
            clientName = name,
        )

    private inner class Harness(bindHintEnabled: Boolean = false) {
        val events = mutableListOf<String>()
        val native = FakeNative(events)
        val network = FakeNetwork()
        val scheduler = VirtualScheduler()
        val sink = RecordingSink()
        val driver = SessionLifecycleDriver(
            native = native,
            network = network,
            scheduler = scheduler,
            mediaFactory = { FakeMedia(events) },
            sink = sink,
            listener = object : SessionEngine.Listener {
                override fun onLog(line: String) = Unit
            },
            random = { 0.5 },
            bindHintEnabled = { bindHintEnabled },
        )

        fun start(configuration: SessionConfiguration = config()) {
            driver.start(configuration)
            scheduler.runUntilIdle()
        }
    }

    private class RecordingSink : OutcomeSink {
        val outcomes = mutableListOf<SessionOutcome>()
        override fun publish(outcome: SessionOutcome) {
            outcomes += outcome
        }
    }

    private class FakeNative(private val events: MutableList<String>) : NativeSession {
        val createResults = ArrayDeque<Long>()
        var nextHandle = 42L
        var state = "PLAYING"
        var stats = """{"connected":true}"""
        var connectResult = true
        var rebindResult = true
        var onCreate: (() -> Unit)? = null
        var createCount = 0
        val createCalls = mutableListOf<SessionConfiguration>()
        val connectCalls = mutableListOf<Pair<Long, String>>()
        val stopCalls = mutableListOf<Long>()
        val destroyCalls = mutableListOf<Long>()
        val rebindCalls = mutableListOf<Pair<Long, Int>>()

        override fun create(configuration: SessionConfiguration): Long {
            createCount += 1
            createCalls += configuration
            onCreate?.invoke()
            return createResults.removeFirstOrNull() ?: nextHandle
        }

        override fun connect(handle: Long, bindLocalAddress: String): Boolean {
            events += "native.connect"
            connectCalls += handle to bindLocalAddress
            return connectResult
        }

        override fun stop(handle: Long) {
            events += "native.stop"
            stopCalls += handle
        }

        override fun destroy(handle: Long) {
            events += "native.destroy"
            destroyCalls += handle
        }

        override fun getState(handle: Long): String = state

        override fun getStats(handle: Long): String = stats

        override fun rebind(handle: Long, fd: Int): Boolean {
            events += "native.rebind"
            rebindCalls += handle to fd
            return rebindResult
        }

        override fun getPairingState(handle: Long): String? = null

        override fun getCredentialSummary(handle: Long): String? = null

        override fun confirmPairing(handle: Long): Boolean = true
    }

    private class FakeMedia(private val events: MutableList<String>) : MediaController {
        override fun attach(handle: Long): Boolean {
            events += "media.attach"
            return true
        }

        override fun start() {
            events += "media.start"
        }

        override fun stop() {
            events += "media.stop"
        }
    }

    private class FakeNetwork : NetworkMonitor {
        var listener: NetworkMonitor.Listener? = null
        var registerCount = 0
        var unregisterCount = 0
        var fd: Int? = 7

        val link = object : NetworkMonitor.Link {
            override val localAddress: String? = "192.0.2.10"
            override fun openFd(): Int? = fd
        }

        override fun register(listener: NetworkMonitor.Listener) {
            this.listener = listener
            registerCount += 1
        }

        override fun unregister() {
            unregisterCount += 1
            listener = null
        }

        fun available() {
            listener?.onAvailable(link)
        }
    }

    /**
     * Virtual time: [execute] queues on the current tick, [postDelayed] queues
     * at a future tick, and [loop] models each poll as a recurring tick. Only
     * [advanceBy] / [runUntilIdle] run anything, so re-entrant lock use cannot
     * hide an ordering bug.
     */
    private class VirtualScheduler : Scheduler {
        private data class Event(val at: Long, val order: Long, val action: () -> Unit)

        private val events = PriorityQueue(compareBy<Event>({ it.at }, { it.order }))
        private val deferred = ArrayDeque<() -> Unit>()
        private var order = 0L
        private var now = 0L

        val delayedDelays = mutableListOf<Long>()
        val loops = mutableListOf<String>()

        override fun execute(task: () -> Unit) {
            deferred.addLast(task)
        }

        override fun postDelayed(delayMs: Long, task: () -> Unit) {
            delayedDelays += delayMs
            events.add(Event(now + delayMs, order++, task))
        }

        override fun loop(name: String, tick: () -> Long?, onExit: () -> Unit) {
            loops += name
            scheduleTick(0, tick, onExit)
        }

        private fun scheduleTick(delayMs: Long, tick: () -> Long?, onExit: () -> Unit) {
            events.add(
                Event(now + delayMs, order++) {
                    val next = tick()
                    if (next == null) onExit() else scheduleTick(next, tick, onExit)
                },
            )
        }

        fun runUntilIdle() {
            while (deferred.isNotEmpty()) {
                deferred.removeFirst().invoke()
            }
        }

        fun advanceBy(milliseconds: Long) {
            val target = now + milliseconds
            while (true) {
                runUntilIdle()
                val next = events.peek() ?: break
                if (next.at > target) break
                events.poll()
                now = maxOf(now, next.at)
                next.action()
            }
            now = target
            runUntilIdle()
        }
    }
}
