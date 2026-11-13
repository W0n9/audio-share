package dev.audioshare.v2

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Binder
import android.os.IBinder
import org.json.JSONObject

/**
 * Owns the control session across Activity recreation and screen lock.
 * It reserves a foreground media-playback placeholder; no playback exists yet.
 */
class ControlSessionService : Service() {
    private var session: ClientSession? = null
    private var stateListener: ((String) -> Unit)? = null
    private val binder = ControlBinder(this)

    override fun onBind(intent: Intent?): IBinder = binder

    fun setStateListener(listener: ((String) -> Unit)?) { stateListener = listener }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopSession()
            return START_NOT_STICKY
        }
        startForegroundCompat()
        if (session == null && intent != null && intent.hasValidConfiguration()) {
            session = ClientSession(this, object : ClientSession.Listener {
                override fun onStats(json: JSONObject) = Unit
                override fun onStateChange(state: String) { stateListener?.invoke(state) }
                override fun onLog(line: String) = Unit
            }).also {
                it.bindHintEnabled = intent.getBooleanExtra("bindHint", false)
                it.start(intent.configuration())
            }
        }
        return if (session == null) { stopSelf(); START_NOT_STICKY } else START_STICKY
    }

    private fun Intent.hasValidConfiguration(): Boolean =
        !getStringExtra("host").isNullOrEmpty() &&
            !getStringExtra("pinnedSpki").isNullOrEmpty() &&
            !getStringExtra("serverKey").isNullOrEmpty()

    private fun Intent.configuration() = SessionConfiguration(
        getStringExtra("host") ?: "",
        getIntExtra("port", 65531),
        getStringExtra("pinnedSpki") ?: "",
        getStringExtra("serverKey") ?: "",
        getStringExtra("clientName") ?: "smoke-client",
    )

    private fun startForegroundCompat() {
        val manager = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            manager.createNotificationChannel(NotificationChannel(
                CHANNEL_ID, "Audio Share session", NotificationManager.IMPORTANCE_LOW,
            ))
        }
        val notification = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION") Notification.Builder(this)
        }.setContentTitle("Audio Share control session")
            .setContentText("Connection running; media playback is not implemented.")
            .setSmallIcon(android.R.drawable.stat_notify_sync_noanim)
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
        } else {
            @Suppress("DEPRECATION") startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun stopSession() {
        session?.stop()
        session = null
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() { session?.stop(); session = null; super.onDestroy() }

    companion object {
        const val ACTION_START = "dev.audioshare.v2.CONTROL_START"
        const val ACTION_STOP = "dev.audioshare.v2.CONTROL_STOP"
        private const val CHANNEL_ID = "v2_control_session"
        private const val NOTIFICATION_ID = 42
    }
}

class ControlBinder(internal val service: ControlSessionService) : Binder()
