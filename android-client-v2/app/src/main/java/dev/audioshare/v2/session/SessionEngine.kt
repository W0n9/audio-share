package dev.audioshare.v2.session

import dev.audioshare.v2.OutcomeSink
import dev.audioshare.v2.SessionConfiguration
import dev.audioshare.v2.StopReason

/**
 * The only session surface [dev.audioshare.v2.ControlSessionService] calls
 * (ticket 21). The outcome sink is supplied by the factory, and the listener is
 * the pairing/log callback surface the Service already needs; there are no
 * future-only members here.
 */
interface SessionEngine {
    fun start(configuration: SessionConfiguration)
    fun stop(reason: StopReason)
    fun confirmPairing(): Boolean

    interface Listener {
        /** Info/error lines for diagnostics (logcat in production). */
        fun onLog(line: String)

        /** Pairing display material/phase changed (native JSON). */
        fun onPairingState(json: String) {}

        /** `PairingResult(OK)` arrived; the Service persists this credential. */
        fun onCredential(json: String) {}
    }
}

/**
 * Hand-written composition seam: the Service owns one factory and creates a
 * fresh engine per session. The factory receives the generation-scoped outcome
 * sink plus the pairing/log callbacks; [dev.audioshare.v2.ClientSession] is the
 * default implementation.
 */
fun interface SessionEngineFactory {
    fun create(sink: OutcomeSink, listener: SessionEngine.Listener): SessionEngine
}
