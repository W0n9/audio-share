package dev.audioshare.v2

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

class CredentialStoreTest {
    private fun record(pin: String = "a".repeat(64)) = CredentialRecord(
        serverSpkiSha256Hex = pin,
        serverPublicKeyHex = "11".repeat(32),
        addressHint = "192.168.1.20:65531",
        keyAlias = "v2-key",
        pairedAt = "2026-11-16T12:00:00Z",
        softwareBacked = true,
        clientId = "cf7940b474310246",
        displayName = "Test Phone",
        host = "192.168.1.20",
        port = 65531,
    )

    @Test
    fun codec_round_trips_every_field() {
        val decoded = CredentialCodec.decodeList(CredentialCodec.encodeList(listOf(record())))
        assertEquals(listOf(record()), decoded)
    }

    @Test
    fun codec_keeps_multiple_servers_and_unknown_version_is_empty() {
        val other = record("b".repeat(64))
        val decoded = CredentialCodec.decodeList(CredentialCodec.encodeList(listOf(record(), other)))
        assertEquals(2, decoded.size)
        assertTrue(decoded.any { it.serverSpkiSha256Hex == "b".repeat(64) })
        assertEquals(emptyList(), CredentialCodec.decodeList("""{"version":2,"credentials":[]}"""))
        assertEquals(emptyList(), CredentialCodec.decodeList("garbage"))
        assertEquals(emptyList(), CredentialCodec.decodeList(null))
    }

    @Test
    fun incomplete_records_are_dropped_not_repaired() {
        val json = """{"version":1,"credentials":[
            {"server_spki_sha256":"short","server_public_key":"${"11".repeat(32)}","key_alias":"a"},
            {"server_spki_sha256":"${"a".repeat(64)}","server_public_key":"short","key_alias":"a"},
            {"server_spki_sha256":"${"a".repeat(64)}","server_public_key":"${"11".repeat(32)}","key_alias":""}
        ]}"""
        assertEquals(emptyList(), CredentialCodec.decodeList(json))
    }

    @Test
    fun pair_record_keeps_software_backed_marker() {
        val k = record()
        val decoded = CredentialCodec.decodeList(CredentialCodec.encodeList(listOf(k))).single()
        assertEquals(true, decoded.softwareBacked)
        assertEquals("v2-key", decoded.keyAlias)
        assertNull(CredentialCodec.decodeList(CredentialCodec.encodeList(emptyList())).firstOrNull())
    }
}

class PairingStateRelayTest {
    @Test
    fun listener_registration_replays_the_current_snapshot() {
        val relay = PairingStateRelay()
        val snapshot = PairingSnapshot(
            "qr", "AWAITING_CONFIRMATION", "123 456", null, null, null, null, "Phone", null,
        )
        relay.publish(snapshot)
        var seen: PairingSnapshot? = null
        relay.setListener { seen = it }
        assertEquals(snapshot, seen)
        assertEquals(snapshot, relay.snapshot())
    }

    @Test
    fun publish_updates_and_clear_resets() {
        val relay = PairingStateRelay()
        var seen: PairingSnapshot? = null
        relay.setListener { seen = it }
        assertNull(seen)
        relay.publishJson("""{"mode":"sas","phase":"AWAITING_RESPONSE"}""")
        assertEquals("sas", seen?.mode)
        relay.clear()
        assertNull(seen)
        assertNull(relay.snapshot())
    }
}
