package dev.audioshare.v2

import android.os.Bundle
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * Ticket 06 §4 restoration contract: a config change saves only screen,
 * address, and SAS input; everything session/pairing-derived replays from the
 * Service; the restored screen is a hint reconciled against that replay.
 *
 * The Bundle round-trip needs Robolectric; the reconcile rules themselves are
 * pure. No MainActivity/Service is launched here (seam: pure functions and the
 * reducer).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class NavigationRestorationTest {

    private fun awaitingSas(sas: String = "385 587") = PairingSnapshot(
        mode = "sas",
        phase = "AWAITING_CONFIRMATION",
        sas = sas,
        fingerprint = "AAAA1111 BBBB2222 CCCC3333 DDDD4444 EEEE5555 FFFF6666 77778888 99990000",
        serverPublicKeyHex = "ee".repeat(32),
        serverSpkiSha256Hex = "ff".repeat(32),
        clientId = null,
        displayName = "Pixel",
        errorCode = null,
    )

    private fun stored() = StoredServer("ab".repeat(32), "Pixel", "10.0.2.2:65531")

    // ---- config-change bundle ----------------------------------------------

    @Test
    fun config_change_saves_only_screen_address_and_sas_input() {
        val state = AppState(
            screen = AppScreen.PAIRING,
            address = "10.0.2.2:65531",
            addressError = AddressError.INVALID_HOST,
            rememberedAddress = "other.example:65531",
            storedServers = listOf(stored()),
            outcome = SessionOutcome.Live(LivePhase.PLAYING),
            stopRequested = true,
            pairing = PairingUi(
                mode = "sas",
                phase = "AWAITING_CONFIRMATION",
                sas = "385 587",
                sasInput = "385587",
            ),
            scanActive = true,
            scanNotice = ScanNotice.CAMERA_PERMISSION_DENIED,
        )
        val bundle = Bundle()

        UiStatePersistence.save(bundle, state)

        assertEquals(
            3,
            bundle.keySet().size,
            "only screen, address, and sasInput may be persisted",
        )
        val restored = UiStatePersistence.restore(bundle)!!
        assertEquals(AppScreen.PAIRING, restored.screen)
        assertEquals("10.0.2.2:65531", restored.address)
        assertEquals("385587", restored.restoredSasInput)
        // Everything session/pairing-derived must replay from the Service.
        assertNull(restored.outcome)
        assertNull(restored.pairing)
        assertTrue(restored.storedServers.isEmpty())
        assertFalse(restored.scanActive, "the camera never reopens automatically")
        assertNull(restored.scanNotice)
        assertNull(restored.addressError, "errors are derived, never persisted")
        assertFalse(restored.stopRequested)
    }

    @Test
    fun no_saved_bundle_keeps_the_cold_start_default() {
        assertNull(UiStatePersistence.restore(null))
    }

    @Test
    fun a_restored_sas_input_survives_the_first_service_snapshot() {
        val restored = AppState(
            screen = AppScreen.PAIRING,
            address = "10.0.2.2:65531",
            restoredSasInput = "385587",
        )

        val reduced = AppReducer.reduce(
            restored,
            UiEvent.PairingSnapshotChanged(awaitingSas()),
        ).state

        assertEquals("385587", reduced.pairing?.sasInput)
        assertEquals("", reduced.restoredSasInput, "the restore hint is consumed once")
        assertTrue(reduced.pairing!!.sasMatches)
    }

    @Test
    fun startup_keeps_the_restored_address_over_the_remembered_one() {
        val restored = AppState(
            screen = AppScreen.PAIRING,
            address = "10.0.2.2:65531",
            restoredSasInput = "385587",
        )

        val reduced = AppReducer.reduce(
            restored,
            UiEvent.StartupLoaded(
                rememberedAddress = "other.example:65531",
                credentials = emptyList(),
            ),
        ).state

        assertEquals("10.0.2.2:65531", reduced.address)
        assertNull(reduced.addressError)
    }

    // ---- reconcile rules ----------------------------------------------------

    @Test
    fun rotation_restores_a_live_session_screen() {
        for (outcome in listOf(
            SessionOutcome.Live(LivePhase.CONNECTING),
            SessionOutcome.Live(LivePhase.AUTHENTICATING),
            SessionOutcome.Live(LivePhase.READY),
            SessionOutcome.Live(LivePhase.PLAYING),
            SessionOutcome.Reconnecting(attempt = 1, lastCloseCode = null),
            SessionOutcome.Reconnecting(attempt = 4, lastCloseCode = 601),
        )) {
            assertEquals(
                AppScreen.SESSION,
                NavigationRestoration.reconcile(AppScreen.SESSION, outcome, false),
                "$outcome owns a live session",
            )
        }
    }

    @Test
    fun a_restored_session_without_a_live_outcome_degrades_home() {
        for (outcome in listOf(
            null,
            SessionOutcome.Ended(EndReason.UserStop),
            SessionOutcome.Ended(EndReason.Surfaced(204)),
            SessionOutcome.Ended(EndReason.Surfaced(601)),
        )) {
            assertEquals(
                AppScreen.HOME,
                NavigationRestoration.reconcile(AppScreen.SESSION, outcome, false),
                "$outcome is not a session worth restoring",
            )
            // If a pairing exchange is live, the SESSION hint has something to
            // reconcile against and is left alone (ticket 16 wording).
            assertEquals(
                AppScreen.SESSION,
                NavigationRestoration.reconcile(AppScreen.SESSION, outcome, true),
            )
        }
    }

    @Test
    fun restored_pairing_without_a_snapshot_degrades_home_on_process_death() {
        for (outcome in listOf(
            null,
            SessionOutcome.Ended(EndReason.UserStop),
            SessionOutcome.Ended(EndReason.Surfaced(204)),
        )) {
            assertEquals(
                AppScreen.HOME,
                NavigationRestoration.reconcile(AppScreen.PAIRING, outcome, false),
                "pairing is never resumed after process death",
            )
        }
    }

    @Test
    fun restored_pairing_without_a_snapshot_lands_session_after_a_missed_handoff() {
        for (outcome in listOf(
            SessionOutcome.Live(LivePhase.CONNECTING),
            SessionOutcome.Live(LivePhase.READY),
            SessionOutcome.Live(LivePhase.PLAYING),
            SessionOutcome.Reconnecting(attempt = 2, lastCloseCode = 601),
        )) {
            assertEquals(
                AppScreen.SESSION,
                NavigationRestoration.reconcile(AppScreen.PAIRING, outcome, false),
                "$outcome means the pairing handoff already happened",
            )
        }
    }

    @Test
    fun a_restored_pairing_with_a_snapshot_stays_pairing() {
        for (outcome in listOf(
            null,
            SessionOutcome.Live(LivePhase.READY),
            SessionOutcome.Reconnecting(attempt = 1, lastCloseCode = null),
            SessionOutcome.Ended(EndReason.UserStop),
        )) {
            assertEquals(
                AppScreen.PAIRING,
                NavigationRestoration.reconcile(AppScreen.PAIRING, outcome, true),
                "an active pairing exchange is restored in place",
            )
        }
    }

    @Test
    fun home_about_and_settings_are_never_reconciled_away() {
        for (screen in listOf(AppScreen.HOME, AppScreen.ABOUT, AppScreen.SETTINGS)) {
            for (outcome in listOf(
                null,
                SessionOutcome.Live(LivePhase.READY),
                SessionOutcome.Ended(EndReason.Surfaced(204)),
            )) {
                assertEquals(
                    screen,
                    NavigationRestoration.reconcile(screen, outcome, false),
                    "$screen is a user-owned or terminal destination",
                )
            }
        }
    }

    // ---- reducer wiring -----------------------------------------------------

    @Test
    fun reconciliation_applies_the_restored_screen_with_the_replayed_outcome() {
        val restored = AppState(
            screen = AppScreen.PAIRING,
            address = "10.0.2.2:65531",
            restoredSasInput = "385587",
        )

        val reduced = AppReducer.reduce(
            restored,
            UiEvent.RestorationReconciled(
                outcome = SessionOutcome.Live(LivePhase.READY),
                pairingSnapshotPresent = false,
            ),
        )

        assertEquals(AppScreen.SESSION, reduced.state.screen)
        assertEquals(SessionOutcome.Live(LivePhase.READY), reduced.state.outcome)
        assertEquals("", reduced.state.restoredSasInput)
        assertEquals(AppCommand.None, reduced.command)
    }

    @Test
    fun process_death_reconciliation_lands_home_and_drops_the_pairing_draft() {
        val restored = AppState(
            screen = AppScreen.PAIRING,
            address = "10.0.2.2:65531",
            restoredSasInput = "385587",
            pairing = PairingUi(mode = "sas", phase = "AWAITING_CONFIRMATION"),
            scanActive = true,
        )

        val reduced = AppReducer.reduce(
            restored,
            UiEvent.RestorationReconciled(outcome = null, pairingSnapshotPresent = false),
        )

        assertEquals(AppScreen.HOME, reduced.state.screen)
        assertNull(reduced.state.pairing)
        assertFalse(reduced.state.scanActive)
        assertEquals("", reduced.state.restoredSasInput)
        assertEquals(AppCommand.None, reduced.command)
    }

    @Test
    fun saved_state_suppresses_auto_connect() {
        assertFalse(
            AutoConnectPolicy.shouldAutoConnect(
                savedStatePresent = true,
                hasCredential = true,
                outcome = null,
            ),
            "a recreated Activity must not auto-connect",
        )
        assertFalse(
            AutoConnectPolicy.shouldAutoConnect(
                savedStatePresent = true,
                hasCredential = true,
                outcome = SessionOutcome.Live(LivePhase.READY),
            ),
        )
    }

    // ---- Back: navigation only ---------------------------------------------

    @Test
    fun back_from_pairing_cancels_the_exchange_then_returns_home() {
        val pairing = AppState(
            screen = AppScreen.PAIRING,
            address = "10.0.2.2:65531",
            pairing = PairingUi(mode = "sas", phase = "AWAITING_CONFIRMATION"),
            scanActive = true,
        )

        val reduced = AppReducer.reduce(pairing, UiEvent.BackPressed)

        assertEquals(AppScreen.HOME, reduced.state.screen)
        assertNull(reduced.state.pairing)
        assertFalse(reduced.state.scanActive)
        assertEquals(AppCommand.CancelPairing, reduced.command)
    }

    @Test
    fun back_from_session_settings_and_about_returns_home_without_stopping() {
        for (screen in listOf(AppScreen.SESSION, AppScreen.SETTINGS, AppScreen.ABOUT)) {
            val reduced = AppReducer.reduce(
                AppState(
                    screen = screen,
                    outcome = SessionOutcome.Live(LivePhase.PLAYING),
                ),
                UiEvent.BackPressed,
            )
            assertEquals(AppScreen.HOME, reduced.state.screen, "Back navigates home")
            assertEquals(AppCommand.None, reduced.command, "Back must never Stop the session")
            assertEquals(
                SessionOutcome.Live(LivePhase.PLAYING),
                reduced.state.outcome,
                "playback keeps running",
            )
        }
    }

    @Test
    fun back_at_home_is_a_no_op_because_the_system_finishes_the_activity() {
        val reduced = AppReducer.reduce(AppState(screen = AppScreen.HOME), UiEvent.BackPressed)
        assertEquals(AppScreen.HOME, reduced.state.screen)
        assertEquals(AppCommand.None, reduced.command)
    }
}
