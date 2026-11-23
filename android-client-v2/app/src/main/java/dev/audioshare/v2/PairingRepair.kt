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
    val target = ServerAddress.parse(address) ?: return null
    return records.firstOrNull { it.serverAddress() == target }?.keyAlias
}

/**
 * QR re-pairing identity rule (ADR-0006 §2): when the scanned payload's Server
 * pin matches a stored credential, reuse that record's `keyAlias` -- so a
 * record created by SAS is re-paired by QR with the same identity and the
 * Server performs the explicit Restore instead of adding a second row. Returns
 * `null` when nothing matches, in which case the caller keeps the pin-derived
 * alias exactly as before.
 */
internal fun storedKeyAliasForPin(
    pinHex: String,
    records: List<CredentialRecord>,
): String? = records.firstOrNull { it.serverSpkiSha256Hex == pinHex }?.keyAlias
