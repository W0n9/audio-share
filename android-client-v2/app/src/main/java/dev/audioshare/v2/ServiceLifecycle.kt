package dev.audioshare.v2

import android.content.Intent

internal const val CONTROL_CHANNEL_ID = "v2_control_session"
internal const val CONTROL_NOTIFICATION_ID = 42
internal const val CONTROL_FOREGROUND_TYPE = "mediaPlayback"

internal const val ACTION_START = "dev.audioshare.v2.CONTROL_START"
internal const val ACTION_STOP = "dev.audioshare.v2.CONTROL_STOP"

internal object ControlSessionDescriptor {
    val channelId = CONTROL_CHANNEL_ID
    val notificationId = CONTROL_NOTIFICATION_ID
    val foregroundType = CONTROL_FOREGROUND_TYPE
    fun startAction() = ACTION_START
    fun stopAction() = ACTION_STOP
}
