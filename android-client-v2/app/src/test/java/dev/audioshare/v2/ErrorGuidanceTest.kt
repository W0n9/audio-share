package dev.audioshare.v2

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class ErrorGuidanceTest {
    @Test
    fun pairing_codes_have_typed_names_and_guidance() {
        val expected = mapOf(
            201 to "PAIRING_REJECTED",
            202 to "AUTHORIZATION_UNKNOWN",
            203 to "AUTHENTICATION_FAILED",
            204 to "AUTHORIZATION_DENIED",
            205 to "SERVER_IDENTITY_MISMATCH",
            206 to "PAIRING_PENDING_TIMEOUT",
        )
        for ((code, name) in expected) {
            val guidance = assertNotNull(ErrorGuidanceBook.forCode(code), "missing $code")
            assertEquals(name, guidance.name)
            assertTrue(guidance.title.isNotBlank())
            assertTrue(guidance.guidance.isNotBlank())
            assertFalse(guidance.fallbackToV1, "v2 never falls back to v1")
        }
    }

    @Test
    fun revoked_unknown_and_pin_mismatch_are_never_automatic_retries() {
        for (code in listOf(202, 204, 205)) {
            val guidance = assertNotNull(ErrorGuidanceBook.forCode(code))
            assertFalse(guidance.automaticRetry, "code $code must not auto-retry")
        }
        assertTrue(assertNotNull(ErrorGuidanceBook.forCode(201)).requiresRepairing)
        assertTrue(assertNotNull(ErrorGuidanceBook.forCode(204)).requiresRepairing)
        assertTrue(assertNotNull(ErrorGuidanceBook.forCode(205)).requiresRepairing)
    }

    @Test
    fun expired_pairing_offers_a_manual_retry_but_not_automatic_reconnect() {
        val guidance = assertNotNull(ErrorGuidanceBook.forCode(206))
        assertTrue(guidance.retryable)
        assertFalse(guidance.automaticRetry)
    }

    @Test
    fun rejected_and_timed_out_pairings_point_to_a_fresh_server_invitation() {
        val rejected = assertNotNull(ErrorGuidanceBook.forCode(201))
        assertTrue(
            rejected.guidance.contains("fresh QR code"),
            "201 must direct the user to a new QR invitation",
        )
        assertTrue(
            rejected.guidance.contains("accept this device", ignoreCase = true),
            "the Server Accept gate stays visible after a decline",
        )

        val timedOut = assertNotNull(ErrorGuidanceBook.forCode(206))
        assertEquals("Pairing timed out", timedOut.title)
        assertTrue(timedOut.guidance.contains("ten-minute"))
        assertTrue(timedOut.guidance.contains("new pairing"))
        assertTrue(timedOut.guidance.contains("fresh QR code"))
    }

    @Test
    fun transport_codes_are_the_only_automatic_retries() {
        val automatic = ErrorGuidanceBook.all().filter { it.automaticRetry }.map { it.code }.sorted()
        assertEquals(listOf(301, 302, 503, 601, 602), automatic)
        assertNull(ErrorGuidanceBook.forCode(9999))
        assertNull(ErrorGuidanceBook.forCode(null))
    }

    @Test
    fun every_guidance_declares_no_v1_fallback() {
        assertTrue(ErrorGuidanceBook.all().all { !it.fallbackToV1 })
    }
}
