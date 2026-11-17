package dev.audioshare.v2

import androidx.annotation.StringRes

/**
 * The one code -> resource mapping for the §6 error registry.
 *
 * Explicit `when` arms are deliberate: `isShrinkResources` is enabled in
 * release builds, so resources referenced only through
 * `Resources.getIdentifier("error_${code}_title")` would be stripped. Unknown
 * and null codes fall back to `error_generic_title` / `error_generic_guidance`.
 */
object ErrorGuidanceResources {
    @StringRes
    fun titleRes(code: Int?): Int = when (code) {
        201 -> R.string.error_201_title
        202 -> R.string.error_202_title
        203 -> R.string.error_203_title
        204 -> R.string.error_204_title
        205 -> R.string.error_205_title
        206 -> R.string.error_206_title
        301 -> R.string.error_301_title
        302 -> R.string.error_302_title
        303 -> R.string.error_303_title
        401 -> R.string.error_401_title
        402 -> R.string.error_402_title
        403 -> R.string.error_403_title
        501 -> R.string.error_501_title
        502 -> R.string.error_502_title
        503 -> R.string.error_503_title
        601 -> R.string.error_601_title
        602 -> R.string.error_602_title
        else -> R.string.error_generic_title
    }

    @StringRes
    fun guidanceRes(code: Int?): Int = when (code) {
        201 -> R.string.error_201_guidance
        202 -> R.string.error_202_guidance
        203 -> R.string.error_203_guidance
        204 -> R.string.error_204_guidance
        205 -> R.string.error_205_guidance
        206 -> R.string.error_206_guidance
        301 -> R.string.error_301_guidance
        302 -> R.string.error_302_guidance
        303 -> R.string.error_303_guidance
        401 -> R.string.error_401_guidance
        402 -> R.string.error_402_guidance
        403 -> R.string.error_403_guidance
        501 -> R.string.error_501_guidance
        502 -> R.string.error_502_guidance
        503 -> R.string.error_503_guidance
        601 -> R.string.error_601_guidance
        602 -> R.string.error_602_guidance
        else -> R.string.error_generic_guidance
    }
}

/** Camera-scanner notices rendered inline on HOME/PAIRING. */
enum class ScanNotice {
    CAMERA_PERMISSION_DENIED,
}

@StringRes
fun ScanNotice.messageRes(): Int = when (this) {
    ScanNotice.CAMERA_PERMISSION_DENIED -> R.string.scan_permission_denied
}

/** Pairing-local notices. PAIRING behaviour is unchanged by ticket 18. */
enum class PairingNotice {
    SAS_MISMATCH,
    INVALID_PAIRING_LINK,
}

@StringRes
fun PairingNotice.messageRes(): Int = when (this) {
    PairingNotice.SAS_MISMATCH -> R.string.pairing_notice_sas_mismatch
    PairingNotice.INVALID_PAIRING_LINK -> R.string.pairing_notice_invalid_link
}

@StringRes
fun AddressError.messageRes(): Int = when (this) {
    AddressError.REQUIRED -> R.string.address_error_required
    AddressError.NO_SAVED_PAIRING -> R.string.address_error_no_saved_pairing
    AddressError.INVALID_ADDRESS_HINT -> R.string.address_error_invalid
    AddressError.INVALID_BRACKETED_IPV6 -> R.string.address_error_invalid_ipv6
    AddressError.BRACKETED_HOST_NOT_IPV6 -> R.string.address_error_bracketed_host_not_ipv6
    AddressError.INVALID_IPV6 -> R.string.address_error_invalid_ipv6
    AddressError.MISSING_PORT -> R.string.address_error_missing_port
    AddressError.IPV6_MUST_BE_BRACKETED -> R.string.address_error_ipv6_bracketed
    AddressError.INVALID_HOST -> R.string.address_error_invalid_host
    AddressError.INVALID_PORT -> R.string.address_error_invalid_port
    AddressError.PORT_OUT_OF_RANGE -> R.string.address_error_port_range
}
