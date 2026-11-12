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

/**
 * Kotlin-side session owner (ticket 08): JNI handle lifecycle, bounded
 * exponential backoff with ±20% jitter, and the ConnectivityManager
 * NetworkCallback migration (rebind with fd handoff, fallback to
 * teardown + backoff reconnect).
 *
 * DEVICE/NETWORK ONLY — exercised on the device path; none of it runs in
 * host JVM tests. Device smoke for this ticket is recorded separately
 * (see the ticket's Comments).
 *
 * Media is not surfaced here: the native ring is a skeleton this ticket.
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
    }

    companion object {
        private const val TAG = "ClientSession"
        /** §6.3 backoff policy lives in [ReconnectPolicy]. */
        private const val STATS_POLL_MS = 250L
    }

    private var handle = AtomicLong(0)
    private val running = AtomicBoolean(false)
    private var host = ""
    private var port = 0
    private var pinnedSpkiHex = ""
    private var serverPublicKeyHex = ""
    private var clientName = ""

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

    fun start(
        host: String,
        port: Int,
        pinnedSpkiHex: String,
        serverPublicKeyHex: String,
        clientName: String,
    ) {
        if (running.getAndSet(true)) return
        this.host = host
        this.port = port
        this.pinnedSpkiHex = pinnedSpkiHex
        this.serverPublicKeyHex = serverPublicKeyHex
        this.clientName = clientName
        backoffAttempts.set(0)
        registerNetworkCallback()
        startSession()
    }

    fun stop() {
        if (!running.getAndSet(false)) return
        unregisterNetworkCallback()
        val h = handle.getAndSet(0)
        if (h != 0L) {
            ClientNative.stop(h)
            ClientNative.destroy(h)
        }
        listener.onStateChange("CLOSED")
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

    private fun startSession() {
        executor.execute {
            val h = ClientNative.createSession(
                host, port, pinnedSpkiHex, serverPublicKeyHex, clientName
            )
            if (h == 0L) {
                listener.onLog("createSession failed; retrying after backoff")
                scheduleReconnect()
                return@execute
            }
            handle.set(h)
            val bind = if (bindHintEnabled) (activeNetworkAddress ?: "") else ""
            listener.onLog("connect(host=$host port=$port bind=$bind)")
            val ok = ClientNative.connect(h, bind)
            if (!ok) {
                listener.onLog("native connect() rejected; retrying after backoff")
                scheduleReconnect()
                return@execute
            }
            startStatsPolling(h)
        }
    }

    private fun scheduleReconnect() {
        val h = handle.getAndSet(0)
        if (h != 0L) ClientNative.destroy(h)
        if (!running.get()) return
        val attempt = backoffAttempts.getAndIncrement()
        val delay = ReconnectPolicy.backoffDelayMs(attempt, Random.nextDouble())
        listener.onLog("reconnect attempt ${attempt + 1} in ${delay}ms")
        mainHandler.postDelayed({ if (running.get()) startSession() }, delay)
    }

    private fun startStatsPolling(h: Long) {
        val poller = Thread {
            val myHandle = h
            // §6.3: only retryable closes (and transport loss with no Close,
            // close_code null) reconnect. Pairing/admin/security closes surface
            // to the user and must not auto-retry.
            var reconnectOnExit = true
            try {
                while (running.get() && handle.get() == myHandle) {
                    val state = ClientNative.getState(myHandle) ?: ""
                    listener.onStateChange(state)
                    if (state == "READY") {
                        // §6.3: backoff resets to base when a session reaches READY
                        backoffAttempts.set(0)
                    }
                    val statsJson = ClientNative.getStats(myHandle)
                    if (statsJson.isNullOrEmpty()) continue
                    try {
                        val obj = JSONObject(statsJson)
                        listener.onStats(obj)
                        if (state == "CLOSED") {
                            val closeCode =
                                if (obj.isNull("close_code")) null else obj.optInt("close_code")
                            reconnectOnExit = ReconnectPolicy.shouldReconnect(closeCode)
                            listener.onLog(
                                if (reconnectOnExit) {
                                    "session closed (close_code=$closeCode); reconnecting"
                                } else {
                                    "session closed (close_code=$closeCode); no automatic reconnect"
                                }
                            )
                            break
                        }
                    } catch (e: Exception) {
                        Log.w(TAG, "stats parse: ${e.message}")
                    }
                    Thread.sleep(STATS_POLL_MS)
                }
            } catch (_: InterruptedException) {
            } finally {
                if (running.get() && handle.get() == myHandle && reconnectOnExit) {
                    scheduleReconnect()
                }
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

    private fun registerNetworkCallback() {
        if (networkCallbackRegistered) return
        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                activeNetworkAddress = localAddressOf(network)
                listener.onLog("network available: $activeNetworkAddress")
                if (!running.get()) return
                val h = handle.get()
                if (h == 0L) {
                    // no live session to migrate: normal reconnect path
                    scheduleReconnect()
                    return
                }
                if (tryRebind(h, network)) return
                // rebind failed or was rejected: teardown + backoff reconnect
                teardownForReconnect("network change (rebind fallback)")
            }

            override fun onLost(network: Network) {
                listener.onLog("network lost")
                if (!running.get()) return
                if (handle.get() != 0L) {
                    teardownForReconnect("network lost")
                }
            }
        }
        connectivityManager.registerNetworkCallback(request, callback)
        registeredCallback = callback
        networkCallbackRegistered = true
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
    private fun teardownForReconnect(reason: String) {
        val h = handle.getAndSet(0)
        if (h != 0L) {
            ClientNative.stop(h)
            ClientNative.destroy(h)
        }
        listener.onLog("teardown for reconnect: $reason")
        scheduleReconnect()
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
internal object ReconnectPolicy {
    /** 301/302 retryable; 503/601/602 reconnect; everything else surfaces. */
    fun shouldReconnect(closeCode: Int?): Boolean = when (closeCode) {
        null -> true
        301, 302, 503, 601, 602 -> true
        else -> false
    }

    /**
     * §6.3 backoff for the nth attempt: 1 s base, ×2 per attempt, 30 s cap,
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
