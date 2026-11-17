package dev.audioshare.v2

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * Ticket 32: the Service-side exactly-once guard for QR auto-confirm. The
 * reducer already suppresses duplicate snapshots inside one Activity, but a
 * re-created Activity replays the same snapshot with a fresh `AppState`; the
 * gate is what stops the second confirmation from reaching the native channel.
 */
class PairingConfirmGateTest {

    @Test
    fun a_pairing_exchange_is_confirmed_only_once() {
        val gate = PairingConfirmGate()
        var dispatches = 0

        assertTrue(gate.confirm { dispatches += 1; true })
        assertFalse(gate.confirm { dispatches += 1; true })
        assertFalse(gate.confirm { dispatches += 1; true })

        assertEquals(1, dispatches, "duplicate confirms must not reach native")
    }

    @Test
    fun a_new_pairing_exchange_gets_a_fresh_confirmation() {
        val gate = PairingConfirmGate()
        assertTrue(gate.confirm { true })
        gate.reset()
        assertTrue(gate.confirm { true })
    }

    @Test
    fun a_rejected_dispatch_does_not_consume_the_attempt() {
        val gate = PairingConfirmGate()
        assertFalse(gate.confirm { false }, "nothing was awaiting confirmation")
        assertTrue(gate.confirm { true }, "the real confirmation is still allowed")
    }
}
