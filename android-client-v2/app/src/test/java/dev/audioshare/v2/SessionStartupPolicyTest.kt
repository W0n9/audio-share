package dev.audioshare.v2

import org.junit.Test
import kotlin.test.assertEquals

/**
 * Ticket 23 startup/migration race: the ConnectivityManager callback fires
 * right after registration. It must never schedule an extra connection while
 * no native handle exists, and it must not rebind before the QUIC endpoint is
 * established (rebind would fail with "no endpoint (not connected)" and the
 * fallback teardown kills the in-flight handshake).
 */
class SessionStartupPolicyTest {

    @Test
    fun availability_with_no_handle_leaves_the_start_to_the_in_flight_session() {
        assertEquals(
            AvailabilityAction.IN_FLIGHT_OWNS_START,
            SessionStartupPolicy.availabilityAction(handle = 0, endpointConnected = false),
        )
        // A connected flag cannot exist without a handle.
        assertEquals(
            AvailabilityAction.IN_FLIGHT_OWNS_START,
            SessionStartupPolicy.availabilityAction(handle = 0, endpointConnected = true),
        )
    }

    @Test
    fun availability_while_the_handshake_is_in_flight_does_not_rebind() {
        assertEquals(
            AvailabilityAction.AWAIT_ENDPOINT,
            SessionStartupPolicy.availabilityAction(handle = 42, endpointConnected = false),
        )
    }

    @Test
    fun availability_with_an_established_endpoint_rebinds() {
        assertEquals(
            AvailabilityAction.REBIND,
            SessionStartupPolicy.availabilityAction(handle = 42, endpointConnected = true),
        )
    }
}
