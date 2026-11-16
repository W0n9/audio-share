package dev.audioshare.v2

import android.app.Activity
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.IBinder

/**
 * TEST-SUPPORT ONLY — compiled only when `-PtestSupport=true` adds
 * `src/testSupport/java` to the main source set.
 *
 * This is the historic adb-extras lifecycle harness. It is reachable only via
 * the `BuildConfig.TEST_SUPPORT` reflection hooks in `MainActivity` and
 * `ControlSessionService`; production builds contain neither this class nor
 * the `host`/`pinnedSpki`/`serverKey` extras path.
 */
object TestSupportHarness {
    private var bound: ServiceConnection? = null

    /** Start and bind the extras-configured session for the instrumentation. */
    @JvmStatic
    fun startActivitySession(activity: Activity): Boolean {
        val configuration = serviceConfiguration(activity.intent) ?: return false
        val intent = Intent(activity, ControlSessionService::class.java).apply {
            action = ControlSessionService.ACTION_START
            putExtras(activity.intent)
        }
        activity.startService(intent)
        if (bound == null) {
            val connection = object : ServiceConnection {
                override fun onServiceConnected(name: ComponentName?, binder: IBinder?) = Unit
                override fun onServiceDisconnected(name: ComponentName?) = Unit
            }
            activity.bindService(intent, connection, Context.BIND_AUTO_CREATE)
            bound = connection
        }
        // Touch the value so the compiler cannot elide the read.
        check(configuration.host.isNotEmpty())
        return true
    }

    /** Parse the old adb-extras configuration, or null when absent. */
    @JvmStatic
    fun serviceConfiguration(intent: Intent?): SessionConfiguration? {
        if (intent == null) return null
        val host = intent.getStringExtra("host").orEmpty()
        val pin = intent.getStringExtra("pinnedSpki").orEmpty()
        val serverKey = intent.getStringExtra("serverKey").orEmpty()
        if (host.isEmpty() || pin.isEmpty() || serverKey.isEmpty()) return null
        return SessionConfiguration(
            host = host,
            port = intent.getIntExtra("port", 65531),
            serverSpkiSha256Hex = pin,
            serverPublicKeyHex = serverKey,
            clientName = intent.getStringExtra("clientName") ?: "smoke-client",
            useNativeTestIdentity = true,
        )
    }
}
