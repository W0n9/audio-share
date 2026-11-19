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
     * Production paired reconnection. The identity private key stays in
     * Android Keystore; [signer] is the `dev.audioshare.v2.ClientSigner`
     * object native code calls back for the public key and raw-context ECDSA
     * signatures. Returns a handle (>0) or 0.
     */
    external fun createAuthSession(
        host: String,
        port: Int,
        pinnedSpkiSha256Hex: String,
        serverPublicKeyHex: String,
        clientName: String,
        signer: ClientSigner,
    ): Long

    /**
     * Start a pairing session (mode 1 = QR, 2 = SAS). QR passes the pin
     * derived from the QR key, the Server key, and the 16-byte nonce; SAS
     * passes empty strings. The session sends `PairingRequest` instead of
     * `AuthInit`. Returns a handle (>0) or 0.
     */
    external fun createPairingSession(
        host: String,
        port: Int,
        mode: Int,
        pinnedSpkiSha256Hex: String,
        serverPublicKeyHex: String,
        pairingNonceHex: String,
        displayName: String,
        signer: ClientSigner,
    ): Long

    /**
     * Confirm a verified pairing. Queues `PairingConfirm` with the binding
     * echo and the Client proof over the raw context. False when no verified
     * pairing is awaiting confirmation.
     */
    external fun confirmPairing(handle: Long): Boolean

    /**
     * JSON pairing display material: `{mode, phase, sas, fingerprint,
     * server_public_key, server_spki_sha256, client_id, display_name,
     * error_code}`. No private keys; SAS/fingerprint are display material.
     * Empty for a non-pairing session.
     */
    external fun getPairingState(handle: Long): String?

    /**
     * JSON public credential after `PairingResult(OK)`: `{
     * server_spki_sha256, server_public_key, client_id, display_name}`.
     * Empty before success.
     */
    external fun getCredentialSummary(handle: Long): String?

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

    /**
     * Attach the one direct [java.nio.ByteBuffer] that carries PCM for this
     * session (ticket 14). Media NEVER crosses JNI per datagram: native writes
     * into this buffer and Kotlin reads from [ringPosition].
     *
     * The buffer must stay alive and untouched for the whole session; a second
     * attachment is refused. Returns false when the buffer is unusable or a
     * ring is already attached.
     */
    external fun attachMediaRing(handle: Long, buffer: java.nio.ByteBuffer): Boolean

    /** Monotonic bytes written into the media ring, or -1 before PLAYING. */
    external fun ringPosition(handle: Long): Long

    /**
     * JSON for the active stream: `instance`, `generation`, `sample_rate`,
     * `channels`, `sample_format` (wire value), `anchor`, `delivered_frames`.
     * Empty before PLAYING; Kotlin rebuilds the AudioTrack only when it changes.
     */
    external fun mediaFormatJson(handle: Long): String?

    /**
     * Report what the platform playback API actually accepted (§9): the bytes
     * successfully written to the AudioTrack since the current instance, the
     * underrun count, and the current ring fill in milliseconds.
     *
     * Received datagrams and ring writes are NOT playback evidence; only this
     * report contributes to `submitted_position`.
     */
    external fun recordPlayback(
        handle: Long,
        submittedBytes: Long,
        underruns: Long,
        bufferCurrentMs: Int,
    )

    /**
     * Report frames dropped by the reader's §8 hard-limit re-anchor. M5-04
     * counts the deletion as `ReAnchor` concealment and advances the
     * submitted position over the skipped media.
     */
    external fun recordTrim(handle: Long, frames: Long)

    /**
     * §8 capture-vs-playback drift estimate in ppm, or 0 until a full 10 s
     * window exists. The accepted §9 diagnostics schema has no drift field;
     * this accessor (and the `playback.drift` log line) is the non-protocol L1
     * channel.
     */
    external fun driftPpm(handle: Long): Int

    /**
     * Smooth, ±100 ppm-clamped playback-rate correction. Apply with
     * `AudioTrack.setPlaybackRate(playbackRateHz(sampleRate, ppm))`.
     */
    external fun driftCorrectionPpm(handle: Long): Int

    /**
     * Take the pending whole-frame drift adjustment (+ insert silence,
     * − delete frames). Taking clears it; the concealment is counted only
     * when [recordDriftIntervention] reports what was applied.
     */
    external fun takeDriftIntervention(handle: Long): Int

    /** Acknowledge the frames actually applied (positive insert, negative delete). */
    external fun recordDriftIntervention(handle: Long, frames: Int)
}
