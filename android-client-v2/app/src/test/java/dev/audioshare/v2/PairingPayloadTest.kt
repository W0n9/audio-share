package dev.audioshare.v2

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class PairingPayloadTest {
    // pairing.md §4.1 worked example (key/nonce are illustrative).
    private val uri =
        "audioshare-pair://p1?pm=2&addr=192.168.1.20:65531" +
            "&key=PQJfWm3y8r0KkQ2x7TnBvD1cHh5ZsA9eUgXwMkIoPjE" +
            "&nonce=3bF9xQ2mN7vRtK1cYdSw4A"

    @Test
    fun parses_the_worked_example_and_derives_the_pin() {
        val payload = PairingPayload.parse(uri)
        assertEquals("192.168.1.20:65531", payload.address)
        assertEquals("192.168.1.20", payload.host)
        assertEquals(65531, payload.port)
        assertEquals(32, payload.serverPublicKey.size)
        assertEquals(16, payload.nonce.size)
        // Independent SHA-256(SPKI prefix || key) value.
        assertEquals(
            "024eec626de36c7627d0afc8e52f8227199ca9b8f88ad7d18f0a868b2d55047d",
            payload.pinHex,
        )
    }

    @Test
    fun accepts_fields_in_any_order() {
        val reordered =
            "audioshare-pair://p1?nonce=3bF9xQ2mN7vRtK1cYdSw4A" +
                "&key=PQJfWm3y8r0KkQ2x7TnBvD1cHh5ZsA9eUgXwMkIoPjE" +
                "&addr=192.168.1.20:65531&pm=2"
        assertEquals(PairingPayload.parse(uri), PairingPayload.parse(reordered))
    }

    @Test
    fun rejects_unknown_and_duplicate_fields() {
        assertFailsWith<QrPayloadException> {
            PairingPayload.parse(uri + "&future=1")
        }
        assertFailsWith<QrPayloadException> {
            PairingPayload.parse(uri + "&pm=2")
        }
        assertFailsWith<QrPayloadException> {
            PairingPayload.parse(uri.replace("addr=", "addr=1.2.3.4:1&addr="))
        }
    }

    @Test
    fun rejects_wrong_scheme_tag_and_major() {
        assertFailsWith<QrPayloadException> {
            PairingPayload.parse(uri.replace("audioshare-pair://p1", "audioshare-pair://p2"))
        }
        assertFailsWith<QrPayloadException> {
            PairingPayload.parse(uri.replace("pm=2", "pm=3"))
        }
        assertFailsWith<QrPayloadException> {
            PairingPayload.parse(uri.replace("pm=2", "pm=x"))
        }
    }

    @Test
    fun rejects_bad_base64_and_wrong_lengths() {
        assertFailsWith<QrPayloadException> {
            PairingPayload.parse(uri.replace("PQJf", "PQ*f"))
        }
        assertFailsWith<QrPayloadException> {
            PairingPayload.parse(uri.replace("&key=PQJfWm3y8r0KkQ2x7TnBvD1cHh5ZsA9eUgXwMkIoPjE", "&key=AAAA"))
        }
        assertFailsWith<QrPayloadException> {
            PairingPayload.parse(uri.replace("&nonce=3bF9xQ2mN7vRtK1cYdSw4A", "&nonce=AAAA"))
        }
        assertFailsWith<QrPayloadException> {
            PairingPayload.parse(uri + "=")
        }
        assertFailsWith<QrPayloadException> {
            PairingPayload.parse(uri.replace("3bF9xQ2mN7vRtK1cYdSw4A", "3bF9xQ2mN7vRtK1cYdSw4A=="))
        }
    }

    @Test
    fun accepts_bracketed_ipv6_and_dns_names() {
        assertEquals("[::1]:65531", PairingPayload.parse(uri.replace("192.168.1.20", "[::1]")).address)
        assertEquals(
            "server.local:1",
            PairingPayload.parse(uri.replace("192.168.1.20:65531", "server.local:1")).address,
        )
        assertEquals(
            "[2001:db8::1]:65535",
            PairingPayload.parse(uri.replace("192.168.1.20", "[2001:db8::1]").replace("65531", "65535")).address,
        )
    }

    @Test
    fun rejects_malformed_addresses_and_ports() {
        for (bad in listOf(
            "192.168.1.20:0",
            "192.168.1.20:65536",
            "192.168.1.20",
            "[::1]65531",
            "[not-ipv6]:65531",
            "host name:65531",
            "2001:db8::1:65531",
            "host:port",
            "host:65531%20",
        )) {
            val candidate = uri.substringBefore("addr=") + "addr=$bad" +
                "&key=PQJfWm3y8r0KkQ2x7TnBvD1cHh5ZsA9eUgXwMkIoPjE" +
                "&nonce=3bF9xQ2mN7vRtK1cYdSw4A"
            assertFailsWith<QrPayloadException>("expected rejection for $bad") {
                PairingPayload.parse(candidate)
            }
        }
    }

    @Test
    fun fingerprint_is_eight_uppercase_groups() {
        val payload = PairingPayload.parse(uri)
        assertEquals(
            "024EEC62 6DE36C76 27D0AFC8 E52F8227 199CA9B8 F88AD7D1 8F0A868B 2D55047D",
            PairingPayload.fingerprintForServerKey(payload.serverPublicKey),
        )
    }

    @Test
    fun sas_formatting_and_normalization() {
        assertEquals("123 456", SasCode.format(123456))
        assertEquals("000 001", SasCode.format(1))
        assertEquals("999 999", SasCode.format(999999))
        assertEquals(123456, SasCode.normalize("123456"))
        assertEquals(123456, SasCode.normalize("123 456"))
        assertEquals(123456, SasCode.normalize(" 123 456 "))
        assertNull(SasCode.normalize("12345"))
        assertNull(SasCode.normalize("1234567"))
        assertNull(SasCode.normalize("12a456"))
    }

    @Test
    fun pairing_snapshot_parses_native_json() {
        val snapshot = PairingSnapshot.fromJson(
            """{"mode":"qr","phase":"AWAITING_CONFIRMATION","sas":"900 067",
               "fingerprint":"F75D188F","server_public_key":"11b3",
               "server_spki_sha256":"f75d","client_id":"cf79",
               "display_name":"Test Phone","error_code":null}"""
        )
        assertNotNull(snapshot)
        assertEquals("qr", snapshot.mode)
        assertTrue(snapshot.isAwaitingConfirmation)
        assertEquals("900 067", snapshot.sas)
        assertNull(snapshot.errorCode)
        assertNull(PairingSnapshot.fromJson(""))
        assertNull(PairingSnapshot.fromJson("not json"))
    }
}
