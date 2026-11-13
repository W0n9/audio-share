package dev.audioshare.v2

/**
 * Service 持有的最近会话状态，新监听者注册时同步重放。
 * 陈旧 native handle 返回的空字符串不属于状态，不能覆盖当前快照。
 */
class SessionStateRelay {
    private val lock = Any()
    private var state = "IDLE"
    private var listener: ((String) -> Unit)? = null

    fun snapshot(): String = synchronized(lock) { state }

    fun publish(next: String) {
        if (next.isEmpty()) return
        val target = synchronized(lock) { state = next; listener }
        target?.invoke(next)
    }

    fun setListener(l: ((String) -> Unit)?) {
        val current = synchronized(lock) { listener = l; state }
        l?.invoke(current)
    }
}
