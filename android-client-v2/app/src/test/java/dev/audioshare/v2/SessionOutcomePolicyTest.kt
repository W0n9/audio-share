package dev.audioshare.v2

import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * Ticket 01 classification table. The policy is the single place that turns a
 * close code (or a call-site stop reason) into "reconnect" or "end". The
 * retryable set comes from [ReconnectPolicy]; there is no second table here.
 */
class SessionOutcomePolicyTest {

    private fun classify(closeCode: Int?, stopReason: StopReason? = null) =
        SessionOutcomePolicy.classify(closeCode, stopReason)

    @Test
    fun a_user_stop_ends_as_user_stop_whatever_the_last_close_code_was() {
        for (code in listOf(null, 301, 601, 204, 701)) {
            assertEquals(
                SessionOutcomePolicy.Action.End(EndReason.UserStop),
                classify(code, StopReason.User),
                "an explicit user Stop is attributed by the call site, not the code ($code)",
            )
        }
    }

    @Test
    fun internal_teardown_reasons_never_surface_an_outcome() {
        for (reason in listOf(
            StopReason.Superseded,
            StopReason.Shutdown,
            StopReason.PairingCancel,
        )) {
            assertEquals(
                SessionOutcomePolicy.Action.Silent,
                classify(null, reason),
                "$reason is an internal teardown",
            )
            assertEquals(
                SessionOutcomePolicy.Action.Silent,
                classify(204, reason),
                "$reason must not surface a close code",
            )
        }
    }

    @Test
    fun a_defensive_701_close_without_a_stop_call_is_a_user_stop() {
        assertEquals(
            SessionOutcomePolicy.Action.End(EndReason.UserStop),
            classify(701),
        )
    }

    @Test
    fun retryable_close_codes_reconnect() {
        for (code in listOf(301, 302, 503, 601, 602)) {
            assertEquals(
                SessionOutcomePolicy.Action.Reconnect,
                classify(code),
                "close $code is retryable",
            )
        }
    }

    @Test
    fun transport_loss_without_a_close_code_reconnects() {
        assertEquals(
            SessionOutcomePolicy.Action.Reconnect,
            classify(null),
            "close_code null means no outcome was recorded yet",
        )
    }

    @Test
    fun every_other_close_code_is_surfaced() {
        val surfaced = listOf(
            101, 102, // version / capability
            201, 202, 203, 204, 205, // pairing / authorization
            303, // another session owns the device
            401, 402, 403, // protocol / media
            501, 502, // capture failure / stream end
            599, // internal
            0, 9999, // unknown codes never reconnect
        )
        for (code in surfaced) {
            assertEquals(
                SessionOutcomePolicy.Action.End(EndReason.Surfaced(code)),
                classify(code),
                "close $code must be surfaced, not retried",
            )
        }
    }
}
