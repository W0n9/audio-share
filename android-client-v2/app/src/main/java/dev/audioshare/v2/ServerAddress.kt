package dev.audioshare.v2

/**
 * A Server's network address. This is the one rule for "same Server address"
 * across the Client: the strict `host:port` pair, never the free-form
 * `addressHint` string (two spellings of the same host are not the same
 * Server, and a hint from a different host must not delete this record).
 */
internal data class ServerAddress(val host: String, val port: Int) {
    companion object {
        /** A complete `host:port`, or null when the record never stored one. */
        fun of(host: String, port: Int): ServerAddress? =
            if (host.isBlank() || port <= 0) null else ServerAddress(host, port)

        /** The `addr` rules of pairing.md §4.2, or null when malformed. */
        fun parse(text: String): ServerAddress? {
            val trimmed = text.trim()
            if (trimmed.isEmpty()) return null
            return try {
                val (host, port) = PairingPayload.validateAddress(trimmed)
                ServerAddress(host, port)
            } catch (_: Exception) {
                null
            }
        }
    }
}

/** The record's strict `host:port`; legacy records without one do not match. */
internal fun CredentialRecord.serverAddress(): ServerAddress? =
    ServerAddress.of(host, port)

/** Whether two records name the same Server (`host:port` only). */
internal fun CredentialRecord.isSameServerAddressAs(other: CredentialRecord): Boolean {
    val mine = serverAddress() ?: return false
    return mine == other.serverAddress()
}

/** The UI's address string, parsed with the same strict rule. */
internal fun StoredServer.serverAddress(): ServerAddress? =
    ServerAddress.parse(address)

/** Whether two UI records name the same Server (`host:port` only). */
internal fun StoredServer.isSameServerAddressAs(other: StoredServer): Boolean {
    val mine = serverAddress() ?: return false
    return mine == other.serverAddress()
}
