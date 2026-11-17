package dev.audioshare.v2.session

import dev.audioshare.v2.ClientIdentityResult
import dev.audioshare.v2.CredentialRecord
import dev.audioshare.v2.CredentialStore
import dev.audioshare.v2.OutcomeSink
import dev.audioshare.v2.PairingConfirmGate
import dev.audioshare.v2.PairingSnapshot
import dev.audioshare.v2.PairingStateRelay
import dev.audioshare.v2.SessionConfiguration
import dev.audioshare.v2.SessionOutcomeStore
import dev.audioshare.v2.StopReason
import dev.audioshare.v2.pairedAtNow
import org.json.JSONObject

/**
 * Everything the Service needs to know about the pairing exchange that owns
 * the credential callback. QR parsing, identity creation and the
 * [SessionConfiguration] itself stay in the Service.
 */
internal data class PairingContext(
    val identity: ClientIdentityResult,
    val host: String,
    val port: Int,
    val displayName: String,
    val mode: String,
)

/**
 * Service-owned session orchestration (ticket 21), extracted so it can be
 * driven by a fake [SessionEngine] on the JVM. It owns the engine's lifecycle,
 * the outcome-store generation, pairing-source ownership and credential
 * persistence. It deliberately has no Android dependencies: the Service
 * supplies the logger and the engine factory.
 *
 * The Service remains the composition root: it constructs the stores, the
 * credential store, the factory and this orchestrator, and it keeps the
 * platform-only work (intent dispatch, QR parsing, identity creation,
 * notification/foreground, stopSelf).
 */
internal class SessionOrchestrator(
    private val engineFactory: SessionEngineFactory,
    private val outcomes: SessionOutcomeStore,
    private val pairing: PairingStateRelay,
    private val credentials: CredentialStore,
    private val log: (String) -> Unit,
) {
    private val lock = Any()
    private var engine: SessionEngine? = null
    /** Ownership token for listener callbacks; also orphans a stopped engine. */
    private var generation = 0L
    private var pairingSource: PairingStateRelay.Source? = null
    private val confirmGate = PairingConfirmGate()

    val hasEngine: Boolean
        get() = synchronized(lock) { engine != null }

    /**
     * Start a session, first superseding any current one. A pairing
     * configuration gets [OutcomeSink.NONE]: only the post-handoff
     * authenticated session feeds the outcome store (ticket 17).
     */
    fun startSession(configuration: SessionConfiguration, pairingContext: PairingContext?) {
        confirmGate.reset()
        teardown(StopReason.Superseded)

        // Every session owns a fresh pairing exchange source. beginSession()
        // invalidates the previous one and clears a stale FAILED/COMPLETE
        // banner, so a stopped engine's late watcher event can never overwrite
        // the new exchange's AWAITING_CONFIRMATION.
        val owningPairingSource = pairing.beginSession()
        val ownerGeneration = synchronized(lock) {
            pairingSource = owningPairingSource
            ++generation
        }
        val sink = if (configuration.pairing != null) {
            OutcomeSink.NONE
        } else {
            outcomes.beginGeneration()
        }
        val listener = object : SessionEngine.Listener {
            override fun onLog(line: String) = log(line)

            override fun onPairingState(json: String) {
                // Source + generation ownership: only the current exchange and
                // the current engine may publish.
                if (isCurrent(ownerGeneration)) owningPairingSource.publishJson(json)
            }

            override fun onCredential(json: String) {
                // Capture the pairing context per engine: a late credential
                // from a superseded pairing must not be persisted with the new
                // exchange's identity.
                if (isCurrent(ownerGeneration)) persistCredential(json, pairingContext)
            }
        }
        val current = engineFactory.create(sink, listener)
        synchronized(lock) { engine = current }
        current.start(configuration)
    }

    /**
     * End the current engine. Only [StopReason.User] leaves a visible outcome;
     * every internal teardown orphans the generation so a late
     * Live/Reconnecting event cannot surface.
     */
    fun teardown(reason: StopReason) {
        val current: SessionEngine?
        val source: PairingStateRelay.Source?
        synchronized(lock) {
            // Invalidate callbacks before stop(), so a stopped engine that
            // races a callback cannot reach the stores.
            ++generation
            current = engine
            engine = null
            source = pairingSource
            pairingSource = null
        }
        // Invalidate the pairing exchange first, matching the pre-ticket-21
        // ordering: a late publication from the old engine is dropped.
        source?.close()
        current?.stop(reason)
        if (reason != StopReason.User) outcomes.invalidate()
    }

    /** Explicit Stop/Cancel: teardown plus the Service-level reset. */
    fun stop(reason: StopReason) {
        teardown(reason)
        confirmGate.reset()
        pairing.clear()
    }

    /** Exactly-once per exchange, mirroring the Service's QR confirm gate. */
    fun confirmPairing(): Boolean = confirmGate.confirm {
        synchronized(lock) { engine }?.confirmPairing() == true
    }

    /**
     * A failure before a session exists (bad payload/address/identity) still
     * needs a replayable FAILED snapshot. Reuse the current exchange when one
     * is live, otherwise begin a fresh one.
     */
    fun reportPairingFailure(mode: String, errorCode: Int?, message: String) {
        log("pairing failed: $message")
        val existing = synchronized(lock) { pairingSource }
        val source = existing?.takeIf { it.isCurrent() }
            ?: pairing.beginSession().also { fresh ->
                synchronized(lock) { pairingSource = fresh }
            }
        source.publish(
            PairingSnapshot(
                mode = mode,
                phase = "FAILED",
                sas = null,
                fingerprint = null,
                serverPublicKeyHex = null,
                serverSpkiSha256Hex = null,
                clientId = null,
                displayName = null,
                errorCode = errorCode,
            )
        )
    }

    private fun isCurrent(ownerGeneration: Long): Boolean =
        synchronized(lock) { ownerGeneration == generation }

    private fun persistCredential(json: String, context: PairingContext?) {
        if (context == null) {
            log("credential arrived without pairing context; not persisted")
            return
        }
        try {
            val obj = JSONObject(json)
            val record = CredentialRecord(
                serverSpkiSha256Hex = obj.getString("server_spki_sha256"),
                serverPublicKeyHex = obj.getString("server_public_key"),
                addressHint = "${context.host}:${context.port}",
                keyAlias = context.identity.alias,
                pairedAt = pairedAtNow(),
                softwareBacked = context.identity.softwareBacked,
                clientId = obj.getString("client_id"),
                displayName = obj.optString("display_name").ifEmpty { context.displayName },
                host = context.host,
                port = context.port,
            )
            // Only PairingResult(OK) produces a credential summary, so this is
            // the only persistence point.
            credentials.upsert(record)
            log("stored pairing for ${record.displayName} (software_backed=${record.softwareBacked})")
        } catch (e: Exception) {
            log("credential parse failed: ${e.message}")
        }
    }
}
