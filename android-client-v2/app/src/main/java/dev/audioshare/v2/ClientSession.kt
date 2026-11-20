package dev.audioshare.v2

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import dev.audioshare.v2.session.MediaController
import dev.audioshare.v2.session.MediaControllerFactory
import dev.audioshare.v2.session.NativeSession
import dev.audioshare.v2.session.NetworkMonitor
import dev.audioshare.v2.session.Scheduler
import dev.audioshare.v2.session.SessionEngine
import dev.audioshare.v2.session.SessionLifecycleDriver
import java.net.Inet4Address
import java.net.Inet6Address
import java.util.concurrent.Executors
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
    override fun recordPlaybackWithHead(
        handle: Long,
        submittedBytes: Long,
        playbackHeadFrames: Long,
        underruns: Long,
        bufferCurrentMs: Int,
    ) = ClientNative.recordPlaybackWithHead(
        handle,
        submittedBytes,
        playbackHeadFrames,
        underruns,
        bufferCurrentMs,
    )
    override fun recordTrim(handle: Long, frames: Long) =
        ClientNative.recordTrim(handle, frames)
    override fun recordSoftTrim(handle: Long, frames: Long) =
        ClientNative.recordSoftTrim(handle, frames)
    override fun recordUnderrun(handle: Long, frames: Long) =
        ClientNative.recordUnderrun(handle, frames)
    override fun jitterTargetMs(handle: Long): Int = ClientNative.jitterTargetMs(handle)
    override fun jitterHardLimitMs(handle: Long): Int = ClientNative.jitterHardLimitMs(handle)
    override fun driftCorrectionPpm(handle: Long): Int =
        ClientNative.driftCorrectionPpm(handle)
    override fun takeDriftIntervention(handle: Long): Int =
        ClientNative.takeDriftIntervention(handle)
    override fun recordDriftIntervention(handle: Long, frames: Int) =
        ClientNative.recordDriftIntervention(handle, frames)
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
 * Android adapter for the JVM-testable [SessionLifecycleDriver] (ticket 22).
 * It owns every platform object — Context, ConnectivityManager, Handler,
 * [MediaPlayer] and [ClientNative] — while the driver owns session timing,
 * generation and teardown order.
 *
 * [bindHintEnabled] is false by default: the UDP bind hint pins the socket to
 * the active network's interface, which breaks routing when the server is
 * reached through a VPN overlay (e.g. tailscale) because the route lives on
 * tun, not on the physical network the hint names. Device finding, 2026-11-11.
 */
class ClientSession(
    appContext: Context,
    private val listener: SessionEngine.Listener,
    /** Generation-scoped outcome owner supplied by the Service. */
    sink: OutcomeSink,
) : SessionEngine {
    @Volatile var bindHintEnabled = false

    private val connectivityManager =
        appContext.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager

    private val driver = SessionLifecycleDriver(
        native = ClientNativeSession(listener),
        network = AndroidNetworkMonitor(connectivityManager),
        scheduler = AndroidScheduler(),
        mediaFactory = MediaControllerFactory {
            MediaPlayerController(MediaPlayer(NativeMediaBridge))
        },
        sink = sink,
        listener = listener,
        random = { Random.nextDouble() },
        bindHintEnabled = { bindHintEnabled },
    )

    override fun start(configuration: SessionConfiguration) = driver.start(configuration)

    override fun stop(reason: StopReason) = driver.stop(reason)

    override fun confirmPairing(): Boolean = driver.confirmPairing()
}

/**
 * Pick the native constructor for this configuration:
 * - a pairing session sends `PairingRequest`;
 * - the explicit test-support path keeps the native fixture identity;
 * - production reconnection passes the Keystore-backed signer.
 */
private class ClientNativeSession(
    private val listener: SessionEngine.Listener,
) : NativeSession {
    override fun create(configuration: SessionConfiguration): Long {
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

    override fun connect(handle: Long, bindLocalAddress: String): Boolean =
        ClientNative.connect(handle, bindLocalAddress)

    override fun stop(handle: Long) = ClientNative.stop(handle)

    override fun destroy(handle: Long) = ClientNative.destroy(handle)

    override fun getState(handle: Long): String? = ClientNative.getState(handle)

    override fun getStats(handle: Long): String? = ClientNative.getStats(handle)

    override fun rebind(handle: Long, fd: Int): Boolean = ClientNative.rebind(handle, fd)

    override fun getPairingState(handle: Long): String? = ClientNative.getPairingState(handle)

    override fun getCredentialSummary(handle: Long): String? =
        ClientNative.getCredentialSummary(handle)

    override fun confirmPairing(handle: Long): Boolean = ClientNative.confirmPairing(handle)
}

/** Production [Scheduler]: one worker for create/connect, main Handler for backoff. */
private class AndroidScheduler : Scheduler {
    private val worker = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "v2-session-worker")
    }
    private val mainHandler = Handler(Looper.getMainLooper())

    override fun execute(task: () -> Unit) {
        worker.execute(task)
    }

    override fun postDelayed(delayMs: Long, task: () -> Unit) {
        mainHandler.postDelayed({ execute(task) }, delayMs)
    }

    override fun loop(name: String, tick: () -> Long?, onExit: () -> Unit) {
        val thread = Thread({
            try {
                while (true) {
                    val nextDelay = tick() ?: break
                    Thread.sleep(nextDelay)
                }
            } catch (_: InterruptedException) {
                // Session teardown interrupts naturally through ownership.
            } finally {
                onExit()
            }
        }, name)
        thread.start()
    }
}

/**
 * Production [NetworkMonitor]. The fd handoff stays here:
 * [Network.bindSocket] route-pins a fresh DatagramSocket and
 * [ParcelFileDescriptor.fromDatagramSocket] wraps its existing fd (no dup).
 * After the rebind call native owns the fd; the JVM side never closes it.
 */
private class AndroidNetworkMonitor(
    private val connectivityManager: ConnectivityManager,
) : NetworkMonitor {
    private var callback: ConnectivityManager.NetworkCallback? = null

    override fun register(listener: NetworkMonitor.Listener) {
        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()
        val registered = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                listener.onAvailable(AndroidLink(network, localAddressOf(network)))
            }

            override fun onLost(network: Network) {
                listener.onLost(AndroidLink(network, localAddress = null))
            }
        }
        connectivityManager.registerNetworkCallback(request, registered)
        callback = registered
    }

    override fun unregister() {
        callback?.let { connectivityManager.unregisterNetworkCallback(it) }
        callback = null
    }

    /** Best effort: IPv4 default-route address first, then any address. */
    private fun localAddressOf(network: Network): String? = try {
        val lp = connectivityManager.getLinkProperties(network) ?: return null
        lp.linkAddresses.firstOrNull { it.address is Inet4Address }?.address?.hostAddress
            ?: lp.linkAddresses.firstOrNull { it.address is Inet6Address }?.address?.hostAddress
    } catch (_: Exception) {
        null
    }

    private class AndroidLink(
        private val network: Network,
        override val localAddress: String?,
    ) : NetworkMonitor.Link {
        override fun openFd(): Int? {
            val socket = java.net.DatagramSocket()
            network.bindSocket(socket) // route-pin to this Network
            return ParcelFileDescriptor.fromDatagramSocket(socket).fd
        }
    }
}

/** [MediaController] over one session's [MediaPlayer] and its native ring. */
private class MediaPlayerController(
    private val player: MediaPlayer,
) : MediaController {
    override fun attach(handle: Long): Boolean = player.attach(handle)
    override fun start() = player.start()
    override fun stop() = player.stop()
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
