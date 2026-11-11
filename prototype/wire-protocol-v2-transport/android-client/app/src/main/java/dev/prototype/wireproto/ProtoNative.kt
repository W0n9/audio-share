package dev.prototype.wireproto

import java.nio.ByteBuffer

/**
 * Low-frequency JNI surface to the Rust cdylib `audio_share_proto_native`.
 *
 * The package/class name is load-bearing: the native side exports
 * Java_dev_prototype_wireproto_ProtoNative_* symbols.
 *
 * Media NEVER crosses JNI per datagram: Kotlin attaches ONE direct ByteBuffer
 * once (see [attachBuffer]); the native ring writer fills it and Kotlin reads
 * it directly, using the monotonic write cursor from getStats JSON.
 */
object ProtoNative {
    init {
        System.loadLibrary("audio_share_proto_native")
    }

    /** Returns a session handle (>0), or 0 on failure. */
    external fun createSession(
        host: String,
        port: Int,
        pinnedSpkiSha256Hex: String,
        credential: String,
    ): Long

    /**
     * Start the QUIC session (non-blocking; connect completes on the native
     * side). [bindLocalAddress] is the network-migration scaffold: the active
     * Android Network's local address, or "" for system default routing.
     */
    external fun connect(handle: Long, bindLocalAddress: String): Boolean

    /** Legal only in READY; rejected (and counted as a control error) otherwise. */
    external fun subscribe(handle: Long): Boolean

    external fun unsubscribe(handle: Long): Boolean

    /** Clean stop: UNSUBSCRIBE if playing, CLOSE(NORMAL), connection close. */
    external fun stop(handle: Long)

    /** Stop + drop the native session (runtime shuts down). Handle is dead after this. */
    external fun destroy(handle: Long)

    /** IDLE / AUTHENTICATING / READY / PLAYING / CLOSED, or "" on error. */
    external fun getState(handle: Long): String?

    /** Full counters JSON; contract documented in the Rust crate (stats.rs). */
    external fun getStats(handle: Long): String?

    /**
     * Attach ONE direct ByteBuffer (must be [ByteBuffer.allocateDirect]); the
     * native side writes verified media bytes into it. Call once per session.
     */
    external fun attachBuffer(handle: Long, buffer: ByteBuffer): Boolean
}
