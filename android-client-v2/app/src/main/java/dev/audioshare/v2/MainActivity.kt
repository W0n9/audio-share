package dev.audioshare.v2

import android.app.Activity
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.Bundle
import android.os.IBinder
import android.widget.LinearLayout
import android.widget.TextView

class MainActivity : Activity() {
    private val stateView by lazy { TextView(this) }
    private val logView by lazy { TextView(this) }

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, binder: IBinder) {
            val local = binder as ControlBinder
            local.service.setStateListener { state ->
                runOnUiThread { stateView.text = state }
            }
        }
        override fun onServiceDisconnected(name: ComponentName?) = Unit
    }

    private val serviceIntent by lazy { Intent(this, ControlSessionService::class.java) }
    private var bound = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val layout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(48, 48, 48, 48)
        }
        val title = TextView(this).apply { text = "audio-share v2 control session" }
        stateView.text = "IDLE"
        logView.text = ""
        layout.addView(title)
        layout.addView(stateView)
        layout.addView(logView)
        setContentView(layout)

        val host = intent.getStringExtra("host") ?: ""
        val pin = intent.getStringExtra("pinnedSpki") ?: ""
        val serverKey = intent.getStringExtra("serverKey") ?: ""
        if (host.isEmpty() || pin.isEmpty() || serverKey.isEmpty()) {
            logView.text = "config missing: pass host/pinnedSpki/serverKey extras (test config)"
            return
        }

        val startIntent = serviceIntent.apply {
            action = ControlSessionService.ACTION_START
            putExtras(intent)
        }
        startService(startIntent)
        bound = bindService(startIntent, connection, Context.BIND_AUTO_CREATE)
    }

    private val connectionIntent by lazy { serviceIntent.apply { action = ControlSessionService.ACTION_START } }

    override fun onStart() { super.onStart(); if (!bound) bound = bindService(connectionIntent, connection, 0) }

    override fun onStop() {
        if (bound) {
            unbindService(connection)
            bound = false
        }
        super.onStop()
    }

    override fun onDestroy() {
        if (isFinishing && !isChangingConfigurations) {
            startService(Intent(this, ControlSessionService::class.java).apply {
                action = ControlSessionService.ACTION_STOP
            })
        }
        super.onDestroy()
    }
}
