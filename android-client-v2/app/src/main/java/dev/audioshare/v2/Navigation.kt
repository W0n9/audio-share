package dev.audioshare.v2

/**
 * Ticket 06 §4 restoration table. The restored screen is a hint, not an
 * authority: on the first Service bind it is reconciled against the replayed
 * outcome and pairing snapshot. Kept pure so the whole table is directly
 * testable (see `NavigationRestorationTest`).
 */
object NavigationRestoration {

    /**
     * @param restored the screen saved from the previous Activity instance.
     * @param outcome the Service-owned outcome replayed on first bind.
     * @param pairingSnapshotPresent whether the Service currently replays a
     *   pairing snapshot (an exchange is still live).
     */
    fun reconcile(
        restored: AppScreen,
        outcome: SessionOutcome?,
        pairingSnapshotPresent: Boolean,
    ): AppScreen = when (restored) {
        // A rotation of a live/reconnecting session keeps the Session screen.
        // With no live outcome and no pairing exchange there is nothing to
        // own the screen, so process death lands HOME.
        AppScreen.SESSION -> {
            val live = outcome is SessionOutcome.Live || outcome is SessionOutcome.Reconnecting
            if (live || pairingSnapshotPresent) AppScreen.SESSION else AppScreen.HOME
        }

        // Pairing is never resumed after it was lost. If the handoff already
        // produced a live session but the completion push was missed, the
        // restored PAIRING hint belongs on SESSION.
        AppScreen.PAIRING -> when {
            pairingSnapshotPresent -> AppScreen.PAIRING
            outcome is SessionOutcome.Live || outcome is SessionOutcome.Reconnecting ->
                AppScreen.SESSION
            else -> AppScreen.HOME
        }

        // User-driven or terminal destinations are never reconciled away.
        AppScreen.HOME,
        AppScreen.ABOUT,
        AppScreen.SETTINGS -> restored
    }
}
