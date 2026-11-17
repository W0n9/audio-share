package dev.audioshare.v2

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.util.Log
import org.json.JSONObject
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicBoolean

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
            // Three platform buffers of headroom: enough for a 100 ms send-ahead
            // burst without an underrun, small enough to stay low-latency.
            val bufferBytes = maxOf(minBuffer, sampleRate / 10 * bytesPerSample * channels)
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
            return AudioTrackSink(track, bytesPerSample * channels, encoding == AudioFormat.ENCODING_PCM_FLOAT)
        }
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
    }

    companion object {
        private const val TAG = "MediaPlayer"
        private const val LOOP_SLEEP_MS = 10L
        private const val REPORT_INTERVAL_MS = 100L
    }

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

    private fun loop() {
        var sink: AudioSink? = null
        var sinkKey: String? = null
        var readPosition = 0L
        var scratch = ByteArray(0)
        var lastReport = 0L
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
                    if (sink == null) {
                        // Unsupported platform mapping is a 403-class condition;
                        // nothing to play, so stop consuming rather than spin.
                        Log.w(TAG, "unsupported playback geometry $key")
                        Thread.sleep(REPORT_INTERVAL_MS)
                        continue
                    }
                    val frameBytes = sink.frameBytes
                    scratch = ByteArray(maxOf(frameBytes, 4096 - 4096 % frameBytes))
                    // Native restarts its ring cursor per instance; begin at the
                    // writer's cursor so a late subscriber never plays pre-anchor audio.
                    readPosition = native.ringPosition(session)
                    lastReport = System.nanoTime()
                }

                val currentSink = sink ?: continue
                val frameBytes = currentSink.frameBytes
                val writePosition = native.ringPosition(session)
                if (ringBuffer.lapped(writePosition, readPosition)) {
                    // The producer outran us: resync to live audio (M4 drops the
                    // backlog; M5 owns re-anchoring policy and concealment).
                    readPosition = writePosition
                }
                val available = ringBuffer.available(writePosition, readPosition)
                if (available >= frameBytes) {
                    val toRead = minOf(available, scratch.size)
                    val copied = ringBuffer.read(readPosition, toRead, frameBytes, scratch)
                    if (copied > 0) {
                        // Only the sink's accepted bytes count as played (§9).
                        val accepted = playOnce(currentSink, scratch, copied)
                        readPosition += accepted
                    }
                }

                val now = System.nanoTime()
                if (now - lastReport >= REPORT_INTERVAL_MS * 1_000_000) {
                    lastReport = now
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
