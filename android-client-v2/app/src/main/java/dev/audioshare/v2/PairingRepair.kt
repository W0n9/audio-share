package dev.audioshare.v2

/**
 * SAS re-pairing identity rule (ADR-0006 §2): when the typed address names an
 * already paired Server, reuse that record's `keyAlias` so the Server sees the
 * same identity and performs the explicit Restore instead of adding a second
 * device row. Returns `null` when nothing matches, in which case the caller
 * mints a fresh identity exactly as before.
 *
 * Pure and Android-free so the rule is unit-testable without Keystore.
 */
internal fun storedKeyAliasForAddress(
    address: String,
    records: List<CredentialRecord>,
): String? {
    val trimmed = address.trim()
    if (trimmed.isEmpty()) return null
    val (host, port) = try {
        PairingPayload.validateAddress(trimmed)
    } catch (_: Exception) {
        return null
    }
    return records.firstOrNull { record ->
        (record.host.isNotEmpty() && record.host == host && record.port == port) ||
            (record.addressHint.isNotEmpty() && record.addressHint == trimmed)
    }?.keyAlias
}
