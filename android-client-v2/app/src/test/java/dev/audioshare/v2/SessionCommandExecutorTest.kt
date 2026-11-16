package dev.audioshare.v2

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

/** Seam 2: the UI's side effects against a fake SessionController. */
class SessionCommandExecutorTest {

    private class FakeController : SessionController {
        val calls = mutableListOf<String>()
        var error: ErrorGuidance? = null

        override fun setStateListener(listener: ((String) -> Unit)?) = Unit
        override fun stateSnapshot(): String = "IDLE"
        override fun setPairingListener(listener: ((PairingSnapshot?) -> Unit)?) = Unit
        override fun pairingSnapshot(): PairingSnapshot? = null
        override fun beginQrPairing(uri: String) { calls += "qr:$uri" }
        override fun beginSasPairing(address: String) { calls += "sas:$address" }
        override fun confirmPairing() { calls += "confirm" }
        override fun cancelPairing() { calls += "cancel" }
        override fun credentials(): List<CredentialRecord> = emptyList()
        override fun connectStored(serverSpkiSha256Hex: String?) {
            calls += "connect:${serverSpkiSha256Hex ?: "-"}"
        }
        override fun disconnect() { calls += "disconnect" }
        override fun lastErrorGuidance(): ErrorGuidance? = error
    }

    private fun executor() = FakeController().let { it to SessionCommandExecutor(it) }

    @Test
    fun pairing_commands_are_forwarded_verbatim() {
        val (fake, executor) = executor()
        executor.execute(AppCommand.BeginQr("audioshare-pair://p1?pm=2&addr=h:1&key=x&nonce=y"))
        executor.execute(AppCommand.BeginSas("10.0.2.2:65531"))
        executor.execute(AppCommand.ConfirmPairing)
        executor.execute(AppCommand.CancelPairing)
        assertEquals(
            listOf(
                "qr:audioshare-pair://p1?pm=2&addr=h:1&key=x&nonce=y",
                "sas:10.0.2.2:65531",
                "confirm",
                "cancel",
            ),
            fake.calls,
        )
    }

    @Test
    fun stop_disconnects_and_suppresses_a_stale_error() {
        val (fake, executor) = executor()
        fake.error = ErrorGuidanceBook.forCode(601)
        executor.execute(AppCommand.Stop)
        assertEquals(listOf("disconnect"), fake.calls)
        assertNull(executor.guidanceOnClosed(), "a user Stop is not an error")
    }

    @Test
    fun an_unexpected_close_surfaces_the_typed_guidance() {
        val (fake, executor) = executor()
        fake.error = ErrorGuidanceBook.forCode(204)
        val guidance = executor.guidanceOnClosed()
        assertEquals(204, guidance?.code)
        assertEquals(false, guidance?.fallbackToV1)
    }

    @Test
    fun a_new_connect_after_stop_surfaces_the_next_error() {
        val (fake, executor) = executor()
        executor.execute(AppCommand.Stop)
        assertNull(executor.guidanceOnClosed())
        fake.error = ErrorGuidanceBook.forCode(202)
        executor.execute(AppCommand.ConnectStored("ab".repeat(32)))
        assertTrue(fake.calls.contains("connect:" + "ab".repeat(32)))
        assertEquals(202, executor.guidanceOnClosed()?.code)
    }
}
