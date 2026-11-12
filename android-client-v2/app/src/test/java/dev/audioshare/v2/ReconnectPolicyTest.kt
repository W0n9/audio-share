package dev.audioshare.v2

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * §6.3 reconnect table: only retryable closes and transport loss (no Close,
 * close_code null) auto-reconnect; pairing, admin, security, protocol and
 * user-stop closes surface to the user.
 */
class ReconnectPolicyTest {

    @Test
    fun retryable_closes_reconnect() {
        for (code in listOf(301, 302, 503, 601, 602)) {
            assertTrue(ReconnectPolicy.shouldReconnect(code), "close $code must reconnect")
        }
    }

    @Test
    fun transport_loss_without_a_close_reconnects_as_601() {
        assertTrue(ReconnectPolicy.shouldReconnect(null))
    }

    @Test
    fun non_retryable_closes_do_not_reconnect() {
        val terminal = listOf(
            101, 102, // version/capability
            201, 202, 203, 204, 205, // pairing + authorization
            303, // another session owns the identity
            401, 402, 403, // malformed/state/media
            599, // internal
            701, // user stop
            501, 502, // capture failure / stream end: surface, never blind retry
        )
        for (code in terminal) {
            assertFalse(ReconnectPolicy.shouldReconnect(code), "close $code must not reconnect")
        }
    }

    @Test
    fun unknown_closes_do_not_reconnect() {
        assertFalse(ReconnectPolicy.shouldReconnect(9999))
        assertFalse(ReconnectPolicy.shouldReconnect(0))
    }

    @Test
    fun backoff_starts_at_one_second_with_plus_minus_twenty_percent() {
        assertEquals(1_000L, ReconnectPolicy.backoffDelayMs(0, 0.5))
        assertEquals(800L, ReconnectPolicy.backoffDelayMs(0, 0.0))
        assertEquals(1_200L, ReconnectPolicy.backoffDelayMs(0, 1.0))
    }

    @Test
    fun backoff_doubles_each_attempt() {
        assertEquals(2_000L, ReconnectPolicy.backoffDelayMs(1, 0.5))
        assertEquals(4_000L, ReconnectPolicy.backoffDelayMs(2, 0.5))
        assertEquals(8_000L, ReconnectPolicy.backoffDelayMs(3, 0.5))
        assertEquals(16_000L, ReconnectPolicy.backoffDelayMs(4, 0.5))
    }

    @Test
    fun backoff_caps_at_thirty_seconds_with_jitter() {
        assertEquals(30_000L, ReconnectPolicy.backoffDelayMs(5, 0.5))
        assertEquals(24_000L, ReconnectPolicy.backoffDelayMs(5, 0.0))
        assertEquals(30_000L, ReconnectPolicy.backoffDelayMs(5, 1.0))
        assertEquals(30_000L, ReconnectPolicy.backoffDelayMs(1000, 0.5))
    }
}
