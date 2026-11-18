package dev.audioshare.v2

/**
 * Typed error names and retry guidance for the §6 registry codes the beta can
 * surface. A v2 failure NEVER silently falls back to v1 or plaintext: every
 * entry states `fallbackToV1 = false` and [automaticRetry] is only true for
 * the transport/capacity codes the bounded-backoff policy already retries.
 *
 * User-visible copy is not carried here: [ErrorGuidanceResources] maps [code]
 * to `error_<code>_title` / `error_<code>_guidance`, with
 * `error_generic_title` / `error_generic_guidance` for unknown or null codes.
 */
data class ErrorGuidance(
    val code: Int,
    val name: String,
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
            retryable = true, requiresRepairing = true, automaticRetry = false,
        ),
        ErrorGuidance(
            202, "AUTHORIZATION_UNKNOWN",
            retryable = true, requiresRepairing = true, automaticRetry = false,
        ),
        ErrorGuidance(
            203, "AUTHENTICATION_FAILED",
            retryable = true, requiresRepairing = true, automaticRetry = false,
        ),
        ErrorGuidance(
            204, "AUTHORIZATION_DENIED",
            retryable = false, requiresRepairing = true, automaticRetry = false,
        ),
        ErrorGuidance(
            205, "SERVER_IDENTITY_MISMATCH",
            retryable = false, requiresRepairing = true, automaticRetry = false,
        ),
        ErrorGuidance(
            206, "PAIRING_PENDING_TIMEOUT",
            retryable = true, requiresRepairing = false, automaticRetry = false,
        ),
        ErrorGuidance(
            301, "BUSY",
            retryable = true, requiresRepairing = false, automaticRetry = true,
        ),
        ErrorGuidance(
            302, "RATE_LIMITED",
            retryable = true, requiresRepairing = false, automaticRetry = true,
        ),
        ErrorGuidance(
            303, "SESSION_REPLACED",
            retryable = false, requiresRepairing = false, automaticRetry = false,
        ),
        ErrorGuidance(
            401, "MALFORMED_MESSAGE",
            retryable = false, requiresRepairing = false, automaticRetry = false,
        ),
        ErrorGuidance(
            402, "INVALID_STATE",
            retryable = false, requiresRepairing = false, automaticRetry = false,
        ),
        ErrorGuidance(
            403, "UNSUPPORTED_MEDIA_CONFIGURATION",
            retryable = false, requiresRepairing = false, automaticRetry = false,
        ),
        ErrorGuidance(
            501, "CAPTURE_UNAVAILABLE",
            retryable = true, requiresRepairing = false, automaticRetry = false,
        ),
        ErrorGuidance(
            502, "STREAM_ENDED",
            retryable = true, requiresRepairing = false, automaticRetry = false,
        ),
        ErrorGuidance(
            503, "SERVER_STOPPING",
            retryable = true, requiresRepairing = false, automaticRetry = true,
        ),
        ErrorGuidance(
            601, "NETWORK_FAILURE",
            retryable = true, requiresRepairing = false, automaticRetry = true,
        ),
        ErrorGuidance(
            602, "DATAGRAM_UNSUPPORTED",
            retryable = true, requiresRepairing = false, automaticRetry = true,
        ),
    ).associateBy { it.code }

    fun forCode(code: Int?): ErrorGuidance? = code?.let(entries::get)

    fun all(): List<ErrorGuidance> = entries.values.toList()
}
