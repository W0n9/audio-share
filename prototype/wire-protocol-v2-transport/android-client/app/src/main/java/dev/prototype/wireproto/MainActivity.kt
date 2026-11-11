package dev.prototype.wireproto

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import org.json.JSONObject

/**
 * Prototype screen: connection inputs, connect/stop, live state + counters.
 * Plain framework widgets (no androidx/compose) to keep the baseline tiny.
 */
class MainActivity : android.app.Activity() {

    private lateinit var session: ProtoSession

    private lateinit var hostInput: EditText
    private lateinit var portInput: EditText
    private lateinit var pinInput: EditText
    private lateinit var credentialInput: EditText
    private lateinit var connectButton: Button
    private lateinit var stopButton: Button
    private lateinit var stateView: TextView
    private lateinit var countersView: TextView
    private lateinit var logView: TextView

    private val logLines = ArrayDeque<String>()
    private var lastStats: JSONObject? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        requestPostNotificationsIfNeeded()

        session = ProtoSession(applicationContext, object : ProtoSession.Listener {
            override fun onStats(json: JSONObject) {
                runOnUiThread { lastStats = json; renderCounters() }
            }

            override fun onStateChange(state: String) {
                runOnUiThread { stateView.text = "state: $state" }
            }

            override fun onLog(line: String) {
                runOnUiThread { appendLog(line) }
            }
        })

        buildUi()
    }

    private fun buildUi() {
        fun field(hint: String, initial: String): EditText {
            return EditText(this).apply {
                this.hint = hint
                setText(initial)
                setSingleLine(true)
            }
        }
        hostInput = field("server host", "192.168.1.10")
        portInput = field("server port (v2 test port)", "4433")
        pinInput = field("pinned server SPKI sha256 (64 hex, from server startup)", "")
        credentialInput = field("test credential", "test-credential-do-not-use")

        connectButton = Button(this).apply { text = "Connect" }
        stopButton = Button(this).apply { text = "Stop"; isEnabled = false }

        stateView = TextView(this).apply { text = "state: IDLE"; textSize = 18f }
        countersView = TextView(this).apply { textSize = 13f; setTextColor(0xFF333333.toInt()) }
        logView = TextView(this).apply { textSize = 11f }

        connectButton.setOnClickListener {
            val host = hostInput.text.toString().trim()
            val port = portInput.text.toString().trim().toIntOrNull() ?: 0
            val pin = pinInput.text.toString().trim()
            val cred = credentialInput.text.toString()
            if (host.isEmpty() || port <= 0 || pin.length != 64) {
                Toast.makeText(this, "need host, port and 64-hex SPKI pin", Toast.LENGTH_SHORT).show()
                return@setOnClickListener
            }
            // foreground service fronts the process while media plays
            startForegroundService(Intent(this, MediaPlaybackService::class.java))
            session.start(host, port, pin, cred)
            connectButton.isEnabled = false
            stopButton.isEnabled = true
            appendLog("starting session $host:$port")
        }

        stopButton.setOnClickListener {
            session.stop()
            stopService(Intent(this, MediaPlaybackService::class.java))
            connectButton.isEnabled = true
            stopButton.isEnabled = false
        }

        val layout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(48, 48, 48, 48)
            addView(hostInput)
            addView(portInput)
            addView(pinInput)
            addView(credentialInput)
            addView(connectButton)
            addView(stopButton)
            addView(stateView)
            addView(countersView)
            addView(logView)
        }
        setContentView(layout)
    }

    private fun appendLog(line: String) {
        logLines.addLast(
            "${java.text.SimpleDateFormat("HH:mm:ss", java.util.Locale.US).format(java.util.Date())}  $line"
        )
        while (logLines.size > 200) logLines.removeFirst()
        logView.text = logLines.joinToString("\n")
    }

    private fun renderCounters() {
        val stats = lastStats ?: return
        val media = stats.optJSONObject("media")
        val buffer = stats.optJSONObject("buffer")
        if (media == null || buffer == null) {
            countersView.text = "no media yet"
            return
        }
        countersView.text = buildString {
            append("state: ").append(stats.optString("state"))
            append("  connected: ").append(stats.optBoolean("connected"))
            append("\nhighest seq: ").append(media.opt("highest_seq") ?: "-")
            append("  gaps: ").append(media.optLong("gaps"))
            append("  dups: ").append(media.optLong("duplicates"))
            append("  reorder: ").append(media.optLong("reordered"))
            append("\nbytes: ").append(media.optLong("bytes"))
            append("  frames: ").append(media.optLong("frames"))
            append("  datagrams: ").append(media.optLong("datagrams_observed"))
            append("\nbyte errors: ").append(media.optLong("byte_errors"))
            append("  malformed: ").append(media.optLong("malformed_payloads"))
            append("  instance changes: ").append(media.optLong("instance_changes"))
            append("\nbuffer write: ").append(buffer.optLong("write_pos"))
            append("/").append(buffer.optLong("capacity_bytes"))
            append("  target: ").append(buffer.optLong("target_bytes"))
            append("\ncontrol errors: ").append(stats.optLong("control_errors"))
            append("\nlast: ").append(stats.optString("last_control_error"))
        }
    }

    private fun requestPostNotificationsIfNeeded() {
        if (Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
    }

    override fun onDestroy() {
        // Leaving the Activity stops the session unless the foreground service
        // is still holding the process for playback (prototype keeps one global
        // session; a real app would bind the service instead).
        if (!MediaPlaybackService.isRunning) {
            session.stop()
        }
        super.onDestroy()
    }
}
