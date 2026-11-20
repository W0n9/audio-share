package dev.audioshare.v2

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTimestamp
import android.media.AudioTrack
import android.util.Log
import org.json.JSONObject
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.roundToInt

/**
 * M5-04: map a §8 drift correction in ppm to the integer sample rate
 * `AudioTrack.setPlaybackRate` accepts.
 *
 * At 48 kHz one Hz is ~20.8 ppm, so ±100 ppm rounds to ±5 Hz — the effective
 * application granularity. The residual (up to ~10 ppm) is absorbed by the
 * buffer, and a sustained >20 ms deviation is corrected by whole frames.
 * The input is defensively clamped to the §8 ±100 ppm bound even though the
 * native controller already clamps.
 */
internal fun playbackRateHz(sampleRate: Int, ppm: Int): Int {
    if (sampleRate <= 0) return sampleRate
    val clamped = ppm.coerceIn(-100, 100)
    val delta = (clamped.toLong() * sampleRate / 1_000_000.0).roundToInt()
    return (sampleRate + delta).coerceIn(1, sampleRate * 2)
}

/**
 * M5-11: extend `AudioTrack`'s playback frame counter into a monotonic `Long`.
 * It wraps roughly every 24.9 hours at 48 kHz: a drop from the top of the
 * 32-bit counter to the bottom is that wrap. Rewinds within 5 ms (converted
 * from the sink's sample rate) are timestamp jitter and hold the last value.
 * A larger rewind is held for one report too — the `getTimestamp` interpolation
 * age can jump — and only two consecutive over-tolerance rewinds restart the
 * base at 0 for native to re-anchor. This is the single owner of the 32-bit
 * wrap; native receives an already-extended value.
 *
 * The counter is per sink. A rebuilt AudioTrack starts at 0 and native treats
 * that regression as a re-anchor.
 */
internal class PlaybackHeadExtender(private val sampleRate: Int) {
    private var base = 0L
    private var last = 0L
    private var seen = false
    /// Consecutive over-tolerance rewinds. One can be an interpolation-age
    /// jump; two in a row are a rebuilt/flushed track.
    private var consecutiveRewinds = 0

    fun update(rawFrames: Long): Long {
        if (!seen) {
            seen = true
            last = rawFrames
            return rawFrames
        }
        if (rawFrames < last) {
            // A wrap runs from the top of the counter to the bottom; a rebuild
            // restarts at 0 from anywhere. Requiring both ends avoids
            // misreading a long-running track's rebuild as a wrap.
            val wrapped = last > 0xF000_0000L && rawFrames < 0x1000_0000L
            when {
                wrapped -> {
                    base += 0x1_0000_0000L
                    consecutiveRewinds = 0
                }
                last - rawFrames <= rewindToleranceFrames(sampleRate) -> {
                    // Timestamp/interpolation jitter: do not report a rewind.
                    consecutiveRewinds = 0
                    return base + last
                }
                else -> {
                    consecutiveRewinds += 1
                    if (consecutiveRewinds < 2) {
                        // A single large rewind can be a drop in the
                        // getTimestamp interpolation age; hold the last value
                        // and see whether the sequence recovers.
                        return base + last
                    }
                    // Two consecutive large rewinds: the track was rebuilt or
                    // flushed. Restart at 0 so native re-anchors the window.
                    base = 0L
                    consecutiveRewinds = 0
                }
            }
        } else {
            consecutiveRewinds = 0
        }
        last = rawFrames
        return base + rawFrames
    }
}

/** Rewinds at or below 5 ms (measured at [sampleRate]) are jitter, not a track
 * restart. */
internal fun rewindToleranceFrames(sampleRate: Int): Long =
    (sampleRate.toLong() * 5 / 1_000).coerceAtLeast(1L)

/** Oldest `getTimestamp()` presentation age treated as current (paused/stale
 * timestamps are not playback progress). */
internal const val MAX_HEAD_AGE_NANOS = 200_000_000L

/**
 * M5-11: the played frame position at a query instant, derived from an
 * `AudioTrack.getTimestamp()` `(framePosition, nanoTime)` pair. The pair is
 * consistent at presentation time, so interpolating by the sink's applied rate
 * removes the mixer-period staircase that a raw head sampled at a jittered
 * 100 ms report instant would otherwise add to the 10 s slope.
 */
internal fun interpolatedHeadFrames(
    framePosition: Long,
    ageNanos: Long,
    rateHz: Int,
): Long {
    if (rateHz <= 0) return framePosition
    return framePosition + ageNanos.coerceAtLeast(0L) * rateHz / 1_000_000_000L
}

/**
 * The wire Transfer Format values Kotlin can actually play.
 *
 * AUTHORITY: the Rust gate is authoritative —
 * `android-client-v2/native/src/control.rs` `configuration_decodable` /
 * `SUPPORTED_SAMPLE_FORMATS` (around line 1112) admits only F32LE (1) and
 * S16LE (3). Kotlin must not widen that set.
 *
 * S24 packed 3-byte has no direct AudioTrack encoding on API 26 (S24LE); the
 * Rust gate rejects it, so it is not mapped here. U8 (2) and S32 (5) are
 * likewise unreachable in production and are deliberately absent.
 */
internal object TransferFormats {
    fun encodingFor(sampleFormat: Int): Int? = when (sampleFormat) {
        1 -> AudioFormat.ENCODING_PCM_FLOAT
        3 -> AudioFormat.ENCODING_PCM_16BIT
        else -> null
    }

    fun bytesPerSample(encoding: Int): Int = when (encoding) {
        AudioFormat.ENCODING_PCM_16BIT -> 2
        else -> 4 // ENCODING_PCM_FLOAT samples
    }
}

/**
 * The platform playback seam (ticket 14). The player loop is testable with a
 * fake sink; production uses [AudioTrackSink].
 *
 * `write` returns the bytes the platform actually accepted. Partial writes and
 * errors are real outcomes — they must never be reported as played.
 */
interface AudioSink {
    /** Bytes of one sample frame (all channels). */
    val frameBytes: Int

    /** Write up to [length] frame-aligned bytes; return the accepted count. */
    fun write(bytes: ByteArray, length: Int): Int

    /** Total underruns reported by the platform since this sink was opened. */
    fun underruns(): Long

    /**
     * M5-11: hardware-played frame position (frames, not bytes) at this call,
     * monotonic since the sink opened, or `null` when the platform cannot
     * report it. `null` is a fallback signal — never a zero position.
     */
    fun playbackHeadFrames(): Long? = null

    /**
     * Apply a §8 playback-rate correction (integer Hz).
     *
     * Default is a no-op so test sinks and sinks without rate control still
     * satisfy the seam.
     */
    fun setPlaybackRateHz(rateHz: Int) {}

    /** Drop anything the platform buffered (stream end / stop). */
    fun flush()

    /** Release the platform resource. */
    fun release()

    companion object {
        /** Open a platform sink for the native geometry, or null when mappable formats run out. */
        fun open(sampleRate: Int, channels: Int, sampleFormat: Int): AudioSink? {
            val encoding = TransferFormats.encodingFor(sampleFormat) ?: return null
            val channelMask = when (channels) {
                1 -> AudioFormat.CHANNEL_OUT_MONO
                2 -> AudioFormat.CHANNEL_OUT_STEREO
                else -> return null
            }
            val bytesPerSample = TransferFormats.bytesPerSample(encoding)
            val minBuffer = AudioTrack.getMinBufferSize(sampleRate, channelMask, encoding)
            if (minBuffer <= 0) return null
            // M5-02: use the platform minimum. The old 100 ms request rounded up
            // to a 128 ms AudioTrack buffer; the 100 ms send-ahead burst is
            // absorbed by the 256 KiB native ring, not by the platform buffer.
            // Sub-floor buffers are not viable (audible crackling), so never
            // shrink below getMinBufferSize. See evidence/m5-02.
            val bufferBytes = playbackBufferBytes(minBuffer)
            val track = AudioTrack.Builder()
                .setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                        .build()
                )
                .setAudioFormat(
                    AudioFormat.Builder()
                        .setEncoding(encoding)
                        .setSampleRate(sampleRate)
                        .setChannelMask(channelMask)
                        .build()
                )
                .setBufferSizeInBytes(bufferBytes)
                .setTransferMode(AudioTrack.MODE_STREAM)
                .build()
            if (track.state != AudioTrack.STATE_INITIALIZED) {
                track.release()
                return null
            }
            track.play()
            Log.i(
                "AudioSink.open",
                "minBuffer=$minBuffer bytes=$bufferBytes " +
                    "frames=${track.bufferSizeInFrames} ms=${track.bufferSizeInFrames * 1000L / sampleRate}",
            )
            return AudioTrackSink(track, bytesPerSample * channels, encoding == AudioFormat.ENCODING_PCM_FLOAT)
        }

        /** Production buffer is the platform minimum; see evidence/m5-02. */
        internal fun playbackBufferBytes(minBuffer: Int): Int = minBuffer
    }
}

/**
 * Reinterpret little-endian PCM f32 bytes as samples.
 *
 * `AudioTrack.write(byte[], ...)` refuses `ENCODING_PCM_FLOAT` tracks, so the
 * float path hands the platform samples instead of bytes. A trailing partial
 * sample is dropped: capture guarantees frame alignment, and a half float is
 * not audio.
 */
internal fun pcmBytesToFloats(bytes: ByteArray, length: Int): FloatArray {
    val usable = minOf(length, bytes.size)
    val samples = FloatArray(usable / FLOAT_BYTES)
    if (samples.isEmpty()) return samples
    val view = ByteBuffer.wrap(bytes, 0, usable).order(ByteOrder.LITTLE_ENDIAN).asFloatBuffer()
    view.get(samples)
    return samples
}

/** Bytes per f32 sample. */
private const val FLOAT_BYTES = 4

/** The production sink: a streaming [AudioTrack]. */
class AudioTrackSink(
    private val track: AudioTrack,
    override val frameBytes: Int,
    private val floatEncoding: Boolean,
) : AudioSink {
    /** Last rate accepted by the platform; native rate until told otherwise. */
    private var appliedRateHz: Int = track.sampleRate

    override fun write(bytes: ByteArray, length: Int): Int {
        // ENCODING_PCM_FLOAT tracks reject byte[] writes outright
        // (ERROR_INVALID_OPERATION), so F32 media must be offered as floats.
        // The accepted count comes back in floats and is reported in bytes,
        // because §9 accounts the platform's accepted bytes.
        return if (floatEncoding) {
            val samples = pcmBytesToFloats(bytes, length)
            if (samples.isEmpty()) {
                0
            } else {
                val written = track.write(samples, 0, samples.size, AudioTrack.WRITE_NON_BLOCKING)
                if (written > 0) written * FLOAT_BYTES else 0
            }
        } else {
            val written = track.write(bytes, 0, length, AudioTrack.WRITE_NON_BLOCKING)
            if (written > 0) written else 0
        }
    }

    override fun underruns(): Long = track.underrunCount.toLong()

    private val headExtender = PlaybackHeadExtender(track.sampleRate)
    private val audioTimestamp = AudioTimestamp()
    private var sawTimestamp = false
    private var loggedHeadClock = false

    /**
     * M5-11: the hardware playout clock. Prefer `getTimestamp()`, whose
     * `(framePosition, nanoTime)` pair is consistent at presentation time;
     * interpolating to the query instant removes the mixer-period
     * quantisation. Fall back to the extended 32-bit
     * `getPlaybackHeadPosition()` only until a timestamp is available. Once a
     * timestamp has worked, a later failure reports `null` (native falls back
     * to accepted bytes) rather than mixing two clocks with different bases.
     */
    override fun playbackHeadFrames(): Long? {
        if (track.state != AudioTrack.STATE_INITIALIZED) return null
        val queryNanos = System.nanoTime()
        if (track.getTimestamp(audioTimestamp)) {
            val ageNanos = queryNanos - audioTimestamp.nanoTime
            // A platform may report a presentation time slightly in the future;
            // interpolatedHeadFrames clamps that age to zero. Only a genuinely
            // stale timestamp is not playback progress.
            if (ageNanos <= MAX_HEAD_AGE_NANOS) {
                sawTimestamp = true
                if (!loggedHeadClock) {
                    loggedHeadClock = true
                    Log.i("AudioTrackSink", "drift clock: AudioTrack.getTimestamp frame+time")
                }
                return headExtender.update(
                    interpolatedHeadFrames(
                        audioTimestamp.framePosition,
                        ageNanos,
                        appliedRateHz,
                    ),
                )
            }
        }
        if (sawTimestamp) return null
        if (!loggedHeadClock) {
            loggedHeadClock = true
            Log.w("AudioTrackSink", "drift clock: raw playbackHeadPosition (no timestamp)")
        }
        return headExtender.update(track.playbackHeadPosition.toLong() and 0xFFFF_FFFFL)
    }

    /**
     * M5-04 §8 drift correction path: `AudioTrack.setPlaybackRate` resamples
     * the stream, so the correction is a rate, not a per-frame edit. One Hz at
     * 48 kHz is ~20.8 ppm; a rejected rate (some routes do not support SRC) is
     * logged and leaves playback at the last accepted rate.
     */
    override fun setPlaybackRateHz(rateHz: Int) {
        if (rateHz == appliedRateHz) return
        if (track.setPlaybackRate(rateHz) == AudioTrack.SUCCESS) {
            appliedRateHz = rateHz
        } else {
            Log.w("AudioTrackSink", "setPlaybackRate($rateHz) rejected")
        }
    }

    override fun flush() {
        track.pause()
        track.flush()
        track.play()
    }

    override fun release() {
        try {
            track.pause()
            track.flush()
        } catch (_: IllegalStateException) {
        }
        track.release()
    }
}

/**
 * One session's playback owner (ticket 14). Reads the native ring and writes
 * to the platform sink, reporting what was actually accepted.
 *
 * Lifecycle: created with the native session, started after [attach], stopped
 * on STOP/close. The thread never touches the control stream; failures are
 * logged and counted as underruns, never thrown into the session.
 */
class MediaPlayer(
    private val native: NativeMedia,
    private val ringBuffer: MediaRingBuffer = MediaRingBuffer(),
) {
    interface NativeMedia {
        fun getState(handle: Long): String?
        fun attachMediaRing(handle: Long, buffer: java.nio.ByteBuffer): Boolean
        fun ringPosition(handle: Long): Long
        fun mediaFormatJson(handle: Long): String?
        fun recordPlayback(handle: Long, submittedBytes: Long, underruns: Long, bufferCurrentMs: Int)
        /**
         * M5-11: the same §9 evidence plus the hardware-played frame position
         * at this report instant. [playbackHeadFrames] is in frames; `-1`
         * means unavailable. Defaults to the pre-M5-11 accepted-bytes path so
         * test fakes keep compiling.
         */
        fun recordPlaybackWithHead(
            handle: Long,
            submittedBytes: Long,
            playbackHeadFrames: Long,
            underruns: Long,
            bufferCurrentMs: Int,
        ) = recordPlayback(handle, submittedBytes, underruns, bufferCurrentMs)
        /** Report frames dropped by the reader's jitter-buffer re-anchor (§8). */
        fun recordTrim(handle: Long, frames: Long)
        /** Report frames dropped by the soft target trim (not concealment). */
        fun recordSoftTrim(handle: Long, frames: Long) = Unit
        /** Report an underrun fill: silence fed because no media was ready. */
        fun recordUnderrun(handle: Long, frames: Long) = Unit
        /** Active §8 target; `0` means "native has no media session yet". */
        fun jitterTargetMs(handle: Long): Int = 0
        /** Active §8 hard limit; `0` means "native has no media session yet". */
        fun jitterHardLimitMs(handle: Long): Int = 0
        /** §8 smooth ±100 ppm playback-rate correction currently commanded. */
        fun driftCorrectionPpm(handle: Long): Int = 0
        /** Take the pending whole-frame drift adjustment (+ insert, − delete). */
        fun takeDriftIntervention(handle: Long): Int = 0
        /** Acknowledge the frames actually applied (positive insert, negative delete). */
        fun recordDriftIntervention(handle: Long, frames: Int) = Unit
    }

    companion object {
        private const val TAG = "MediaPlayer"
        private const val LOOP_SLEEP_MS = 10L
        private const val REPORT_INTERVAL_MS = 100L
        // §8 jitter-buffer bounds come from native (`JitterConfig`) so the two
        // sides cannot silently diverge. These are only fallbacks for the
        // window before a native media session exists / when the accessor
        // fails; they are not the authority.
        internal const val FALLBACK_TARGET_MS = 60
        internal const val FALLBACK_HARD_LIMIT_MS = 200

        /**
         * Soft-trim hysteresis: only trim the application ring when the
         * backlog exceeds the target by more than this band. This keeps normal
         * inter-datagram variation from trimming a frame on every loop tick.
         *
         * DELIBERATE M5-03 DEVIATION: §8's four-intervention list governs
         * concealment accounting, not buffer management. A soft trim is
         * counted separately (`recordSoftTrim` / `playback.trim`) and is never
         * a concealment; only the hard-limit jump is `ReAnchor`.
         */
        internal const val SOFT_TRIM_HYSTERESIS_MS = 20

        /**
         * M5-08: consecutive loop iterations with fewer than one whole frame
         * available while playback is under way before the reader treats the
         * starvation as a real §8 underrun. The loop sleeps [LOOP_SLEEP_MS]
         * (10 ms) when starved, so 3 iterations ≈ 30 ms — longer than one
         * paced-datagram period, so ordinary inter-datagram backpressure never
         * opens the gate.
         *
         * The platform `AudioTrack.underrunCount` is deliberately NOT a
         * trigger: MIUI reports millions of bogus per-track events while the
         * mixer reports no underruns (M5-08 evidence). It stays a §9
         * diagnostic field only.
         */
        internal const val UNDERRUN_EMPTY_ITERATIONS = 3
    }

    /** Resolved §8 bounds in bytes for the active geometry. */
    internal data class JitterBounds(
        val targetBytes: Int,
        val hardLimitBytes: Int,
        val hysteresisBytes: Int,
    )

    private val running = AtomicBoolean(false)
    @Volatile private var thread: Thread? = null
    @Volatile private var handle: Long = 0

    /** Attach the ring to a freshly created native session (once). */
    fun attach(sessionHandle: Long): Boolean {
        handle = sessionHandle
        return native.attachMediaRing(sessionHandle, ringBuffer.direct)
    }

    fun start() {
        if (!running.compareAndSet(false, true)) return
        val worker = Thread({ loop() }, "v2-media-playback")
        thread = worker
        worker.start()
    }

    fun stop() {
        running.set(false)
        thread?.join(2_000)
        thread = null
    }

    // ---- the playback step (the loop's whole §9 rule, testable) -------------

    private var submittedBytes = 0L

    /**
     * Offer [length] bytes to [sink] and account only what it accepted.
     * Exposed to the unit tests: the loop calls exactly this.
     */
    internal fun offerForPlayback(bytes: ByteArray, length: Int, sink: AudioSink): Int =
        playOnce(sink, bytes, length)

    /** Bytes the sink accepted so far (never ring writes or received media). */
    internal fun submittedBytesTotal(): Long = submittedBytes

    /**
     * M5-11: report accepted bytes plus the hardware-played frame position.
     * `headFrames = null` is sent to native as `-1` (unavailable), which counts
     * a fallback and keeps the accepted-bytes clock; it is never frame 0.
     */
    internal fun reportPlaybackToNative(
        session: Long,
        submittedBytes: Long,
        headFrames: Long?,
        underruns: Long,
        bufferMs: Int,
    ) {
        native.recordPlaybackWithHead(
            session,
            submittedBytes,
            headFrames ?: -1L,
            underruns,
            bufferMs,
        )
    }

    /**
     * M5-03: apply the jitter-buffer steering policy at the reader cursor.
     * A backlog above the target is trimmed (soft, counted separately); above
     * the hard limit it is a §8 `ReAnchor` concealment. Both report the frame
     * count to native. Exposed to the unit tests: the loop calls exactly this.
     */
    internal fun applyReadPosition(
        writePosition: Long,
        readPosition: Long,
        targetBytes: Int,
        hardLimitBytes: Int,
        hysteresisBytes: Int,
        frameBytes: Int,
    ): Long {
        val result = ringBuffer.trim(
            writePosition, readPosition, targetBytes, hardLimitBytes, hysteresisBytes, frameBytes,
        )
        if (result.trimmedFrames > 0) {
            if (result.reanchor) {
                native.recordTrim(handle, result.trimmedFrames)
            } else {
                native.recordSoftTrim(handle, result.trimmedFrames)
            }
        }
        return result.readPosition
    }

    /**
     * Resolve the §8 bounds from native, falling back to the normative values
     * only when the accessor cannot answer (no media session yet).
     */
    internal fun resolveJitterBounds(
        session: Long,
        sampleRate: Int,
        frameBytes: Int,
    ): JitterBounds {
        val targetMs = native.jitterTargetMs(session).takeIf { it > 0 } ?: FALLBACK_TARGET_MS
        val hardLimitMs =
            native.jitterHardLimitMs(session).takeIf { it > 0 } ?: FALLBACK_HARD_LIMIT_MS
        val rate = sampleRate.coerceAtLeast(0)
        val usableFrameBytes = frameBytes.coerceAtLeast(0)
        val targetBytes = (targetMs.coerceAtLeast(0) * rate / 1000) * usableFrameBytes
        val hardLimitBytes = (hardLimitMs.coerceAtLeast(0) * rate / 1000) * usableFrameBytes
        val hysteresisBytes =
            (SOFT_TRIM_HYSTERESIS_MS * rate / 1000) * usableFrameBytes
        return JitterBounds(targetBytes, hardLimitBytes, hysteresisBytes)
    }

    /**
     * §8 underrun fill: no media was ready for one playback period. Feed one
     * period of silence so the platform sink stays fed, and report the
     * inserted frames to native as an `Underrun` concealment. The platform's
     * own `underruns` counter remains a separate §9 field.
     *
     * Returns the whole frames of silence the sink accepted.
     */
    internal fun fillUnderrun(session: Long, sampleRate: Int, sink: AudioSink): Int {
        val frameBytes = sink.frameBytes
        if (frameBytes <= 0 || sampleRate <= 0) return 0
        val frames = (sampleRate.toLong() * LOOP_SLEEP_MS / 1000).toInt().coerceAtLeast(1)
        val silence = ByteArray(frames * frameBytes)
        val accepted = sink.write(silence, silence.size).coerceIn(0, silence.size)
        val acceptedFrames = accepted / frameBytes
        if (acceptedFrames > 0) {
            submittedBytes += acceptedFrames.toLong() * frameBytes
            native.recordUnderrun(session, acceptedFrames.toLong())
        }
        return acceptedFrames
    }

    /** Result of evaluating the §8 underrun gate for one loop iteration. */
    internal data class UnderrunGate(
        /** Whether one period of silence should be fed this iteration. */
        val fill: Boolean,
        /** The platform underrun count seen (a §9 diagnostic, never a trigger). */
        val lastPlatformUnderruns: Long,
        /** Consecutive starving iterations accumulated so far. */
        val consecutiveEmpty: Int,
    )

    /**
     * §8 underrun gating (M5-08). An empty application ring is NOT evidence of
     * an underrun: between two paced datagrams the ring is normally empty for
     * a few milliseconds, and inserting silence for every such gap would
     * inflate the concealment counters.
     *
     * The gate opens only after [UNDERRUN_EMPTY_ITERATIONS] consecutive
     * iterations with fewer than one whole frame available while playback is
     * under way (~30 ms at the 10 ms starved-loop sleep) — a vendor-independent
     * measure of sustained starvation. The platform's `underrunCount` is
     * consumed for the §9 diagnostic but never triggers a fill; on MIUI it
     * counts bogus per-track events (M5-08).
     */
    internal fun underrunGate(
        hasPlayed: Boolean,
        availableBytes: Int,
        frameBytes: Int,
        platformUnderruns: Long,
        lastPlatformUnderruns: Long,
        consecutiveEmpty: Int,
    ): UnderrunGate {
        val emptyNow = hasPlayed && frameBytes > 0 && availableBytes < frameBytes
        val sustainedEmpty = if (emptyNow) consecutiveEmpty + 1 else 0
        val fill = sustainedEmpty >= UNDERRUN_EMPTY_ITERATIONS
        return UnderrunGate(
            fill = fill,
            lastPlatformUnderruns = platformUnderruns,
            // A fill has been charged for this stall; start a fresh window so
            // a continuous stall is not counted on every iteration.
            consecutiveEmpty = if (fill) 0 else sustainedEmpty,
        )
    }

    /**
     * Prebuffer gate: do not start playback until the ring holds the jitter
     * target. Starting on the first datagram would leave the platform starved
     * for the rest of the target interval and manufacture underruns that are
     * not concealment. Once playing, the ring is allowed to drain.
     */
    internal fun readyForPlayback(
        hasPlayed: Boolean,
        availableBytes: Int,
        targetBytes: Int,
    ): Boolean = hasPlayed || targetBytes <= 0 || availableBytes >= targetBytes

    /** Result of applying one §8 drift frame adjustment. */
    internal data class DriftApplication(val readPosition: Long, val appliedFrames: Int)

    /**
     * M5-04 §8 sustained-deviation correction at the playout point.
     *
     * Native decides the signed frame count (positive insert, negative delete)
     * after the buffer has stayed >20 ms from target for >5 s. Insertion feeds
     * silent frames to the sink (the platform accepted them, so they count as
     * submitted); deletion advances the reader over buffered media without
     * submitting it. Returns what was actually applied so the caller can
     * acknowledge exactly that (native counts the concealment on the ack).
     */
    internal fun applyDriftIntervention(
        frames: Int,
        sink: AudioSink,
        readPosition: Long,
        writePosition: Long,
    ): DriftApplication {
        val frameBytes = sink.frameBytes
        if (frames == 0 || frameBytes <= 0) return DriftApplication(readPosition, 0)
        return if (frames > 0) {
            val silence = ByteArray(frames * frameBytes)
            val accepted = sink.write(silence, silence.size).coerceIn(0, silence.size)
            val acceptedFrames = accepted / frameBytes
            submittedBytes += acceptedFrames.toLong() * frameBytes
            DriftApplication(readPosition, acceptedFrames)
        } else {
            val wanted = -frames
            val availableFrames =
                ((writePosition - readPosition).coerceAtLeast(0) / frameBytes).toInt()
            val deleted = minOf(wanted, availableFrames)
            DriftApplication(readPosition + deleted.toLong() * frameBytes, -deleted)
        }
    }

    /**
     * Apply the §8 rate correction and any pending whole-frame adjustment.
     * Returns the (possibly advanced) reader position. Exposed to the unit
     * tests: the playback loop calls exactly this once per report interval.
     */
    internal fun applyDriftControl(
        session: Long,
        sampleRate: Int,
        sink: AudioSink,
        readPosition: Long,
        writePosition: Long,
    ): Long {
        sink.setPlaybackRateHz(playbackRateHz(sampleRate, native.driftCorrectionPpm(session)))
        val adjust = native.takeDriftIntervention(session)
        if (adjust == 0) return readPosition
        val applied = applyDriftIntervention(adjust, sink, readPosition, writePosition)
        if (applied.appliedFrames != 0) {
            native.recordDriftIntervention(session, applied.appliedFrames)
        }
        return applied.readPosition
    }

    private fun loop() {
        var sink: AudioSink? = null
        var sinkKey: String? = null
        var readPosition = 0L
        var scratch = ByteArray(0)
        var lastReport = 0L
        // §8 underrun fill is only counted after real media has played once:
        // an empty ring before the first data is startup, not an underrun.
        var hasPlayed = false
        // Last platform underrun count seen (§9 diagnostic, never a trigger).
        var lastPlatformUnderruns = 0L
        // M5-08: consecutive starving iterations while playing.
        var consecutiveEmpty = 0
        // Native's JitterConfig is fixed for a geometry; cache it per sink.
        var jitterBounds: JitterBounds? = null
        // M5-11: keep the final best-effort report on the same clock source as
        // the last periodic report, so closing does not reset the window and
        // leave the closing interval with an invalid estimate.
        var usingHardwareHead = false
        var lastHeadFrames: Long? = null
        try {
            while (running.get()) {
                val session = handle
                val state = native.getState(session)
                if (state.isNullOrEmpty() || state == "CLOSED") break
                val formatJson = native.mediaFormatJson(session)

                if (formatJson.isNullOrEmpty()) {
                    // READY / not PLAYING: no media exists. Release the sink but
                    // keep the ring (the same attachment serves the next instance).
                    if (sink != null) {
                        sink.flush()
                        sink.release()
                        sink = null
                        sinkKey = null
                    }
                    usingHardwareHead = false
                    lastHeadFrames = null
                    readPosition = 0
                    submittedBytes = 0
                    hasPlayed = false
                    lastPlatformUnderruns = 0
                    consecutiveEmpty = 0
                    jitterBounds = null
                    Thread.sleep(LOOP_SLEEP_MS)
                    continue
                }

                val format = JSONObject(formatJson)
                val instance = format.getInt("instance")
                val generation = format.getInt("generation")
                val sampleRate = format.getInt("sample_rate")
                val channels = format.getInt("channels")
                val sampleFormat = format.getInt("sample_format")
                val key = "$instance/$generation/$sampleRate/$channels/$sampleFormat"
                if (key != sinkKey) {
                    sink?.release()
                    sink = AudioSink.open(sampleRate, channels, sampleFormat)
                    sinkKey = key
                    usingHardwareHead = false
                    lastHeadFrames = null
                    readPosition = 0
                    submittedBytes = 0
                    hasPlayed = false
                    lastPlatformUnderruns = 0
                    consecutiveEmpty = 0
                    if (sink == null) {
                        // Unsupported platform mapping is a 403-class condition;
                        // nothing to play, so stop consuming rather than spin.
                        Log.w(TAG, "unsupported playback geometry $key")
                        Thread.sleep(REPORT_INTERVAL_MS)
                        continue
                    }
                    val frameBytes = sink.frameBytes
                    scratch = ByteArray(maxOf(frameBytes, 4096 - 4096 % frameBytes))
                    jitterBounds = resolveJitterBounds(session, sampleRate, frameBytes)
                    // Native restarts its ring cursor per instance; begin at the
                    // writer's cursor so a late subscriber never plays pre-anchor audio.
                    readPosition = native.ringPosition(session)
                    lastReport = System.nanoTime()
                }

                val currentSink = sink ?: continue
                val frameBytes = currentSink.frameBytes
                val writePosition = native.ringPosition(session)
                // §8: the application-layer jitter buffer keeps a bounded
                // backlog. Backlog above the target is trimmed (soft, counted
                // separately); above the hard limit it is a ReAnchor
                // concealment. The bounds come from native JitterConfig.
                val bounds = jitterBounds
                    ?: resolveJitterBounds(session, sampleRate, frameBytes).also { jitterBounds = it }
                readPosition = applyReadPosition(
                    writePosition,
                    readPosition,
                    bounds.targetBytes,
                    bounds.hardLimitBytes,
                    bounds.hysteresisBytes,
                    frameBytes,
                )
                val available = ringBuffer.available(writePosition, readPosition)
                // Consume any platform underrun event for §9 (it is never a
                // fill trigger), and require sustained ring emptiness.
                val gate = underrunGate(
                    hasPlayed,
                    available,
                    frameBytes,
                    currentSink.underruns(),
                    lastPlatformUnderruns,
                    consecutiveEmpty,
                )
                lastPlatformUnderruns = gate.lastPlatformUnderruns
                consecutiveEmpty = gate.consecutiveEmpty
                // Prebuffer to the jitter target before the first write: an
                // empty ring at startup is not an underrun.
                if (readyForPlayback(hasPlayed, available, bounds.targetBytes) &&
                    available >= frameBytes
                ) {
                    val toRead = minOf(available, scratch.size)
                    val copied = ringBuffer.read(readPosition, toRead, frameBytes, scratch)
                    if (copied > 0) {
                        // Only the sink's accepted bytes count as played (§9).
                        val accepted = playOnce(currentSink, scratch, copied)
                        readPosition += accepted
                        if (accepted > 0) hasPlayed = true
                    }
                } else if (gate.fill) {
                    // §8 underrun fill is gated on sustained ring emptiness
                    // (~30 ms while playing), not the unreliable platform
                    // underrun counter. A brief empty ring between paced
                    // datagrams is normal backpressure, not concealment.
                    fillUnderrun(session, sampleRate, currentSink)
                }

                val now = System.nanoTime()
                if (now - lastReport >= REPORT_INTERVAL_MS * 1_000_000) {
                    lastReport = now
                    // §8 drift correction: smooth ±100 ppm rate plus a bounded
                    // whole-frame last resort. Native owns the measurement and
                    // the decision; this applies and acknowledges it.
                    readPosition = applyDriftControl(
                        session,
                        sampleRate,
                        currentSink,
                        readPosition,
                        native.ringPosition(session),
                    )
                    val bufferedFrames = (available / frameBytes).toLong()
                    val bufferMs = ((bufferedFrames * 1_000) / sampleRate.coerceAtLeast(1)).toInt()
                    // §9: report the ACTUAL buffered media only. M4 has no
                    // target-buffer algorithm; inventing a 40 ms floor would
                    // be a fabricated diagnostic value.
                    val headFrames = currentSink.playbackHeadFrames()
                    usingHardwareHead = headFrames != null
                    if (headFrames != null) lastHeadFrames = headFrames
                    reportPlaybackToNative(
                        session,
                        submittedBytes,
                        headFrames,
                        currentSink.underruns(),
                        bufferMs,
                    )
                }
                val sleepFor = if (available >= frameBytes) 1L else LOOP_SLEEP_MS
                Thread.sleep(sleepFor)
            }
        } catch (interrupted: InterruptedException) {
            Thread.currentThread().interrupt()
        } catch (error: Throwable) {
            Log.w(TAG, "playback loop stopped: ${error.message}")
        } finally {
            sink?.release()
            // Best-effort final report so the Server's last snapshot is honest.
            if (handle != 0L) {
                try {
                    // The sink is released, so reuse the last sampled head to
                    // keep the closing report on the same clock source.
                    reportPlaybackToNative(
                        handle,
                        submittedBytes,
                        if (usingHardwareHead) lastHeadFrames else null,
                        sink?.underruns() ?: 0,
                        0,
                    )
                } catch (_: Throwable) {
                }
            }
        }
    }

    /**
     * Offer [length] bytes to [sink] and account what it accepted.
     *
     * The whole §9 rule lives here: `write`'s return value is the only playback
     * evidence — a partial write or an error is not progress.
     */
    private fun playOnce(sink: AudioSink, bytes: ByteArray, length: Int): Int {
        val frameBytes = sink.frameBytes
        if (frameBytes <= 0) return 0
        val usable = length - length % frameBytes
        if (usable <= 0) return 0
        val accepted = sink.write(bytes, usable).coerceIn(0, usable)
        val acceptedFrames = accepted / frameBytes
        submittedBytes += acceptedFrames.toLong() * frameBytes
        return acceptedFrames * frameBytes
    }
}
