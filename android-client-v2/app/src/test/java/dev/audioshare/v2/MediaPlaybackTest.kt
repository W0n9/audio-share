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
        val trimmed = ring.trim(
            writePosition = 8L * 1200,
            readPosition = 0L,
            targetBytes = target,
            hardLimitBytes = hardLimit,
            hysteresisBytes = 8 * 20,
            frameBytes = 8,
        ).readPosition
        assertEquals((8L * 1200 - target), trimmed, "re-anchor to write minus target")
        assertEquals(0, (trimmed % 8).toInt(), "frame aligned")
        // A backlog below the target is left untouched.
        assertEquals(
            8L * 100,
            ring.trim(
                writePosition = 8L * 200,
                readPosition = 8L * 100,
                targetBytes = target,
                hardLimitBytes = hardLimit,
                hysteresisBytes = 8 * 20,
                frameBytes = 8,
            ).readPosition,
        )
    }

    /** A full lap still snaps to the write cursor (all buffered media is stale). */
    @Test
    fun ring_trim_on_lap_snaps_to_live_audio() {
        val ring = MediaRingBuffer(64)
        val trimmed = ring.trim(
            writePosition = 104,
            readPosition = 0,
            targetBytes = 8,
            hardLimitBytes = 64,
            hysteresisBytes = 0,
            frameBytes = 8,
        ).readPosition
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
            hysteresisBytes = 8 * 20,
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

    /**
     * M5-03 ticket: backlog above the *target + 20 ms hysteresis* is trimmed
     * even below the hard limit, and that soft trim is counted separately from
     * the hard-limit `ReAnchor` concealment.
     */
    @Test
    fun soft_target_trim_is_reported_separately_from_reanchor() {
        val fake = RecordingNative()
        val player = MediaPlayer(fake)
        val trimmed = player.applyReadPosition(
            writePosition = 8L * 800,
            readPosition = 0L,
            targetBytes = 8 * 240,
            hardLimitBytes = 8 * 2_000,
            hysteresisBytes = 8 * 20,
            frameBytes = 8,
        )
        assertEquals(8L * 560, trimmed, "drop the oldest frames down to the target")
        assertEquals(560L, fake.softTrimmedFramesTotal, "soft trim is counted")
        assertEquals(0L, fake.trimmedFramesTotal, "not mislabelled ReAnchor")
    }

    /**
     * Hysteresis: a backlog inside the target + 20 ms band is ordinary
     * inter-datagram variation and must not be trimmed. Crossing the band
     * trims down to the target.
     */
    @Test
    fun soft_trim_waits_for_the_hysteresis_band() {
        val ring = MediaRingBuffer(64 * 1024)
        val frameBytes = 8
        val target = 8 * 240
        val hysteresis = 8 * 20 // 20 frames at test scale
        // 260 frames is within the 260-frame band exactly: no trim.
        val within = ring.trim(
            writePosition = 8L * 260,
            readPosition = 0L,
            targetBytes = target,
            hardLimitBytes = 8 * 2_000,
            hysteresisBytes = hysteresis,
            frameBytes = frameBytes,
        )
        assertEquals(0L, within.trimmedFrames, "inside the band is untouched")
        // 261 frames is one frame past the band: trim to the target.
        val beyond = ring.trim(
            writePosition = 8L * 261,
            readPosition = 0L,
            targetBytes = target,
            hardLimitBytes = 8 * 2_000,
            hysteresisBytes = hysteresis,
            frameBytes = frameBytes,
        )
        assertEquals(21L, beyond.trimmedFrames, "outside the band trims to target")
        assertFalse(beyond.reanchor, "still buffer steering, not ReAnchor")
    }

    @Test
    fun ring_trim_reports_whether_it_was_a_hard_limit_reanchor() {
        val ring = MediaRingBuffer(64 * 1024)
        val soft = ring.trim(
            writePosition = 8L * 800,
            readPosition = 0L,
            targetBytes = 8 * 240,
            hardLimitBytes = 8 * 2_000,
            hysteresisBytes = 8 * 20,
            frameBytes = 8,
        )
        assertFalse(soft.reanchor)
        assertEquals(560L, soft.trimmedFrames)

        val hard = ring.trim(
            writePosition = 8L * 1_200,
            readPosition = 0L,
            targetBytes = 8 * 240,
            hardLimitBytes = 8 * 960,
            hysteresisBytes = 8 * 20,
            frameBytes = 8,
        )
        assertTrue(hard.reanchor, "above the hard limit is a ReAnchor")
        assertEquals(960L, hard.trimmedFrames)
    }

    @Test
    fun soft_trim_is_frame_aligned_with_an_unaligned_target() {
        val ring = MediaRingBuffer(64 * 1024)
        val result = ring.trim(
            writePosition = 8L * 1_200,
            readPosition = 0L,
            targetBytes = 1_919, // just under 240 frames
            hardLimitBytes = 8 * 2_000,
            hysteresisBytes = 8 * 20,
            frameBytes = 8,
        )
        assertEquals(0, (result.readPosition % 8).toInt(), "frame aligned")
        assertEquals(1_200L - 239L, result.trimmedFrames)
        assertEquals(0L, result.readPosition - (8L * 1_200 - 1_912))
    }

    /**
     * Finding 6: the active §8 bounds come from native, so the JVM cannot
     * silently diverge from `JitterConfig`. The normative values are only a
     * fallback when native has no media session yet.
     */
    @Test
    fun jitter_bounds_use_native_values_with_normative_fallbacks() {
        val fake = RecordingNative()
        fake.jitterTarget = 40
        fake.jitterHardLimit = 120
        val player = MediaPlayer(fake)
        val bounds = player.resolveJitterBounds(7, 48_000, 8)
        assertEquals(40 * 48 * 8, bounds.targetBytes)
        assertEquals(120 * 48 * 8, bounds.hardLimitBytes)
        assertEquals(
            20 * 48 * 8,
            bounds.hysteresisBytes,
            "soft-trim hysteresis is 20 ms in bytes",
        )

        val fallback = MediaPlayer(RecordingNative()).resolveJitterBounds(7, 48_000, 8)
        assertEquals(
            MediaPlayer.FALLBACK_TARGET_MS * 48 * 8,
            fallback.targetBytes,
            "a native session without media falls back to 60 ms",
        )
        assertEquals(MediaPlayer.FALLBACK_HARD_LIMIT_MS * 48 * 8, fallback.hardLimitBytes)
        assertEquals(60, MediaPlayer.FALLBACK_TARGET_MS)
        assertEquals(200, MediaPlayer.FALLBACK_HARD_LIMIT_MS)
    }

    /**
     * §8 underrun fill: one playback period of silence is fed and reported as
     * a native underrun concealment; the platform `underruns` field is a
     * different counter and is not touched by this path.
     */
    @Test
    fun an_underrun_feeds_one_playback_period_of_silence_and_reports_it() {
        val fake = RecordingNative()
        val player = MediaPlayer(fake)
        val sink = FakeSink(frameBytes = 8)
        val frames = player.fillUnderrun(session = 7, sampleRate = 48_000, sink = sink)
        assertEquals(480, frames, "10 ms at 48 kHz = 480 frames of silence")
        assertEquals(480L, fake.underrunFramesTotal)
        assertEquals(480L * 8, player.submittedBytesTotal())
        assertEquals(480 * 8, sink.accepted)
    }

    @Test
    fun an_underrun_the_sink_refuses_is_not_counted() {
        val fake = RecordingNative()
        val player = MediaPlayer(fake)
        val sink = FakeSink(frameBytes = 8, acceptAtMost = 0)
        assertEquals(0, player.fillUnderrun(7, 48_000, sink))
        assertEquals(0L, fake.underrunFramesTotal)
        assertEquals(0L, player.submittedBytesTotal())
    }

    /**
     * M5-08 (a): MIUI's `AudioTrack.underrunCount` advances with bogus
     * per-track events (millions over a track lifetime) while AudioFlinger
     * reports no mixer underruns. A platform-counter bump must never open the
     * fill gate; the counter stays a §9 diagnostic.
     */
    @Test
    fun a_platform_underrun_bump_alone_does_not_fill() {
        val player = MediaPlayer(RecordingNative())
        val gate = player.underrunGate(
            hasPlayed = true,
            availableBytes = 0,
            frameBytes = 8,
            platformUnderruns = 3_508_242,
            lastPlatformUnderruns = 0,
            consecutiveEmpty = 0,
        )
        assertFalse(gate.fill, "the platform counter is not a fill trigger")
        assertEquals(3_508_242L, gate.lastPlatformUnderruns, "consumed for §9 only")
        assertEquals(1, gate.consecutiveEmpty)
    }

    /**
     * M5-08 (b): ~20–30 ms of sustained ring emptiness while playing is a real
     * stall and must fill. The loop sleeps 10 ms when starved, so
     * [MediaPlayer.UNDERRUN_EMPTY_ITERATIONS] iterations is 30 ms.
     */
    @Test
    fun sustained_emptiness_for_twenty_to_thirty_ms_fills() {
        val player = MediaPlayer(RecordingNative())
        var empty = 0
        repeat(MediaPlayer.UNDERRUN_EMPTY_ITERATIONS - 1) {
            val gate = player.underrunGate(true, 0, 8, 0, 0, empty)
            assertFalse(gate.fill, "a few ms between paced datagrams is backpressure")
            empty = gate.consecutiveEmpty
        }
        val gate = player.underrunGate(true, 0, 8, 0, 0, empty)
        assertTrue(gate.fill, "30 ms of sustained emptiness is an underrun")
        assertEquals(0, gate.consecutiveEmpty, "a fill starts a fresh window")
    }

    /** M5-08 (c): one sustained stall fills once, not on every iteration. */
    @Test
    fun one_sustained_stall_is_counted_once() {
        val player = MediaPlayer(RecordingNative())
        val first = player.underrunGate(
            true, 0, 8, 0, 0, MediaPlayer.UNDERRUN_EMPTY_ITERATIONS - 1,
        )
        assertTrue(first.fill)
        // The very next starved iteration belongs to the same stall.
        val second = player.underrunGate(true, 0, 8, 0, 0, first.consecutiveEmpty)
        assertFalse(second.fill, "the same gap must not be charged twice")
        val third = player.underrunGate(true, 0, 8, 0, 0, second.consecutiveEmpty)
        assertFalse(third.fill)
        // A fresh full window of emptiness opens the gate again.
        val fourth = player.underrunGate(true, 0, 8, 0, 0, third.consecutiveEmpty)
        assertTrue(fourth.fill)
    }

    @Test
    fun data_or_startup_does_not_open_the_fill_gate() {
        val player = MediaPlayer(RecordingNative())
        // Data is available: normal feed, no silence.
        val fed = player.underrunGate(true, 8, 8, 0, 0, 0)
        assertFalse(fed.fill)
        assertEquals(0, fed.consecutiveEmpty)
        // Before the first real media an empty ring is startup, not underrun.
        val startup = player.underrunGate(false, 0, 8, 0, 0, 0)
        assertFalse(startup.fill)
        assertEquals(0, startup.consecutiveEmpty)
    }

    /** Playback does not start until the ring holds the jitter target. */
    @Test
    fun playback_prebuffers_to_the_jitter_target_before_starting() {
        val player = MediaPlayer(RecordingNative())
        assertFalse(
            player.readyForPlayback(hasPlayed = false, availableBytes = 100, targetBytes = 480),
            "half a target is not enough to start",
        )
        assertTrue(
            player.readyForPlayback(hasPlayed = false, availableBytes = 480, targetBytes = 480),
        )
        assertTrue(
            player.readyForPlayback(hasPlayed = true, availableBytes = 0, targetBytes = 480),
            "once playing, the ring may drain",
        )
    }

    private class RecordingNative : MediaPlayer.NativeMedia {
        var trimmedFramesTotal: Long = 0
        var softTrimmedFramesTotal: Long = 0
        var underrunFramesTotal: Long = 0
        var jitterTarget: Int = 0
        var jitterHardLimit: Int = 0
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
        override fun recordSoftTrim(handle: Long, frames: Long) {
            softTrimmedFramesTotal += frames
        }
        override fun recordUnderrun(handle: Long, frames: Long) {
            underrunFramesTotal += frames
        }
        override fun jitterTargetMs(handle: Long): Int = jitterTarget
        override fun jitterHardLimitMs(handle: Long): Int = jitterHardLimit
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
