package dev.audioshare.v2.session

import dev.audioshare.v2.ClientIdentityResult
import dev.audioshare.v2.ClientSigner
import dev.audioshare.v2.CredentialRecord
import dev.audioshare.v2.CredentialStore
import dev.audioshare.v2.EndReason
import dev.audioshare.v2.LivePhase
import dev.audioshare.v2.PairingStart
import dev.audioshare.v2.PairingStateRelay
import dev.audioshare.v2.SessionConfiguration
import dev.audioshare.v2.SessionOutcome
import dev.audioshare.v2.SessionOutcomeStore
import dev.audioshare.v2.StopReason
import org.json.JSONObject
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * Ticket 21: the Service orchestration seam. A fake [SessionEngine] is the only
 * collaborator, so these tests never touch Android, JNI or Keystore. They drive
 * the same calls [dev.audioshare.v2.ControlSessionService] makes and assert
 * user-visible outcomes: which engine was created, what it was started/stopped
 * with, what reached the replayable stores, and what a stale engine can still
 * publish.
 */
class SessionOrchestratorTest {

    @Test
    fun connect_starts_an_engine_with_the_configuration_and_a_live_outcome_reaches_the_ui() {
        val fixture = Fixture()
        val seen = mutableListOf<SessionOutcome>()
        fixture.outcomes.setListener { seen += it }
        val configuration = authenticatedConfiguration()

        fixture.orchestrator.startSession(configuration, null)

        assertTrue(fixture.orchestrator.hasEngine)
        assertEquals(1, fixture.factory.engines.size)
        val engine = fixture.factory.engines.single()
        assertEquals(listOf(configuration), engine.started)

        val ready = SessionOutcome.Live(LivePhase.READY)
        engine.sink.publish(ready)

        assertEquals(ready, fixture.outcomes.snapshot())
        assertEquals(listOf<SessionOutcome>(ready), seen)
    }

    @Test
    fun a_second_connect_supersedes_the_first_engine_and_its_late_outcome_is_dropped() {
        val fixture = Fixture()
        val configuration = authenticatedConfiguration()

        fixture.orchestrator.startSession(configuration, null)
        val first = fixture.factory.engines.single()
        fixture.orchestrator.startSession(configuration, null)
        val second = fixture.factory.engines.last()

        assertEquals(listOf(StopReason.Superseded), first.stops)
        assertEquals(2, fixture.factory.engines.size)

        val seen = mutableListOf<SessionOutcome>()
        fixture.outcomes.setListener { seen += it }
        first.sink.publish(SessionOutcome.Live(LivePhase.READY))
        second.sink.publish(SessionOutcome.Live(LivePhase.AUTHENTICATING))

        assertEquals(SessionOutcome.Live(LivePhase.AUTHENTICATING), fixture.outcomes.snapshot())
        assertEquals(listOf<SessionOutcome>(SessionOutcome.Live(LivePhase.AUTHENTICATING)), seen)
    }

    @Test
    fun stop_user_stops_the_engine_and_keeps_the_user_stop_outcome_replayable() {
        val fixture = Fixture()
        fixture.orchestrator.startSession(authenticatedConfiguration(), null)
        val engine = fixture.factory.engines.single()
        val seen = mutableListOf<SessionOutcome>()
        fixture.outcomes.setListener { seen += it }
        val ended = SessionOutcome.Ended(EndReason.UserStop)

        fixture.orchestrator.stop(StopReason.User)

        assertEquals(listOf(StopReason.User), engine.stops)
        assertFalse(fixture.orchestrator.hasEngine)
        assertEquals(ended, fixture.outcomes.snapshot())
        assertEquals(listOf<SessionOutcome>(ended), seen)

        // The engine already published its one Ended; nothing later surfaces.
        engine.sink.publish(SessionOutcome.Live(LivePhase.READY))
        assertEquals(ended, fixture.outcomes.snapshot())
        assertEquals(listOf<SessionOutcome>(ended), seen)
    }

    @Test
    fun an_internal_stop_invalidates_the_generation_without_surfacing_an_outcome() {
        for (reason in listOf(StopReason.PairingCancel, StopReason.Shutdown)) {
            val fixture = Fixture()
            fixture.orchestrator.startSession(authenticatedConfiguration(), null)
            val engine = fixture.factory.engines.single()

            fixture.orchestrator.stop(reason)

            assertEquals(listOf(reason), engine.stops)
            assertNull(fixture.outcomes.snapshot(), "$reason must not surface an outcome")

            engine.sink.publish(SessionOutcome.Reconnecting(attempt = 1, lastCloseCode = 601))
            assertNull(fixture.outcomes.snapshot(), "$reason must orphan the old generation")
        }
    }

    @Test
    fun confirm_pairing_is_forwarded_once_per_exchange_and_resets_for_a_new_one() {
        val fixture = Fixture()
        fixture.orchestrator.startSession(pairingConfiguration(), pairingContext())
        val first = fixture.factory.engines.single()

        assertTrue(fixture.orchestrator.confirmPairing())
        assertFalse(fixture.orchestrator.confirmPairing())
        assertEquals(1, first.confirmCalls, "the gate stops a duplicate confirm")

        fixture.orchestrator.startSession(pairingConfiguration(), pairingContext())
        val second = fixture.factory.engines.last()

        assertTrue(fixture.orchestrator.confirmPairing())
        assertEquals(1, second.confirmCalls, "a new exchange gets a fresh confirm")
    }

    @Test
    fun a_rejected_confirm_does_not_consume_the_one_attempt_for_the_exchange() {
        val fixture = Fixture()
        fixture.orchestrator.startSession(pairingConfiguration(), pairingContext())
        val engine = fixture.factory.engines.single()
        engine.confirmResult = false

        assertFalse(fixture.orchestrator.confirmPairing())
        engine.confirmResult = true
        assertTrue(fixture.orchestrator.confirmPairing())
        assertEquals(2, engine.confirmCalls)
    }

    @Test
    fun a_current_engines_credential_is_persisted_with_the_pairing_context() {
        val fixture = Fixture()
        fixture.orchestrator.startSession(pairingConfiguration(), pairingContext())

        fixture.factory.engines.single().listener.onCredential(credentialJson(clientId = "client-1"))

        val record = fixture.credentials.all().single()
        assertEquals(PIN, record.serverSpkiSha256Hex)
        assertEquals(SERVER_KEY, record.serverPublicKeyHex)
        assertEquals("v2-test", record.keyAlias)
        assertEquals("192.0.2.10", record.host)
        assertEquals(65531, record.port)
        assertEquals("192.0.2.10:65531", record.addressHint)
        assertEquals("client-1", record.clientId)
        assertEquals("phone", record.displayName, "an empty native name falls back to the pairing context")
        assertTrue(record.softwareBacked)
    }

    @Test
    fun a_superseded_engines_late_pairing_and_credential_events_are_dropped() {
        val fixture = Fixture()
        fixture.orchestrator.startSession(pairingConfiguration(), pairingContext())
        val first = fixture.factory.engines.single()
        first.listener.onPairingState(awaitingJson())
        assertNotNull(fixture.pairing.snapshot())

        fixture.orchestrator.startSession(pairingConfiguration(), pairingContext())
        val second = fixture.factory.engines.last()
        assertNull(fixture.pairing.snapshot(), "a new exchange clears the old snapshot")

        first.listener.onPairingState(awaitingJson())
        first.listener.onCredential(credentialJson(clientId = "stale"))
        assertNull(fixture.pairing.snapshot())
        assertTrue(fixture.credentials.all().isEmpty(), "a superseded engine cannot persist")

        second.listener.onPairingState(awaitingJson())
        assertNotNull(fixture.pairing.snapshot())
        second.listener.onCredential(credentialJson(clientId = "current"))
        assertEquals("current", fixture.credentials.all().single().clientId)
    }

    @Test
    fun a_stopped_engines_late_pairing_credential_and_outcome_events_are_dropped() {
        val fixture = Fixture()
        fixture.orchestrator.startSession(pairingConfiguration(), pairingContext())
        val engine = fixture.factory.engines.single()
        engine.listener.onPairingState(awaitingJson())
        assertNotNull(fixture.pairing.snapshot())

        fixture.orchestrator.stop(StopReason.User)
        assertNull(fixture.pairing.snapshot(), "Stop clears the visible pairing exchange")

        engine.listener.onPairingState(awaitingJson())
        engine.listener.onCredential(credentialJson(clientId = "stale"))
        engine.sink.publish(SessionOutcome.Live(LivePhase.READY))

        assertNull(fixture.pairing.snapshot())
        assertTrue(fixture.credentials.all().isEmpty())
        assertNull(fixture.outcomes.snapshot(), "a pairing exchange never feeds the outcome store")
    }

    @Test
    fun a_pairing_exchange_never_feeds_the_session_outcome_store() {
        val fixture = Fixture()
        fixture.orchestrator.startSession(pairingConfiguration(), pairingContext())
        val engine = fixture.factory.engines.single()

        engine.sink.publish(SessionOutcome.Live(LivePhase.READY))
        engine.sink.publish(SessionOutcome.Ended(EndReason.Surfaced(204)))

        assertNull(fixture.outcomes.snapshot())
    }

    // ---- ticket 03: needs-repairing persistence ---------------------------

    @Test
    fun a_repairing_surfaced_close_persists_needs_repairing_on_the_credential() {
        val fixture = Fixture()
        fixture.seed(credentialRecord())
        fixture.orchestrator.startSession(authenticatedConfiguration(), null)
        val engine = fixture.factory.engines.single()

        engine.sink.publish(SessionOutcome.Ended(EndReason.Surfaced(204)))

        assertTrue(fixture.credentials.find(PIN)!!.needsRepairing)
    }

    @Test
    fun a_non_repairing_surfaced_close_leaves_the_credential_paired() {
        val fixture = Fixture()
        fixture.seed(credentialRecord())
        fixture.orchestrator.startSession(authenticatedConfiguration(), null)

        fixture.factory.engines.single().sink.publish(
            SessionOutcome.Ended(EndReason.Surfaced(601)),
        )

        assertFalse(fixture.credentials.find(PIN)!!.needsRepairing)
    }

    @Test
    fun successful_authentication_clears_needs_repairing_once() {
        val fixture = Fixture()
        fixture.seed(credentialRecord().copy(needsRepairing = true))
        fixture.orchestrator.startSession(authenticatedConfiguration(), null)
        val engine = fixture.factory.engines.single()
        val upsertsAfterSeed = fixture.credentials.upserts

        engine.sink.publish(SessionOutcome.Live(LivePhase.READY))
        assertFalse(fixture.credentials.find(PIN)!!.needsRepairing)
        assertEquals(upsertsAfterSeed + 1, fixture.credentials.upserts)

        engine.sink.publish(SessionOutcome.Live(LivePhase.PLAYING))
        engine.sink.publish(SessionOutcome.Live(LivePhase.READY))
        assertEquals(
            upsertsAfterSeed + 1,
            fixture.credentials.upserts,
            "a session that is already repaired must not commit on every tick",
        )
    }

    @Test
    fun authenticating_is_not_success_and_does_not_clear_the_flag() {
        val fixture = Fixture()
        fixture.seed(credentialRecord().copy(needsRepairing = true))
        fixture.orchestrator.startSession(authenticatedConfiguration(), null)

        fixture.factory.engines.single().sink.publish(
            SessionOutcome.Live(LivePhase.AUTHENTICATING),
        )

        assertTrue(fixture.credentials.find(PIN)!!.needsRepairing)
    }

    @Test
    fun pairing_persists_a_record_with_needs_repairing_cleared() {
        val fixture = Fixture()
        fixture.seed(credentialRecord().copy(needsRepairing = true))
        fixture.orchestrator.startSession(pairingConfiguration(), pairingContext())

        fixture.factory.engines.single().listener.onCredential(credentialJson("client-1"))

        assertFalse(
            fixture.credentials.find(PIN)!!.needsRepairing,
            "a successful pairing is the explicit re-pair",
        )
    }

    // ---- ticket 04: successful pairing replaces the address ----------------

    @Test
    fun a_successful_pairing_removes_other_credentials_on_the_same_address() {
        val fixture = Fixture()
        val staleOnSameAddress = credentialRecord().copy(
            serverSpkiSha256Hex = "cc".repeat(32),
            clientId = "old-client",
        )
        val otherServer = credentialRecord().copy(
            serverSpkiSha256Hex = "dd".repeat(32),
            host = "198.51.100.7",
            addressHint = "198.51.100.7:65531",
        )
        fixture.seed(credentialRecord().copy(needsRepairing = true))
        fixture.seed(staleOnSameAddress)
        fixture.seed(otherServer)
        fixture.orchestrator.startSession(pairingConfiguration(), pairingContext())

        fixture.factory.engines.single().listener.onCredential(credentialJson("client-1"))

        assertEquals(
            setOf(PIN, "dd".repeat(32)),
            fixture.credentials.all().map { it.serverSpkiSha256Hex }.toSet(),
            "same host:port records are replaced; a different Server is kept",
        )
        val repaired = fixture.credentials.find(PIN)!!
        assertFalse(repaired.needsRepairing, "a successful pairing clears needs-repairing")
        assertEquals("client-1", repaired.clientId, "the new identity replaces the old record")
    }

    @Test
    fun a_missing_key_stored_connect_failure_marks_the_record() {
        val fixture = Fixture()
        fixture.seed(credentialRecord())

        fixture.orchestrator.reportStoredConnectFailure(
            code = 205,
            message = "the stored Client key is gone",
            serverSpkiSha256Hex = PIN,
        )

        assertTrue(fixture.credentials.find(PIN)!!.needsRepairing)
    }

    @Test
    fun a_non_repairing_stored_connect_failure_does_not_mark_the_record() {
        val fixture = Fixture()
        fixture.seed(credentialRecord())

        fixture.orchestrator.reportStoredConnectFailure(
            code = 601,
            message = "network failure",
            serverSpkiSha256Hex = PIN,
        )

        assertFalse(fixture.credentials.find(PIN)!!.needsRepairing)
    }

    @Test
    fun a_superseded_engine_cannot_change_the_repair_state() {
        val fixture = Fixture()
        fixture.seed(credentialRecord())
        fixture.orchestrator.startSession(authenticatedConfiguration(), null)
        val first = fixture.factory.engines.single()

        fixture.orchestrator.startSession(authenticatedConfiguration(), null)
        first.sink.publish(SessionOutcome.Ended(EndReason.Surfaced(204)))

        assertFalse(fixture.credentials.find(PIN)!!.needsRepairing)
    }

    // ---- ticket 17: Service-owned pairing handoff --------------------------

    @Test
    fun a_persisted_credential_hands_off_asynchronously_with_one_authenticated_session() {
        val fixture = Fixture(manualDispatch = true)
        fixture.orchestrator.startSession(pairingConfiguration(), pairingContext())
        val pairingEngine = fixture.factory.engines.single()

        pairingEngine.listener.onPairingState(completeJson())
        pairingEngine.listener.onCredential(credentialJson(clientId = "client-1"))

        assertEquals(1, fixture.factory.engines.size, "persistence alone must not start the session inline")
        assertTrue(fixture.completed.isEmpty(), "the push waits for the dispatched handoff")

        fixture.drain()

        assertEquals(2, fixture.factory.engines.size)
        val authenticated = fixture.factory.engines.last()
        assertEquals(listOf(authenticatedConfiguration().copy(signer = FakeSigner)), authenticated.started)
        assertEquals(listOf(fixture.credentials.all().single()), fixture.completed)
        assertNull(fixture.outcomes.snapshot(), "the authenticated session has published nothing yet")
    }

    @Test
    fun a_persistence_failure_keeps_complete_and_does_not_start_or_push() {
        val fixture = Fixture(manualDispatch = true)
        fixture.credentials.failUpsert = true
        fixture.orchestrator.startSession(pairingConfiguration(), pairingContext())
        val pairingEngine = fixture.factory.engines.single()
        pairingEngine.listener.onPairingState(completeJson())

        pairingEngine.listener.onCredential(credentialJson(clientId = "client-1"))
        fixture.drain()

        assertEquals(1, fixture.factory.engines.size, "no authenticated session")
        assertTrue(fixture.completed.isEmpty(), "no PairingCompleted push")
        assertEquals("COMPLETE", fixture.pairing.snapshot()?.phase, "the terminal pairing stays visible")
        assertTrue(fixture.credentials.all().isEmpty())
    }

    @Test
    fun a_second_credential_callback_for_the_same_exchange_does_not_start_or_push_again() {
        val fixture = Fixture(manualDispatch = true)
        fixture.orchestrator.startSession(pairingConfiguration(), pairingContext())
        val pairingEngine = fixture.factory.engines.single()

        pairingEngine.listener.onCredential(credentialJson(clientId = "client-1"))
        pairingEngine.listener.onCredential(credentialJson(clientId = "client-2"))
        fixture.drain()

        assertEquals(2, fixture.factory.engines.size, "exactly one authenticated session")
        assertEquals(1, fixture.completed.size, "exactly one PairingCompleted")
    }

    @Test
    fun a_handoff_posted_before_cancel_is_dropped() {
        val fixture = Fixture(manualDispatch = true)
        fixture.orchestrator.startSession(pairingConfiguration(), pairingContext())
        val pairingEngine = fixture.factory.engines.single()
        pairingEngine.listener.onCredential(credentialJson(clientId = "client-1"))

        fixture.orchestrator.stop(StopReason.PairingCancel)
        fixture.drain()

        assertEquals(1, fixture.factory.engines.size, "a cancelled exchange does not reconnect")
        assertTrue(fixture.completed.isEmpty())
        assertNull(fixture.outcomes.snapshot())
    }

    @Test
    fun a_stored_connect_without_a_credential_surfaces_one_ended_205_outcome() {
        val fixture = Fixture()
        val seen = mutableListOf<SessionOutcome>()
        fixture.outcomes.setListener { seen += it }

        fixture.orchestrator.reportStoredConnectFailure(205, "no saved pairing for this Server")

        val ended = SessionOutcome.Ended(EndReason.Surfaced(205))
        assertEquals(ended, fixture.outcomes.snapshot())
        assertEquals(listOf<SessionOutcome>(ended), seen)
        assertNull(fixture.pairing.snapshot(), "a stored-connect failure is not a pairing terminal")
    }

    @Test
    fun a_stored_connect_failure_supersedes_any_live_session_before_publishing() {
        val fixture = Fixture()
        fixture.orchestrator.startSession(authenticatedConfiguration(), null)
        val engine = fixture.factory.engines.single()

        fixture.orchestrator.reportStoredConnectFailure(205, "the stored Client key is gone")

        assertEquals(listOf(StopReason.Superseded), engine.stops)
        assertEquals(SessionOutcome.Ended(EndReason.Surfaced(205)), fixture.outcomes.snapshot())
        assertFalse(fixture.orchestrator.hasEngine)
    }

    @Test
    fun the_factory_accepts_the_test_support_configuration_unchanged() {
        val fixture = Fixture()
        val harnessConfiguration = SessionConfiguration(
            host = "127.0.0.1",
            port = 65531,
            serverSpkiSha256Hex = PIN,
            serverPublicKeyHex = SERVER_KEY,
            clientName = "smoke-client",
            useNativeTestIdentity = true,
        )

        fixture.orchestrator.startSession(harnessConfiguration, null)

        assertEquals(listOf(harnessConfiguration), fixture.factory.engines.single().started)
    }

    // ---- fixtures ----------------------------------------------------------

    private class Fixture(manualDispatch: Boolean = false) {
        private val pending = ArrayDeque<() -> Unit>()
        val factory = FakeSessionEngineFactory()
        val outcomes = SessionOutcomeStore()
        val pairing = PairingStateRelay()
        val credentials = InMemoryCredentialStore()
        val logs = mutableListOf<String>()
        val completed = mutableListOf<CredentialRecord>()
        val orchestrator = SessionOrchestrator(
            engineFactory = factory,
            outcomes = outcomes,
            pairing = pairing,
            credentials = credentials,
            log = { logs += it },
            schedule = { task -> if (manualDispatch) pending += task else task() },
            onPairingCompleted = { completed += it },
        )

        /** Seed a previously paired record, as a cold start would load it. */
        fun seed(record: CredentialRecord) = credentials.upsert(record)

        /** Run the tasks the Service's main-thread handler would run. */
        fun drain() {
            while (pending.isNotEmpty()) pending.removeFirst()()
        }
    }

    private companion object {
        const val PIN = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        const val SERVER_KEY = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"

        fun authenticatedConfiguration() = SessionConfiguration(
            host = "192.0.2.10",
            port = 65531,
            serverSpkiSha256Hex = PIN,
            serverPublicKeyHex = SERVER_KEY,
            clientName = "phone",
        )

        fun pairingConfiguration() = authenticatedConfiguration().copy(
            pairing = PairingStart(mode = 1, nonceHex = "00".repeat(16), displayName = "phone"),
        )

        fun pairingContext() = PairingContext(
            identity = ClientIdentityResult(
                signer = FakeSigner,
                alias = "v2-test",
                softwareBacked = true,
                hardwareBacked = false,
            ),
            host = "192.0.2.10",
            port = 65531,
            displayName = "phone",
            mode = "qr",
        )

        fun credentialRecord() = CredentialRecord(
            serverSpkiSha256Hex = PIN,
            serverPublicKeyHex = SERVER_KEY,
            addressHint = "192.0.2.10:65531",
            keyAlias = "v2-test",
            pairedAt = "2026-11-16T00:00:00Z",
            softwareBacked = true,
            clientId = "client-1",
            displayName = "phone",
            host = "192.0.2.10",
            port = 65531,
        )

        fun credentialJson(clientId: String) = JSONObject().apply {
            put("server_spki_sha256", PIN)
            put("server_public_key", SERVER_KEY)
            put("client_id", clientId)
            put("display_name", "")
        }.toString()

        fun awaitingJson() = JSONObject().apply {
            put("mode", "qr")
            put("phase", "AWAITING_CONFIRMATION")
            put("sas", "123456")
            put("fingerprint", "ABCD-EFGH")
            put("server_public_key", SERVER_KEY)
            put("server_spki_sha256", PIN)
            put("client_id", "client-1")
            put("display_name", "phone")
        }.toString()

        fun completeJson() = JSONObject().apply {
            put("mode", "qr")
            put("phase", "COMPLETE")
            put("server_public_key", SERVER_KEY)
            put("server_spki_sha256", PIN)
            put("client_id", "client-1")
            put("display_name", "phone")
        }.toString()
    }
}

/** A minimal signer so [ClientIdentityResult] can be built without Keystore. */
private object FakeSigner : ClientSigner {
    override fun publicKeySec1(): ByteArray = ByteArray(33)
    override fun sign(rawContext: ByteArray): ByteArray = ByteArray(64)
}

private class InMemoryCredentialStore : CredentialStore {
    private val records = mutableListOf<CredentialRecord>()
    var failUpsert = false
    var upserts = 0
        private set

    override fun upsert(record: CredentialRecord) {
        if (failUpsert) throw IllegalStateException("credential store unavailable")
        records.removeAll { it.serverSpkiSha256Hex == record.serverSpkiSha256Hex }
        records += record
        upserts += 1
    }

    override fun all(): List<CredentialRecord> = records.toList()

    override fun find(serverSpkiSha256Hex: String): CredentialRecord? =
        records.firstOrNull { it.serverSpkiSha256Hex == serverSpkiSha256Hex }

    override fun delete(serverSpkiSha256Hex: String) {
        records.removeAll { it.serverSpkiSha256Hex == serverSpkiSha256Hex }
    }
}
