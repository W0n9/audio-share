package dev.audioshare.v2

import android.media.AudioAttributes
import android.media.AudioFormat
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
    }

    /** Resolved §8 bounds in bytes for the active geometry. */
    internal data class JitterBounds(val targetBytes: Int, val hardLimitBytes: Int)

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
        frameBytes: Int,
    ): Long {
        val result = ringBuffer.trim(
            writePosition, readPosition, targetBytes, hardLimitBytes, frameBytes,
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
        return JitterBounds(targetBytes, hardLimitBytes)
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
        // Native's JitterConfig is fixed for a geometry; cache it per sink.
        var jitterBounds: JitterBounds? = null
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
                    readPosition = 0
                    submittedBytes = 0
                    hasPlayed = false
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
                    readPosition = 0
                    submittedBytes = 0
                    hasPlayed = false
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
                    frameBytes,
                )
                val available = ringBuffer.available(writePosition, readPosition)
                if (available >= frameBytes) {
                    val toRead = minOf(available, scratch.size)
                    val copied = ringBuffer.read(readPosition, toRead, frameBytes, scratch)
                    if (copied > 0) {
                        // Only the sink's accepted bytes count as played (§9).
                        val accepted = playOnce(currentSink, scratch, copied)
                        readPosition += accepted
                        if (accepted > 0) hasPlayed = true
                    }
                } else if (hasPlayed) {
                    // §8 underrun fill: nothing was ready for this playback
                    // period while playing, so feed one period of silence.
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
                    native.recordPlayback(
                        session,
                        submittedBytes,
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
                    native.recordPlayback(handle, submittedBytes, sink?.underruns() ?: 0, 0)
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
