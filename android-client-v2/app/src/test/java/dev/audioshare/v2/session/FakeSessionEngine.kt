package dev.audioshare.v2.session

import dev.audioshare.v2.EndReason
import dev.audioshare.v2.OutcomeSink
import dev.audioshare.v2.SessionConfiguration
import dev.audioshare.v2.SessionOutcome
import dev.audioshare.v2.StopReason

/**
 * Hand-written test doubles for the Service orchestration seam (ticket 21).
 * They model the engine contract the real [dev.audioshare.v2.ClientSession]
 * implements: [stop] publishes exactly one Ended(UserStop) for a user stop and
 * nothing for an internal teardown.
 */
internal class FakeSessionEngine(
    val sink: OutcomeSink,
    val listener: SessionEngine.Listener,
) : SessionEngine {
    val started = mutableListOf<SessionConfiguration>()
    val stops = mutableListOf<StopReason>()
    var confirmCalls = 0
    var confirmResult = true

    override fun start(configuration: SessionConfiguration) {
        started += configuration
    }

    override fun stop(reason: StopReason) {
        stops += reason
        if (reason == StopReason.User) {
            sink.publish(SessionOutcome.Ended(EndReason.UserStop))
        }
    }

    override fun confirmPairing(): Boolean {
        confirmCalls += 1
        return confirmResult
    }
}

internal class FakeSessionEngineFactory : SessionEngineFactory {
    val engines = mutableListOf<FakeSessionEngine>()

    override fun create(sink: OutcomeSink, listener: SessionEngine.Listener): SessionEngine =
        FakeSessionEngine(sink, listener).also { engines += it }
}
