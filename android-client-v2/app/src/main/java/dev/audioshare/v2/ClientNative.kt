package dev.audioshare.v2

/**
 * Low-frequency JNI surface to the Rust cdylib `audio_share_client_v2_native`
 * (ticket 08): connect/auth/stop/state/stats/rebind only. No per-datagram
 * calls — the media path is NOT activated this ticket (the direct-ByteBuffer
 * ring exists on the native side as a skeleton).
 *
 * The package/class name is load-bearing: the native side exports
 * Java_dev_audioshare_v2_ClientNative_* symbols.
 *
 * JNIEnv is used BY VALUE on the native side — the prototype's recorded
 * lesson (jni 0.21 with &mut JNIEnv failed every createSession on device).
 */
object ClientNative {
    init {
        System.loadLibrary("audio_share_client_v2_native")
    }

    /**
     * Returns a session handle (>0), or 0 on failure.
     *
     * [pinnedSpkiSha256Hex] and [serverPublicKeyHex] are the test-only
     * pre-provisioned Server identity material (test config per the ticket;
     * production pairing is a later phase).
     */
    external fun createSession(
        host: String,
        port: Int,
        pinnedSpkiSha256Hex: String,
        serverPublicKeyHex: String,
        clientName: String,
    ): Long

    /**
     * Start the QUIC session (non-blocking; connect completes on the native
     * side, including AuthInit/AuthProof). [bindLocalAddress] is the
     * network-migration scaffold: the active Android Network's local address,
     * or "" for system default routing. The bind hint must stay OFF ("" /
     * noBindHint) when the server is reached through a VPN overlay
     * (prototype device finding, 2026-11-11).
     */
    external fun connect(handle: Long, bindLocalAddress: String): Boolean

    /** Clean stop: CLOSE(701), transport close, no reconnect (spec §6.3). */
    external fun stop(handle: Long)

    /** Stop + drop the native session (runtime shuts down). Handle is dead after this. */
    external fun destroy(handle: Long)

    /** IDLE / AUTHENTICATING / NEGOTIATING / READY / PLAYING / CLOSING / CLOSED, or "" on unknown handle. */
    external fun getState(handle: Long): String?

    /** Full counters JSON (stable shape, no secrets); "" on error. */
    external fun getStats(handle: Long): String?

    /**
     * Network-change migration: rebind the QUIC endpoint onto the UDP socket
     * whose raw fd is passed in. The fd must come from a socket created and
     * route-pinned with [android.net.Network.bindSocket].
     *
     * FD OWNERSHIP: the fd transfers to native on call — on success AND on
     * failure. The JVM side must never close or reuse the socket afterwards.
     *
     * Returns false when the rebind failed; the caller then falls back to
     * teardown + bounded-backoff reconnect.
     */
    external fun rebind(handle: Long, fd: Int): Boolean
}
