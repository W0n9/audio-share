package dev.audioshare.v2

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Handler
import android.os.Looper
import android.util.Log
import org.json.JSONObject
import java.net.Inet4Address
import java.net.Inet6Address
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import kotlin.math.min
import kotlin.random.Random

/** The production adapter from [MediaPlayer.NativeMedia] to the JNI surface. */
internal object NativeMediaBridge : MediaPlayer.NativeMedia {
    override fun getState(handle: Long): String? = ClientNative.getState(handle)
    override fun attachMediaRing(handle: Long, buffer: java.nio.ByteBuffer): Boolean =
        ClientNative.attachMediaRing(handle, buffer)
    override fun ringPosition(handle: Long): Long = ClientNative.ringPosition(handle)
    override fun mediaFormatJson(handle: Long): String? = ClientNative.mediaFormatJson(handle)
    override fun recordPlayback(handle: Long, submittedBytes: Long, underruns: Long, bufferCurrentMs: Int) =
        ClientNative.recordPlayback(handle, submittedBytes, underruns, bufferCurrentMs)
}

/** What the ConnectivityManager `onAvailable` callback should do. */
internal enum class AvailabilityAction {
    /** No native handle yet: the queued start/reconnect attempt owns the connect. */
    IN_FLIGHT_OWNS_START,

    /** Handle exists but the QUIC endpoint is not stored yet (handshake in flight). */
    AWAIT_ENDPOINT,

    /** Live connection: migrate it with the network-change rebind. */
    REBIND,
}

/** Ticket 23: pure startup/migration decision shared with the JVM tests. */
internal object SessionStartupPolicy {
    fun availabilityAction(handle: Long, endpointConnected: Boolean): AvailabilityAction = when {
        handle == 0L -> AvailabilityAction.IN_FLIGHT_OWNS_START
        !endpointConnected -> AvailabilityAction.AWAIT_ENDPOINT
        else -> AvailabilityAction.REBIND
    }
}

/**
 * Kotlin-side session owner (ticket 08): JNI handle lifecycle, bounded
 * exponential backoff with ±20% jitter, and the ConnectivityManager
 * NetworkCallback migration (rebind with fd handoff, fallback to
 * teardown + backoff reconnect).
 *
 * Ticket 14 adds playback: [MediaPlayer] owns the native ring and the
 * AudioTrack, attached once per native session and started only while the
 * session can carry media.
 */
class ClientSession(
    private val appContext: Context,
    private val listener: Listener,
) {
    interface Listener {
        /** Called on a worker thread whenever counters/state are refreshed. */
        fun onStats(json: JSONObject)

        /** Called when the user-visible session state name changes. */
        fun onStateChange(state: String)

        /** Info/error lines for a log view. */
        fun onLog(line: String)

        /** Pairing display material/phase changed (native JSON). */
        fun onPairingState(json: String) {}

        /** `PairingResult(OK)` arrived; persist this public credential. */
        fun onCredential(json: String) {}
    }

    companion object {
        private const val TAG = "ClientSession"
        /** §6.3 backoff policy lives in [ReconnectPolicy]. */
        private const val STATS_POLL_MS = 250L
    }

    private val handle = AtomicLong(0)
    private val running = AtomicBoolean(false)
    private val lifecycleLock = Any()
    private var lifecycle: Any? = null
    private val currentState = SessionStateRelay(listener::onStateChange)
    private var stateSource: SessionStateRelay.Source? = null
    private var mediaPlayer: MediaPlayer? = null

    /** 最近一次实际 native 状态；停止后保持 CLOSED，不暴露已失效的 handle。 */
    fun stateSnapshot(): String = currentState.snapshot()

    private var configuration = SessionConfiguration("", 0, "", "", "")
    /** Reconnect attempt counter; reset on start(). */
    private val backoffAttempts = AtomicLong(0)

    // ---- migration scaffold -----------------------------------------------
    private val connectivityManager =
        appContext.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    private var networkCallbackRegistered = false
    private val executor = Executors.newSingleThreadExecutor()
    private val mainHandler = Handler(Looper.getMainLooper())

    @Volatile private var activeNetworkAddress: String? = null
    @Volatile private var registeredCallback: ConnectivityManager.NetworkCallback? = null

    fun start(configuration: SessionConfiguration) {
        synchronized(lifecycleLock) {
            if (running.getAndSet(true)) return
            this.configuration = configuration
            backoffAttempts.set(0)
            val lifetime = Any().also { lifecycle = it }
            val source = currentState.beginSession().also { stateSource = it }
            registerNetworkCallback(lifetime)
            startSession(lifetime, source)
        }
    }

    fun stop() {
        val player: MediaPlayer?
        val h = synchronized(lifecycleLock) {
            if (!running.getAndSet(false)) return
            lifecycle = null
            unregisterNetworkCallback()
            val current = handle.getAndSet(0)
            // 先使状态来源失效，再清理 native；进行中的旧读不能发布，
            // 已排队的创建/重连任务也属于旧生命周期。
            stateSource?.close()
            stateSource = null
            // The playback owner must stop before the native session dies.
            player = stopMediaLocked()
            current
        }
        player?.stop()
        if (h != 0L) {
            ClientNative.stop(h)
            ClientNative.destroy(h)
        }
        listener.onLog("stopped by user")
    }

    fun isRunning() = running.get()

    /**
     * Automation/testing switch: when false the UDP socket uses system
     * default routing instead of the NetworkCallback's local-address hint.
     * The hint pins the socket to the active network's interface — which
     * breaks routing when the server is reached through a VPN overlay
     * (e.g. tailscale), because the route to it lives on tun, not on the
     * physical network the hint names. Device finding, 2026-11-11.
     */
    @Volatile var bindHintEnabled = false

    // ---- session lifecycle with backoff ------------------------------------

    /** 调用者持有 lifecycleLock；数值 handle 不能独自标识事件来源。 */
    private fun ownsSessionLocked(lifetime: Any, source: SessionStateRelay.Source, h: Long): Boolean =
        running.get() && lifecycle === lifetime && stateSource === source && handle.get() == h

    private fun startSession(lifetime: Any, previousSource: SessionStateRelay.Source) {
        executor.execute {
            val configuration = synchronized(lifecycleLock) {
                if (!ownsSessionLocked(lifetime, previousSource, 0)) return@execute
                this.configuration
            }
            val h = createNativeSession(configuration)
            if (h == 0L) {
                synchronized(lifecycleLock) {
                    if (ownsSessionLocked(lifetime, previousSource, 0)) {
                        listener.onLog("createSession failed; retrying after backoff")
                    }
                }
                scheduleReconnect(0, previousSource, lifetime)
                return@execute
            }
            val connected = synchronized(lifecycleLock) {
                if (!ownsSessionLocked(lifetime, previousSource, 0)) {
                    null
                } else {
                    handle.set(h)
                    val source = currentState.beginSession().also { stateSource = it }
                    val bind = if (bindHintEnabled) (activeNetworkAddress ?: "") else ""
                    listener.onLog("connect(host=${configuration.host} port=${configuration.port} bind=$bind)")
                    // The ring is attached once per native session, before any
                    // media can arrive; playback starts with the session and
                    // simply idles until native reports PLAYING.
                    val player = MediaPlayer(NativeMediaBridge).also { it.attach(h) }
                    synchronized(lifecycleLock) { mediaPlayer = player }
                    player.start()
                    if (configuration.pairing != null) {
                        // Pairing results must be captured even when the
                        // Server closes right after PairingResult (as-cmd pair
                        // exits by design), so this watcher does not depend on
                        // the state source or the 250 ms stats poll.
                        startPairingWatcher(h)
                    }
                    source to ClientNative.connect(h, bind)
                }
            }
            if (connected == null) {
                // createSession 执行期间，STOP 或其他尝试已取得所有权。
                ClientNative.destroy(h)
                return@execute
            }
            val (source, ok) = connected
            if (!ok) {
                synchronized(lifecycleLock) {
                    if (ownsSessionLocked(lifetime, source, h)) {
                        listener.onLog("native connect() rejected; retrying after backoff")
                    }
                }
                scheduleReconnect(h, source, lifetime)
                return@execute
            }
            startStatsPolling(h, source, lifetime)
        }
    }

    /**
     * Dedicated pairing-state/credential watcher. Native pairing state is a
     * one-shot result that must survive transport teardown: `as-cmd pair`
     * closes the connection immediately after `PairingResult(OK)`, so relying
     * on the state relay (which is invalidated by reconnect) would lose the
     * credential.
     */
    private fun startPairingWatcher(h: Long) {
        val watcher = Thread {
            var credentialSeen = false
            var failed = false
            while (true) {
                val pairingJson = ClientNative.getPairingState(h)
                if (!pairingJson.isNullOrEmpty()) {
                    listener.onPairingState(pairingJson)
                    failed = pairingJson.contains("\"phase\":\"FAILED\"")
                }
                val credential = ClientNative.getCredentialSummary(h)
                if (!credential.isNullOrEmpty()) {
                    if (!credentialSeen) {
                        credentialSeen = true
                        listener.onCredential(credential)
                    }
                    break
                }
                val state = ClientNative.getState(h) ?: ""
                if (state.isEmpty() && pairingJson.isNullOrEmpty()) break
                if (failed) break
                if (state.isNotEmpty() && state != "IDLE" &&
                    state != "AUTHENTICATING" && state != "NEGOTIATING" &&
                    state != "READY" && state != "PLAYING" && pairingJson.isNullOrEmpty()
                ) {
                    break
                }
                Thread.sleep(25)
            }
        }
        watcher.name = "v2-pairing-watch"
        watcher.start()
    }

    /**
     * Pick the native constructor for this configuration:
     * - a pairing session sends `PairingRequest`;
     * - the explicit test-support path keeps the native fixture identity;
     * - production reconnection passes the Keystore-backed signer.
     */
    private fun createNativeSession(configuration: SessionConfiguration): Long {
        val pairing = configuration.pairing
        if (pairing != null) {
            val signer = configuration.signer
                ?: run {
                    listener.onLog("pairing requires a ClientSigner")
                    return 0L
                }
            return ClientNative.createPairingSession(
                configuration.host,
                configuration.port,
                pairing.mode,
                configuration.serverSpkiSha256Hex,
                configuration.serverPublicKeyHex,
                pairing.nonceHex,
                pairing.displayName,
                signer,
            )
        }
        if (configuration.useNativeTestIdentity) {
            // TEST-SUPPORT ONLY: the native library carries the fixture key
            // only with `-PtestSupport=true`; a production build returns 0.
            return ClientNative.createSession(
                configuration.host,
                configuration.port,
                configuration.serverSpkiSha256Hex,
                configuration.serverPublicKeyHex,
                configuration.clientName,
            )
        }
        val signer = configuration.signer
            ?: run {
                listener.onLog("no ClientSigner for an authenticated session")
                return 0L
            }
        return ClientNative.createAuthSession(
            configuration.host,
            configuration.port,
            configuration.serverSpkiSha256Hex,
            configuration.serverPublicKeyHex,
            configuration.clientName,
            signer,
        )
    }

    /** Confirm a verified pairing; true when the confirmation was queued. */
    fun confirmPairing(): Boolean {
        val h = handle.get()
        return h != 0L && ClientNative.confirmPairing(h)
    }

    /** The current native pairing JSON, or null when not pairing/unknown. */
    fun pairingStateJson(): String? {
        val h = handle.get()
        if (h == 0L) return null
        val json = ClientNative.getPairingState(h)
        return if (json.isNullOrEmpty()) null else json
    }

    private fun scheduleReconnect(h: Long, source: SessionStateRelay.Source, lifetime: Any) {
        synchronized(lifecycleLock) {
            if (!ownsSessionLocked(lifetime, source, h)) return
            handle.set(0)
            source.close()
            val attempt = backoffAttempts.getAndIncrement()
            val delay = ReconnectPolicy.backoffDelayMs(attempt, Random.nextDouble())
            listener.onLog("reconnect attempt ${attempt + 1} in ${delay}ms")
            mainHandler.postDelayed({
                synchronized(lifecycleLock) {
                    if (ownsSessionLocked(lifetime, source, 0)) startSession(lifetime, source)
                }
            }, delay)
        }
        if (h != 0L) ClientNative.destroy(h)
        stopMedia()
    }

    /** Stop and drop the playback owner for the current native session. */
    private fun stopMedia() {
        val player = synchronized(lifecycleLock) { stopMediaLocked() }
        player?.stop()
    }

    /** Caller holds [lifecycleLock]; returns the detached player to stop outside the lock. */
    private fun stopMediaLocked(): MediaPlayer? {
        val current = mediaPlayer
        mediaPlayer = null
        return current
    }

    private fun startStatsPolling(
        h: Long,
        source: SessionStateRelay.Source,
        lifetime: Any,
    ) {
        val poller = Thread {
            val myHandle = h
            // §6.3: only retryable closes (and transport loss with no Close,
            // close_code null) reconnect. Pairing/admin/security closes surface
            // to the user and must not auto-retry.
            var reconnectOnExit = true
            try {
                while (synchronized(lifecycleLock) { ownsSessionLocked(lifetime, source, myHandle) }) {
                    val state = source.poll { ClientNative.getState(myHandle) ?: "" }
                    if (state.isEmpty() && !source.isCurrent()) break
                    if (state == "READY") {
                        // §6.3：仅当前会话可以重置 backoff。
                        synchronized(lifecycleLock) {
                            if (ownsSessionLocked(lifetime, source, myHandle)) backoffAttempts.set(0)
                        }
                    }
                    val statsJson = ClientNative.getStats(myHandle)
                    if (statsJson.isNullOrEmpty()) continue
                    try {
                        val obj = JSONObject(statsJson)
                        synchronized(lifecycleLock) {
                            if (ownsSessionLocked(lifetime, source, myHandle)) listener.onStats(obj)
                        }
                        if (state == "CLOSED") {
                            val closeCode =
                                if (obj.isNull("close_code")) null else obj.optInt("close_code")
                            reconnectOnExit = ReconnectPolicy.shouldReconnect(closeCode)
                            synchronized(lifecycleLock) {
                                if (ownsSessionLocked(lifetime, source, myHandle)) {
                                    listener.onLog(
                                        if (reconnectOnExit) {
                                            "session closed (close_code=$closeCode); reconnecting"
                                        } else {
                                            "session closed (close_code=$closeCode); no automatic reconnect"
                                        }
                                    )
                                }
                            }
                            break
                        }
                    } catch (e: Exception) {
                        Log.w(TAG, "stats parse: ${e.message}")
                    }
                    Thread.sleep(STATS_POLL_MS)
                }
            } catch (_: InterruptedException) {
            } finally {
                stopMedia()
                if (reconnectOnExit) scheduleReconnect(myHandle, source, lifetime)
            }
        }
        poller.name = "v2-stats-poll"
        poller.start()
    }

    // ---- NetworkCallback migration -----------------------------------------
    //
    // We observe the default network; on onAvailable we create a
    // DatagramSocket bound to the new Network (Network.bindSocket — real
    // route pinning) and hand its raw fd to ClientNative.rebind(), which
    // calls quinn Endpoint::rebind(), moving the connection to the new
    // socket live. If the rebind fails, the pre-existing teardown +
    // bounded-backoff reconnect is the fallback (noBindHint: the fallback
    // session starts without a bind hint). onLost always tears down: the
    // route is gone, there is nothing to rebind to.
    //
    // FD OWNERSHIP: the fd (and its owning socket/pfd objects) transfers to
    // native on the rebind call — never close them on the JVM side.

    private fun registerNetworkCallback(lifetime: Any) {
        if (networkCallbackRegistered) return
        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                val address = localAddressOf(network)
                val current = synchronized(lifecycleLock) {
                    if (!running.get() || lifecycle !== lifetime) return
                    activeNetworkAddress = address
                    listener.onLog("network available: $activeNetworkAddress")
                    stateSource?.let { it to handle.get() }
                } ?: return
                val (source, h) = current
                when (SessionStartupPolicy.availabilityAction(h, h != 0L && nativeEndpointConnected(h))) {
                    AvailabilityAction.IN_FLIGHT_OWNS_START -> {
                        // 注册回调会立即投递当前网络。此时启动/重连任务已在队列中，
                        // 额外 scheduleReconnect 会关闭来源并制造第二条连接。
                        listener.onLog("network available before a native session; in-flight start owns it")
                        return
                    }
                    AvailabilityAction.AWAIT_ENDPOINT -> {
                        // handle 已存在但 QUIC endpoint 尚未写入（握手进行中）。
                        // 此刻 rebind 必定返回 "no endpoint (not connected)"，
                        // 其 teardown 回退会杀掉正在进行的握手。
                        listener.onLog("network available before the QUIC endpoint exists; skipping rebind")
                        return
                    }
                    AvailabilityAction.REBIND -> Unit
                }
                if (tryRebind(h, network)) return
                // rebind 可与 STOP/重连交错；只清理本次来源。
                teardownForReconnect("network change (rebind fallback)", h, source, lifetime)
            }

            override fun onLost(network: Network) {
                val current = synchronized(lifecycleLock) {
                    if (!running.get() || lifecycle !== lifetime) return
                    listener.onLog("network lost")
                    stateSource?.let { it to handle.get() }
                } ?: return
                val (source, h) = current
                if (h != 0L) teardownForReconnect("network lost", h, source, lifetime)
            }
        }
        connectivityManager.registerNetworkCallback(request, callback)
        registeredCallback = callback
        networkCallbackRegistered = true
    }

    /**
     * True only after the native run has stored the QUIC endpoint (the
     * `connected` stat is set immediately after it). rebind() before that
     * point fails with "no endpoint (not connected)".
     */
    private fun nativeEndpointConnected(h: Long): Boolean = try {
        val stats = ClientNative.getStats(h)
        !stats.isNullOrEmpty() && JSONObject(stats).optBoolean("connected", false)
    } catch (e: Exception) {
        Log.w(TAG, "read connected stat: ${e.message}")
        false
    }

    /**
     * Try the migration path: a socket route-pinned to [network], handed to
     * native rebind() as a raw fd. Returns true when the endpoint was
     * rebound (fd ownership moved to native either way).
     */
    private fun tryRebind(h: Long, network: Network): Boolean {
        return try {
            val socket = java.net.DatagramSocket()
            network.bindSocket(socket) // route-pin to this Network
            // fromDatagramSocket wraps the socket's existing fd (no dup);
            // after the handoff below, native owns the fd and neither the
            // socket nor the pfd may be closed on the JVM side.
            val pfd = android.os.ParcelFileDescriptor.fromDatagramSocket(socket)
            val fd = pfd.fd
            val ok = ClientNative.rebind(h, fd)
            if (ok) {
                listener.onLog("endpoint rebound to network $network (fd $fd)")
            } else {
                listener.onLog("native rebind failed; falling back to reconnect")
            }
            ok
        } catch (e: Exception) {
            Log.w(TAG, "rebind: ${e.message}")
            listener.onLog("rebind failed: ${e.message}")
            false
        }
    }

    /** Teardown of the current session, then the bounded-backoff reconnect. */
    private fun teardownForReconnect(reason: String, h: Long, source: SessionStateRelay.Source, lifetime: Any) {
        synchronized(lifecycleLock) {
            if (!ownsSessionLocked(lifetime, source, h)) return
            if (h != 0L) ClientNative.stop(h)
            listener.onLog("teardown for reconnect: $reason")
        }
        scheduleReconnect(h, source, lifetime)
    }

    private fun unregisterNetworkCallback() {
        if (!networkCallbackRegistered) return
        registeredCallback?.let { connectivityManager.unregisterNetworkCallback(it) }
        registeredCallback = null
        networkCallbackRegistered = false
    }

    /**
     * Best-effort local address of [network]: prefers the IPv4 address of
     * the default route; falls back to any address. Migration scaffold only.
     */
    private fun localAddressOf(network: Network): String? {
        return try {
            val lp = connectivityManager.getLinkProperties(network) ?: return null
            lp.linkAddresses.firstOrNull { it.address is Inet4Address }?.address?.hostAddress
                ?: lp.linkAddresses.firstOrNull { it.address is Inet6Address }?.address?.hostAddress
        } catch (e: Exception) {
            Log.w(TAG, "localAddressOf: ${e.message}")
            null
        }
    }
}

/**
 * §6.3 Client reconnect policy — pure JVM-testable logic shared by the session
 * owner. `closeCode == null` means transport loss without a Close (classified
 * 601 by the native side) and IS retryable.
 */
data class SessionConfiguration(
    val host: String,
    val port: Int,
    val serverSpkiSha256Hex: String,
    val serverPublicKeyHex: String,
    val clientName: String,
    /** The Client identity signer (Keystore in production). */
    val signer: ClientSigner? = null,
    /** Set for a pairing session; null for paired reconnection. */
    val pairing: PairingStart? = null,
    /**
     * TEST-SUPPORT ONLY: use the native fixture identity (`createSession`).
     * Production service paths never set this.
     */
    val useNativeTestIdentity: Boolean = false,
)

/** The native pairing mode and its session-scoped inputs. */
data class PairingStart(
    /** 1 = QR, 2 = SAS (the frozen PairingMode wire values). */
    val mode: Int,
    /** The 16-byte QR nonce as hex; empty for SAS. */
    val nonceHex: String = "",
    /** The display name offered to the Server (≤ 63 UTF-8 bytes). */
    val displayName: String,
)

/** Wire registry value at the Kotlin/JNI boundary; raw integers never define policy. */
@JvmInline
value class CloseCode(val wireValue: Int) {
    companion object {
        val BUSY = CloseCode(301)
        val RATE_LIMITED = CloseCode(302)
        val SERVER_STOPPING = CloseCode(503)
        val NETWORK_FAILURE = CloseCode(601)
        val DATAGRAM_UNSUPPORTED = CloseCode(602)

        fun fromWire(value: Int): CloseCode = CloseCode(value)
        fun transportLoss(): CloseCode? = null
    }
}

internal object ReconnectPolicy {
    fun shouldReconnect(closeCode: CloseCode?): Boolean = when (closeCode) {
        null -> true
        CloseCode.BUSY,
        CloseCode.RATE_LIMITED,
        CloseCode.SERVER_STOPPING,
        CloseCode.NETWORK_FAILURE,
        CloseCode.DATAGRAM_UNSUPPORTED -> true
        else -> false
    }

    fun shouldReconnect(closeCode: Int?): Boolean = shouldReconnect(closeCode?.let(CloseCode::fromWire))

    /**     * §6.3 backoff for the nth attempt: 1 s base, ×2 per attempt, 30 s cap,
     * ±20% uniform jitter. [random] is injected for deterministic tests.
     */
    fun backoffDelayMs(attempt: Long, random: Double): Long {
        val base = min(
            BACKOFF_INITIAL_MS * (1L shl min(attempt, 20L).toInt()),
            BACKOFF_MAX_MS,
        )
        val jitter = (base * 0.2 * (random * 2 - 1)).toLong()
        return (base + jitter).coerceIn(800L, BACKOFF_MAX_MS)
    }

    private const val BACKOFF_INITIAL_MS = 1_000L
    private const val BACKOFF_MAX_MS = 30_000L
}
