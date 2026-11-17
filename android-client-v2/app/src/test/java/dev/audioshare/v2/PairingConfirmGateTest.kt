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

    @Test
    fun a_snapshot_replayed_after_activity_recreation_is_gated() {
        fun awaiting() = PairingSnapshot(
            mode = "qr",
            phase = "AWAITING_CONFIRMATION",
            sas = "123 456",
            fingerprint = "AAAA1111 BBBB2222",
            serverPublicKeyHex = null,
            serverSpkiSha256Hex = null,
            clientId = null,
            displayName = null,
            errorCode = null,
        )

        // Each fresh AppState models one Activity lifetime receiving the same
        // replayed AWAITING_CONFIRMATION snapshot.
        fun reduceFresh() = AppReducer.reduce(
            AppState(
                screen = AppScreen.PAIRING,
                pairing = PairingUi(mode = "qr", phase = "STARTING"),
            ),
            UiEvent.PairingSnapshotChanged(awaiting()),
        )

        val first = reduceFresh()
        assertEquals(
            AppCommand.ConfirmPairing,
            first.command,
            "the first Activity dispatches the one automatic confirmation",
        )
        var dispatches = 0
        val gate = PairingConfirmGate()
        assertTrue(gate.confirm { dispatches += 1; true })

        val replayed = reduceFresh()
        assertEquals(
            AppCommand.ConfirmPairing,
            replayed.command,
            "a re-created UI cannot know the old lifetime already confirmed",
        )
        assertFalse(gate.confirm { dispatches += 1; true })
        assertEquals(1, dispatches, "the Service gate is the exactly-once boundary")
    }
}
