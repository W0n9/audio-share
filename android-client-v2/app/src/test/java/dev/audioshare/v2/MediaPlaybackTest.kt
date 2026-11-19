package dev.audioshare.v2

import android.media.AudioFormat
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotEquals
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
        var lastPlaybackRateHz = -1

        override fun write(bytes: ByteArray, length: Int): Int {
            val acceptedNow = minOf(length, acceptAtMost)
            offered += length
            accepted += acceptedNow
            return acceptedNow
        }

        override fun setPlaybackRateHz(rateHz: Int) {
            lastPlaybackRateHz = rateHz
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

    /**
     * M5-03: when the backlog exceeds the jitter target, the reader re-anchors
     * to `write - target` (frame-aligned) instead of jumping all the way to the
     * write cursor. This avoids the permanent ~46 ms startup backlog and keeps
     * the transition smooth (no click from dropping a large contiguous run).
     */
    @Test
    fun ring_trims_backlog_down_to_the_target_frame_aligned() {
        val ring = MediaRingBuffer(64 * 1024)
        val target = 8 * 240 // 240 frames = 5 ms @ 48 kHz (test-scale target)
        val hardLimit = 8 * 960 // 960 frames = 20 ms (test-scale hard limit)
        // Backlog of 1200 frames exceeds the 960-frame hard limit: trim to the
        // target, keeping the newest `target` bytes.
        val trimmed = ring.trimmedReadPosition(
            writePosition = 8L * 1200,
            readPosition = 0L,
            targetBytes = target,
            hardLimitBytes = hardLimit,
            frameBytes = 8,
        )
        assertEquals((8L * 1200 - target), trimmed, "re-anchor to write minus target")
        assertEquals(0, (trimmed % 8).toInt(), "frame aligned")
        // A backlog below the target is left untouched.
        assertEquals(
            8L * 100,
            ring.trimmedReadPosition(
                writePosition = 8L * 200,
                readPosition = 8L * 100,
                targetBytes = target,
                hardLimitBytes = hardLimit,
                frameBytes = 8,
            ),
        )
    }

    /** A full lap still snaps to the write cursor (all buffered media is stale). */
    @Test
    fun ring_trim_on_lap_snaps_to_live_audio() {
        val ring = MediaRingBuffer(64)
        val trimmed = ring.trimmedReadPosition(
            writePosition = 104,
            readPosition = 0,
            targetBytes = 8,
            hardLimitBytes = 64,
            frameBytes = 8,
        )
        assertEquals(96L, trimmed, "write minus target, frame aligned")
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
     * M5-02: the production playback buffer is the platform minimum, never the
     * old `sampleRate/10` request that rounded up to a 128 ms AudioTrack buffer.
     */
    @Test
    fun playback_buffer_is_the_platform_minimum() {
        assertEquals(16_416, AudioSink.playbackBufferBytes(16_416))
        assertEquals(24_624, AudioSink.playbackBufferBytes(24_624))
        assertNotEquals(38_400, AudioSink.playbackBufferBytes(16_416), "not the old 100 ms request")
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

    /**
     * The §8 hard-limit re-anchor reports dropped frames to native, which
     * counts them as `ReAnchor` concealment and advances the submitted
     * position. The JVM side reports the actual trim it performed.
     */
    @Test
    fun reanchor_trim_reports_dropped_frames_to_native() {
        val fake = RecordingNative()
        val player = MediaPlayer(fake)
        val trimmed = player.applyReadPosition(
            writePosition = 8L * 1200,
            readPosition = 0L,
            targetBytes = 8 * 240,
            hardLimitBytes = 8 * 960,
            frameBytes = 8,
        )
        assertEquals(8L * 960, trimmed)
        assertEquals(
            (1200 - 240).toLong(),
            fake.trimmedFramesTotal,
            "960 frames trimmed are reported, not counted as concealment",
        )
    }

    /**
     * M5-04: the ±100 ppm correction is applied as an integer `AudioTrack`
     * sample rate. At 48 kHz one Hz is ~20.8 ppm, so 100 ppm rounds to 5 Hz.
     */
    @Test
    fun drift_ppm_maps_to_the_platform_hz_granularity() {
        assertEquals(48_000, playbackRateHz(48_000, 0))
        assertEquals(48_005, playbackRateHz(48_000, 100))
        assertEquals(47_995, playbackRateHz(48_000, -100))
        assertEquals(48_002, playbackRateHz(48_000, 50), "2.4 Hz rounds to 2")
        assertEquals(44_104, playbackRateHz(44_100, 100), "4.41 Hz rounds to 4")
        assertEquals(48_005, playbackRateHz(48_000, 10_000), "defensive clamp to ±100 ppm")
        assertEquals(47_995, playbackRateHz(48_000, -10_000))
    }

    /** A positive drift adjustment inserts silence and is platform-accepted. */
    @Test
    fun drift_insertion_feeds_silence_and_counts_platform_acceptance() {
        val player = MediaPlayer(RecordingNative())
        val sink = FakeSink(frameBytes = 8)
        val applied = player.applyDriftIntervention(
            frames = 96,
            sink = sink,
            readPosition = 0,
            writePosition = 0,
        )
        assertEquals(0L, applied.readPosition, "insertion is written, not read from the ring")
        assertEquals(96, applied.appliedFrames)
        assertEquals(96 * 8, sink.offered)
        assertEquals(96L * 8, player.submittedBytesTotal())
    }

    /** A negative adjustment skips buffered frames without submitting them. */
    @Test
    fun drift_deletion_advances_the_reader_without_submitting() {
        val player = MediaPlayer(RecordingNative())
        val sink = FakeSink(frameBytes = 8)
        val applied = player.applyDriftIntervention(
            frames = -96,
            sink = sink,
            readPosition = 0,
            writePosition = 8L * 800,
        )
        assertEquals(8L * 96, applied.readPosition)
        assertEquals(-96, applied.appliedFrames)
        assertEquals(0L, player.submittedBytesTotal(), "deleted media was never played")
    }

    /** A deletion may never jump past the writer cursor. */
    @Test
    fun drift_deletion_never_passes_the_writer() {
        val player = MediaPlayer(RecordingNative())
        val sink = FakeSink(frameBytes = 8)
        val applied = player.applyDriftIntervention(
            frames = -10_000,
            sink = sink,
            readPosition = 0,
            writePosition = 8L * 100,
        )
        assertEquals(8L * 100, applied.readPosition)
        assertEquals(-100, applied.appliedFrames, "only applied frames are acknowledged")
    }

    /**
     * The loop applies the native rate to the sink and acknowledges only the
     * frames it actually applied — native counts the concealment on the ack.
     */
    @Test
    fun the_player_applies_the_native_rate_and_acks_only_applied_frames() {
        val fake = RecordingNative()
        fake.driftRatePpm = -100
        fake.pendingAdjust = -48
        val player = MediaPlayer(fake)
        val sink = FakeSink(frameBytes = 8)

        val read = player.applyDriftControl(
            session = 7,
            sampleRate = 48_000,
            sink = sink,
            readPosition = 0,
            writePosition = 8L * 500,
        )

        assertEquals(47_995, sink.lastPlaybackRateHz)
        assertEquals(8L * 48, read)
        assertEquals(-48, fake.driftFramesTotal)
    }

    private class RecordingNative : MediaPlayer.NativeMedia {
        var trimmedFramesTotal: Long = 0
        var driftRatePpm: Int = 0
        var pendingAdjust: Int = 0
        var driftFramesTotal: Int = 0
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
        override fun recordTrim(handle: Long, frames: Long) {
            trimmedFramesTotal += frames
        }
        override fun driftCorrectionPpm(handle: Long): Int = driftRatePpm
        override fun takeDriftIntervention(handle: Long): Int = pendingAdjust.also { pendingAdjust = 0 }
        override fun recordDriftIntervention(handle: Long, frames: Int) {
            driftFramesTotal += frames
        }
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
