package dev.audioshare.v2

/**
 * The current answer to what a Client Session is doing (ticket 01 / ADR-0005).
 * The UI consumes only this type; the native state string never crosses the
 * Service seam.
 */
sealed interface SessionOutcome {
    /** The session is alive; [phase] is the semantic projection of native state. */
    data class Live(val phase: LivePhase) : SessionOutcome

    /** The connection dropped and bounded backoff is scheduled; the session lives on. */
    data class Reconnecting(val attempt: Int, val lastCloseCode: Int?) : SessionOutcome

    /** The session terminated; exactly one per generation. */
    data class Ended(val reason: EndReason) : SessionOutcome
}

/** Semantic projection of the native `state` names. */
enum class LivePhase { CONNECTING, AUTHENTICATING, READY, PLAYING }

/** Why a Client Session ended. */
sealed interface EndReason {
    /** The user ended it (in-app Stop, notification Stop, Activity finish). */
    data object UserStop : EndReason

    /** Automatic reconnection is not allowed; [code] feeds ErrorGuidanceBook. */
    data class Surfaced(val code: Int?) : EndReason
}

/** Why [ClientSession.stop] was called. Only [User] produces a visible outcome. */
enum class StopReason { User, Superseded, Shutdown, PairingCancel }

/**
 * A generation-scoped sink owned by [SessionOutcomeStore]. The store drops
 * callbacks from stale generations and anything after the generation's one
 * allowed [SessionOutcome.Ended].
 */
fun interface OutcomeSink {
    fun publish(outcome: SessionOutcome)

    companion object {
        /** Pairing exchanges do not feed the session outcome store (ticket 17). */
        val NONE: OutcomeSink = OutcomeSink { }
    }
}

/**
 * The single classification table. Reuses [ReconnectPolicy] so the retryable
 * set is never duplicated. Attribution comes from the call site: a user Stop
 * is a user Stop regardless of the last close code.
 */
object SessionOutcomePolicy {

    sealed interface Action {
        /** Let [ClientSession] schedule another connection attempt. */
        data object Reconnect : Action

        /** Terminate the Client Session with this reason. */
        data class End(val reason: EndReason) : Action

        /** Internal teardown; no user-visible outcome. */
        data object Silent : Action
    }

    fun classify(closeCode: Int?, stopReason: StopReason? = null): Action = when (stopReason) {
        StopReason.User -> Action.End(EndReason.UserStop)
        StopReason.Superseded,
        StopReason.Shutdown,
        StopReason.PairingCancel -> Action.Silent
        null -> when {
            // Defensive: a 701 is the native user-stop close. Normal user stops
            // are attributed by the call site above, never by this code.
            closeCode == 701 -> Action.End(EndReason.UserStop)
            ReconnectPolicy.shouldReconnect(closeCode) -> Action.Reconnect
            else -> Action.End(EndReason.Surfaced(closeCode))
        }
    }
}

/**
 * Replayable outcome store owned by [ControlSessionService]. Each Client
 * Session receives one [OutcomeSink] from [beginGeneration]; a newer
 * generation (or [invalidate]) orphans every earlier sink. [setListener]
 * replays synchronously, so an Activity recreated mid-session sees the latest
 * outcome without any local persistence.
 */
class SessionOutcomeStore @JvmOverloads constructor(
    initialListener: ((SessionOutcome) -> Unit)? = null,
) {
    private val lock = Any()
    private var latest: SessionOutcome? = null
    private var listener: ((SessionOutcome) -> Unit)? = initialListener
    private var generation = 0L

    /** Start a new Client Session generation. */
    fun beginGeneration(): OutcomeSink = synchronized(lock) {
        generation += 1
        StoreSink(generation)
    }

    /**
     * Orphan the current generation without publishing. Used for Superseded,
     * Shutdown and PairingCancel, whose late events must never reach the UI.
     */
    fun invalidate() {
        synchronized(lock) { generation += 1 }
    }

    /** Replay the latest outcome immediately, if any. */
    fun setListener(listener: ((SessionOutcome) -> Unit)?) {
        synchronized(lock) {
            this.listener = listener
            if (listener != null) latest?.let(listener)
        }
    }

    /** `null` until any generation has published. */
    fun snapshot(): SessionOutcome? = synchronized(lock) { latest }

    private inner class StoreSink(private val ownerGeneration: Long) : OutcomeSink {
        private var ended = false

        override fun publish(outcome: SessionOutcome) {
            synchronized(lock) {
                if (ownerGeneration != generation) return
                if (ended) return
                if (outcome == latest) return
                if (outcome is SessionOutcome.Ended) ended = true
                latest = outcome
                listener?.invoke(outcome)
            }
        }
    }
}
