package dev.prototype.wireproto

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
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import kotlin.math.min
import kotlin.random.Random

/**
 * Kotlin-side session owner: JNI handle lifecycle, direct-ByteBuffer media
 * consumer, bounded exponential backoff with jitter, and the
 * ConnectivityManager.NetworkCallback migration scaffold.
 *
 * DEVICE/NETWORK ONLY — this class is exercised on the emulator/device path;
 * none of it runs in host JVM tests. Untested parts are called out inline.
 */
class ProtoSession(
    private val appContext: Context,
    private val listener: Listener,
) {
    interface Listener {
        /** Called on a worker thread whenever counters/state are refreshed. */
        fun onStats(json: JSONObject)
        /** Called when the user-visible session state name changes. */
        fun onStateChange(state: String)
        /** Info/error lines for the prototype log view. */
        fun onLog(line: String)
    }

    companion object {
        private const val TAG = "ProtoSession"
        private const val RING_CAPACITY = 4 * 1024 * 1024
        /** Prefetch target: 50 ms of stereo f32 at 48 kHz (matches native). */
        private const val BUFFER_TARGET_BYTES = 9_600L
        /** Backoff bounds. */
        private const val BACKOFF_INITIAL_MS = 500L
        private const val BACKOFF_MAX_MS = 30_000L
        private const val STATS_POLL_MS = 250L
    }

    private var handle: AtomicLong = AtomicLong(0)
    private val running = AtomicBoolean(false)
    private var credential = ""
    private var host = ""
    private var port = 0
    private var pinnedSpkiHex = ""

    /** Reconnect attempt counter; reset on READY (AUTH_OK observed). */
    private val backoffAttempts = AtomicLong(0)

    // ---- migration scaffold -----------------------------------------------
    private val connectivityManager =
        appContext.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    private var networkCallbackRegistered = false
    private val executor = Executors.newSingleThreadExecutor()
    private val mainHandler = Handler(Looper.getMainLooper())

    @Volatile private var activeNetworkAddress: String? = null
    @Volatile private var registeredCallback: ConnectivityManager.NetworkCallback? = null

    // ---- direct buffer + consumer -----------------------------------------
    private val mediaBuffer: ByteBuffer = ByteBuffer.allocateDirect(RING_CAPACITY)
    private var consumerThread: Thread? = null
    private val consumerStop = AtomicBoolean(false)
    private var audioTrack: android.media.AudioTrack? = null
    private var localReadPos = 0L

    /** Set once playback actually started; cleared on stop (never claims playback after). */
    @Volatile var playbackActive = false
        private set

    fun start(host: String, port: Int, pinnedSpkiHex: String, credential: String) {
        if (running.getAndSet(true)) return
        this.host = host
        this.port = port
        this.pinnedSpkiHex = pinnedSpkiHex
        this.credential = credential
        backoffAttempts.set(0)
        registerNetworkCallback()
        startSession()
    }

    fun stop() {
        if (!running.getAndSet(false)) return
        unregisterNetworkCallback()
        stopPlayback("user stop")
        val h = handle.getAndSet(0)
        if (h != 0L) {
            ProtoNative.stop(h)
            ProtoNative.destroy(h)
        }
        listener.onStateChange("CLOSED")
        listener.onLog("stopped by user")
    }

    fun isRunning() = running.get()

    // ---- session lifecycle with backoff ------------------------------------

    private fun startSession() {
        executor.execute {
            val h = ProtoNative.createSession(host, port, pinnedSpkiHex, credential)
            if (h == 0L) {
                listener.onLog("createSession failed; retrying after backoff")
                scheduleReconnect()
                return@execute
            }
            handle.set(h)
            // attach the ONE direct buffer (media path)
            mediaBuffer.clear()
            if (!ProtoNative.attachBuffer(h, mediaBuffer)) {
                listener.onLog("attachBuffer failed")
            }
            val bind = activeNetworkAddress ?: ""
            listener.onLog("connect(host=$host port=$port bind=$bind)")
            val ok = ProtoNative.connect(h, bind)
            if (!ok) {
                listener.onLog("native connect() rejected; retrying after backoff")
                scheduleReconnect()
                return@execute
            }
            startStatsPolling(h)
            // subscribe once READY is observed by the poller (see maybeSubscribe)
        }
    }

    private fun scheduleReconnect() {
        val h = handle.getAndSet(0)
        if (h != 0L) ProtoNative.destroy(h)
        if (!running.get()) return
        val attempt = backoffAttempts.getAndIncrement()
        // bounded exponential backoff with +/-20% jitter
        val base = min(BACKOFF_INITIAL_MS * (1L shl min(attempt, 20).toInt()), BACKOFF_MAX_MS)
        val jitter = (base * 0.2 * (Random.nextDouble() * 2 - 1)).toLong()
        val delay = (base + jitter).coerceIn(BACKOFF_INITIAL_MS, BACKOFF_MAX_MS)
        listener.onLog("reconnect attempt ${attempt + 1} in ${delay}ms")
        mainHandler.postDelayed({ if (running.get()) startSession() }, delay)
    }

    /** Subscribes exactly when the session first reaches READY. */
    private val subscribed = AtomicBoolean(false)

    private fun startStatsPolling(h: Long) {
        val poller = Thread {
            val myHandle = h
            try {
                while (running.get() && handle.get() == myHandle) {
                    val state = ProtoNative.getState(myHandle) ?: ""
                    listener.onStateChange(state)
                    val statsJson = ProtoNative.getStats(myHandle)
                    if (statsJson.isNullOrEmpty()) continue
                    try {
                        val obj = JSONObject(statsJson)
                        listener.onStats(obj)
                        maybeSubscribe(myHandle, obj, state)
                        if (state == "CLOSED") {
                            if (!running.get() || !obj.optBoolean("connected", false)) {
                                // server closed / auth failed / error: reconnect path
                                break
                            }
                        }
                    } catch (e: Exception) {
                        Log.w(TAG, "stats parse: ${e.message}")
                    }
                    Thread.sleep(STATS_POLL_MS)
                }
            } catch (_: InterruptedException) {
            } finally {
                if (running.get() && handle.get() == myHandle) {
                    stopPlayback("session ended")
                    scheduleReconnect()
                }
            }
        }
        poller.name = "proto-stats-poll"
        poller.start()
    }

    private fun maybeSubscribe(h: Long, stats: JSONObject, state: String) {
        if (state == "READY" && subscribed.compareAndSet(false, true)) {
            val ok = ProtoNative.subscribe(h)
            listener.onLog(if (ok) "SUBSCRIBE sent" else "subscribe rejected by native")
        } else if (state != "READY") {
            // session recreated or left READY: allow a future subscribe
            if (state == "CLOSED" || state == "IDLE") subscribed.set(false)
        }
        if (state == "PLAYING" && !playbackActive) {
            startConsumer(stats)
        }
    }

    // ---- media consumer (direct ByteBuffer -> AudioTrack) ------------------

    private fun startConsumer(stats: JSONObject) {
        val media = stats.optJSONObject("media") ?: return
        val sampleRate = media.optInt("sample_rate", 48000)
        val channels = media.optInt("channels", 2)
        if (audioTrack == null) {
            val minBuf = android.media.AudioTrack.getMinBufferSize(
                sampleRate,
                if (channels == 1) android.media.AudioFormat.CHANNEL_OUT_MONO
                else android.media.AudioFormat.CHANNEL_OUT_STEREO,
                android.media.AudioFormat.ENCODING_PCM_FLOAT,
            )
            audioTrack = android.media.AudioTrack.Builder()
                .setAudioAttributes(
                    android.media.AudioAttributes.Builder()
                        .setUsage(android.media.AudioAttributes.USAGE_MEDIA)
                        .setContentType(android.media.AudioAttributes.CONTENT_TYPE_MUSIC)
                        .build()
                )
                .setAudioFormat(
                    android.media.AudioFormat.Builder()
                        .setSampleRate(sampleRate)
                        .setEncoding(android.media.AudioFormat.ENCODING_PCM_FLOAT)
                        .setChannelMask(
                            if (channels == 1) android.media.AudioFormat.CHANNEL_OUT_MONO
                            else android.media.AudioFormat.CHANNEL_OUT_STEREO
                        )
                        .build()
                )
                .setTransferMode(android.media.AudioTrack.MODE_STREAM)
                .setBufferSizeInBytes(maxOf(minBuf, sampleRate * channels * 4)) // ~1 s
                .build()
        }
        audioTrack?.play()
        playbackActive = true
        localReadPos = 0L
        consumerStop.set(false)
        val t = Thread {
            try {
                while (!consumerStop.get() && running.get()) {
                    val writePos = currentWritePos()
                    val avail = writePos - localReadPos
                    if (avail <= 0) {
                        Thread.sleep(20)
                        continue
                    }
                    if (avail > RING_CAPACITY) {
                        // reader lapped by the overwrite-oldest ring: resync
                        listener.onLog("ring lapped (${avail}B ahead); resynced")
                        localReadPos = writePos
                        continue
                    }
                    val want = min(avail, (sampleRate * channels * 4).toLong()).toInt()
                    val bytes = readFromRing(localReadPos, want) ?: break
                    val floats = FloatArray(want / 4)
                    ByteBuffer.wrap(bytes)
                        .order(ByteOrder.LITTLE_ENDIAN)
                        .asFloatBuffer()
                        .get(floats)
                    val written =
                        audioTrack?.write(floats, 0, floats.size, android.media.AudioTrack.WRITE_BLOCKING) ?: 0
                    if (written > 0) {
                        localReadPos += written.toLong() * 4
                    } else {
                        Thread.sleep(10)
                    }
                }
            } catch (_: InterruptedException) {
            } finally {
                playbackActive = false
            }
        }
        t.name = "proto-media-consumer"
        consumerThread = t
        t.start()
        listener.onLog("playback consumer started (${sampleRate} Hz, $channels ch)")
    }

    private fun stopPlayback(reason: String) {
        consumerStop.set(true)
        consumerThread?.interrupt()
        consumerThread = null
        audioTrack?.pause()
        audioTrack?.flush()
        audioTrack?.release()
        audioTrack = null
        playbackActive = false
        Log.i(TAG, "playback stopped: $reason")
    }

    /** write_pos cursor from getStats (low-frequency JNI use). */
    private fun currentWritePos(): Long {
        val json = ProtoNative.getStats(handle.get()) ?: return localReadPos
        return try {
            JSONObject(json).optJSONObject("buffer")
                ?.optLong("write_pos", localReadPos) ?: localReadPos
        } catch (_: Exception) {
            localReadPos
        }
    }

    /** Copies up to [len] bytes starting at ring offset [start]. */
    private fun readFromRing(start: Long, len: Int): ByteArray? {
        if (len <= 0) return ByteArray(0)
        val out = ByteArray(len)
        val cap = RING_CAPACITY
        var i = 0
        var off = (start % cap).toInt()
        while (i < len) {
            val run = min(len - i, cap - off)
            for (k in 0 until run) {
                out[i + k] = mediaBuffer.get(off + k)
            }
            i += run
            off = (off + run) % cap
        }
        return out
    }

    // ---- NetworkCallback migration scaffold -------------------------------
    //
    // SCAFFOLD, honestly labeled: we observe the default network and, on
    // change, hand its local address to native connect() as a UDP bind hint.
    // This does NOT pin the route the way Network.bindSocket(fd) would, and
    // quinn endpoint rebind / true migration is NOT wired here — a network
    // change tears the session down and reconnects with backoff. All of that
    // is device-gated (see BUILD.md).

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
                    if (h != 0L) {
                        // kill the session on the stale network; backoff reconnects
                        ProtoNative.stop(h)
                        ProtoNative.destroy(h)
                        handle.set(0)
                        subscribed.set(false)
                        stopPlayback("network change")
                        scheduleReconnect()
                    } else {
                        scheduleReconnect()
                    }
                }

                override fun onLost(network: Network) {
                    listener.onLog("network lost: stopping media")
                    if (!running.get()) return
                    val h = handle.get()
                    if (h != 0L) {
                        ProtoNative.stop(h)
                        ProtoNative.destroy(h)
                        handle.set(0)
                        subscribed.set(false)
                        stopPlayback("network lost")
                        scheduleReconnect()
                    }
                }
            }
        connectivityManager.registerNetworkCallback(request, callback)
        registeredCallback = callback
        networkCallbackRegistered = true
    }

    private fun unregisterNetworkCallback() {
        if (!networkCallbackRegistered) return
        registeredCallback?.let { connectivityManager.unregisterNetworkCallback(it) }
        registeredCallback = null
        networkCallbackRegistered = false
    }

    /**
     * Best-effort local address of [network]: prefers the IPv4 address of the
     * default route; falls back to any address. Migration scaffold only.
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
