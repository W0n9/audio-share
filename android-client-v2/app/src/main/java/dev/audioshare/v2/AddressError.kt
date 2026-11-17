package dev.audioshare.v2

/**
 * Typed address-input outcome. State never carries English copy; the UI maps
 * each value to a string resource (`address_error_*`).
 *
 * [REQUIRED] and [NO_SAVED_PAIRING] are form-level errors, not wire-format
 * validation errors, but they occupy the same inline slot on HOME.
 */
enum class AddressError {
    REQUIRED,
    NO_SAVED_PAIRING,
    INVALID_ADDRESS_HINT,
    INVALID_BRACKETED_IPV6,
    BRACKETED_HOST_NOT_IPV6,
    INVALID_IPV6,
    MISSING_PORT,
    IPV6_MUST_BE_BRACKETED,
    INVALID_HOST,
    INVALID_PORT,
    PORT_OUT_OF_RANGE,
}

/**
 * A [QrPayloadException] whose user-facing failure category is typed. The
 * English [message] remains for logs and pairing-payload diagnostics.
 */
class AddressValidationException(
    val reason: AddressError,
    message: String,
) : QrPayloadException(message)
