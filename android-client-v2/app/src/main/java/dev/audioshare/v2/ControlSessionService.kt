package dev.audioshare.v2

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.media.session.MediaSession
import android.os.Build
import android.os.Binder
import android.os.IBinder
import android.util.Log
import org.json.JSONObject
import java.util.UUID

/**
 * The session/pairing interface the UI depends on (beta-spec "Android Client:
 * session/pairing interface"). The UI must not call JNI, QUIC, or Keystore
 * directly; everything goes through this binder seam, and the current pairing
 * snapshot is replayed to a listener registered after Activity recreation.
 */
interface SessionController {
    /** The single replayable outcome owner; registering replays the latest. */
    fun setOutcomeListener(listener: ((SessionOutcome) -> Unit)?)

    /** `null` until this Service has carried a Client Session. */
    fun outcomeSnapshot(): SessionOutcome?

    fun setPairingListener(listener: ((PairingSnapshot?) -> Unit)?)
    /** Replayable current pairing state (null when no pairing is active). */
    fun pairingSnapshot(): PairingSnapshot?

    /** Parse and begin a QR pairing from the scanned/pasted payload URI. */
    fun beginQrPairing(uri: String)

    /** Begin SAS pairing against `host:port` (no camera). */
    fun beginSasPairing(address: String)

    /** The user confirmed the displayed SAS/fingerprint. */
    fun confirmPairing()

    /** Cancel/stop the current pairing or session. */
    fun cancelPairing()

    /** Persisted credentials for the Server list (public data only). */
    fun credentials(): List<CredentialRecord>

    /** Connect using a stored credential (first record when null). */
    fun connectStored(serverSpkiSha256Hex: String? = null)

    /** Stop the session. */
    fun disconnect()
}

/**
 * Owns the v2 session (control plane + media playback) across Activity
 * recreation and screen lock, as a foreground media-playback Service.
 */
class ControlSessionService : Service(), SessionController {
    private var session: ClientSession? = null
    /** Source of the pairing exchange owned by the current ClientSession. */
    private var pairingSource: PairingStateRelay.Source? = null
    /** The single replayable Session outcome owner (ticket 01 / ADR-0005). */
    private val outcomeStore = SessionOutcomeStore()
    private val pairingRelay = PairingStateRelay()
    private val binder = ControlBinder(this)
    private val credentials by lazy { SharedPreferencesCredentialStore(this) }

    /** Context for the in-flight pairing, used when the credential arrives. */
    private var pairingContext: PairingContext? = null

    /**
     * QR mode auto-confirms once per exchange. This gate is the durable guard
     * for Activity recreation: a re-created Activity may replay the same
     * AWAITING_CONFIRMATION snapshot before the native `CONFIRMING` phase is
     * published, and a second queued confirmation would end the session with
     * 402 (the native confirm channel is not idempotent).
     */
    private val confirmGate = PairingConfirmGate()

    /** Platform media session carried by the foreground notification. */
    private var mediaSession: MediaSession? = null

    private data class PairingContext(
        val identity: ClientIdentityResult,
        val host: String,
        val port: Int,
        val displayName: String,
        val mode: String,
    )

    override fun onBind(intent: Intent?): IBinder = binder

    // ---- SessionController -------------------------------------------------

    override fun setOutcomeListener(listener: ((SessionOutcome) -> Unit)?) =
        outcomeStore.setListener(listener)

    /** A terminal outcome stays replayable after the session is gone. */
    override fun outcomeSnapshot(): SessionOutcome? = outcomeStore.snapshot()

    override fun setPairingListener(listener: ((PairingSnapshot?) -> Unit)?) =
        pairingRelay.setListener(listener)

    override fun pairingSnapshot(): PairingSnapshot? = pairingRelay.snapshot()

    override fun beginQrPairing(uri: String) {
        dispatch(ACTION_PAIR_QR) { putExtra(EXTRA_PAIRING_URI, uri) }
    }

    override fun beginSasPairing(address: String) {
        dispatch(ACTION_PAIR_SAS) { putExtra(EXTRA_ADDRESS, address) }
    }

    override fun confirmPairing() {
        dispatch(ACTION_CONFIRM_PAIRING)
    }

    override fun cancelPairing() {
        dispatch(ACTION_CANCEL_PAIRING)
    }

    override fun credentials(): List<CredentialRecord> = credentials.all()

    override fun connectStored(serverSpkiSha256Hex: String?) {
        dispatch(ACTION_CONNECT_STORED) {
            if (serverSpkiSha256Hex != null) putExtra(EXTRA_SERVER_PIN, serverSpkiSha256Hex)
        }
    }

    override fun disconnect() {
        dispatch(ACTION_STOP)
    }

    private fun dispatch(action: String, configure: Intent.() -> Unit = {}) {
        val intent = Intent(this, ControlSessionService::class.java).setAction(action).apply(configure)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(intent)
        } else {
            startService(intent)
        }
    }

    // ---- Service lifecycle -------------------------------------------------

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                stopSession(StopReason.User)
                return START_NOT_STICKY
            }
            ACTION_PAIR_QR -> {
                startForegroundCompat()
                beginQr(intent.getStringExtra(EXTRA_PAIRING_URI))
                return START_STICKY
            }
            ACTION_PAIR_SAS -> {
                startForegroundCompat()
                beginSas(intent.getStringExtra(EXTRA_ADDRESS))
                return START_STICKY
            }
            ACTION_CONFIRM_PAIRING -> {
                startForegroundCompat()
                val queued = confirmGate.confirm { session?.confirmPairing() == true }
                if (!queued) {
                    Log.i(
                        TAG,
                        "confirmPairing ignored: already confirmed for this exchange " +
                            "or no verified pairing is awaiting confirmation",
                    )
                }
                return START_STICKY
            }
            ACTION_CANCEL_PAIRING -> {
                stopSession(StopReason.PairingCancel)
                return START_NOT_STICKY
            }
            ACTION_CONNECT_STORED -> {
                startForegroundCompat()
                connectStoredNow(intent.getStringExtra(EXTRA_SERVER_PIN))
                return START_STICKY
            }
        }
        startForegroundCompat()
        // TEST-SUPPORT ONLY: reflectively load the adb-extras harness compiled
        // only with -PtestSupport=true. Production builds have no extras path.
        if (BuildConfig.TEST_SUPPORT && session == null && intent != null) {
            val configuration = try {
                Class.forName("dev.audioshare.v2.TestSupportHarness")
                    .getMethod("serviceConfiguration", Intent::class.java)
                    .invoke(null, intent) as SessionConfiguration?
            } catch (_: Throwable) {
                null
            }
            if (configuration != null) {
                startSession(configuration)
                return START_STICKY
            }
        }
        if (session == null) {
            stopSelf()
            return START_NOT_STICKY
        }
        return START_STICKY
    }

    // ---- pairing -----------------------------------------------------------

    private fun beginQr(uri: String?) {
        val payload = try {
            PairingPayload.parse(uri ?: "")
        } catch (e: Exception) {
            reportPairingFailure("qr", null, e.message ?: "invalid QR payload")
            return
        }
        val displayName = deviceDisplayName()
        val alias = "v2-${payload.pinHex.take(16)}"
        val identity = try {
            ClientIdentity.create(this, alias)
        } catch (e: Exception) {
            reportPairingFailure("qr", null, "cannot provision Client identity: ${e.message}")
            return
        }
        pairingContext = PairingContext(identity, payload.host, payload.port, displayName, "qr")
        val configuration = SessionConfiguration(
            host = payload.host,
            port = payload.port,
            serverSpkiSha256Hex = payload.pinHex,
            serverPublicKeyHex = payload.serverPublicKey.toHex(),
            clientName = displayName,
            signer = identity.signer,
            pairing = PairingStart(mode = MODE_QR, nonceHex = payload.nonce.toHex(), displayName = displayName),
        )
        startSession(configuration)
    }

    private fun beginSas(address: String?) {
        val (host, port) = try {
            PairingPayload.validateAddress(address ?: "")
        } catch (e: Exception) {
            reportPairingFailure("sas", null, e.message ?: "invalid Server address")
            return
        }
        val displayName = deviceDisplayName()
        // The SAS pin is unknown until PairingResponse; use a unique alias and
        // persist it in the credential record.
        val alias = "v2-sas-${UUID.randomUUID().toString().take(13)}"
        val identity = try {
            ClientIdentity.create(this, alias)
        } catch (e: Exception) {
            reportPairingFailure("sas", null, "cannot provision Client identity: ${e.message}")
            return
        }
        pairingContext = PairingContext(identity, host, port, displayName, "sas")
        val configuration = SessionConfiguration(
            host = host,
            port = port,
            serverSpkiSha256Hex = "",
            serverPublicKeyHex = "",
            clientName = displayName,
            signer = identity.signer,
            pairing = PairingStart(mode = MODE_SAS, nonceHex = "", displayName = displayName),
        )
        startSession(configuration)
    }

    private fun reportPairingFailure(mode: String, errorCode: Int?, message: String) {
        Log.i(TAG, "pairing failed: $message")
        // A failure before startSession (invalid payload/address/identity) has
        // no live exchange yet; begin one so the snapshot stays replayable.
        val source = pairingSource?.takeIf { it.isCurrent() }
            ?: pairingRelay.beginSession().also { pairingSource = it }
        source.publish(
            PairingSnapshot(
                mode = mode,
                phase = "FAILED",
                sas = null,
                fingerprint = null,
                serverPublicKeyHex = null,
                serverSpkiSha256Hex = null,
                clientId = null,
                displayName = null,
                errorCode = errorCode,
            )
        )
    }

    private fun connectStoredNow(serverSpkiSha256Hex: String?) {
        val record = if (serverSpkiSha256Hex != null) {
            credentials.find(serverSpkiSha256Hex)
        } else {
            credentials.all().firstOrNull()
        }
        if (record == null) {
            Log.i(TAG, "connectStored: no credential; pairing is required")
            reportPairingFailure(
                "sas",
                errorCode = 205,
                message = "no saved pairing for this Server; re-pair required",
            )
            stopForegroundCompat()
            stopSelf()
            return
        }
        val identity = try {
            ClientIdentity.load(this, record.keyAlias)
        } catch (e: Exception) {
            // A missing key means re-pairing; never regenerate silently.
            Log.i(TAG, "stored Client key missing for ${record.keyAlias}: ${e.message}")
            reportPairingFailure(
                "sas",
                errorCode = 205,
                message = "the stored Client key is gone; re-pair the phone",
            )
            stopForegroundCompat()
            stopSelf()
            return
        }
        val configuration = SessionConfiguration(
            host = record.host.ifEmpty { addressHost(record.addressHint) },
            port = if (record.port != 0) record.port else addressPort(record.addressHint),
            serverSpkiSha256Hex = record.serverSpkiSha256Hex,
            serverPublicKeyHex = record.serverPublicKeyHex,
            clientName = record.displayName ?: "AudioShare phone",
            signer = identity.signer,
        )
        startSession(configuration)
    }

    private fun startSession(configuration: SessionConfiguration) {
        // A QR one-step confirmation belongs to exactly one exchange.
        confirmGate.reset()
        // Replacing a session is an internal teardown: it never surfaces an
        // outcome for the session being replaced.
        teardownSession(StopReason.Superseded)
        // Every session owns a fresh pairing exchange source. beginSession()
        // invalidates the previous one and clears a stale FAILED/COMPLETE
        // banner, so a stopped session's late watcher event can never
        // overwrite the new exchange's AWAITING_CONFIRMATION.
        val owningPairingSource = pairingRelay.beginSession().also { pairingSource = it }
        // A pairing exchange is not a Client Session outcome (ticket 17 owns
        // the handoff); only the authenticated session feeds the store.
        val sink = if (configuration.pairing != null) {
            OutcomeSink.NONE
        } else {
            outcomeStore.beginGeneration()
        }
        val current = ClientSession(this, object : ClientSession.Listener {
            override fun onPairingState(json: String) {
                // Source ownership: only the current exchange may publish.
                owningPairingSource.publishJson(json)
            }
            override fun onCredential(json: String) { persistCredential(json) }
            // Reconnect/migration diagnostics are the only evidence that the
            // startup race happened; surface them in logcat (no secrets).
            override fun onLog(line: String) { Log.i(TAG, line) }
        }, sink)
        session = current
        current.start(configuration)
    }

    private fun persistCredential(json: String) {
        val context = pairingContext ?: run {
            Log.i(TAG, "credential arrived without pairing context; not persisted")
            return
        }
        try {
            val obj = JSONObject(json)
            val record = CredentialRecord(
                serverSpkiSha256Hex = obj.getString("server_spki_sha256"),
                serverPublicKeyHex = obj.getString("server_public_key"),
                addressHint = "${context.host}:${context.port}",
                keyAlias = context.identity.alias,
                pairedAt = pairedAtNow(),
                softwareBacked = context.identity.softwareBacked,
                clientId = obj.getString("client_id"),
                displayName = obj.optString("display_name").ifEmpty { context.displayName },
                host = context.host,
                port = context.port,
            )
            // Only PairingResult(OK) produces a credential summary, so this is
            // the only persistence point.
            credentials.upsert(record)
            Log.i(TAG, "stored pairing for ${record.displayName} (software_backed=${record.softwareBacked})")
        } catch (e: Exception) {
            Log.i(TAG, "credential parse failed: ${e.message}")
        }
    }

    private fun teardownSession(reason: StopReason) {
        // Invalidate the pairing exchange first: a late watcher publication
        // from the old ClientSession must not reach the new one.
        pairingSource?.close()
        pairingSource = null
        session?.stop(reason)
        session = null
        // Only a user Stop is a visible outcome. Every internal teardown
        // orphans the generation so a late Live/Reconnecting cannot surface.
        if (reason != StopReason.User) outcomeStore.invalidate()
    }

    private fun deviceDisplayName(): String {
        val model = (Build.MODEL ?: "").trim().ifEmpty { "Android device" }
        val bytes = model.toByteArray(Charsets.UTF_8)
        if (bytes.size <= 63) return model
        var end = 63
        while (end > 0 && (bytes[end].toInt() and 0xC0) == 0x80) end--
        return String(bytes, 0, end, Charsets.UTF_8)
    }

    private fun startForegroundCompat() {
        val manager = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            manager.createNotificationChannel(NotificationChannel(
                CHANNEL_ID, "Audio Share session", NotificationManager.IMPORTANCE_LOW,
            ))
        }
        val pendingFlags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        val contentIntent = PendingIntent.getActivity(
            this,
            REQUEST_OPEN,
            Intent(this, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
            },
            pendingFlags,
        )
        // The lock-screen/notification Stop uses the same ACTION_STOP semantics
        // as the in-app Stop button; no separate shutdown path exists.
        val stopIntent = PendingIntent.getService(
            this,
            REQUEST_STOP,
            Intent(this, ControlSessionService::class.java).setAction(ACTION_STOP),
            pendingFlags,
        )
        val action = Notification.Action.Builder(
            Icon.createWithResource(this, android.R.drawable.ic_media_pause),
            "Stop",
            stopIntent,
        ).build()
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION") Notification.Builder(this)
        }.setContentTitle("Audio Share")
            .setContentText("Media session active. Audio plays while the screen is locked.")
            .setSmallIcon(android.R.drawable.stat_notify_sync_noanim)
            .setOngoing(true)
            .setCategory(Notification.CATEGORY_TRANSPORT)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setContentIntent(contentIntent)
            .addAction(action)
            .setStyle(
                Notification.MediaStyle()
                    .setMediaSession(mediaSessionToken(contentIntent))
                    .setShowActionsInCompactView(0)
            )
        val notification = builder.build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
        } else {
            @Suppress("DEPRECATION") startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun mediaSessionToken(contentIntent: PendingIntent): MediaSession.Token {
        val session = mediaSession ?: MediaSession(this, "AudioShareV2").also {
            it.setSessionActivity(contentIntent)
            it.isActive = true
            mediaSession = it
        }
        return session.sessionToken
    }

    private fun releaseMediaSession() {
        mediaSession?.isActive = false
        mediaSession?.release()
        mediaSession = null
    }

    private fun stopForegroundCompat() {
        stopForeground(STOP_FOREGROUND_REMOVE)
    }

    private fun stopSession(reason: StopReason) {
        // An explicit Stop is not an error; the outcome store keeps the single
        // Ended(UserStop) replayable for a recreated Activity.
        teardownSession(reason)
        confirmGate.reset()
        pairingRelay.clear()
        pairingContext = null
        releaseMediaSession()
        stopForegroundCompat()
        stopSelf()
    }

    override fun onDestroy() {
        teardownSession(StopReason.Shutdown)
        releaseMediaSession()
        super.onDestroy()
    }

    companion object {
        private const val TAG = "ControlSessionService"
        const val ACTION_START = "dev.audioshare.v2.CONTROL_START"
        const val ACTION_STOP = "dev.audioshare.v2.CONTROL_STOP"
        const val ACTION_PAIR_QR = "dev.audioshare.v2.PAIR_QR"
        const val ACTION_PAIR_SAS = "dev.audioshare.v2.PAIR_SAS"
        const val ACTION_CONFIRM_PAIRING = "dev.audioshare.v2.CONFIRM_PAIRING"
        const val ACTION_CANCEL_PAIRING = "dev.audioshare.v2.CANCEL_PAIRING"
        const val ACTION_CONNECT_STORED = "dev.audioshare.v2.CONNECT_STORED"

        const val EXTRA_PAIRING_URI = "pairingUri"
        const val EXTRA_ADDRESS = "address"
        const val EXTRA_SERVER_PIN = "serverSpki"

        private const val MODE_QR = 1
        private const val MODE_SAS = 2
        private const val CHANNEL_ID = "v2_control_session"
        private const val NOTIFICATION_ID = 42
        private const val REQUEST_OPEN = 1
        private const val REQUEST_STOP = 2
    }
}

class ControlBinder(val service: ControlSessionService) : Binder()

/**
 * Exactly-once guard for one pairing exchange. The reducer already suppresses
 * duplicate snapshots, but an Activity recreation starts from a fresh
 * `AppState`; the Service must therefore also refuse a second confirmation
 * until a new exchange starts. A rejected dispatch (native has nothing
 * awaiting confirmation) does not consume the one attempt.
 */
internal class PairingConfirmGate {
    private var dispatched = false

    fun confirm(dispatch: () -> Boolean): Boolean {
        if (dispatched) return false
        if (!dispatch()) return false
        dispatched = true
        return true
    }

    fun reset() {
        dispatched = false
    }
}

/** Hex decoding helpers shared by the pairing payload paths. */
internal fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }

internal fun addressHost(address: String): String = try {
    PairingPayload.validateAddress(address).first
} catch (_: Exception) {
    address.substringBeforeLast(':')
}

internal fun addressPort(address: String): Int = try {
    PairingPayload.validateAddress(address).second
} catch (_: Exception) {
    65531
}
