package dev.audioshare.v2

import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import kotlin.concurrent.thread
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/** 通过公开快照/注册接口验证会话状态重放；真实 native 路径另由设备测试覆盖。 */
class SessionStateRelayTest {
    @Test
    fun stop_during_native_update_retains_closed_snapshot_on_rebind() {
        val relay = SessionStateRelay()
        val source = relay.beginSession()
        source.publish("READY")
        val nativeRead = CountDownLatch(1)
        val resumeNativeRead = CountDownLatch(1)
        val failure = AtomicReference<Throwable?>()
        val poller = thread(name = "controlled-native-state") {
            try {
                source.poll {
                    nativeRead.countDown()
                    assertTrue(resumeNativeRead.await(5, TimeUnit.SECONDS), "resume native read")
                    "READY"
                }
            } catch (error: Throwable) {
                failure.set(error)
            }
        }
        try {
            assertTrue(nativeRead.await(5, TimeUnit.SECONDS), "poller reached native read")
            source.close()
            assertEquals("CLOSED", relay.snapshot(), "STOP publishes terminal snapshot")
            resumeNativeRead.countDown()
            poller.join(5_000)
            assertFalse(poller.isAlive, "poller finished")
            failure.get()?.let { throw it }
            assertEquals("CLOSED", relay.snapshot(), "late native READY must not revive a stopped session")
            var replay: String? = null
            relay.setListener { replay = it }
            assertEquals("CLOSED", replay, "Service rebind must synchronously replay CLOSED")
        } finally {
            resumeNativeRead.countDown()
            poller.join(5_000)
        }
    }

    @Test
    fun service_same_owner_reconnect_replays_the_new_native_ready() {
        val service = SessionStateRelay()
        val owner = service.beginSession()
        val nativeState = SessionStateRelay { owner.publish(it) }
        val first = nativeState.beginSession()
        assertEquals("READY", first.poll { "READY" })
        assertEquals("CLOSED", first.poll { "CLOSED" })
        assertFalse(first.publish("READY"), "native CLOSED is terminal for its source")
        assertEquals("CLOSED", service.snapshot())

        // bounded backoff 期间 Service 仍拥有 ClientSession；
        // 新 native 会话必须正常发布，旧来源已经终止。
        val second = nativeState.beginSession()
        assertEquals("AUTHENTICATING", second.poll { "AUTHENTICATING" })
        assertFalse(first.publish("READY"), "old native event source is no longer current")
        assertEquals("READY", second.poll { "READY" })
        assertEquals("READY", nativeState.snapshot())
        assertEquals("READY", service.snapshot(), "valid reconnect must reach the Service")
        var replay: String? = null
        service.setListener { replay = it }
        assertEquals("READY", replay, "rebind observes the new native READY")
    }

    @Test
    fun old_native_read_cannot_overwrite_a_later_session() {
        val service = SessionStateRelay()
        val owner = service.beginSession()
        val nativeState = SessionStateRelay { owner.publish(it) }
        val old = nativeState.beginSession()
        val nativeRead = CountDownLatch(1)
        val resume = CountDownLatch(1)
        val pool = Executors.newSingleThreadExecutor()
        try {
            val poll = pool.submit<String> {
                old.poll {
                    nativeRead.countDown()
                    assertTrue(resume.await(5, TimeUnit.SECONDS))
                    "READY"
                }
            }
            assertTrue(nativeRead.await(5, TimeUnit.SECONDS))
            old.close()
            val current = nativeState.beginSession()
            current.poll { "AUTHENTICATING" }
            resume.countDown()
            assertEquals("", poll.get(5, TimeUnit.SECONDS), "obsolete native input is discarded")
            old.close()
            assertEquals("AUTHENTICATING", nativeState.snapshot())
            assertEquals("AUTHENTICATING", service.snapshot(), "old READY must not overwrite the new session")
            var replay: String? = null
            service.setListener { replay = it }
            assertEquals("AUTHENTICATING", replay)
            current.poll { "READY" }
            assertEquals("READY", nativeState.snapshot())
            assertEquals("READY", service.snapshot())
        } finally {
            resume.countDown()
            pool.shutdownNow()
            assertTrue(pool.awaitTermination(5, TimeUnit.SECONDS))
        }
    }

    @Test
    fun stop_serializes_with_state_callback_before_terminal_replay() {
        val service = SessionStateRelay()
        val enteredCallback = CountDownLatch(1)
        val resumeCallback = CountDownLatch(1)
        val nativeState = SessionStateRelay { next ->
            if (next == "READY") {
                enteredCallback.countDown()
                assertTrue(resumeCallback.await(5, TimeUnit.SECONDS))
            }
            service.publish(next)
        }
        val source = nativeState.beginSession()
        val stopStarted = CountDownLatch(1)
        val stopFinished = CountDownLatch(1)
        val pool = Executors.newFixedThreadPool(2)
        try {
            val publication = pool.submit<Boolean> { source.publish("READY") }
            assertTrue(enteredCallback.await(5, TimeUnit.SECONDS))
            val stop = pool.submit {
                stopStarted.countDown()
                source.close()
                stopFinished.countDown()
            }
            assertTrue(stopStarted.await(5, TimeUnit.SECONDS))
            val closedBeforePublication = stopFinished.await(250, TimeUnit.MILLISECONDS)
            resumeCallback.countDown()
            assertTrue(publication.get(5, TimeUnit.SECONDS))
            stop.get(5, TimeUnit.SECONDS)
            assertFalse(closedBeforePublication, "STOP cannot interpose between accepted state and its callback")
            assertEquals("CLOSED", nativeState.snapshot())
            assertEquals("CLOSED", service.snapshot(), "late callback cannot permanently revive the Service relay")
            var replay: String? = null
            service.setListener { replay = it }
            assertEquals("CLOSED", replay)
        } finally {
            resumeCallback.countDown()
            pool.shutdownNow()
            assertTrue(pool.awaitTermination(5, TimeUnit.SECONDS))
        }
    }

    @Test
    fun stopped_service_owner_cannot_overwrite_a_new_owner() {
        val service = SessionStateRelay()
        val oldOwner = service.beginSession()
        oldOwner.publish("READY")
        oldOwner.close()
        val newOwner = service.beginSession()
        newOwner.publish("AUTHENTICATING")
        assertFalse(oldOwner.publish("READY"))
        oldOwner.close()
        assertEquals("AUTHENTICATING", service.snapshot())
        assertTrue(newOwner.publish("READY"))
        var replay: String? = null
        service.setListener { replay = it }
        assertEquals("READY", replay)
    }

    @Test
    fun snapshot_starts_idle() {
        assertEquals("IDLE", SessionStateRelay().snapshot())
    }

    @Test
    fun late_listener_is_replayed_the_existing_ready_state() {
        val relay = SessionStateRelay()
        relay.publish("READY")
        val seen = mutableListOf<String>()
        relay.setListener { seen += it }
        assertEquals(listOf("READY"), seen)
    }

    @Test
    fun rebinding_activity_gets_current_state_not_idle() {
        val relay = SessionStateRelay()
        val first = mutableListOf<String>()
        relay.setListener { first += it }
        relay.publish("AUTHENTICATING")
        relay.publish("READY")
        relay.setListener(null)
        val second = mutableListOf<String>()
        relay.setListener { second += it }
        assertEquals(listOf("IDLE", "AUTHENTICATING", "READY"), first)
        assertEquals(listOf("READY"), second)
    }

    @Test
    fun unknown_handle_blank_state_from_a_stale_poller_never_overwrites_snapshot() {
        val relay = SessionStateRelay()
        relay.publish("READY")
        relay.publish("")
        assertEquals("READY", relay.snapshot())
    }

    @Test
    fun closed_is_retained_after_stop() {
        val relay = SessionStateRelay()
        relay.publish("READY")
        relay.publish("CLOSED")
        val seen = mutableListOf<String>()
        relay.setListener { seen += it }
        assertEquals(listOf("CLOSED"), seen)
    }
}
