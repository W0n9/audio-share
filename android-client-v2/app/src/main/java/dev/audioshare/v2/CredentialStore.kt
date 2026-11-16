package dev.audioshare.v2

import android.content.Context
import java.time.Instant
import org.json.JSONArray
import org.json.JSONObject

/**
 * One paired Server record (pairing.md §8.2, extended with the fields this
 * beta needs). Public data only: the Client private key never appears here.
 */
data class CredentialRecord(
    val serverSpkiSha256Hex: String,
    val serverPublicKeyHex: String,
    val addressHint: String,
    val keyAlias: String,
    val pairedAt: String,
    val softwareBacked: Boolean,
    val clientId: String,
    val displayName: String?,
    /** The Server host the record was created against (editable hint). */
    val host: String = "",
    val port: Int = 65531,
)

/** Client-side pairing store (one record per paired Server). */
interface CredentialStore {
    /** Insert or replace the record for its Server pin. */
    fun upsert(record: CredentialRecord)
    fun all(): List<CredentialRecord>
    fun find(serverSpkiSha256Hex: String): CredentialRecord?
    fun delete(serverSpkiSha256Hex: String)
}

/** JSON codec for the credential list; pure and JVM-testable. */
object CredentialCodec {
    private const val VERSION = 1

    fun encodeList(records: List<CredentialRecord>): String {
        val root = JSONObject()
        root.put("version", VERSION)
        val array = JSONArray()
        for (record in records) array.put(encode(record))
        root.put("credentials", array)
        return root.toString()
    }

    fun decodeList(json: String?): List<CredentialRecord> {
        if (json.isNullOrEmpty()) return emptyList()
        return try {
            val root = JSONObject(json)
            if (root.optInt("version", 0) != VERSION) return emptyList()
            val array = root.optJSONArray("credentials") ?: return emptyList()
            buildList {
                for (i in 0 until array.length()) {
                    decode(array.getJSONObject(i))?.let(::add)
                }
            }
        } catch (_: Exception) {
            emptyList()
        }
    }

    fun encode(record: CredentialRecord): JSONObject = JSONObject().apply {
        put("server_spki_sha256", record.serverSpkiSha256Hex)
        put("server_public_key", record.serverPublicKeyHex)
        put("address_hint", record.addressHint)
        put("key_alias", record.keyAlias)
        put("paired_at", record.pairedAt)
        put("software_backed", record.softwareBacked)
        put("client_id", record.clientId)
        put("display_name", record.displayName ?: "")
        put("host", record.host)
        put("port", record.port)
    }

    fun decode(obj: JSONObject): CredentialRecord? {
        val pin = obj.optString("server_spki_sha256")
        val serverKey = obj.optString("server_public_key")
        val alias = obj.optString("key_alias")
        // A record without its public identity material is unusable; never
        // silently "repair" it with a different key.
        if (pin.length != 64 || serverKey.length != 64 || alias.isEmpty()) return null
        return CredentialRecord(
            serverSpkiSha256Hex = pin,
            serverPublicKeyHex = serverKey,
            addressHint = obj.optString("address_hint"),
            keyAlias = alias,
            pairedAt = obj.optString("paired_at"),
            softwareBacked = obj.optBoolean("software_backed", false),
            clientId = obj.optString("client_id"),
            displayName = obj.optString("display_name").ifEmpty { null },
            host = obj.optString("host"),
            port = obj.optInt("port", 65531),
        )
    }
}

/** SharedPreferences-backed store (no new dependency; app-private storage). */
class SharedPreferencesCredentialStore(context: Context) : CredentialStore {
    private val prefs = context.applicationContext
        .getSharedPreferences(FILE, Context.MODE_PRIVATE)

    override fun upsert(record: CredentialRecord) {
        synchronized(this) {
            val records = all().filterNot { it.serverSpkiSha256Hex == record.serverSpkiSha256Hex }
                .toMutableList()
            records.add(record)
            prefs.edit().putString(KEY, CredentialCodec.encodeList(records)).apply()
        }
    }

    override fun all(): List<CredentialRecord> =
        CredentialCodec.decodeList(prefs.getString(KEY, null))

    override fun find(serverSpkiSha256Hex: String): CredentialRecord? =
        all().firstOrNull { it.serverSpkiSha256Hex == serverSpkiSha256Hex }

    override fun delete(serverSpkiSha256Hex: String) {
        synchronized(this) {
            val records = all().filterNot { it.serverSpkiSha256Hex == serverSpkiSha256Hex }
            prefs.edit().putString(KEY, CredentialCodec.encodeList(records)).apply()
        }
    }

    companion object {
        private const val FILE = "v2_paired_servers"
        private const val KEY = "credentials"
    }
}

/** RFC 3339 UTC timestamp for [CredentialRecord.pairedAt]. */
fun pairedAtNow(): String = Instant.now().toString()
