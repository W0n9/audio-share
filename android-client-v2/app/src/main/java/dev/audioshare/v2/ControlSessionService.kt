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
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log
import dev.audioshare.v2.session.PairingContext
import dev.audioshare.v2.session.SessionEngineFactory
import dev.audioshare.v2.session.SessionOrchestrator
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

    /**
     * One-shot pairing handoff (ticket 17): after the credential is persisted
     * the Service starts the authenticated session and pushes this exactly
     * once per exchange. A listener registered later gets no replay; a
     * recreated Activity relies on the outcome and the restoration rule.
     */
    fun setPairingCompletedListener(listener: ((CredentialRecord) -> Unit)?)

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

    /**
     * Forget this Server locally (ADR-0006 §4): delete the credential and the
     * Client identity key, then stop its session. The UI confirms first; the
     * Service never deletes silently.
     */
    fun forgetServer(serverSpkiSha256Hex: String)
}

/**
 * Owns the v2 session (control plane + media playback) across Activity
 * recreation and screen lock, as a foreground media-playback Service.
 */
class ControlSessionService : Service(), SessionController {
    /** The single replayable Session outcome owner (ticket 01 / ADR-0005). */
    private val outcomeStore = SessionOutcomeStore()
    private val pairingRelay = PairingStateRelay()
    private val binder = ControlBinder(this)
    private val credentials by lazy { SharedPreferencesCredentialStore(this) }
    private val mainHandler by lazy { Handler(Looper.getMainLooper()) }
    private val forgetter by lazy {
        ForgetServer(
            credentials = credentials,
            deleteIdentity = IdentityKeyDeleter { alias -> ClientIdentity.delete(this, alias) },
            log = { Log.i(TAG, it) },
        )
    }

    /**
     * One-shot handoff target. It is deliberately not replayed: a late
     * Activity registration must not re-enter SESSION after the user left.
     */
    @Volatile
    private var pairingCompletedListener: ((CredentialRecord) -> Unit)? = null

    /**
     * Hand-written composition root for the session graph (ticket 21). This is
     * the only place a [ClientSession] is constructed; the Service keeps the
     * pairing orchestration (QR parse, identity creation, SessionConfiguration)
     * and all platform work.
     */
    private val orchestrator by lazy {
        SessionOrchestrator(
            engineFactory = SessionEngineFactory { sink, listener ->
                ClientSession(this, listener, sink)
            },
            outcomes = outcomeStore,
            pairing = pairingRelay,
            credentials = credentials,
            log = { Log.i(TAG, it) },
            // The watcher callback persists on its own thread; the handoff
            // (teardown joins the media thread) runs later on main.
            schedule = { task -> mainHandler.post(task) },
            onPairingCompleted = { record -> pairingCompletedListener?.invoke(record) },
        )
    }

    /** Platform media session carried by the foreground notification. */
    private var mediaSession: MediaSession? = null

    override fun onBind(intent: Intent?): IBinder = binder

    // ---- SessionController -------------------------------------------------

    override fun setOutcomeListener(listener: ((SessionOutcome) -> Unit)?) =
        outcomeStore.setListener(listener)

    /** A terminal outcome stays replayable after the session is gone. */
    override fun outcomeSnapshot(): SessionOutcome? = outcomeStore.snapshot()

    override fun setPairingListener(listener: ((PairingSnapshot?) -> Unit)?) =
        pairingRelay.setListener(listener)

    override fun pairingSnapshot(): PairingSnapshot? = pairingRelay.snapshot()

    override fun setPairingCompletedListener(listener: ((CredentialRecord) -> Unit)?) {
        pairingCompletedListener = listener
    }

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

    override fun forgetServer(serverSpkiSha256Hex: String) {
        dispatch(ACTION_FORGET_SERVER) { putExtra(EXTRA_SERVER_PIN, serverSpkiSha256Hex) }
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
                val queued = orchestrator.confirmPairing()
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
            ACTION_FORGET_SERVER -> {
                // dispatch() used startForegroundService(); claim the slot
                // before the (fast) local cleanup removes it again.
                startForegroundCompat()
                forgetServerNow(intent.getStringExtra(EXTRA_SERVER_PIN))
                return START_NOT_STICKY
            }
        }
        startForegroundCompat()
        // TEST-SUPPORT ONLY: reflectively load the adb-extras harness compiled
        // only with -PtestSupport=true. Production builds have no extras path.
        if (BuildConfig.TEST_SUPPORT && !orchestrator.hasEngine && intent != null) {
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
        if (!orchestrator.hasEngine) {
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
        val pairingContext = PairingContext(identity, payload.host, payload.port, displayName, "qr")
        val configuration = SessionConfiguration(
            host = payload.host,
            port = payload.port,
            serverSpkiSha256Hex = payload.pinHex,
            serverPublicKeyHex = payload.serverPublicKey.toHex(),
            clientName = displayName,
            signer = identity.signer,
            pairing = PairingStart(mode = MODE_QR, nonceHex = payload.nonce.toHex(), displayName = displayName),
        )
        startSession(configuration, pairingContext)
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
        val pairingContext = PairingContext(identity, host, port, displayName, "sas")
        val configuration = SessionConfiguration(
            host = host,
            port = port,
            serverSpkiSha256Hex = "",
            serverPublicKeyHex = "",
            clientName = displayName,
            signer = identity.signer,
            pairing = PairingStart(mode = MODE_SAS, nonceHex = "", displayName = displayName),
        )
        startSession(configuration, pairingContext)
    }

    private fun reportPairingFailure(mode: String, errorCode: Int?, message: String) {
        orchestrator.reportPairingFailure(mode, errorCode, message)
    }

    private fun connectStoredNow(serverSpkiSha256Hex: String?) {
        val record = if (serverSpkiSha256Hex != null) {
            credentials.find(serverSpkiSha256Hex)
        } else {
            credentials.all().firstOrNull()
        }
        if (record == null) {
            Log.i(TAG, "connectStored: no credential; pairing is required")
            orchestrator.reportStoredConnectFailure(
                code = 205,
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
            orchestrator.reportStoredConnectFailure(
                code = 205,
                message = "the stored Client key is gone; re-pair the phone",
                serverSpkiSha256Hex = record.serverSpkiSha256Hex,
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

    private fun forgetServerNow(serverSpkiSha256Hex: String?) {
        if (serverSpkiSha256Hex != null) {
            try {
                forgetter.forget(serverSpkiSha256Hex)
            } catch (e: Exception) {
                // The credential delete is authoritative and already logged
                // inside ForgetServer; a store failure here must not keep the
                // session (and the Server card) alive.
                Log.e(TAG, "forget failed for $serverSpkiSha256Hex: ${e.message}")
            }
        }
        // Forget is local cleanup, not a user Stop: a Shutdown teardown
        // invalidates the replayable outcome, so Home returns to the
        // unpaired state without a STOPPED card.
        orchestrator.stop(StopReason.Shutdown)
        releaseMediaSession()
        stopForegroundCompat()
        stopSelf()
    }

    private fun startSession(configuration: SessionConfiguration, pairingContext: PairingContext? = null) {
        orchestrator.startSession(configuration, pairingContext)
    }

    private fun teardownSession(reason: StopReason) {
        orchestrator.teardown(reason)
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
                CHANNEL_ID, getString(R.string.notification_channel_name),
                NotificationManager.IMPORTANCE_LOW,
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
            getString(R.string.notification_stop),
            stopIntent,
        ).build()
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION") Notification.Builder(this)
        }.setContentTitle(getString(R.string.notification_title))
            .setContentText(getString(R.string.notification_body))
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
        orchestrator.stop(reason)
        releaseMediaSession()
        stopForegroundCompat()
        stopSelf()
    }

    override fun onDestroy() {
        // A pending handoff must not start a session after the Service is gone.
        mainHandler.removeCallbacksAndMessages(null)
        pairingCompletedListener = null
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
        const val ACTION_FORGET_SERVER = "dev.audioshare.v2.FORGET_SERVER"

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
