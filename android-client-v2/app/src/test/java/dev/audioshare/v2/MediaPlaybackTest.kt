package dev.audioshare.v2

import android.media.AudioFormat
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * The Android playback seam (ticket 14): the ring cursor, and the player's
 * accounted bytes against a fake [AudioSink] whose accepted count is the only
 * playback evidence (§9).
 */
class MediaPlaybackTest {
    private class FakeSink(
        override val frameBytes: Int,
        private val acceptAtMost: Int = Int.MAX_VALUE,
    ) : AudioSink {
        var accepted = 0
        var offered = 0
        var released = false
        var underrunCount = 0L

        override fun write(bytes: ByteArray, length: Int): Int {
            val acceptedNow = minOf(length, acceptAtMost)
            offered += length
            accepted += acceptedNow
            return acceptedNow
        }

        override fun underruns(): Long = underrunCount
        override fun flush() = Unit
        override fun release() { released = true }
    }

    @Test
    fun ring_reports_only_unread_bytes_and_detects_lapping() {
        val ring = MediaRingBuffer(64)
        assertEquals(32, ring.available(writePosition = 32, readPosition = 0))
        assertEquals(0, ring.available(writePosition = 10, readPosition = 10))
        assertFalse(ring.lapped(32, 0))
        assertTrue(ring.lapped(100, 0), "writer lapped the reader past capacity")
    }

    @Test
    fun ring_reads_whole_frames_and_wraps_at_the_end() {
        val ring = MediaRingBuffer(32)
        val backing = ring.direct
        // Fill the ring with a ramp so a wrapped read is visible.
        for (i in 0 until 32) backing.put(i, i.toByte())
        val out = ByteArray(24)
        // 24-byte read starting at ring offset 24: 8 bytes then wraps 16 bytes.
        val copied = ring.read(readPosition = 24, maxBytes = 24, frameBytes = 8, out = out)
        assertEquals(24, copied)
        assertEquals(24.toByte(), out[0])
        assertEquals(31.toByte(), out[7])
        assertEquals(0.toByte(), out[8], "wraps to the ring start")
        assertEquals(15.toByte(), out[23])
    }

    @Test
    fun ring_rounds_down_to_whole_frames() {
        val ring = MediaRingBuffer(32)
        val out = ByteArray(16)
        // 10 requested bytes with 8-byte frames: one whole frame, never 10.
        assertEquals(8, ring.read(0, maxBytes = 10, frameBytes = 8, out = out))
        // Fewer than one frame is not playable at all.
        assertEquals(0, ring.read(0, maxBytes = 7, frameBytes = 8, out = out))
    }

    /** Only bytes the sink accepted are accounted as played. */
    @Test
    fun playback_accounts_only_accepted_bytes() {
        val player = MediaPlayer(RecordingNative())
        val sink = FakeSink(frameBytes = 8, acceptAtMost = 8)
        val copied = player.offerForPlayback(ByteArray(32), 32, sink)
        assertEquals(32, sink.offered, "the whole frame-aligned fill was offered")
        assertEquals(8, copied, "one accepted frame")
        assertEquals(8L, player.submittedBytesTotal())
    }

    /** A sink that accepts everything reports every offered frame. */
    @Test
    fun full_acceptance_reports_every_offered_frame() {
        val player = MediaPlayer(RecordingNative())
        val sink = FakeSink(frameBytes = 8)
        player.offerForPlayback(ByteArray(64), 64, sink)
        assertEquals(64, sink.accepted)
        assertEquals(64L, player.submittedBytesTotal())
    }

    /** A sink error (0 accepted) reports nothing played — never a fake count. */
    @Test
    fun a_failing_sink_reports_no_playback() {
        val player = MediaPlayer(RecordingNative())
        val sink = FakeSink(frameBytes = 8, acceptAtMost = 0)
        assertEquals(0, player.offerForPlayback(ByteArray(64), 64, sink))
        assertEquals(0L, player.submittedBytesTotal())
    }

    /** A partial write accounts whole frames only. */
    @Test
    fun a_partial_write_accounts_whole_frames_only() {
        val player = MediaPlayer(RecordingNative())
        val sink = FakeSink(frameBytes = 8, acceptAtMost = 12)
        player.offerForPlayback(ByteArray(32), 32, sink)
        assertEquals(8L, player.submittedBytesTotal(), "12 bytes is one whole frame")
    }

    /**
     * Ticket 22: the Kotlin table mirrors the authoritative Rust gate
     * (F32LE / S16LE). U8 (2), S24LE (4) and S32 (5) are unreachable in
     * production and must not silently map to a platform encoding.
     */
    @Test
    fun supported_transfer_formats_map_and_unreachable_formats_are_refused() {
        assertEquals(AudioFormat.ENCODING_PCM_FLOAT, TransferFormats.encodingFor(1))
        assertEquals(AudioFormat.ENCODING_PCM_16BIT, TransferFormats.encodingFor(3))
        for (unsupported in listOf(0, 2, 4, 5)) {
            assertNull(
                TransferFormats.encodingFor(unsupported),
                "sample format $unsupported must have no silent fallback",
            )
        }
    }

    private class RecordingNative : MediaPlayer.NativeMedia {
        override fun getState(handle: Long): String = "PLAYING"
        override fun attachMediaRing(handle: Long, buffer: java.nio.ByteBuffer): Boolean = true
        override fun ringPosition(handle: Long): Long = 0
        override fun mediaFormatJson(handle: Long): String = ""
        override fun recordPlayback(
            handle: Long,
            submittedBytes: Long,
            underruns: Long,
            bufferCurrentMs: Int,
        ) = Unit
    }

    // ENCODING_PCM_FLOAT tracks reject byte[] writes (AudioTrack returns
    // ERROR_INVALID_OPERATION), so the float path is a necessary conversion:
    // little-endian f32 bytes in, the same sample values out, whole floats
    // only.
    @Test
    fun pcm_float_bytes_convert_to_the_same_samples() {
        val samples = floatArrayOf(1.0f, -2.5f, 0.0f, 0.125f)
        val bytes = java.nio.ByteBuffer.allocate(samples.size * 4)
            .order(java.nio.ByteOrder.LITTLE_ENDIAN)
            .also { buffer -> samples.forEach { buffer.putFloat(it) } }
            .array()

        val decoded = pcmBytesToFloats(bytes, bytes.size)

        assertEquals(samples.toList(), decoded.toList())
    }

    @Test
    fun pcm_float_conversion_ignores_a_partial_trailing_sample() {
        val bytes = java.nio.ByteBuffer.allocate(6)
            .order(java.nio.ByteOrder.LITTLE_ENDIAN)
            .putFloat(0.5f)
            .put(0x01)
            .put(0x02)
            .array()

        val decoded = pcmBytesToFloats(bytes, bytes.size)

        assertEquals(1, decoded.size, "a partial float tail is not a sample")
        assertEquals(0.5f, decoded[0])
    }
}
