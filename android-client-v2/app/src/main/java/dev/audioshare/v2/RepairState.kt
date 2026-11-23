package dev.audioshare.v2

/**
 * The persisted needs-repairing rule (ADR-0006 §4). A successful
 * authentication clears the flag; a surfaced close whose guidance requires
 * re-pairing sets it. Every other outcome says nothing.
 *
 * One pure function shared by the Service (the durable write) and the UI
 * projection, so the two can never disagree about the same outcome.
 */
internal fun repairStateForOutcome(outcome: SessionOutcome): Boolean? = when (outcome) {
    is SessionOutcome.Live ->
        if (outcome.phase == LivePhase.READY || outcome.phase == LivePhase.PLAYING) {
            false
        } else {
            null
        }

    is SessionOutcome.Ended -> {
        val reason = outcome.reason as? EndReason.Surfaced
        if (reason != null && ErrorGuidanceBook.forCode(reason.code)?.requiresRepairing == true) {
            true
        } else {
            null
        }
    }

    is SessionOutcome.Reconnecting -> null
}
