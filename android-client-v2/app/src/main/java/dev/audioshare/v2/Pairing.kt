package dev.audioshare.v2

import java.security.MessageDigest
import java.util.Base64
import org.json.JSONObject

/** A validated `audioshare-pair://p1?...` payload (pairing.md §4.2). */
data class QrPairingPayload(
    /** The address hint, `host:port` exactly as carried. */
    val address: String,
    /** The host part of [address] (IPv6 without brackets). */
    val host: String,
    /** The port part of [address] (1..65535). */
    val port: Int,
    /** The raw 32-byte Ed25519 Server identity key. */
    val serverPublicKey: ByteArray,
    /** The raw 16-byte single-use nonce. */
    val nonce: ByteArray,
    /** SHA-256 of the Server SPKI DER (lowercase hex). */
    val pinHex: String,
) {
    override fun equals(other: Any?): Boolean = other is QrPairingPayload &&
        address == other.address && host == other.host && port == other.port &&
        serverPublicKey.contentEquals(other.serverPublicKey) &&
        nonce.contentEquals(other.nonce) && pinHex == other.pinHex

    override fun hashCode(): Int {
        var result = address.hashCode()
        result = 31 * result + host.hashCode()
        result = 31 * result + port
        result = 31 * result + serverPublicKey.contentHashCode()
        result = 31 * result + nonce.contentHashCode()
        result = 31 * result + pinHex.hashCode()
        return result
    }
}

class QrPayloadException(message: String) : Exception(message)

/**
 * QR payload parsing and identity derivation (pairing.md §4). Fields may
 * appear in any order; unknown or duplicate fields are rejected because a
 * different shape requires a `p2` payload tag.
 */
object PairingPayload {
    const val SCHEME = "audioshare-pair"
    const val TAG = "p1"
    private const val PREFIX = "$SCHEME://$TAG?"
    private const val PROTOCOL_MAJOR = 2
    private const val KEY_LEN = 32
    private const val NONCE_LEN = 16

    /** The fixed 12-byte Ed25519 SubjectPublicKeyInfo algorithm prefix. */
    val ED25519_SPKI_PREFIX = byteArrayOf(
        0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
    )

    fun parse(uri: String): QrPairingPayload {
        if (!uri.startsWith(PREFIX)) throw QrPayloadException("not a $SCHEME://$TAG payload")
        val query = uri.substring(PREFIX.length)
        if (query.isEmpty()) throw QrPayloadException("empty pairing payload")

        var pm: Int? = null
        var addr: String? = null
        var key: ByteArray? = null
        var nonce: ByteArray? = null
        for (pair in query.split('&')) {
            val separator = pair.indexOf('=')
            if (separator <= 0) throw QrPayloadException("malformed query field")
            val name = pair.substring(0, separator)
            val value = pair.substring(separator + 1)
            if (value.contains('%')) {
                throw QrPayloadException("percent-encoding is not permitted")
            }
            when (name) {
                "pm" -> {
                    if (pm != null) throw QrPayloadException("duplicate field pm")
                    pm = value.toIntOrNull() ?: throw QrPayloadException("pm is not decimal")
                }
                "addr" -> {
                    if (addr != null) throw QrPayloadException("duplicate field addr")
                    validateAddress(value)
                    addr = value
                }
                "key" -> {
                    if (key != null) throw QrPayloadException("duplicate field key")
                    key = decodeBase64Url("key", value)
                }
                "nonce" -> {
                    if (nonce != null) throw QrPayloadException("duplicate field nonce")
                    nonce = decodeBase64Url("nonce", value)
                }
                else -> throw QrPayloadException("unknown field $name")
            }
        }
        if (pm != PROTOCOL_MAJOR) throw QrPayloadException("pm must be $PROTOCOL_MAJOR")
        val address = addr ?: throw QrPayloadException("missing field addr")
        val publicKey = key ?: throw QrPayloadException("missing field key")
        val nonceBytes = nonce ?: throw QrPayloadException("missing field nonce")
        if (publicKey.size != KEY_LEN) {
            throw QrPayloadException("key is ${publicKey.size} bytes; expected $KEY_LEN")
        }
        if (nonceBytes.size != NONCE_LEN) {
            throw QrPayloadException("nonce is ${nonceBytes.size} bytes; expected $NONCE_LEN")
        }
        val (host, port) = splitAddress(address)
        return QrPairingPayload(
            address = address,
            host = host,
            port = port,
            serverPublicKey = publicKey,
            nonce = nonceBytes,
            pinHex = pinForServerKey(publicKey),
        )
    }

    /** SHA-256 of the Ed25519 SPKI DER, lowercase hex (pairing.md §4.2/§6.1). */
    fun pinForServerKey(serverPublicKey: ByteArray): String {
        require(serverPublicKey.size == KEY_LEN) { "Server key must be 32 bytes" }
        val spki = ED25519_SPKI_PREFIX + serverPublicKey
        return MessageDigest.getInstance("SHA-256").digest(spki)
            .joinToString("") { "%02x".format(it) }
    }

    /** §7.4 fingerprint: 8 groups of 8 uppercase hex, space-separated. */
    fun fingerprintForServerKey(serverPublicKey: ByteArray): String {
        require(serverPublicKey.size == KEY_LEN) { "Server key must be 32 bytes" }
        val spki = ED25519_SPKI_PREFIX + serverPublicKey
        val digest = MessageDigest.getInstance("SHA-256").digest(spki)
        val hex = digest.joinToString("") { "%02X".format(it) }
        return hex.chunked(8).joinToString(" ")
    }

    /**
     * `addr` validation (pairing.md §4.2): IPv4 dotted quad, bracketed IPv6,
     * or a DNS/mDNS name; port 1–65535; no percent-encoding or whitespace.
     */
    fun validateAddress(value: String): Pair<String, Int> {
        if (value.isEmpty() || value.any { it.isWhitespace() } || value.contains('%')) {
            throw QrPayloadException("invalid address hint")
        }
        val (host, portText) = if (value.startsWith("[")) {
            val end = value.indexOf(']')
            if (end <= 1) throw QrPayloadException("invalid bracketed IPv6")
            val host = value.substring(1, end)
            if (!host.contains(':')) throw QrPayloadException("bracketed host is not IPv6")
            if (!host.matches(Regex("^[0-9A-Fa-f:.]+$"))) {
                throw QrPayloadException("invalid IPv6 address")
            }
            val suffix = value.substring(end + 1)
            if (!suffix.startsWith(":")) throw QrPayloadException("missing port")
            host to suffix.substring(1)
        } else {
            val colon = value.lastIndexOf(':')
            if (colon <= 0) throw QrPayloadException("missing port")
            val host = value.substring(0, colon)
            if (host.contains(':')) throw QrPayloadException("IPv6 must be bracketed")
            if (!host.matches(Regex("^[A-Za-z0-9.-]+$"))) {
                throw QrPayloadException("invalid host")
            }
            host to value.substring(colon + 1)
        }
        val port = portText.toIntOrNull() ?: throw QrPayloadException("invalid port")
        if (port !in 1..65535) throw QrPayloadException("port must be 1..65535")
        return host to port
    }

    private fun splitAddress(value: String): Pair<String, Int> = validateAddress(value)

    private fun decodeBase64Url(field: String, value: String): ByteArray {
        if (value.isEmpty()) throw QrPayloadException("$field is empty")
        if (value.contains('=')) throw QrPayloadException("$field must be unpadded base64url")
        if (!value.matches(Regex("^[A-Za-z0-9_-]+$"))) {
            throw QrPayloadException("$field is not base64url")
        }
        return try {
            Base64.getUrlDecoder().decode(value)
        } catch (e: IllegalArgumentException) {
            throw QrPayloadException("$field is not base64url: ${e.message}")
        }
    }
}

/** The six-digit SAS presentation (pairing.md §7.1/§7.3). */
object SasCode {
    /** `code` → `123 456`. */
    fun format(code: Int): String {
        require(code in 0..999_999) { "SAS code must be 0..999999" }
        return "%03d %03d".format(code / 1000, code % 1000)
    }

    /** Accept `123456`, `123 456`, or ` 123 456 `; null when invalid. */
    fun normalize(input: String): Int? {
        val digits = input.filter { !it.isWhitespace() }
        if (digits.length != 6 || !digits.all { it.isDigit() }) return null
        return digits.toIntOrNull()
    }
}

/** A parsed pairing-state event from the native `getPairingState` JSON. */
data class PairingSnapshot(
    val mode: String,
    val phase: String,
    val sas: String?,
    val fingerprint: String?,
    val serverPublicKeyHex: String?,
    val serverSpkiSha256Hex: String?,
    val clientId: String?,
    val displayName: String?,
    val errorCode: Int?,
) {
    val isAwaitingConfirmation: Boolean get() = phase == "AWAITING_CONFIRMATION"
    val isComplete: Boolean get() = phase == "COMPLETE"
    val isFailed: Boolean get() = phase == "FAILED"

    companion object {
        fun fromJson(json: String?): PairingSnapshot? {
            if (json.isNullOrEmpty()) return null
            return try {
                val obj = JSONObject(json)
                PairingSnapshot(
                    mode = obj.getString("mode"),
                    phase = obj.getString("phase"),
                    sas = obj.optStringOrNull("sas"),
                    fingerprint = obj.optStringOrNull("fingerprint"),
                    serverPublicKeyHex = obj.optStringOrNull("server_public_key"),
                    serverSpkiSha256Hex = obj.optStringOrNull("server_spki_sha256"),
                    clientId = obj.optStringOrNull("client_id"),
                    displayName = obj.optStringOrNull("display_name"),
                    errorCode = if (obj.isNull("error_code")) null else obj.optInt("error_code"),
                )
            } catch (_: Exception) {
                null
            }
        }

        private fun JSONObject.optStringOrNull(name: String): String? =
            if (isNull(name)) null else optString(name).ifEmpty { null }
    }
}

/**
 * Replayable pairing-state holder shared by the Service and the UI.
 *
 * A pairing exchange is a one-shot transcript: after a revoke/reject retry a
 * stopped `ClientSession`'s watcher may still deliver a late snapshot. Source
 * ownership makes the relay reject every event that does not belong to the
 * exchange the Service started last, exactly like [SessionStateRelay].
 */
class PairingStateRelay {
    private val lock = Any()
    private var current: PairingSnapshot? = null
    private var listener: ((PairingSnapshot?) -> Unit)? = null
    private var activeSource: Source? = null

    fun snapshot(): PairingSnapshot? = synchronized(lock) { current }

    fun setListener(next: ((PairingSnapshot?) -> Unit)?) {
        synchronized(lock) {
            listener = next
            next?.invoke(current)
        }
    }

    /**
     * Start a fresh pairing exchange. The previous exchange loses ownership
     * immediately and its snapshot is cleared (and replayed as null), so a
     * stale FAILED/COMPLETE banner cannot survive a new attempt.
     */
    fun beginSession(): Source = synchronized(lock) {
        val previous = activeSource
        activeSource = null
        previous?.closed = true
        Source().also {
            activeSource = it
            publishLocked(null)
        }
    }

    /**
     * Clear the visible snapshot without ending the current exchange: a live
     * source may publish again (for example when the Service returns to HOME).
     */
    fun clear() = synchronized(lock) { publishLocked(null) }

    private fun publishLocked(snapshot: PairingSnapshot?) {
        current = snapshot
        listener?.invoke(snapshot)
    }

    /** The event source of exactly one pairing exchange. */
    inner class Source internal constructor() {
        internal var closed = false

        fun isCurrent(): Boolean = synchronized(lock) { activeSource === this && !closed }

        /** Publish this exchange's snapshot; false when it lost ownership. */
        fun publish(snapshot: PairingSnapshot?): Boolean = synchronized(lock) {
            if (activeSource !== this || closed) return false
            publishLocked(snapshot)
            true
        }

        /** Publish a native JSON snapshot; invalid JSON clears to null. */
        fun publishJson(json: String?): Boolean =
            publish(PairingSnapshot.fromJson(json))

        /** Invalidate this source. It no longer changes the snapshot. */
        fun close() {
            synchronized(lock) { closed = true }
        }
    }
}
