package dev.audioshare.v2

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
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
    private fun snapshot(
        phase: String = "AWAITING_CONFIRMATION",
        mode: String = "sas",
        errorCode: Int? = null,
    ) = PairingSnapshot(
        mode, phase, "123 456", null, null, null, null, "Phone", errorCode,
    )

    @Test
    fun listener_registration_replays_the_current_snapshot() {
        val relay = PairingStateRelay()
        val current = snapshot()
        val source = relay.beginSession()
        source.publish(current)
        var seen: PairingSnapshot? = null
        relay.setListener { seen = it }
        assertEquals(current, seen)
        assertEquals(current, relay.snapshot())
    }

    @Test
    fun publish_updates_and_clear_resets() {
        val relay = PairingStateRelay()
        val source = relay.beginSession()
        var seen: PairingSnapshot? = null
        relay.setListener { seen = it }
        assertNull(seen)
        source.publishJson("""{"mode":"sas","phase":"AWAITING_RESPONSE"}""")
        assertEquals("sas", seen?.mode)
        relay.clear()
        assertNull(seen)
        assertNull(relay.snapshot())
    }

    @Test
    fun a_new_exchange_clears_the_previous_failure_and_ignores_the_old_source() {
        val relay = PairingStateRelay()
        val old = relay.beginSession()
        val rejected = snapshot(phase = "FAILED", errorCode = 201)
        assertTrue(old.publish(rejected))
        var seen: PairingSnapshot? = null
        relay.setListener { seen = it }
        assertEquals(rejected, seen)

        // A revoke/reject retry starts a fresh exchange: the old banner must
        // vanish and the stopped session's late snapshots must be discarded.
        val fresh = relay.beginSession()
        assertNull(seen, "beginSession clears the previous snapshot")
        assertNull(relay.snapshot())
        assertFalse(old.publish(rejected), "the old source lost ownership")
        assertFalse(old.publishJson("""{"mode":"sas","phase":"FAILED","error_code":204}"""))

        val awaiting = snapshot()
        assertTrue(fresh.publish(awaiting))
        assertFalse(old.publish(rejected), "late FAILED cannot overwrite AWAITING_CONFIRMATION")
        assertEquals(awaiting, relay.snapshot())
        assertEquals(awaiting, seen)
    }

    @Test
    fun a_closed_source_can_no_longer_publish() {
        val relay = PairingStateRelay()
        val source = relay.beginSession()
        assertTrue(source.publish(snapshot(phase = "COMPLETE")))
        source.close()
        val complete = relay.snapshot()
        assertFalse(source.publish(snapshot()), "close() revokes publication")
        assertFalse(source.publishJson("""{"mode":"sas","phase":"FAILED"}"""))
        assertEquals(complete, relay.snapshot(), "a closed source never changes the snapshot")
        assertFalse(source.isCurrent())
    }
}
