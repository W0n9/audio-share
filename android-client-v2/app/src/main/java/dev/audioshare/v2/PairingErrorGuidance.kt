package dev.audioshare.v2

/**
 * Typed error names and retry guidance for the §6 registry codes the beta can
 * surface. A v2 failure NEVER silently falls back to v1 or plaintext: every
 * entry states `fallbackToV1 = false` and [automaticRetry] is only true for
 * the transport/capacity codes the bounded-backoff policy already retries.
 */
data class ErrorGuidance(
    val code: Int,
    val name: String,
    val title: String,
    val guidance: String,
    /** The user may retry this operation (possibly with fresh inputs). */
    val retryable: Boolean,
    /** The identity/authorization state needs an explicit re-pair. */
    val requiresRepairing: Boolean,
    /** Whether the session owner may reconnect automatically. */
    val automaticRetry: Boolean,
    /** Always false: v2 never downgrades to v1/plaintext. */
    val fallbackToV1: Boolean = false,
)

object ErrorGuidanceBook {
    private val entries: Map<Int, ErrorGuidance> = listOf(
        ErrorGuidance(
            201, "PAIRING_REJECTED",
            "Pairing was rejected",
            "Start a new pairing on the Server and scan a fresh QR code or use a new pairing code. " +
                "The previous invitation is single-use.",
            retryable = true, requiresRepairing = true, automaticRetry = false,
        ),
        ErrorGuidance(
            202, "AUTHORIZATION_UNKNOWN",
            "This phone is not paired with the Server",
            "Pair the phone again. The Server does not recognize this identity.",
            retryable = true, requiresRepairing = true, automaticRetry = false,
        ),
        ErrorGuidance(
            203, "AUTHENTICATION_FAILED",
            "The Server rejected the identity proof",
            "Re-pair the phone. If the problem repeats, check the Server clock and identity files.",
            retryable = true, requiresRepairing = true, automaticRetry = false,
        ),
        ErrorGuidance(
            204, "AUTHORIZATION_DENIED",
            "This device was revoked",
            "Ask the Server operator to pair this device again. A revoked device cannot reconnect.",
            retryable = false, requiresRepairing = true, automaticRetry = false,
        ),
        ErrorGuidance(
            205, "SERVER_IDENTITY_MISMATCH",
            "The Server identity does not match the saved pin",
            "Verify you are connecting to the right Server, then remove the saved pairing and pair " +
                "again. The saved pin is never replaced automatically.",
            retryable = false, requiresRepairing = true, automaticRetry = false,
        ),
        ErrorGuidance(
            206, "PAIRING_PENDING_TIMEOUT",
            "The pairing invitation expired",
            "Start a new pairing on the Server and try again. Pairing invitations last ten minutes.",
            retryable = true, requiresRepairing = false, automaticRetry = false,
        ),
        ErrorGuidance(
            301, "BUSY",
            "The Server is busy",
            "Wait a moment; the app retries automatically with bounded backoff.",
            retryable = true, requiresRepairing = false, automaticRetry = true,
        ),
        ErrorGuidance(
            302, "RATE_LIMITED",
            "Too many attempts",
            "Wait a minute before trying again. Repeated failures are rate-limited by the Server.",
            retryable = true, requiresRepairing = false, automaticRetry = true,
        ),
        ErrorGuidance(
            303, "SESSION_REPLACED",
            "Another session took over this device",
            "This session was closed because the same device connected elsewhere. Reconnect if needed.",
            retryable = false, requiresRepairing = false, automaticRetry = false,
        ),
        ErrorGuidance(
            401, "MALFORMED_MESSAGE",
            "Protocol error",
            "The Server and app disagreed on the wire format. Update the app or report the issue.",
            retryable = false, requiresRepairing = false, automaticRetry = false,
        ),
        ErrorGuidance(
            402, "INVALID_STATE",
            "Unexpected protocol state",
            "Reconnect. If this repeats, report the issue.",
            retryable = false, requiresRepairing = false, automaticRetry = false,
        ),
        ErrorGuidance(
            403, "UNSUPPORTED_MEDIA_CONFIGURATION",
            "This phone cannot play the Server's audio format",
            "Ask the Server operator to choose a supported PCM format.",
            retryable = false, requiresRepairing = false, automaticRetry = false,
        ),
        ErrorGuidance(
            501, "CAPTURE_UNAVAILABLE",
            "The Server has no audio to send",
            "Wait for the Server to start capture, then retry.",
            retryable = true, requiresRepairing = false, automaticRetry = false,
        ),
        ErrorGuidance(
            502, "STREAM_ENDED",
            "The audio stream ended",
            "The Server stopped sending audio. Reconnect to continue.",
            retryable = true, requiresRepairing = false, automaticRetry = false,
        ),
        ErrorGuidance(
            503, "SERVER_STOPPING",
            "The Server is stopping",
            "The Server is shutting down; reconnect after it restarts.",
            retryable = true, requiresRepairing = false, automaticRetry = true,
        ),
        ErrorGuidance(
            601, "NETWORK_FAILURE",
            "Connection lost",
            "Check the network. The app retries automatically with bounded backoff.",
            retryable = true, requiresRepairing = false, automaticRetry = true,
        ),
        ErrorGuidance(
            602, "DATAGRAM_UNSUPPORTED",
            "The network path cannot carry audio",
            "Reconnect. If the network is blocking UDP, switch networks or use a VPN.",
            retryable = true, requiresRepairing = false, automaticRetry = true,
        ),
    ).associateBy { it.code }

    fun forCode(code: Int?): ErrorGuidance? = code?.let(entries::get)

    fun all(): List<ErrorGuidance> = entries.values.toList()
}
