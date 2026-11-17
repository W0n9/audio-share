package dev.audioshare.v2.session

import dev.audioshare.v2.AvailabilityAction
import dev.audioshare.v2.LivePhase
import dev.audioshare.v2.OutcomeSink
import dev.audioshare.v2.ReconnectPolicy
import dev.audioshare.v2.SessionConfiguration
import dev.audioshare.v2.SessionOutcome
import dev.audioshare.v2.SessionOutcomePolicy
import dev.audioshare.v2.SessionStartupPolicy
import dev.audioshare.v2.StopReason
import org.json.JSONObject
import kotlin.random.Random

/**
 * The native session operations the driver needs (ticket 22). The Android
 * adapter maps this onto [dev.audioshare.v2.ClientNative]; JVM tests use a
 * fake. `create` owns the create-variant choice (pairing / test identity /
 * authenticated).
 */
internal interface NativeSession {
    fun create(configuration: SessionConfiguration): Long
    fun connect(handle: Long, bindLocalAddress: String): Boolean
    fun stop(handle: Long)
    fun destroy(handle: Long)
    fun getState(handle: Long): String?
    fun getStats(handle: Long): String?
    fun rebind(handle: Long, fd: Int): Boolean
    fun getPairingState(handle: Long): String?
    fun getCredentialSummary(handle: Long): String?
    fun confirmPairing(handle: Long): Boolean
}

/**
 * ConnectivityManager callback seam. The Android adapter owns the platform
 * socket work: [Link.openFd] route-pins a fresh UDP socket to the network and
 * hands its raw fd to the driver, which passes it to [NativeSession.rebind].
 *
 * FD OWNERSHIP: once [NativeSession.rebind] is called the fd belongs to native
 * on success AND on failure; the adapter must never close or reuse it.
 */
internal interface NetworkMonitor {
    interface Link {
        /** Active network local address, used for the (default-off) bind hint. */
        val localAddress: String?

        /** Route-pin a socket to this network; null when the platform refused. */
        fun openFd(): Int?
    }

    interface Listener {
        fun onAvailable(link: Link)
        fun onLost(link: Link)
    }

    fun register(listener: Listener)
    fun unregister()
}

/**
 * Timing/threading seam. [execute] is the single worker that serializes native
 * create/connect work; [postDelayed] carries bounded backoff; [loop] runs a
 * poll tick on its own named thread. Tests inject virtual time.
 */
internal interface Scheduler {
    fun execute(task: () -> Unit)

    fun postDelayed(delayMs: Long, task: () -> Unit)

    /**
     * Run [tick] immediately, then again after the delay it returns; a null
     * return exits the loop. [onExit] always runs, including on interruption.
     */
    fun loop(name: String, tick: () -> Long?, onExit: () -> Unit)
}

/** The playback owner for one native session (ticket 14). */
internal interface MediaController {
    fun attach(handle: Long): Boolean
    fun start()
    fun stop()
}

internal fun interface MediaControllerFactory {
    fun create(): MediaController
}

/**
 * Session timing and ownership, extracted from [dev.audioshare.v2.ClientSession]
 * so every race is JVM-testable (ticket 22 / ticket 07 candidate 05).
 *
 * Behaviour is the pre-ticket-22 `ClientSession` behaviour, with one intentional
 * ordering tightening: the playback owner is always stopped before the native
 * session is stopped/destroyed. Close classification is never re-implemented
 * here; [SessionOutcomePolicy] and [ReconnectPolicy] remain the only policy
 * sources.
 */
internal class SessionLifecycleDriver(
    private val native: NativeSession,
    private val network: NetworkMonitor,
    private val scheduler: Scheduler,
    private val mediaFactory: MediaControllerFactory,
    private val sink: OutcomeSink,
    private val listener: SessionEngine.Listener,
    private val random: () -> Double = { Random.nextDouble() },
    /** Default-off: see the VPN-overlay note in [dev.audioshare.v2.ClientSession]. */
    private val bindHintEnabled: () -> Boolean = { false },
) {
    companion object {
        private const val STATS_POLL_MS = 250L
        private const val PAIRING_POLL_MS = 25L
    }

    private val lock = Any()
    private var running = false
    private var lifecycle: Any? = null
    private var handle = 0L
    private var media: MediaController? = null
    private var configuration = SessionConfiguration("", 0, "", "", "")
    private var backoffAttempts = 0L
    private var networkRegistered = false
    @Volatile private var activeNetworkAddress: String? = null

    fun start(configuration: SessionConfiguration) {
        synchronized(lock) {
            if (running) return
            running = true
            this.configuration = configuration
            backoffAttempts = 0
            val lifetime = Any()
            lifecycle = lifetime
            registerNetworkLocked(lifetime)
            // startSession queues itself on the single worker.
            startSession(lifetime)
        }
        // A Client Session begins by connecting; native may still report IDLE.
        sink.publish(SessionOutcome.Live(LivePhase.CONNECTING))
    }

    /**
     * End the session. Only [StopReason.User] publishes an outcome; internal
     * teardowns leave the store to the orchestrator.
     */
    fun stop(reason: StopReason) {
        val player: MediaController?
        val h: Long
        synchronized(lock) {
            if (!running) return
            running = false
            lifecycle = null
            unregisterNetworkLocked()
            h = handle
            handle = 0
            player = media
            media = null
        }
        when (val action = SessionOutcomePolicy.classify(closeCode = null, stopReason = reason)) {
            is SessionOutcomePolicy.Action.End -> sink.publish(SessionOutcome.Ended(action.reason))
            SessionOutcomePolicy.Action.Reconnect,
            SessionOutcomePolicy.Action.Silent -> Unit
        }
        // The playback owner must stop before the native session dies.
        player?.stop()
        if (h != 0L) {
            native.stop(h)
            native.destroy(h)
        }
        listener.onLog("stopped ($reason)")
    }

    fun isRunning(): Boolean = synchronized(lock) { running }

    fun confirmPairing(): Boolean {
        val h = synchronized(lock) { handle }
        return h != 0L && native.confirmPairing(h)
    }

    fun pairingStateJson(): String? {
        val h = synchronized(lock) { handle }
        if (h == 0L) return null
        val json = native.getPairingState(h)
        return if (json.isNullOrEmpty()) null else json
    }

    // ---- create/connect ----------------------------------------------------

    /** 调用者持有 lifecycleLock；数值 handle 不能独自标识事件来源。 */
    private fun ownsLocked(lifetime: Any, h: Long): Boolean =
        running && lifecycle === lifetime && handle == h

    private fun startSession(lifetime: Any) {
        scheduler.execute worker@{
            val configuration = synchronized(lock) {
                if (!ownsLocked(lifetime, 0)) return@worker
                this.configuration
            }
            val h = native.create(configuration)
            if (h == 0L) {
                synchronized(lock) {
                    if (ownsLocked(lifetime, 0)) {
                        listener.onLog("createSession failed; retrying after backoff")
                    }
                }
                scheduleReconnect(0, lifetime, lastCloseCode = null)
                return@worker
            }
            val connected: Boolean? = synchronized(lock) {
                if (!ownsLocked(lifetime, 0)) {
                    null
                } else {
                    handle = h
                    val bind = if (bindHintEnabled()) (activeNetworkAddress ?: "") else ""
                    listener.onLog(
                        "connect(host=${configuration.host} port=${configuration.port} bind=$bind)",
                    )
                    // The ring is attached once per native session, before any
                    // media can arrive; playback starts with the session and
                    // simply idles until native reports PLAYING.
                    val player = mediaFactory.create().also { it.attach(h) }
                    media = player
                    player.start()
                    if (configuration.pairing != null) {
                        startPairingWatcher(h, lifetime)
                    }
                    native.connect(h, bind)
                }
            }
            if (connected == null) {
                // createSession 执行期间，STOP 或其他尝试已取得所有权。
                native.destroy(h)
                return@worker
            }
            if (!connected) {
                synchronized(lock) {
                    if (ownsLocked(lifetime, h)) {
                        listener.onLog("native connect() rejected; retrying after backoff")
                    }
                }
                scheduleReconnect(h, lifetime, lastCloseCode = null)
                return@worker
            }
            startStatsPolling(h, lifetime)
        }
    }

    /**
     * Dedicated pairing-state/credential poll. It is tied to the lifetime, not
     * the handle, because `as-cmd pair` closes the transport immediately after
     * `PairingResult(OK)`; the credential must still be captured.
     */
    private fun startPairingWatcher(h: Long, lifetime: Any) {
        var credentialSeen = false
        var failed = false
        scheduler.loop(
            name = "v2-pairing-watch",
            tick = loop@{
                fun ownsLifetime(): Boolean = synchronized(lock) {
                    running && lifecycle === lifetime
                }
                if (!ownsLifetime()) return@loop null
                val pairingJson = native.getPairingState(h)
                if (!pairingJson.isNullOrEmpty() && ownsLifetime()) {
                    listener.onPairingState(pairingJson)
                    failed = pairingJson.contains("\"phase\":\"FAILED\"")
                }
                val credential = native.getCredentialSummary(h)
                if (!credential.isNullOrEmpty()) {
                    if (!credentialSeen && ownsLifetime()) {
                        credentialSeen = true
                        listener.onCredential(credential)
                    }
                    return@loop null
                }
                val state = native.getState(h) ?: ""
                if (state.isEmpty() && pairingJson.isNullOrEmpty()) return@loop null
                if (failed) return@loop null
                if (state.isNotEmpty() &&
                    state != "IDLE" &&
                    state != "AUTHENTICATING" &&
                    state != "NEGOTIATING" &&
                    state != "READY" &&
                    state != "PLAYING" &&
                    pairingJson.isNullOrEmpty()
                ) {
                    return@loop null
                }
                PAIRING_POLL_MS
            },
            onExit = {},
        )
    }

    // ---- reconnect ---------------------------------------------------------

    private fun scheduleReconnect(h: Long, lifetime: Any, lastCloseCode: Int?) {
        var attempt = 0L
        val player: MediaController?
        synchronized(lock) {
            if (!ownsLocked(lifetime, h)) return
            handle = 0
            attempt = backoffAttempts
            backoffAttempts += 1
            val delay = ReconnectPolicy.backoffDelayMs(attempt, random())
            listener.onLog("reconnect attempt ${attempt + 1} in ${delay}ms")
            scheduler.postDelayed(delay) {
                synchronized(lock) {
                    if (ownsLocked(lifetime, 0)) startSession(lifetime)
                }
            }
            player = detachMediaLocked()
        }
        // The Client Session stays alive across attempts; the UI stays on
        // SESSION and only sees that a retry is in flight.
        sink.publish(SessionOutcome.Reconnecting((attempt + 1).toInt(), lastCloseCode))
        // Ordering invariant: playback stops before the native handle dies.
        player?.stop()
        if (h != 0L) native.destroy(h)
    }

    /** Detach the playback owner under the lock; callers stop it outside. */
    private fun detachMediaLocked(): MediaController? {
        val current = media
        media = null
        return current
    }

    // ---- stats loop --------------------------------------------------------

    private fun startStatsPolling(h: Long, lifetime: Any) {
        var lastCloseCode: Int? = null
        var reconnectOnExit = false
        scheduler.loop(
            name = "v2-stats-poll",
            tick = loop@{
                if (!synchronized(lock) { ownsLocked(lifetime, h) }) return@loop null
                val state = native.getState(h) ?: ""
                if (state.isEmpty()) return@loop STATS_POLL_MS
                publishLive(state)
                if (state == "READY") {
                    // §6.3：仅当前会话可以重置 backoff。
                    synchronized(lock) {
                        if (ownsLocked(lifetime, h)) backoffAttempts = 0
                    }
                }
                val statsJson = native.getStats(h)
                if (state == "CLOSED" && statsJson.isNullOrEmpty()) {
                    // No recorded outcome: treat it exactly like transport loss
                    // (close_code null) and let the policy reconnect.
                    listener.onLog("session closed (close_code=null); reconnecting")
                    reconnectOnExit = true
                    return@loop null
                }
                if (!statsJson.isNullOrEmpty()) {
                    try {
                        val obj = JSONObject(statsJson)
                        if (state == "CLOSED") {
                            lastCloseCode =
                                if (obj.isNull("close_code")) null else obj.optInt("close_code")
                            reconnectOnExit = when (
                                val action = SessionOutcomePolicy.classify(lastCloseCode)
                            ) {
                                SessionOutcomePolicy.Action.Reconnect -> true
                                is SessionOutcomePolicy.Action.End -> {
                                    sink.publish(SessionOutcome.Ended(action.reason))
                                    false
                                }
                                SessionOutcomePolicy.Action.Silent -> false
                            }
                            listener.onLog(
                                if (reconnectOnExit) {
                                    "session closed (close_code=$lastCloseCode); reconnecting"
                                } else {
                                    "session closed (close_code=$lastCloseCode); no automatic reconnect"
                                },
                            )
                            return@loop null
                        }
                    } catch (e: Exception) {
                        listener.onLog("stats parse: ${e.message}")
                    }
                }
                STATS_POLL_MS
            },
            onExit = {
                val player = synchronized(lock) { detachMediaLocked() }
                player?.stop()
                if (reconnectOnExit) scheduleReconnect(h, lifetime, lastCloseCode)
            },
        )
    }

    /** Project a native state name onto the four semantic Live phases. */
    private fun publishLive(state: String) {
        val phase = when (state) {
            "IDLE", "CONNECTING" -> LivePhase.CONNECTING
            "AUTHENTICATING", "NEGOTIATING" -> LivePhase.AUTHENTICATING
            "READY" -> LivePhase.READY
            "PLAYING" -> LivePhase.PLAYING
            else -> return
        }
        sink.publish(SessionOutcome.Live(phase))
    }

    // ---- network callback / rebind ----------------------------------------

    private fun registerNetworkLocked(lifetime: Any) {
        if (networkRegistered) return
        networkRegistered = true
        network.register(
            object : NetworkMonitor.Listener {
                override fun onAvailable(link: NetworkMonitor.Link) =
                    handleAvailable(lifetime, link)

                override fun onLost(link: NetworkMonitor.Link) = handleLost(lifetime)
            },
        )
    }

    private fun unregisterNetworkLocked() {
        if (!networkRegistered) return
        network.unregister()
        networkRegistered = false
    }

    private fun handleAvailable(lifetime: Any, link: NetworkMonitor.Link) {
        val h = synchronized(lock) {
            if (!running || lifecycle !== lifetime) return
            activeNetworkAddress = link.localAddress
            listener.onLog("network available: $activeNetworkAddress")
            handle
        }
        val endpointConnected = h != 0L && nativeEndpointConnected(h)
        when (SessionStartupPolicy.availabilityAction(h, endpointConnected)) {
            AvailabilityAction.IN_FLIGHT_OWNS_START -> {
                // 注册回调会立即投递当前网络。此时启动/重连任务已在队列中，
                // 额外 scheduleReconnect 会制造第二条连接。
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
        if (tryRebind(h, link)) return
        // rebind 可与 STOP/重连交错；只清理本次生命周期。
        teardownForReconnect("network change (rebind fallback)", h, lifetime)
    }

    private fun handleLost(lifetime: Any) {
        val h = synchronized(lock) {
            if (!running || lifecycle !== lifetime) return
            listener.onLog("network lost")
            handle
        }
        if (h != 0L) teardownForReconnect("network lost", h, lifetime)
    }

    /**
     * True only after the native run has stored the QUIC endpoint (the
     * `connected` stat is set immediately after it). rebind() before that point
     * fails with "no endpoint (not connected)".
     */
    private fun nativeEndpointConnected(h: Long): Boolean = try {
        val stats = native.getStats(h)
        !stats.isNullOrEmpty() && JSONObject(stats).optBoolean("connected", false)
    } catch (e: Exception) {
        listener.onLog("read connected stat: ${e.message}")
        false
    }

    /**
     * Try the migration path. The link route-pins a socket and hands over its
     * raw fd; the fd transfers to native on [NativeSession.rebind] either way.
     */
    private fun tryRebind(h: Long, link: NetworkMonitor.Link): Boolean {
        return try {
            val fd = link.openFd()
            if (fd == null) {
                listener.onLog("rebind failed: could not bind a socket to the network")
                return false
            }
            val ok = native.rebind(h, fd)
            if (ok) {
                listener.onLog("endpoint rebound (fd $fd)")
            } else {
                listener.onLog("native rebind failed; falling back to reconnect")
            }
            ok
        } catch (e: Exception) {
            listener.onLog("rebind failed: ${e.message}")
            false
        }
    }

    /** Teardown of the current session, then the bounded-backoff reconnect. */
    private fun teardownForReconnect(reason: String, h: Long, lifetime: Any) {
        // Detach under the lock, stop outside it: MediaPlayer.stop() joins the
        // playback thread for up to 2 s and must never run while locked.
        val player = synchronized(lock) {
            if (!ownsLocked(lifetime, h)) return
            listener.onLog("teardown for reconnect: $reason")
            detachMediaLocked()
        }
        player?.stop()
        val owned = synchronized(lock) {
            if (!ownsLocked(lifetime, h)) {
                false
            } else {
                if (h != 0L) native.stop(h)
                true
            }
        }
        if (owned) scheduleReconnect(h, lifetime, lastCloseCode = null)
    }
}
