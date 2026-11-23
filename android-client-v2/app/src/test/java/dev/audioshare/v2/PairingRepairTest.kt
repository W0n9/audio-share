package dev.audioshare.v2

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

/**
 * Ticket 04 seam: SAS re-pairing keeps the same Client identity when the typed
 * address names an already paired Server (ADR-0006 §2). The rule is pure so it
 * is testable without Keystore: a match returns the stored alias, otherwise the
 * caller mints a new one.
 */
class PairingRepairTest {

    private fun record(
        pin: String = "ab".repeat(32),
        alias: String = "v2-sas-stored",
        host: String = "10.0.2.2",
        port: Int = 65531,
    ) = CredentialRecord(
        serverSpkiSha256Hex = pin,
        serverPublicKeyHex = "cd".repeat(32),
        addressHint = "$host:$port",
        keyAlias = alias,
        pairedAt = "2026-11-16T00:00:00Z",
        softwareBacked = true,
        clientId = "client-1",
        displayName = "Pixel",
        host = host,
        port = port,
    )

    @Test
    fun a_typed_address_that_matches_a_stored_host_and_port_reuses_its_alias() {
        val alias = storedKeyAliasForAddress(
            "10.0.2.2:65531",
            listOf(record(alias = "v2-sas-keep-me")),
        )
        assertEquals("v2-sas-keep-me", alias)
    }

    @Test
    fun an_unmatched_port_returns_null_so_the_caller_mints_a_new_identity() {
        assertNull(storedKeyAliasForAddress("10.0.2.2:47820", listOf(record())))
    }

    @Test
    fun an_unmatched_host_returns_null() {
        assertNull(storedKeyAliasForAddress("192.168.1.20:65531", listOf(record())))
    }

    @Test
    fun an_empty_store_returns_null() {
        assertNull(storedKeyAliasForAddress("10.0.2.2:65531", emptyList()))
    }

    @Test
    fun a_malformed_address_returns_null_instead_of_throwing() {
        assertNull(storedKeyAliasForAddress("10.0.2.2", listOf(record())))
        assertNull(storedKeyAliasForAddress("", listOf(record())))
    }

    @Test
    fun the_first_matching_record_wins_when_several_share_the_address() {
        val alias = storedKeyAliasForAddress(
            "10.0.2.2:65531",
            listOf(
                record(pin = "11".repeat(32), alias = "v2-sas-first"),
                record(pin = "12".repeat(32), alias = "v2-sas-second"),
            ),
        )
        assertEquals("v2-sas-first", alias)
    }

    @Test
    fun a_hint_only_match_is_not_the_same_address_under_the_strict_rule() {
        // The addressHint string is never the identity rule; a record without
        // a stored host:port simply does not match.
        val hintOnly = record().copy(host = "", port = 0)
        assertNull(storedKeyAliasForAddress("10.0.2.2:65531", listOf(hintOnly)))
    }

    @Test
    fun a_sas_origin_record_is_reused_when_the_qr_pin_matches() {
        // QR re-pair reaches the same Server by pin, not by address: the
        // stored SAS identity must be reused so the Server restores the record.
        val sasOrigin = record(pin = "ab".repeat(32), alias = "v2-sas-origin")
        assertEquals(
            "v2-sas-origin",
            storedKeyAliasForPin("ab".repeat(32), listOf(sasOrigin)),
        )
    }

    @Test
    fun an_unknown_qr_pin_returns_null_so_the_pin_alias_still_applies() {
        assertNull(storedKeyAliasForPin("cd".repeat(32), listOf(record())))
    }
}
