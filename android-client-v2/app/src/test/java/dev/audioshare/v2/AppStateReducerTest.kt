package dev.audioshare.v2

import java.util.Base64
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * Seam 1 of ticket 21, rewritten for ticket 15: the UI state is driven by a
 * single [SessionOutcome]. Every test asserts a user-visible destination, the
 * derived error card, the emitted command, or an auto-connect decision.
 */
class AppStateReducerTest {

    private fun qrUri(
        address: String = "10.0.2.2:65531",
        key: ByteArray = ByteArray(32) { it.toByte() },
        nonce: ByteArray = ByteArray(16) { (it + 1).toByte() },
    ): String {
        val encoder = Base64.getUrlEncoder().withoutPadding()
        return "audioshare-pair://p1?pm=2&addr=$address" +
            "&key=${encoder.encodeToString(key)}&nonce=${encoder.encodeToString(nonce)}"
    }

    private fun record(
        pin: String = "ab".repeat(32),
        address: String = "10.0.2.2:65531",
    ) = CredentialRecord(
        serverSpkiSha256Hex = pin,
        serverPublicKeyHex = "cd".repeat(32),
        addressHint = address,
        keyAlias = "v2-test",
        pairedAt = "2026-11-16T00:00:00Z",
        softwareBacked = false,
        clientId = "client-1",
        displayName = "Pixel",
        host = address.substringBeforeLast(':'),
        port = address.substringAfterLast(':').toInt(),
    )

    private fun stored() = StoredServer("ab".repeat(32), "Pixel", "10.0.2.2:65531")

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

    private fun awaitingQr(
        sas: String = "123 456",
        fingerprint: String = "AAAA1111 BBBB2222",
    ) = awaitingSas(sas).copy(mode = "qr", fingerprint = fingerprint)

    private fun sessionState(
        outcome: SessionOutcome?,
        screen: AppScreen = AppScreen.SESSION,
    ) = AppState(
        screen = screen,
        address = "10.0.2.2:65531",
        storedServers = listOf(stored()),
        outcome = outcome,
    )

    // ---- outcome -> screen (ticket 06 / 08 scenarios 1-7) -------------------

    @Test
    fun live_phases_stay_on_the_session_screen() {
        for (phase in LivePhase.values()) {
            val reduced = AppReducer.reduce(
                sessionState(SessionOutcome.Live(LivePhase.CONNECTING)),
                UiEvent.SessionOutcomeChanged(SessionOutcome.Live(phase)),
            )
            assertEquals(AppScreen.SESSION, reduced.state.screen)
            assertEquals(SessionOutcome.Live(phase), reduced.state.outcome)
            assertEquals(AppCommand.None, reduced.command)
        }
    }

    @Test
    fun reconnecting_stays_on_the_session_screen_and_keeps_stop_available() {
        val playing = sessionState(SessionOutcome.Live(LivePhase.PLAYING))
        val reconnecting = SessionOutcome.Reconnecting(attempt = 3, lastCloseCode = 601)
        val reduced = AppReducer.reduce(playing, UiEvent.SessionOutcomeChanged(reconnecting))
        assertEquals(AppScreen.SESSION, reduced.state.screen)
        assertEquals(reconnecting, reduced.state.outcome)
        assertFalse(reduced.state.stopRequested, "Stop stays available while reconnecting")
    }

    @Test
    fun a_reconnect_does_not_pull_the_user_back_from_home() {
        val session = sessionState(SessionOutcome.Live(LivePhase.PLAYING))
        val home = AppReducer.reduce(session, UiEvent.ShowHome).state
        val reconnecting = AppReducer.reduce(
            home,
            UiEvent.SessionOutcomeChanged(SessionOutcome.Reconnecting(1, null)),
        ).state
        assertEquals(AppScreen.HOME, reconnecting.screen)
    }

    @Test
    fun ended_user_stop_stays_on_session_without_an_error_card() {
        val reduced = AppReducer.reduce(
            sessionState(SessionOutcome.Live(LivePhase.PLAYING)),
            UiEvent.SessionOutcomeChanged(SessionOutcome.Ended(EndReason.UserStop)),
        )
        assertEquals(AppScreen.SESSION, reduced.state.screen)
        assertEquals(SessionOutcome.Ended(EndReason.UserStop), reduced.state.outcome)
        assertNull(reduced.state.sessionError, "a user Stop is not an error")
    }

    @Test
    fun ended_surfaced_stays_on_session_with_typed_guidance() {
        val reduced = AppReducer.reduce(
            sessionState(SessionOutcome.Live(LivePhase.READY)),
            UiEvent.SessionOutcomeChanged(SessionOutcome.Ended(EndReason.Surfaced(204))),
        )
        assertEquals(AppScreen.SESSION, reduced.state.screen)
        assertEquals(204, reduced.state.sessionError?.code)
        assertTrue(reduced.state.sessionError!!.requiresRepairing)
    }

    @Test
    fun a_retryable_ended_outcome_shows_guidance_that_does_not_require_repairing() {
        val reduced = AppReducer.reduce(
            sessionState(SessionOutcome.Live(LivePhase.PLAYING)),
            UiEvent.SessionOutcomeChanged(SessionOutcome.Ended(EndReason.Surfaced(402))),
        )
        assertEquals(402, reduced.state.sessionError?.code)
        assertFalse(reduced.state.sessionError!!.requiresRepairing)
    }

    @Test
    fun pairing_failure_stays_on_pairing_and_has_no_session_error() {
        val started = AppReducer.reduce(
            AppState(address = "10.0.2.2:65531"),
            UiEvent.StartSasPairing,
        ).state
        val failed = AppReducer.reduce(
            started,
            UiEvent.PairingSnapshotChanged(awaitingSas().copy(phase = "FAILED", errorCode = 201)),
        ).state
        assertEquals(AppScreen.PAIRING, failed.screen)
        assertNull(failed.sessionError)
        assertTrue(failed.pairing!!.isBlockingInvitationFailure)
    }

    @Test
    fun a_session_outcome_does_not_pull_the_user_off_about() {
        val about = AppState(screen = AppScreen.ABOUT)
        val ended = AppReducer.reduce(
            about,
            UiEvent.SessionOutcomeChanged(SessionOutcome.Ended(EndReason.Surfaced(204))),
        ).state
        assertEquals(AppScreen.ABOUT, ended.screen)
        assertEquals(204, ended.sessionError?.code)
    }

    // ---- Connect / auto-connect --------------------------------------------

    @Test
    fun connect_navigates_to_session_and_requests_the_selected_credential() {
        val startup = AppReducer.reduce(
            AppState(),
            UiEvent.StartupLoaded(rememberedAddress = null, credentials = listOf(record())),
        ).state
        val requested = AppReducer.reduce(startup, UiEvent.ConnectRequested)
        assertEquals(AppScreen.SESSION, requested.state.screen)
        assertEquals(AppCommand.ConnectStored("ab".repeat(32)), requested.command)
        assertNull(requested.state.outcome, "the Service is the only outcome owner")
    }

    @Test
    fun connect_without_a_credential_stays_home_with_an_address_error() {
        val requested = AppReducer.reduce(AppState(), UiEvent.ConnectRequested)
        assertEquals(AppScreen.HOME, requested.state.screen)
        assertNotNull(requested.state.addressError)
        assertEquals(AppCommand.None, requested.command)
    }

    @Test
    fun connect_while_live_only_navigates_and_never_reissues_connect_stored() {
        for (outcome in listOf(
            SessionOutcome.Live(LivePhase.READY),
            SessionOutcome.Live(LivePhase.PLAYING),
            SessionOutcome.Reconnecting(2, 601),
        )) {
            val live = sessionState(outcome, screen = AppScreen.HOME)
            val reduced = AppReducer.reduce(live, UiEvent.ConnectRequested)
            assertEquals(AppScreen.SESSION, reduced.state.screen)
            assertEquals(AppCommand.None, reduced.command)
            assertEquals(outcome, reduced.state.outcome)
        }
    }

    @Test
    fun connect_after_ended_issues_a_new_connect() {
        for (reason in listOf(EndReason.UserStop, EndReason.Surfaced(601))) {
            val ended = sessionState(SessionOutcome.Ended(reason))
            val reduced = AppReducer.reduce(ended, UiEvent.ConnectRequested)
            assertEquals(AppScreen.SESSION, reduced.state.screen)
            assertEquals(AppCommand.ConnectStored("ab".repeat(32)), reduced.command)
        }
    }

    @Test
    fun stop_disables_the_button_until_the_service_publishes_ended() {
        val playing = sessionState(SessionOutcome.Live(LivePhase.PLAYING))
        val stopping = AppReducer.reduce(playing, UiEvent.StopRequested)
        assertEquals(AppCommand.Stop, stopping.command)
        assertTrue(stopping.state.stopRequested)
        val ended = AppReducer.reduce(
            stopping.state,
            UiEvent.SessionOutcomeChanged(SessionOutcome.Ended(EndReason.UserStop)),
        ).state
        assertFalse(ended.stopRequested, "the terminal outcome re-enables the primary action")
    }

    @Test
    fun auto_connect_is_eligible_only_for_no_outcome_or_an_ended_session() {
        assertTrue(AutoConnectPolicy.shouldAutoConnect(false, true, null))
        assertTrue(
            AutoConnectPolicy.shouldAutoConnect(
                false, true, SessionOutcome.Ended(EndReason.UserStop),
            )
        )
        assertTrue(
            AutoConnectPolicy.shouldAutoConnect(
                false, true, SessionOutcome.Ended(EndReason.Surfaced(601)),
            )
        )
        assertFalse(
            AutoConnectPolicy.shouldAutoConnect(
                false, true, SessionOutcome.Ended(EndReason.Surfaced(204)),
            ),
            "a repairing close must not auto-retry",
        )
        for (outcome in listOf(
            SessionOutcome.Live(LivePhase.CONNECTING),
            SessionOutcome.Live(LivePhase.PLAYING),
            SessionOutcome.Reconnecting(1, null),
        )) {
            assertFalse(
                AutoConnectPolicy.shouldAutoConnect(false, true, outcome),
                "$outcome already owns the session",
            )
        }
        assertFalse(AutoConnectPolicy.shouldAutoConnect(true, true, null), "Activity recreate")
        assertFalse(AutoConnectPolicy.shouldAutoConnect(false, false, null), "no pairing")
    }

    @Test
    fun auto_connect_event_is_ignored_when_a_session_already_exists() {
        val startup = AppReducer.reduce(
            AppState(),
            UiEvent.StartupLoaded(null, listOf(record())),
        ).state
        val live = AppReducer.reduce(
            startup,
            UiEvent.SessionOutcomeChanged(SessionOutcome.Live(LivePhase.PLAYING)),
        ).state
        val auto = AppReducer.reduce(live, UiEvent.AutoConnectStarted)
        assertEquals(AppCommand.None, auto.command)
        assertEquals(SessionOutcome.Live(LivePhase.PLAYING), auto.state.outcome)
    }

    @Test
    fun auto_connect_event_starts_from_home_with_a_credential_and_no_session() {
        val startup = AppReducer.reduce(
            AppState(),
            UiEvent.StartupLoaded(null, listOf(record())),
        ).state
        val auto = AppReducer.reduce(startup, UiEvent.AutoConnectStarted)
        assertEquals(AppScreen.SESSION, auto.state.screen)
        assertEquals(AppCommand.ConnectStored("ab".repeat(32)), auto.command)
    }

    // ---- pairing / credential handoff (unchanged behaviour) ----------------

    @Test
    fun address_input_is_validated_with_the_pairing_payload_rules() {
        val invalid = AppReducer.reduce(AppState(), UiEvent.AddressChanged("10.0.2.2"))
        assertNotNull(invalid.state.addressError, "a bare host has no port")
        assertEquals(AppCommand.None, invalid.command)

        val valid = AppReducer.reduce(invalid.state, UiEvent.AddressChanged("10.0.2.2:65531"))
        assertNull(valid.state.addressError)

        val empty = AppReducer.reduce(valid.state, UiEvent.AddressChanged(""))
        assertNull(empty.state.addressError, "an empty field is incomplete, not invalid")
    }

    @Test
    fun sas_confirmation_requires_the_code_the_server_shows() {
        val started = AppReducer.reduce(
            AppState(address = "10.0.2.2:65531"),
            UiEvent.StartSasPairing,
        )
        assertEquals(AppCommand.BeginSas("10.0.2.2:65531"), started.command)
        assertEquals(AppScreen.PAIRING, started.state.screen)

        val awaiting = AppReducer.reduce(started.state, UiEvent.PairingSnapshotChanged(awaitingSas())).state
        assertEquals("385 587", awaiting.pairing?.sas)
        assertNotNull(awaiting.pairing?.fingerprint)

        val mismatch = AppReducer.reduce(awaiting, UiEvent.SasInputChanged("385588")).state
        val rejected = AppReducer.reduce(mismatch, UiEvent.ConfirmPairing)
        assertEquals(AppCommand.None, rejected.command, "a mismatching code must never confirm")
        assertFalse(rejected.state.pairing!!.sasMatches)

        val matching = AppReducer.reduce(awaiting, UiEvent.SasInputChanged("385587")).state
        assertTrue(matching.pairing!!.sasMatches)
        assertEquals(AppCommand.ConfirmPairing, AppReducer.reduce(matching, UiEvent.ConfirmPairing).command)
    }

    @Test
    fun qr_mode_auto_confirms_exactly_once_after_the_native_binding_check() {
        val started = AppReducer.reduce(
            AppState(address = "10.0.2.2:65531"),
            UiEvent.StartQrPairing(qrUri()),
        )

        val awaiting = AppReducer.reduce(
            started.state,
            UiEvent.PairingSnapshotChanged(awaitingQr()),
        )
        assertEquals(
            AppCommand.ConfirmPairing,
            awaiting.command,
            "the scan is the Client's action; QR mode confirms without a tap",
        )
        assertTrue(awaiting.state.pairing!!.autoConfirmSent)
        assertEquals("123 456", awaiting.state.pairing!!.sas)
        assertEquals("AAAA1111 BBBB2222", awaiting.state.pairing!!.fingerprint)

        val duplicate = AppReducer.reduce(
            awaiting.state,
            UiEvent.PairingSnapshotChanged(awaitingQr()),
        )
        assertEquals(AppCommand.None, duplicate.command)
        assertTrue(duplicate.state.pairing!!.autoConfirmSent)

        val confirming = AppReducer.reduce(
            awaiting.state,
            UiEvent.PairingSnapshotChanged(awaitingQr().copy(phase = "CONFIRMING")),
        )
        assertEquals(AppCommand.None, confirming.command)
        assertTrue(confirming.state.pairing!!.isConfirming)
    }

    @Test
    fun qr_auto_confirm_rearms_if_the_native_phase_never_advances() {
        val started = AppReducer.reduce(
            AppState(address = "10.0.2.2:65531"),
            UiEvent.StartQrPairing(qrUri()),
        ).state
        var state = AppReducer.reduce(
            started,
            UiEvent.PairingSnapshotChanged(awaitingQr()),
        ).state
        var retried = false
        repeat(AppReducer.QR_AUTO_CONFIRM_RETRY_SNAPSHOTS) {
            val next = AppReducer.reduce(state, UiEvent.PairingSnapshotChanged(awaitingQr()))
            state = next.state
            if (next.command == AppCommand.ConfirmPairing) retried = true
        }
        assertTrue(
            retried,
            "a confirmation the Service rejected must not wedge the screen in AWAITING",
        )
    }

    @Test
    fun sas_mode_never_auto_confirms_and_keeps_the_explicit_tap() {
        val started = AppReducer.reduce(
            AppState(address = "10.0.2.2:65531"),
            UiEvent.StartSasPairing,
        )
        val awaiting = AppReducer.reduce(
            started.state,
            UiEvent.PairingSnapshotChanged(awaitingSas()),
        )
        assertEquals(AppCommand.None, awaiting.command, "SAS is not one-action")
        assertFalse(awaiting.state.pairing!!.autoConfirmSent)
        assertFalse(awaiting.state.pairing!!.canConfirm)

        val matching = AppReducer.reduce(awaiting.state, UiEvent.SasInputChanged("385587")).state
        assertEquals(
            AppCommand.ConfirmPairing,
            AppReducer.reduce(matching, UiEvent.ConfirmPairing).command,
            "the explicit SAS Confirm still works",
        )
    }

    @Test
    fun invalid_or_timed_out_qr_blocks_and_waits_for_an_explicit_rescan() {
        val started = AppReducer.reduce(
            AppState(address = "10.0.2.2:65531"),
            UiEvent.StartQrPairing(qrUri()),
        ).state
        val scanning = started.copy(scanActive = true)

        for (code in listOf(201, 206)) {
            val failed = AppReducer.reduce(
                scanning,
                UiEvent.PairingSnapshotChanged(
                    awaitingQr().copy(phase = "FAILED", errorCode = code)
                ),
            )
            assertEquals(AppCommand.None, failed.command)
            assertEquals(AppScreen.PAIRING, failed.state.screen)
            assertFalse(failed.state.scanActive, "the scanner closes and stays closed")
            assertTrue(failed.state.pairing!!.isBlockingInvitationFailure)
            assertEquals(code, failed.state.pairing!!.errorCode)

            val rescan = AppReducer.reduce(failed.state, UiEvent.RequestScan)
            assertTrue(rescan.state.scanActive)
        }
    }

    @Test
    fun a_relay_clear_does_not_erase_a_blocking_pairing_failure() {
        val started = AppReducer.reduce(
            AppState(address = "10.0.2.2:65531"),
            UiEvent.StartQrPairing(qrUri()),
        ).state
        val failed = AppReducer.reduce(
            started,
            UiEvent.PairingSnapshotChanged(awaitingQr().copy(phase = "FAILED", errorCode = 201)),
        ).state
        val cleared = AppReducer.reduce(failed, UiEvent.PairingSnapshotChanged(null)).state
        assertEquals(AppScreen.PAIRING, cleared.screen)
        assertTrue(cleared.pairing!!.isBlockingInvitationFailure)
    }

    @Test
    fun pairing_completion_does_not_connect_until_the_service_pushes_pairing_completed() {
        val started = AppReducer.reduce(
            AppState(address = "10.0.2.2:65531"),
            UiEvent.StartQrPairing(qrUri()),
        )
        assertEquals(AppScreen.PAIRING, started.state.screen)
        assertEquals("10.0.2.2:65531", started.state.address)
        assertTrue(started.command is AppCommand.BeginQr)

        val complete = AppReducer.reduce(
            started.state,
            UiEvent.PairingSnapshotChanged(awaitingSas().copy(phase = "COMPLETE")),
        )
        assertTrue(complete.state.pairing!!.isComplete)
        assertEquals(AppScreen.PAIRING, complete.state.screen)
        assertEquals(AppCommand.None, complete.command, "the credential may not be stored yet")

        // Ticket 17: the credential observation is list-only; the Service owns
        // the one authenticated connection and the one-shot push.
        val stored = AppReducer.reduce(complete.state, UiEvent.CredentialStored(record()))
        assertEquals(AppScreen.PAIRING, stored.state.screen, "CredentialStored must not navigate")
        assertTrue(stored.state.hasStoredPairing)
        assertTrue(stored.state.pairing!!.isComplete, "the visible COMPLETE state is untouched")
        assertEquals(AppCommand.None, stored.command, "the UI must not start a second connection")
    }

    @Test
    fun pairing_completed_navigates_to_session_and_updates_the_server_list_without_connecting() {
        val complete = AppReducer.reduce(
            AppState(address = "10.0.2.2:65531"),
            UiEvent.StartSasPairing,
        ).let {
            AppReducer.reduce(
                it.state,
                UiEvent.PairingSnapshotChanged(awaitingSas().copy(phase = "COMPLETE")),
            ).state
        }

        val completed = AppReducer.reduce(complete, UiEvent.PairingCompleted(record()))

        assertEquals(AppScreen.SESSION, completed.state.screen)
        assertEquals(listOf(stored()), completed.state.storedServers)
        assertNull(completed.state.pairing)
        assertFalse(completed.state.stopRequested)
        assertEquals(AppCommand.None, completed.command, "the Service already started the only session")
    }

    @Test
    fun invalid_qr_payload_is_a_pairing_failure_not_a_crash() {
        val rejected = AppReducer.reduce(
            AppState(address = "10.0.2.2:65531"),
            UiEvent.StartQrPairing("https://example.invalid/nope"),
        )
        assertEquals(AppScreen.PAIRING, rejected.state.screen)
        assertTrue(rejected.state.pairing!!.isFailed)
        assertNotNull(rejected.state.pairing!!.message)
        assertEquals(AppCommand.None, rejected.command)
    }

    @Test
    fun cancel_pairing_returns_home_without_confirming() {
        val started = AppReducer.reduce(AppState(address = "10.0.2.2:65531"), UiEvent.StartSasPairing).state
        val cancelled = AppReducer.reduce(started, UiEvent.CancelPairing)
        assertEquals(AppScreen.HOME, cancelled.state.screen)
        assertNull(cancelled.state.pairing)
        assertEquals(AppCommand.CancelPairing, cancelled.command)
    }

    // ---- retry --------------------------------------------------------------

    @Test
    fun retry_after_a_repairing_close_opens_pairing() {
        val ended = sessionState(SessionOutcome.Ended(EndReason.Surfaced(204)))
        val retry = AppReducer.reduce(ended, UiEvent.Retry)
        assertEquals(AppScreen.PAIRING, retry.state.screen)
        assertNull(retry.state.pairing)
        assertEquals(AppCommand.None, retry.command)
    }

    @Test
    fun retry_after_a_retryable_close_reconnects_without_repairing() {
        val ended = sessionState(SessionOutcome.Ended(EndReason.Surfaced(601)))
        val retry = AppReducer.reduce(ended, UiEvent.Retry)
        assertEquals(AppScreen.SESSION, retry.state.screen)
        assertEquals(AppCommand.ConnectStored("ab".repeat(32)), retry.command)
    }

    // ---- startup ------------------------------------------------------------

    @Test
    fun startup_restores_the_last_address_and_chooses_its_saved_server() {
        val other = record(pin = "12".repeat(32), address = "other.example:65531")
        val matching = record()
        val loaded = AppReducer.reduce(
            AppState(),
            UiEvent.StartupLoaded("10.0.2.2:65531", listOf(other, matching)),
        ).state
        assertEquals("10.0.2.2:65531", loaded.address)
        assertEquals(matching.serverSpkiSha256Hex, loaded.selectedPin())

        val typedOther = AppReducer.reduce(loaded, UiEvent.AddressChanged("other.example:65531")).state
        assertEquals(other.serverSpkiSha256Hex, typedOther.selectedPin())
    }
}
