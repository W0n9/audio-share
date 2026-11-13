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

    /** 保留既有调用者的无来源发布接口；会话使用 [Source]。 */
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

    /** 每个新 native 会话有独立事件来源，即使数值 handle 被复用。 */
    fun beginSession(): Source = synchronized(lock) {
        Source().also { activeSource = it; publishLocked("IDLE") }
    }

    inner class Source internal constructor() {
        private var closed = false

        fun isCurrent(): Boolean = synchronized(lock) { activeSource === this && !closed }

        /** native 读取不持有发布锁；读取期间的 STOP/新会话会在提交结果前使旧来源失效。 */
        fun poll(readNativeState: () -> String): String {
            val next = readNativeState()
            return if (publish(next, terminal = next == "CLOSED")) next else ""
        }

        /** 转发此所有者的状态；[poll] 中 native CLOSED 终止 native 来源，
         * Service 所有者则保持到 [close]，允许合法重连。 */
        fun publish(next: String): Boolean = publish(next, terminal = false)

        /** 来源校验、终态写入与回调发布原子化，避免已检查但迟到的 READY 跟在 CLOSED 后面。 */
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
