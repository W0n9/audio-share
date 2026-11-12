package dev.audioshare.v2

import android.app.Activity
import android.os.Bundle
import android.widget.LinearLayout
import android.widget.TextView

/**
 * Smoke-install surface only (ticket 08): shows the native library loaded
 * and a session's state string. Real UI is a later ticket.
 *
 * Test config (pre-provisioned Server identity material) is supplied via
 * intent extras:
 *   host (String), port (Int), pinnedSpki (String hex),
 *   serverKey (String hex), clientName (String), bindHint (Boolean).
 */
class MainActivity : Activity() {
    private var session: ClientSession? = null
    private val stateView by lazy { TextView(this) }
    private val logView by lazy { TextView(this) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val layout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(48, 48, 48, 48)
        }
        val title = TextView(this).apply { text = "audio-share v2 (ticket 08)" }
        stateView.text = "IDLE"
        logView.text = ""
        layout.addView(title)
        layout.addView(stateView)
        layout.addView(logView)
        setContentView(layout)

        val host = intent.getStringExtra("host") ?: ""
        val port = intent.getIntExtra("port", 65531)
        val pin = intent.getStringExtra("pinnedSpki") ?: ""
        val serverKey = intent.getStringExtra("serverKey") ?: ""
        val clientName = intent.getStringExtra("clientName") ?: "smoke-client"
        val bindHint = intent.getBooleanExtra("bindHint", false)

        if (host.isEmpty() || pin.isEmpty() || serverKey.isEmpty()) {
            logView.text =
                "config missing: pass host/pinnedSpki/serverKey extras (test config)"
            return
        }

        session = ClientSession(this, object : ClientSession.Listener {
            override fun onStats(json: org.json.JSONObject) { /* smoke: ignored */ }
            override fun onStateChange(state: String) {
                runOnUiThread { stateView.text = state }
            }

            override fun onLog(line: String) {
                runOnUiThread { logView.text = logView.text.toString() + "\n" + line }
            }
        }).also {
            it.bindHintEnabled = bindHint
            it.start(host, port, pin, serverKey, clientName)
        }
    }

    override fun onDestroy() {
        session?.stop()
        session = null
        super.onDestroy()
    }
}
