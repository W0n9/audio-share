package dev.audioshare.v2

/**
 * Service 持有的最近会话状态，新监听者注册时同步重放。
 * 陈旧 native handle 返回的空字符串不属于状态，不能覆盖当前快照。
 */
class SessionStateRelay @JvmOverloads constructor(initialListener: ((String) -> Unit)? = null) {
    private val lock = Any()
    private var state = "IDLE"
    private var listener: ((String) -> Unit)? = initialListener
    private var activeSource: Source? = null

    fun snapshot(): String = synchronized(lock) { state }

    /** Unscoped publication retained for existing callers; sessions use [Source]. */
    fun publish(next: String) {
        if (next.isEmpty()) return
        synchronized(lock) { publishLocked(next) }
    }

    fun setListener(l: ((String) -> Unit)?) {
        synchronized(lock) { listener = l; l?.invoke(state) }
    }

    private fun publishLocked(next: String) {
        state = next
        listener?.invoke(next)
    }

    /** A new native session has its own event source, even if its handle is reused. */
    fun beginSession(): Source = synchronized(lock) {
        Source().also { activeSource = it; publishLocked("IDLE") }
    }

    inner class Source internal constructor() {
        private var closed = false

        fun isCurrent(): Boolean = synchronized(lock) { activeSource === this && !closed }

        /** Native reads never hold the publication lock. A STOP/new session
         * during the read invalidates this source before its result is committed. */
        fun poll(readNativeState: () -> String): String {
            val next = readNativeState()
            return if (publish(next, terminal = next == "CLOSED")) next else ""
        }

        /** Forward a state for this owner. Native CLOSED ends its native source
         * in [poll]; a Service owner stays valid across reconnect until [close]. */
        fun publish(next: String): Boolean = publish(next, terminal = false)

        /** Source validation, terminal state and callback publication are one
         * operation, so a checked-but-late READY cannot follow CLOSED. */
        private fun publish(next: String, terminal: Boolean): Boolean = synchronized(lock) {
            if (next.isEmpty() || activeSource !== this || closed) return false
            if (terminal) closed = true
            publishLocked(next)
            true
        }

        fun close() {
            synchronized(lock) {
                if (activeSource !== this || closed) return
                closed = true
                publishLocked("CLOSED")
            }
        }
    }
}
