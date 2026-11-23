package dev.audioshare.v2

/** Deletes one stored Client identity (Keystore entry or software key files). */
fun interface IdentityKeyDeleter {
    fun delete(alias: String)
}

/**
 * Local "Forget Server" (ADR-0006 §4, GLOSSARY "Forget Server"). No Server
 * channel is involved: the credential record and the Client identity key are
 * removed from this phone.
 *
 * Order matters. The credential delete is authoritative and happens first; a
 * failure to delete the key is logged but never rolls back or hides the
 * credential result. A credential-delete failure propagates to the caller.
 */
class ForgetServer(
    private val credentials: CredentialStore,
    private val deleteIdentity: IdentityKeyDeleter,
    private val log: (String) -> Unit = {},
) {
    /** Returns the removed record, or null when the pin was not paired. */
    fun forget(serverSpkiSha256Hex: String): CredentialRecord? {
        val record = credentials.find(serverSpkiSha256Hex) ?: return null
        credentials.delete(serverSpkiSha256Hex)
        try {
            deleteIdentity.delete(record.keyAlias)
        } catch (e: Exception) {
            log("identity key deletion failed for ${record.keyAlias}: ${e.message}")
        }
        return record
    }
}
