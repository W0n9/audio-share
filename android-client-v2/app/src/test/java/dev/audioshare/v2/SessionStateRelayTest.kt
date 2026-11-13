package dev.audioshare.v2

import org.junit.Test
import kotlin.test.assertEquals

/** 通过公开快照/注册接口验证会话状态重放；真实 native 路径另由设备测试覆盖。 */
class SessionStateRelayTest {
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
