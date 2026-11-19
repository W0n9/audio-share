package dev.audioshare.v2

import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * The one PCM ring per session (ticket 14). Native writes into this direct
 * buffer; [MediaPlayer] reads from it. No PCM ever crosses JNI per datagram.
 *
 * The reader is a monotonic byte cursor, not a mutating `position()`: native
 * owns an independent write cursor ([ClientNative.ringPosition]). A reader
 * that falls further behind than the capacity has been lapped and resyncs to
 * the writer's cursor — old audio is dropped rather than played late.
 */
class MediaRingBuffer(capacityBytes: Int = DEFAULT_CAPACITY) {
    companion object {
        /** ~1.3 s at 48 kHz stereo F32LE; comfortably above the 500 ms native bound. */
        const val DEFAULT_CAPACITY = 256 * 1024
    }

    private val buffer: ByteBuffer =
        ByteBuffer.allocateDirect(capacityBytes).order(ByteOrder.LITTLE_ENDIAN)

    /** The direct buffer handed to native exactly once. */
    val direct: ByteBuffer get() = buffer

    val capacity: Int get() = buffer.capacity()

    /** Bytes the reader still has to consume, given the writer's cursor. */
    fun available(writePosition: Long, readPosition: Long): Int {
        if (writePosition < 0 || writePosition <= readPosition) return 0
        val lag = writePosition - readPosition
        if (lag > capacity) return 0 // lapped: caller must resync
        return lag.toInt()
    }

    /** Whether the writer lapped this reader (data was lost). */
    fun lapped(writePosition: Long, readPosition: Long): Boolean =
        writePosition >= 0 && writePosition - readPosition > capacity

    /**
     * M5-03: the reader's re-anchor position given the jitter-buffer bounds.
     *
     * When the unread backlog (`write - read`) exceeds [hardLimitBytes], the
     * oldest media is stale: jump the read cursor forward to `write - target`,
     * frame-aligned, so only [targetBytes] of the newest media remain. Keeping
     * a small target backlog (rather than snapping to `write`) avoids the
     * permanent startup fill and gives the sink a little cushion, which is how
     * a re-anchor avoids an audible click. A backlog within the hard limit is
     * left untouched.
     *
     * Returns the new read position (never greater than the write cursor).
     */
    fun trimmedReadPosition(
        writePosition: Long,
        readPosition: Long,
        targetBytes: Int,
        hardLimitBytes: Int,
        frameBytes: Int,
    ): Long {
        if (frameBytes <= 0 || writePosition < 0 || writePosition <= readPosition) return readPosition
        val backlog = writePosition - readPosition
        if (backlog <= hardLimitBytes) return readPosition
        var target = targetBytes.coerceAtLeast(0)
        target -= target % frameBytes // frame-align
        val reanchor = writePosition - target
        return if (reanchor > readPosition) reanchor else readPosition
    }

    /**
     * Copy up to [maxBytes] whole frames from ring position [readPosition] into
     * [out]. Returns the number of bytes copied (frame-aligned, 0 when empty).
     */
    fun read(readPosition: Long, maxBytes: Int, frameBytes: Int, out: ByteArray): Int {
        if (frameBytes <= 0 || maxBytes < frameBytes) return 0
        val writable = buffer.duplicate().apply { position(0) }
        val bytes = minOf(maxBytes - maxBytes % frameBytes, out.size - out.size % frameBytes)
        var remaining = bytes
        var ringOffset = (readPosition % capacity).toInt()
        var outOffset = 0
        while (remaining > 0) {
            val run = minOf(remaining, capacity - ringOffset)
            writable.position(ringOffset)
            writable.get(out, outOffset, run)
            outOffset += run
            remaining -= run
            ringOffset = 0
        }
        return bytes
    }
}
