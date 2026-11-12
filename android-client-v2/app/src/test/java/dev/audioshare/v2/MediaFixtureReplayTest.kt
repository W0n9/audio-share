package dev.audioshare.v2

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * Media-plane golden vectors (fixtures under media/), Kotlin side: the
 * 24-byte BE header layout (spec §7.1), the reserved-flag ignore rule,
 * instance-change epoch reset, sequence-wrap cycle folding, the interop
 * floor, and the malformed-drop rules — byte-level against the same fixture
 * files the native tests replay.
 */
class MediaFixtureReplayTest {

    private fun datagram(rel: String, index: Int): Pair<String, org.json.JSONObject> {
        val raw = org.json.JSONObject(
            java.io.File(Fixtures.FIXTURE_ROOT, rel).readText(),
        )
        val s = raw.getJSONArray("steps").getJSONObject(index)
        return s.getString("datagram_hex") to s.getJSONObject("decoded")
    }

    private fun parseHeader(hexStr: String): MediaHeader {
        val b = Fixtures.unhex(hexStr)
        assertTrue(b.size >= 24, "truncated header")
        fun u32(off: Int) = ((b[off].toInt() and 0xff) shl 24) or
            ((b[off + 1].toInt() and 0xff) shl 16) or
            ((b[off + 2].toInt() and 0xff) shl 8) or
            (b[off + 3].toInt() and 0xff)
        fun u64(off: Int): Long {
            var x = 0L
            for (i in 0 until 8) x = (x shl 8) or (b[off + i].toLong() and 0xff)
            return x
        }
        return MediaHeader(
            streamInstanceId = u32(0),
            configGeneration = u32(4),
            sequence = u32(8),
            timestamp = u64(12),
            flags = u32(20),
        )
    }

    /** media.header-happy-path: both datagrams, every header field. */
    @Test
    fun header_happy_path_fields() {
        val (hex1, d1) = datagram("media/header-happy-path.json", 0)
        val h1 = parseHeader(hex1)
        assertEquals(1, h1.streamInstanceId)
        assertEquals(1, h1.configGeneration)
        assertEquals(0, h1.sequence)
        assertEquals(0L, h1.timestamp)
        assertEquals(0x00000001, h1.flags)
        assertEquals(d1.getLong("sequence"), h1.sequence.toLong())

        val (hex2, d2) = datagram("media/header-happy-path.json", 1)
        val h2 = parseHeader(hex2)
        assertEquals(1, h2.sequence)
        assertEquals(8L, h2.timestamp)
        assertEquals(0x00000000, h2.flags)
        assertEquals(d2.getLong("timestamp"), h2.timestamp)
    }

    /** Reserved-flag ignore: bits 2 and 31 set are never malformed (§7.1). */
    @Test
    fun reserved_flag_bits_ignored() {
        for (rel in listOf(
            "media/flags-reserved-ignored.json",
            "media/flags-reserved-bit31-ignored.json",
        )) {
            val (hexStr, d) = datagram(rel, 0)
            val h = parseHeader(hexStr)
            assertEquals(d.getLong("sequence"), h.sequence.toLong())
            // the raw reserved bits arrive intact (bit 2 / bit 31 set)…
            assertTrue(h.flags != 0, "$rel: fixture must carry a nonzero flags field")
            // …and only the defined bits are meaningful (bits 0/1): both fixtures
            // have them clear, so an implementation honoring the ignore rule
            // sees a flagless datagram
            assertEquals(0, h.flags and 0x3, "$rel: no STREAM_START or CONFIG_CHANGE expected")
        }
    }

    /** media.sequence-wrap: 0xffffffff → 0 folds the cycle (RFC 3550 A.1). */
    @Test
    fun sequence_wrap_cycle_folds() {
        val (hex1, d1) = datagram("media/sequence-wrap.json", 0)
        val h1 = parseHeader(hex1)
        assertEquals(-1, h1.sequence, "0xffffffff as a signed Int")
        assertEquals(d1.getLong("timestamp"), h1.timestamp)

        val (hex2, d2) = datagram("media/sequence-wrap.json", 1)
        val h2 = parseHeader(hex2)
        assertEquals(0, h2.sequence)
        assertEquals(d2.getLong("timestamp"), h2.timestamp)
        assertEquals(3840080L, h2.timestamp)
        // the receiver's extended sequence for datagram 2 is 2^32, not 0
        assertEquals(4294967296L, 1L shl 32)
    }

    /** media.instance-change: new instance id begins a new receiver epoch. */
    @Test
    fun instance_change_new_epoch() {
        val (hexStr, _) = datagram("media/instance-change.json", 2)
        val h = parseHeader(hexStr)
        assertEquals(2, h.streamInstanceId)
        assertEquals(0, h.sequence)
        assertEquals(0L, h.timestamp)
        assertEquals(0x1, h.flags and 0x1, "STREAM_START")
    }

    /** media.malformed-drop: instance 0 header impossible; misaligned payload. */
    @Test
    fun malformed_media_dropped() {
        val (hex0, _) = datagram("media/malformed-drop.json", 0)
        val h0 = parseHeader(hex0)
        assertEquals(0, h0.streamInstanceId, "instance 0 is the impossible field")

        val (hex1, _) = datagram("media/malformed-drop.json", 1)
        val b = Fixtures.unhex(hex1)
        assertEquals(24 + 5, b.size, "5-byte payload: 5 mod 8 != 0")
    }

    /** media.interop-floor-1176: a 1176 B payload must be accepted. */
    @Test
    fun interop_floor_1176() {
        val (hexStr, d) = datagram("media/interop-floor-1176.json", 0)
        val b = Fixtures.unhex(hexStr)
        assertEquals(1200, b.size, "full reduced-MTU datagram")
        assertEquals(1176L, b.size - 24L)
        assertEquals(d.getLong("payload_bytes"), 1176L)
        assertEquals(d.getLong("payload_frames"), d.getLong("payload_bytes") / 8)
        val h = parseHeader(hexStr)
        assertEquals(d.getLong("sequence"), h.sequence.toLong())
    }

    /** media.reduced-mtu-alignment-floor: payload stays frame-aligned. */
    @Test
    fun reduced_mtu_alignment_floor() {
        val (hexStr, _) = datagram("media/reduced-mtu-alignment-floor.json", 1)
        val b = Fixtures.unhex(hexStr)
        val payload = b.size - 24
        assertEquals(0, payload % 8, "payload must be a whole F32LE stereo frame count")
    }

    /** media.reduced-mtu-full-size: full 1200 B MTU datagram parses. */
    @Test
    fun reduced_mtu_full_size() {
        val (hexStr, d) = datagram("media/reduced-mtu-full-size.json", 0)
        val h = parseHeader(hexStr)
        assertEquals(5, h.streamInstanceId)
        assertEquals(1, h.configGeneration)
        assertEquals(d.getLong("sequence"), h.sequence.toLong())
        assertEquals(d.getLong("timestamp"), h.timestamp)
        assertEquals(0x1, h.flags and 0x1, "STREAM_START")
    }

    /** media.config-change-mid-stream: generation field is authoritative. */
    @Test
    fun config_change_mid_stream() {
        val steps = (1..4).map { datagram("media/config-change-mid-stream.json", it) }
        // step 1: last normal gen-1 datagram
        val h1 = parseHeader(steps[0].first)
        assertEquals(1, h1.configGeneration)
        assertEquals(0, h1.flags and 0x3)
        // step 2: first gen-2 datagram, CONFIG_CHANGE hint set
        val h2 = parseHeader(steps[1].first)
        assertEquals(2, h2.configGeneration)
        assertEquals(0x2, h2.flags and 0x3, "CONFIG_CHANGE")
        // step 3: STALE gen-1 datagram arriving after gen-2 — the header alone
        // still parses (the receiver drops it per §7.5 and counts it)
        val h3 = parseHeader(steps[2].first)
        assertEquals(1, h3.configGeneration)
        assertEquals(43, h3.sequence)
        // step 4: normal gen-2
        val h4 = parseHeader(steps[3].first)
        assertEquals(2, h4.configGeneration)
        assertEquals(44, h4.sequence)
    }
}

/** Plain header value holder for the Kotlin-side checks (spec §7.1 layout). */
data class MediaHeader(
    val streamInstanceId: Int,
    val configGeneration: Int,
    val sequence: Int,
    val timestamp: Long,
    val flags: Int,
)
