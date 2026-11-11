package dev.prototype.wireproto

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log

/**
 * Minimal foreground media-playback Service for the prototype.
 *
 * startForeground is typed correctly per API level:
 *  - API 26-28: startForeground(id, notification)
 *  - API 29+:   startForeground(id, notification, FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
 *
 * START_NOT_STICKY: after process death the system does NOT restart a phantom
 * playback — the service never claims playback after termination (ticket gate).
 */
class MediaPlaybackService : Service() {

    companion object {
        const val CHANNEL_ID = "proto_v2_media"
        const val NOTIFICATION_ID = 41
        const val ACTION_STOP = "dev.prototype.wireproto.STOP"

        @Volatile var isRunning = false
            private set
    }

    private var session: ProtoSession? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            session?.stop()
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
            return START_NOT_STICKY
        }
        promoteToForeground()
        // START_NOT_STICKY: no phantom restart after process death
        return START_NOT_STICKY
    }

    private fun promoteToForeground() {
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= 26) {
            manager.createNotificationChannel(
                NotificationChannel(
                    CHANNEL_ID,
                    "proto v2 media playback",
                    NotificationManager.IMPORTANCE_LOW,
                )
            )
        }
        val contentIntent = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val stopIntent = PendingIntent.getService(
            this,
            1,
            Intent(this, MediaPlaybackService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val notification: Notification = Notification.Builder(this, CHANNEL_ID)
            .setContentTitle("proto-v2 transport session")
            .setContentText("wire-protocol-v2 media prototype")
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setContentIntent(contentIntent)
            .addAction(
                Notification.Action.Builder(
                    null,
                    "Stop",
                    stopIntent,
                ).build()
            )
            .setOngoing(true)
            .build()

        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
        isRunning = true
    }

    /**
     * Attach the session this service fronts. The service itself does not own
     * the connection logic (ProtoSession does); it guarantees the process stays
     * foregrounded while media plays and never represents playback after stop.
     */
    fun attachSession(s: ProtoSession) {
        session = s
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        // app task swiped away: stop everything; never claim playback after
        Log.i("MediaPlaybackService", "task removed: stopping session")
        session?.stop()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        session?.stop()
        isRunning = false
        super.onDestroy()
    }
}
