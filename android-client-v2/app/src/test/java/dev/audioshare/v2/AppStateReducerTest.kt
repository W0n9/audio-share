package dev.audioshare.v2

import java.util.Base64
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * Seam 1 of ticket 21: the pure AppState/UiEvent reducer. Every test drives a
 * user-visible transition through [AppReducer.reduce] and asserts the state
 * and the session command the Activity would execute.
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

    @Test
    fun connecting_session_maps_idle_to_connecting_and_tracks_native_states() {
        val startup = AppReducer.reduce(
            AppState(),
            UiEvent.StartupLoaded(rememberedAddress = null, credentials = listOf(record())),
        ).state
        val requested = AppReducer.reduce(startup, UiEvent.ConnectRequested)
        assertEquals(SessionPhase.CONNECTING, requested.state.sessionPhase)
        assertEquals(AppScreen.SESSION, requested.state.screen)
        assertEquals(AppCommand.ConnectStored("ab".repeat(32)), requested.command)

        // The native side reports IDLE before authentication; the UI must keep
        // showing CONNECTING for a connection the user asked for.
        assertEquals(
            SessionPhase.CONNECTING,
            AppReducer.reduce(requested.state, UiEvent.SessionStateChanged("IDLE")).state.sessionPhase,
        )
        assertEquals(
            SessionPhase.AUTHENTICATING,
            AppReducer.reduce(requested.state, UiEvent.SessionStateChanged("AUTHENTICATING")).state.sessionPhase,
        )
        assertEquals(
            SessionPhase.READY,
            AppReducer.reduce(requested.state, UiEvent.SessionStateChanged("READY")).state.sessionPhase,
        )
        assertEquals(
            SessionPhase.PLAYING,
            AppReducer.reduce(requested.state, UiEvent.SessionStateChanged("PLAYING")).state.sessionPhase,
        )
    }

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
    fun qr_mode_confirms_without_typing_the_sas_because_the_qr_pinned_the_server() {
        val awaiting = AppState(
            screen = AppScreen.PAIRING,
            address = "10.0.2.2:65531",
            pairing = PairingUi(
                mode = "qr",
                phase = "AWAITING_CONFIRMATION",
                sas = "123 456",
                fingerprint = "AAAA1111 BBBB2222",
            ),
        )
        assertTrue(awaiting.pairing!!.canConfirm)
        assertEquals(
            AppCommand.ConfirmPairing,
            AppReducer.reduce(awaiting, UiEvent.ConfirmPairing).command,
        )
    }

    @Test
    fun pairing_completion_waits_for_the_credential_before_connecting() {
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

        val stored = AppReducer.reduce(complete.state, UiEvent.CredentialStored(record()))
        assertEquals(AppScreen.SESSION, stored.state.screen)
        assertEquals(SessionPhase.CONNECTING, stored.state.sessionPhase)
        assertTrue(stored.state.hasStoredPairing)
        assertEquals(AppCommand.ConnectStored("ab".repeat(32)), stored.command)
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
    fun terminal_errors_show_guidance_and_repairing_retry_opens_pairing() {
        val startup = AppReducer.reduce(
            AppState(),
            UiEvent.StartupLoaded(null, listOf(record())),
        ).state
        val connected = AppReducer.reduce(startup, UiEvent.ConnectRequested).state
        val revoked = ErrorGuidanceBook.forCode(204)!!
        val failed = AppReducer.reduce(connected, UiEvent.SessionError(revoked)).state
        assertEquals(AppScreen.HOME, failed.screen)
        assertEquals(204, failed.error?.code)
        assertEquals(SessionPhase.CLOSED, failed.sessionPhase)
        assertTrue(failed.error!!.requiresRepairing)

        val retry = AppReducer.reduce(failed, UiEvent.Retry)
        assertEquals(AppScreen.PAIRING, retry.state.screen)
        assertNull(retry.state.error)
        assertEquals(AppCommand.None, retry.command)
    }

    @Test
    fun retryable_network_error_reconnects_without_repairing() {
        val startup = AppReducer.reduce(
            AppState(),
            UiEvent.StartupLoaded(null, listOf(record())),
        ).state
        val connected = AppReducer.reduce(startup, UiEvent.ConnectRequested).state
        val dropped = AppReducer.reduce(
            connected,
            UiEvent.SessionError(ErrorGuidanceBook.forCode(601)!!),
        ).state
        val retry = AppReducer.reduce(dropped, UiEvent.Retry)
        assertEquals(AppCommand.ConnectStored("ab".repeat(32)), retry.command)
        assertEquals(SessionPhase.CONNECTING, retry.state.sessionPhase)
    }

    @Test
    fun stop_returns_home_and_shows_closed() {
        val startup = AppReducer.reduce(
            AppState(),
            UiEvent.StartupLoaded(null, listOf(record())),
        ).state
        val connected = AppReducer.reduce(startup, UiEvent.ConnectRequested).state
        val playing = AppReducer.reduce(connected, UiEvent.SessionStateChanged("PLAYING")).state
        val stopping = AppReducer.reduce(playing, UiEvent.StopRequested)
        assertEquals(AppCommand.Stop, stopping.command)
        assertTrue(stopping.state.stopping)
        val closed = AppReducer.reduce(stopping.state, UiEvent.SessionStateChanged("CLOSED")).state
        assertEquals(AppScreen.HOME, closed.screen)
        assertEquals(SessionPhase.CLOSED, closed.sessionPhase)
    }

    @Test
    fun explicit_stop_clears_a_stale_connection_lost_banner() {
        val startup = AppReducer.reduce(
            AppState(),
            UiEvent.StartupLoaded(null, listOf(record())),
        ).state
        val connected = AppReducer.reduce(startup, UiEvent.ConnectRequested).state
        val playing = AppReducer.reduce(connected, UiEvent.SessionStateChanged("PLAYING")).state
        // A previous network blip left a 601 guidance card on screen even
        // though the session recovered; an explicit Stop must not carry it to
        // HOME.
        val stale = playing.copy(error = ErrorGuidanceBook.forCode(601))
        val stopping = AppReducer.reduce(stale, UiEvent.StopRequested).state
        assertNull(stopping.error, "Stop dismisses the stale banner immediately")
        val closed = AppReducer.reduce(stopping, UiEvent.SessionStateChanged("CLOSED")).state
        assertEquals(AppScreen.HOME, closed.screen)
        assertNull(closed.error, "HOME must not show Connection lost after a user Stop")
    }

    @Test
    fun a_recovered_session_dismisses_the_stale_connection_lost_banner() {
        // The user saw an earlier 601, then the bounded-backoff reconnect
        // succeeded in the background; the banner is no longer true.
        val stale = AppState(
            screen = AppScreen.HOME,
            sessionPhase = SessionPhase.CLOSED,
            error = ErrorGuidanceBook.forCode(601),
        )
        val recovered = AppReducer.reduce(stale, UiEvent.SessionStateChanged("PLAYING")).state
        assertNull(recovered.error, "a live session dismisses the stale error card")
        assertEquals(SessionPhase.PLAYING, recovered.sessionPhase)

        // A later user Stop (including the notification path, which never
        // dispatches StopRequested) must still land on a clean HOME.
        val stopped = AppReducer.reduce(recovered, UiEvent.SessionStateChanged("CLOSED")).state
        assertEquals(AppScreen.HOME, stopped.screen)
        assertNull(stopped.error)
    }

    @Test
    fun a_close_error_arriving_after_the_user_stop_is_not_shown() {
        val playing = AppState(
            screen = AppScreen.SESSION,
            sessionPhase = SessionPhase.PLAYING,
            connectRequested = true,
        )
        val stopping = AppReducer.reduce(playing, UiEvent.StopRequested).state
        val late = AppReducer.reduce(
            stopping,
            UiEvent.SessionError(ErrorGuidanceBook.forCode(601)),
        ).state
        assertNull(late.error, "a close caused by the user's own Stop is not an error")
        assertTrue(late.stopping, "the reducer keeps waiting for the CLOSED replay")
    }

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

    @Test
    fun auto_connect_is_only_for_a_launch_with_a_credential() {
        assertTrue(AutoConnectPolicy.shouldAutoConnect(false, true, "IDLE"))
        assertTrue(AutoConnectPolicy.shouldAutoConnect(false, true, "CLOSED"))
        assertFalse(AutoConnectPolicy.shouldAutoConnect(true, true, "IDLE"), "Activity recreate")
        assertFalse(AutoConnectPolicy.shouldAutoConnect(false, false, "IDLE"), "no pairing")
        assertFalse(AutoConnectPolicy.shouldAutoConnect(false, true, "CONNECTING"), "already live")
        assertFalse(AutoConnectPolicy.shouldAutoConnect(false, true, "AUTHENTICATING"), "already live")
        assertFalse(AutoConnectPolicy.shouldAutoConnect(false, true, "READY"), "already live")
        assertFalse(AutoConnectPolicy.shouldAutoConnect(false, true, "PLAYING"), "already live")
    }

    @Test
    fun auto_connect_event_is_ignored_when_a_session_is_already_active() {
        val startup = AppReducer.reduce(
            AppState(),
            UiEvent.StartupLoaded(null, listOf(record())),
        ).state
        val live = AppReducer.reduce(startup, UiEvent.SessionStateChanged("PLAYING")).state
        val auto = AppReducer.reduce(live, UiEvent.AutoConnectStarted)
        assertEquals(AppCommand.None, auto.command)
        assertEquals(SessionPhase.PLAYING, auto.state.sessionPhase)
    }

    @Test
    fun cancel_pairing_returns_home_without_confirming() {
        val started = AppReducer.reduce(AppState(address = "10.0.2.2:65531"), UiEvent.StartSasPairing).state
        val cancelled = AppReducer.reduce(started, UiEvent.CancelPairing)
        assertEquals(AppScreen.HOME, cancelled.state.screen)
        assertNull(cancelled.state.pairing)
        assertEquals(AppCommand.CancelPairing, cancelled.command)
    }
}
