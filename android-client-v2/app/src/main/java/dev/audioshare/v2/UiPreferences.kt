package dev.audioshare.v2

import android.content.Context

/**
 * UI preferences separate from the pairing credential store. Only the last
 * Server address is persisted; the credential store owns paired-server data.
 */
interface UiPreferences {
    fun lastAddress(): String?
    fun saveLastAddress(address: String)
}

class SharedPreferencesUiPreferences(context: Context) : UiPreferences {
    private val prefs = context.applicationContext
        .getSharedPreferences(FILE, Context.MODE_PRIVATE)

    override fun lastAddress(): String? =
        prefs.getString(KEY_LAST_ADDRESS, null)?.trim()?.ifEmpty { null }

    override fun saveLastAddress(address: String) {
        prefs.edit().putString(KEY_LAST_ADDRESS, address.trim()).apply()
    }

    companion object {
        private const val FILE = "v2_ui"
        private const val KEY_LAST_ADDRESS = "last_server_address"
    }
}
