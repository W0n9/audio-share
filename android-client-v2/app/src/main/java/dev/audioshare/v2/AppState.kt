package dev.audioshare.v2

/** The five beta screens (ticket 06 "Navigation": HOME, PAIRING, SESSION, ABOUT, SETTINGS). */
enum class AppScreen { HOME, PAIRING, SESSION, ABOUT, SETTINGS }

/**
 * Why the PAIRING screen was opened (ticket 04). `First` is the general
 * pairing page; `RePair` is the dedicated re-pairing flow for one stored
 * Server, which keeps the failure cause visible and shows the fixed
 * "ask the Server operator to start pairing" instruction. The five-screen
 * model is unchanged: both intents share the PAIRING screen.
 */
sealed interface PairingIntent {
    data object First : PairingIntent
    data class RePair(val serverSpkiSha256Hex: String) : PairingIntent
}

/** A paired Server as the UI needs it (public credential data only). */
data class StoredServer(
    val serverSpkiSha256Hex: String,
    val displayName: String?,
    val address: String,
    /** Persisted credential state: the Server must be paired again. */
    val needsRepairing: Boolean = false,
) {
    companion object {
        fun from(record: CredentialRecord) = StoredServer(
            serverSpkiSha256Hex = record.serverSpkiSha256Hex,
            displayName = record.displayName,
            address = record.addressHint.ifEmpty { "${record.host}:${record.port}" },
            needsRepairing = record.needsRepairing,
        )
    }
}

/** Pairing screen state; SAS/fingerprint come from the Service's snapshot. */
data class PairingUi(
    val mode: String,
    val phase: String,
    val sas: String? = null,
    val fingerprint: String? = null,
    /** What the user typed after reading the code on the Server screen. */
    val sasInput: String = "",
    val errorCode: Int? = null,
    /** Localizable pairing notice; copy resolves through resources. */
    val notice: PairingNotice? = null,
    /**
     * QR mode only: the Client already dispatched the one automatic
     * `PairingConfirm` for this exchange. The Service and native phase
     * (`CONFIRMING`) are the durable guards; this flag stops duplicate
     * snapshots in one UI lifetime from queueing a second confirmation.
     */
    val autoConfirmSent: Boolean = false,
    /**
     * Repeated `AWAITING_CONFIRMATION` snapshots seen after the automatic
     * confirm. If the Service rejected the queued intent the native phase
     * never advances; past a small bound the reducer re-arms instead of
     * leaving the screen stuck.
     */
    val awaitingSnapshots: Int = 0,
) {
    val isStarting: Boolean get() = phase == "STARTING"
    val isAwaitingConfirmation: Boolean get() = phase == "AWAITING_CONFIRMATION"
    val isConfirming: Boolean get() = phase == "CONFIRMING"
    val isComplete: Boolean get() = phase == "COMPLETE"
    val isFailed: Boolean get() = phase == "FAILED"

    /** The typed guidance for a §6 pairing failure, when the code is known. */
    val guidance: ErrorGuidance? get() = ErrorGuidanceBook.forCode(errorCode)

    /**
     * `201`/`206` end the exchange: the invitation cannot be reused and the
     * Server must present a fresh one. The UI blocks here instead of falling
     * back to the general error card so the recovery action stays visible.
     */
    val isBlockingInvitationFailure: Boolean
        get() = isFailed && (errorCode == 201 || errorCode == 206)

    /** QR recovery is always an explicit user action; the camera never reopens alone. */
    val requiresExplicitRescan: Boolean
        get() = isBlockingInvitationFailure && mode == "qr"

    /**
     * The SAS is a human comparison, never an automatic one: Confirm is only
     * valid when the code the user read on the Server normalizes to the code
     * this Client derived. QR mode has no manual Confirm at all — the scan is
     * the Client's explicit action and the reducer auto-dispatches the one
     * `PairingConfirm` (pairing.md §6.2, ticket 30 D5).
     */
    val sasMatches: Boolean
        get() {
            val expected = sas?.let(SasCode::normalize) ?: return false
            val entered = SasCode.normalize(sasInput) ?: return false
            return expected == entered
        }

    val canConfirm: Boolean
        get() = mode == "sas" && sasMatches
}

/** The error presentation for `Ended(Surfaced)`; copy resolves by [code]. */
data class SessionError(
    val code: Int?,
    val guidance: ErrorGuidance?,
) {
    val requiresRepairing: Boolean get() = guidance?.requiresRepairing == true
}

/** Complete UI state; the reducer is pure and Android-free. */
data class AppState(
    val screen: AppScreen = AppScreen.HOME,
    val address: String = "",
    val addressError: AddressError? = null,
    val rememberedAddress: String? = null,
    val storedServers: List<StoredServer> = emptyList(),
    /** The Service-owned single outcome; the UI never derives one. */
    val outcome: SessionOutcome? = null,
    /** Local button-disable while an asynchronous Stop is in flight. */
    val stopRequested: Boolean = false,
    val pairing: PairingUi? = null,
    /** Why PAIRING is open: the general page or a stored Server's re-pair. */
    val pairingIntent: PairingIntent = PairingIntent.First,
    val scanActive: Boolean = false,
    val scanNotice: ScanNotice? = null,
    /**
     * Config-change hint only: the SAS input typed before the Activity was
     * recreated. The Service replay owns the pairing snapshot, so this hint is
     * applied to the first snapshot and then consumed. It is never used when a
     * PairingUi already exists, and it is dropped whenever pairing is cleared.
     */
    val restoredSasInput: String = "",
    /**
     * `Forget this Server` confirmation target. Set only by an explicit user
     * request; the dialog is the confirmation, never a silent delete.
     */
    val pendingForgetPin: String? = null,
) {
    val hasStoredPairing: Boolean get() = storedServers.isNotEmpty()

    /**
     * The surfaced-close presentation, derived from the outcome only. [code]
     * is the close code the card prints (null when native recorded none);
     * [guidance] is the typed registry entry, null when the code has none (the
     * card still renders the documented generic copy).
     */
    val sessionError: SessionError?
        get() = (outcome as? SessionOutcome.Ended)
            ?.reason
            ?.let { it as? EndReason.Surfaced }
            ?.let { SessionError(it.code, ErrorGuidanceBook.forCode(it.code)) }

    /** Prefer the credential matching the typed address, else the first. */
    fun selectedPin(): String? {
        val typed = address.trim()
        return storedServers.firstOrNull { it.address == typed }?.serverSpkiSha256Hex
            ?: storedServers.firstOrNull()?.serverSpkiSha256Hex
    }
}

sealed interface UiEvent {
    data class AddressChanged(val text: String) : UiEvent
    data object ConnectRequested : UiEvent
    data object AutoConnectStarted : UiEvent
    data object StartSasPairing : UiEvent
    data class StartQrPairing(val uri: String) : UiEvent
    data class PairingSnapshotChanged(val snapshot: PairingSnapshot?) : UiEvent
    data class SasInputChanged(val text: String) : UiEvent
    data object ConfirmPairing : UiEvent
    data object CancelPairing : UiEvent
    /** Android system Back. Pure navigation; the Activity decides HOME finish. */
    data object BackPressed : UiEvent
    data class SessionOutcomeChanged(val outcome: SessionOutcome?) : UiEvent
    /**
     * First-bind replay of the Service-owned session/pairing state, followed by
     * reconciliation of the restored screen (ticket 06 §4). The pairing
     * snapshot itself was already replayed through the pairing listener; this
     * event only carries whether one exists, so it never rebuilds PairingUi.
     */
    data class RestorationReconciled(
        val outcome: SessionOutcome?,
        val pairingSnapshotPresent: Boolean,
    ) : UiEvent
    data object StopRequested : UiEvent
    data class StartupLoaded(
        val rememberedAddress: String?,
        val credentials: List<CredentialRecord>,
    ) : UiEvent
    data class CredentialStored(val record: CredentialRecord) : UiEvent
    /**
     * One-shot Service handoff after a successful pairing: the authenticated
     * session already exists (the Service started it), so the UI only
     * navigates and refreshes the Server list (ticket 17).
     */
    data class PairingCompleted(val record: CredentialRecord) : UiEvent
    data object ShowHome : UiEvent
    data object ShowPairing : UiEvent
    /**
     * Open the PAIRING screen in the dedicated re-pairing context for one
     * stored Server (ticket 04). The address is prefilled from the record and
     * the failure cause (when one is showing) stays visible.
     */
    data class RePairRequested(val serverSpkiSha256Hex: String) : UiEvent
    data object ShowAbout : UiEvent
    data object ShowSettings : UiEvent
    data object RequestScan : UiEvent
    data object ScanPermissionDenied : UiEvent
    data object ScanClosed : UiEvent
    data object PairingUriRejected : UiEvent
    data object Retry : UiEvent
    /** Open the `Forget this Server?` confirmation; nothing is deleted yet. */
    data class ForgetServerRequested(val serverSpkiSha256Hex: String) : UiEvent
    /** The user confirmed the open Forget dialog. */
    data object ForgetServerConfirmed : UiEvent
    /** The user cancelled the open Forget dialog. */
    data object ForgetServerDismissed : UiEvent
}

/** Session side effects the Activity executes after reducing an event. */
sealed interface AppCommand {
    data object None : AppCommand
    data class BeginQr(val uri: String) : AppCommand
    data class BeginSas(val address: String) : AppCommand
    data object ConfirmPairing : AppCommand
    data object CancelPairing : AppCommand
    data class ConnectStored(val serverSpkiSha256Hex: String?) : AppCommand
    data object Stop : AppCommand
    /** Delete the credential and Client identity key, then stop the session. */
    data class ForgetServer(val serverSpkiSha256Hex: String) : AppCommand
}

data class Reduction(val state: AppState, val command: AppCommand = AppCommand.None)

/**
 * Auto-connect is a launch-time policy, never a boot policy: it only fires on
 * a fresh Activity launch, only with a saved credential, and only when no
 * Client Session owns the Service. The outcome is the only session input.
 */
object AutoConnectPolicy {
    fun shouldAutoConnect(
        savedStatePresent: Boolean,
        hasCredential: Boolean,
        outcome: SessionOutcome?,
        /**
         * Read from the *persisted* credential, not the in-memory outcome: a
         * cold start must not reconnect a device the Server revoked.
         */
        needsRepairing: Boolean = false,
    ): Boolean = !savedStatePresent && hasCredential && !needsRepairing &&
        isEligibleOutcome(outcome)

    /** `null` (no session) and a terminal Ended are eligible; Live/Reconnecting are not. */
    fun isEligibleOutcome(outcome: SessionOutcome?): Boolean = when (outcome) {
        null -> true
        is SessionOutcome.Ended -> {
            val reason = outcome.reason
            !(reason is EndReason.Surfaced &&
                ErrorGuidanceBook.forCode(reason.code)?.requiresRepairing == true)
        }
        is SessionOutcome.Live,
        is SessionOutcome.Reconnecting -> false
    }
}

object AppReducer {
    /**
     * ~1 s at the native pairing watcher's 25 ms cadence. A queued confirm
     * normally advances the phase to `CONFIRMING` long before this bound.
     */
    internal const val QR_AUTO_CONFIRM_RETRY_SNAPSHOTS = 40

    fun reduce(state: AppState, event: UiEvent): Reduction = when (event) {
        is UiEvent.AddressChanged -> Reduction(
            state.copy(
                address = event.text,
                addressError = validateAddressInput(event.text),
            )
        )

        UiEvent.ConnectRequested -> connect(state)

        UiEvent.AutoConnectStarted -> {
            // Gate on the record connect() will actually target: the persisted
            // needs-repairing flag wins over the outcome, so a revoked
            // credential is never reconnected after a cold start even when
            // another saved Server is clean.
            val selected = state.storedServers.firstOrNull {
                it.serverSpkiSha256Hex == state.selectedPin()
            }
            val canStart = state.screen == AppScreen.HOME &&
                selected != null &&
                !selected.needsRepairing &&
                AutoConnectPolicy.isEligibleOutcome(state.outcome)
            if (canStart) connect(state) else Reduction(state)
        }

        UiEvent.StartSasPairing -> startSas(state)

        is UiEvent.StartQrPairing -> startQr(state, event.uri)

        is UiEvent.PairingSnapshotChanged -> onPairingSnapshot(state, event.snapshot)

        is UiEvent.SasInputChanged -> Reduction(
            state.copy(pairing = state.pairing?.copy(sasInput = event.text))
        )

        UiEvent.ConfirmPairing -> {
            val pairing = state.pairing
            if (pairing != null && pairing.isAwaitingConfirmation && pairing.canConfirm) {
                Reduction(state, AppCommand.ConfirmPairing)
            } else {
                val next = if (pairing != null && pairing.isAwaitingConfirmation && !pairing.canConfirm) {
                    pairing.copy(notice = PairingNotice.SAS_MISMATCH)
                } else {
                    pairing
                }
                Reduction(state.copy(pairing = next))
            }
        }

        UiEvent.CancelPairing -> cancelPairing(state)

        // Back is navigation only (ticket 06 §5). Pairing cancels the exchange;
        // every other non-Home screen returns Home without touching the
        // session/channel. HOME is left to the system, which finishes the
        // Activity; only a real finish sends ACTION_STOP.
        UiEvent.BackPressed -> when (state.screen) {
            AppScreen.PAIRING -> cancelPairing(state)
            AppScreen.SESSION,
            AppScreen.ABOUT,
            AppScreen.SETTINGS -> Reduction(
                state.copy(
                    screen = AppScreen.HOME,
                    scanActive = false,
                    scanNotice = null,
                    restoredSasInput = "",
                )
            )

            AppScreen.HOME -> Reduction(state)
        }

        // The restored screen is only a hint. The first bind replays the
        // Service-owned outcome/pairing state, then this event reconciles the
        // hint; the reducer stays the single owner of the screen choice.
        is UiEvent.RestorationReconciled -> {
            val screen = NavigationRestoration.reconcile(
                restored = state.screen,
                outcome = event.outcome,
                pairingSnapshotPresent = event.pairingSnapshotPresent,
            )
            val leftPairing = screen != AppScreen.PAIRING
            val cleared = screen == AppScreen.HOME
            Reduction(
                state.copy(
                    screen = screen,
                    outcome = event.outcome,
                    storedServers = state.projectedRepairState(event.outcome),
                    stopRequested = if (event.outcome is SessionOutcome.Ended) {
                        false
                    } else {
                        state.stopRequested
                    },
                    pairing = if (cleared) null else state.pairing,
                    scanActive = if (cleared) false else state.scanActive,
                    scanNotice = if (cleared) null else state.scanNotice,
                    restoredSasInput = if (leftPairing) "" else state.restoredSasInput,
                )
            )
        }

        // The outcome never chooses the screen. Live/Reconnecting stay wherever
        // the user is; Ended stays in place with different content (ticket 06).
        // PAIRING is therefore never pulled away by a session outcome: pairing
        // terminal state is owned by PairingStateRelay.
        is UiEvent.SessionOutcomeChanged -> Reduction(
            state.copy(
                outcome = event.outcome,
                // The Service persists the flag; this keeps the visible card
                // in step until the next credential load.
                storedServers = state.projectedRepairState(event.outcome),
                stopRequested = if (event.outcome is SessionOutcome.Ended) {
                    false
                } else {
                    state.stopRequested
                },
            )
        )

        UiEvent.StopRequested -> Reduction(
            state.copy(stopRequested = true),
            AppCommand.Stop,
        )

        is UiEvent.StartupLoaded -> {
            val servers = event.credentials.map(StoredServer::from)
            val address = state.address.ifBlank {
                event.rememberedAddress
                    ?: servers.firstOrNull()?.address
                    ?: ""
            }
            Reduction(
                state.copy(
                    address = address,
                    rememberedAddress = event.rememberedAddress,
                    storedServers = servers,
                    addressError = validateAddressInput(address),
                )
            )
        }

        is UiEvent.CredentialStored -> {
            // List-only observation: the Service owns the handoff and the only
            // authenticated connection (ticket 17). Never emit ConnectStored.
            val server = StoredServer.from(event.record)
            val servers = state.storedServers
                .filterNot { it.serverSpkiSha256Hex == server.serverSpkiSha256Hex } + server
            Reduction(state.copy(storedServers = servers))
        }

        is UiEvent.PairingCompleted -> {
            val server = StoredServer.from(event.record)
            // A successful pairing replaces every record for the same address
            // (ADR-0006 §3): the new record wins and the stale one on that
            // host:port disappears from the list too.
            val servers = state.storedServers
                .filterNot {
                    it.serverSpkiSha256Hex == server.serverSpkiSha256Hex ||
                        it.address == server.address
                } + server
            Reduction(
                state.copy(
                    screen = AppScreen.SESSION,
                    storedServers = servers,
                    pairingIntent = PairingIntent.First,
                    stopRequested = false,
                    pairing = null,
                    scanActive = false,
                    scanNotice = null,
                    restoredSasInput = "",
                ),
            )
        }

        UiEvent.ShowHome -> Reduction(
            state.copy(
                screen = AppScreen.HOME,
                pairingIntent = PairingIntent.First,
                scanActive = false,
                scanNotice = null,
                restoredSasInput = "",
            )
        )

        // A repair entry point (Home's Needs re-pairing card or the 204 error
        // card) opens the same PAIRING screen but keeps the stored pin, the
        // prefilled address and the visible failure cause (ticket 04).
        is UiEvent.RePairRequested -> rePair(state, event.serverSpkiSha256Hex)

        UiEvent.ShowPairing -> Reduction(
            state.copy(
                screen = AppScreen.PAIRING,
                pairing = null,
                scanActive = false,
                scanNotice = null,
                restoredSasInput = "",
            )
        )

        UiEvent.ShowAbout -> Reduction(state.copy(screen = AppScreen.ABOUT, scanActive = false))

        // SETTINGS is a user-driven destination; no session outcome or pairing
        // terminal ever navigates to or away from it (ticket 06 §2).
        UiEvent.ShowSettings -> Reduction(
            state.copy(screen = AppScreen.SETTINGS, scanActive = false)
        )

        UiEvent.RequestScan -> Reduction(
            state.copy(
                screen = AppScreen.PAIRING,
                scanActive = true,
                scanNotice = null,
            )
        )

        UiEvent.ScanPermissionDenied -> Reduction(
            state.copy(
                scanActive = false,
                scanNotice = ScanNotice.CAMERA_PERMISSION_DENIED,
            )
        )

        UiEvent.ScanClosed -> Reduction(state.copy(scanActive = false))

        is UiEvent.PairingUriRejected -> Reduction(
            state.copy(
                screen = AppScreen.PAIRING,
                scanActive = false,
                pairing = PairingUi(
                    mode = "qr",
                    phase = "FAILED",
                    notice = PairingNotice.INVALID_PAIRING_LINK,
                ),
                restoredSasInput = "",
            )
        )

        UiEvent.Retry -> retry(state)

        is UiEvent.ForgetServerRequested -> {
            val known = state.storedServers.any {
                it.serverSpkiSha256Hex == event.serverSpkiSha256Hex
            }
            Reduction(
                if (known) {
                    state.copy(pendingForgetPin = event.serverSpkiSha256Hex)
                } else {
                    state
                }
            )
        }

        UiEvent.ForgetServerConfirmed -> forgetServer(state)

        UiEvent.ForgetServerDismissed -> Reduction(state.copy(pendingForgetPin = null))
    }

    private fun cancelPairing(state: AppState): Reduction = Reduction(
        state.copy(
            screen = AppScreen.HOME,
            // Cancelling abandons the exchange and tears its pairing session
            // down (StopReason.PairingCancel), so no Client Session remains.
            // Keeping the superseded session's outcome would make a later
            // Connect merely navigate instead of requesting a new session.
            outcome = null,
            stopRequested = false,
            pairing = null,
            pairingIntent = PairingIntent.First,
            scanActive = false,
            scanNotice = null,
            restoredSasInput = "",
        ),
        AppCommand.CancelPairing,
    )

    private fun connect(state: AppState): Reduction {
        val outcome = state.outcome
        // A live or reconnecting session already owns the Service: navigate,
        // but never issue a second ConnectStored that would tear it down.
        if (outcome is SessionOutcome.Live || outcome is SessionOutcome.Reconnecting) {
            return Reduction(
                state.copy(
                    screen = AppScreen.SESSION,
                    stopRequested = false,
                    pairing = null,
                    scanActive = false,
                    scanNotice = null,
                )
            )
        }
        // A revoked device must be re-paired, never reconnected (ticket 04).
        val target = state.storedServers.firstOrNull {
            it.serverSpkiSha256Hex == state.selectedPin()
        }
        if (target?.needsRepairing == true) {
            return Reduction(state)
        }
        if (!state.hasStoredPairing) {
            return Reduction(
                state.copy(
                    screen = AppScreen.HOME,
                    addressError = AddressError.NO_SAVED_PAIRING,
                )
            )
        }
        return Reduction(
            state.copy(
                screen = AppScreen.SESSION,
                stopRequested = false,
                pairing = null,
                scanActive = false,
                scanNotice = null,
            ),
            AppCommand.ConnectStored(state.selectedPin()),
        )
    }

    private fun startSas(state: AppState): Reduction {
        val address = state.address.trim()
        if (address.isEmpty()) {
            return Reduction(state.copy(addressError = AddressError.REQUIRED))
        }
        val error = validateAddressInput(address)
        if (error != null) return Reduction(state.copy(addressError = error))
        return Reduction(
            state.copy(
                address = address,
                addressError = null,
                screen = AppScreen.PAIRING,
                // BeginSas supersedes any live Client Session in the Service,
                // so the UI drops the superseded outcome immediately. Otherwise
                // leaving PAIRING by Home keeps a stale Live and Connect only
                // navigates instead of starting a session.
                outcome = null,
                stopRequested = false,
                pairing = PairingUi(mode = "sas", phase = "STARTING"),
                scanActive = false,
                scanNotice = null,
                restoredSasInput = "",
            ),
            AppCommand.BeginSas(address),
        )
    }

    private fun startQr(state: AppState, uri: String): Reduction {
        val payload = try {
            PairingPayload.parse(uri.trim())
        } catch (_: Exception) {
            return Reduction(
                state.copy(
                    screen = AppScreen.PAIRING,
                    scanActive = false,
                    pairing = PairingUi(
                        mode = "qr",
                        phase = "FAILED",
                        notice = PairingNotice.INVALID_PAIRING_LINK,
                    ),
                    restoredSasInput = "",
                )
            )
        }
        return Reduction(
            state.copy(
                address = payload.address,
                addressError = null,
                screen = AppScreen.PAIRING,
                // BeginQr supersedes any live Client Session in the Service.
                outcome = null,
                stopRequested = false,
                pairing = PairingUi(mode = "qr", phase = "STARTING"),
                scanActive = false,
                scanNotice = null,
                restoredSasInput = "",
            ),
            AppCommand.BeginQr(uri.trim()),
        )
    }

    private fun onPairingSnapshot(state: AppState, snapshot: PairingSnapshot?): Reduction {
        if (snapshot == null) {
            // The Service clears its pairing relay on connect/cancel; keep the
            // terminal screens whose copy is still the user's only guidance.
            val current = state.pairing
            return if (current?.isComplete == true || current?.isBlockingInvitationFailure == true) {
                Reduction(state)
            } else {
                Reduction(
                    state.copy(pairing = null, scanActive = false, restoredSasInput = "")
                )
            }
        }
        val sas = snapshot.sas?.let(::formatSas)
        val previous = state.pairing
        val awaitingSnapshots = when {
            !snapshot.isAwaitingConfirmation -> 0
            previous?.isAwaitingConfirmation == true -> previous.awaitingSnapshots + 1
            else -> 0
        }
        val pairing = PairingUi(
            mode = snapshot.mode,
            phase = snapshot.phase,
            sas = sas,
            fingerprint = snapshot.fingerprint,
            sasInput = previous?.sasInput ?: state.restoredSasInput,
            errorCode = snapshot.errorCode,
            notice = null,
            // A fresh exchange always has a fresh PairingUi (STARTING), so
            // carrying the flag only suppresses duplicate snapshots.
            autoConfirmSent = previous?.autoConfirmSent ?: false,
            awaitingSnapshots = awaitingSnapshots,
        )
        // QR mode's Client action is the scan (pairing.md §6.2, ticket 30 D5):
        // once the native side has verified the binding and published
        // AWAITING_CONFIRMATION, send PairingConfirm exactly once. SAS mode
        // keeps its code entry plus an explicit Confirm (ticket 30 D1).
        val autoConfirm = pairing.mode == "qr" &&
            pairing.isAwaitingConfirmation &&
            (!pairing.autoConfirmSent ||
                awaitingSnapshots >= QR_AUTO_CONFIRM_RETRY_SNAPSHOTS)
        return if (autoConfirm) {
            Reduction(
                state.copy(
                    pairing = pairing.copy(
                        autoConfirmSent = true,
                        awaitingSnapshots = 0,
                    ),
                    scanActive = false,
                    restoredSasInput = "",
                ),
                AppCommand.ConfirmPairing,
            )
        } else {
            Reduction(
                state.copy(pairing = pairing, scanActive = false, restoredSasInput = "")
            )
        }
    }

    /**
     * Remove the confirmed Server from the UI and return to the unpaired
     * Home state. The Service performs the durable delete (credential + key)
     * and stops the session; the Activity clears a matching remembered
     * address. The address field is cleared only when it belonged to this
     * record, so another Server's address is never thrown away.
     */
    private fun forgetServer(state: AppState): Reduction {
        val pin = state.pendingForgetPin ?: return Reduction(state)
        val record = state.storedServers.firstOrNull { it.serverSpkiSha256Hex == pin }
            ?: return Reduction(state.copy(pendingForgetPin = null))
        val addressMatches = state.address.trim() == record.address
        return Reduction(
            state.copy(
                screen = AppScreen.HOME,
                storedServers = state.storedServers.filterNot {
                    it.serverSpkiSha256Hex == pin
                },
                address = if (addressMatches) "" else state.address,
                addressError = if (addressMatches) null else state.addressError,
                rememberedAddress = if (state.rememberedAddress == record.address) {
                    null
                } else {
                    state.rememberedAddress
                },
                outcome = null,
                stopRequested = false,
                pairing = null,
                scanActive = false,
                scanNotice = null,
                restoredSasInput = "",
                pendingForgetPin = null,
            ),
            AppCommand.ForgetServer(pin),
        )
    }

    private fun retry(state: AppState): Reduction {
        val error = state.sessionError
        if (error?.requiresRepairing == true) {
            // The 204/205/202 card's action is the same dedicated re-pair
            // context as Home's Needs re-pairing entry. Keep the outcome so
            // the failure cause stays visible on the PAIRING screen.
            val pin = state.selectedPin()
            return if (pin != null) {
                rePair(state, pin)
            } else {
                Reduction(
                    state.copy(
                        screen = AppScreen.PAIRING,
                        pairingIntent = PairingIntent.First,
                        pairing = null,
                        restoredSasInput = "",
                    )
                )
            }
        }
        if (state.hasStoredPairing) {
            return connect(state)
        }
        return Reduction(state.copy(screen = AppScreen.HOME))
    }

    /** Open PAIRING for one stored Server, prefilling its saved address. */
    private fun rePair(state: AppState, serverSpkiSha256Hex: String): Reduction {
        val server = state.storedServers
            .firstOrNull { it.serverSpkiSha256Hex == serverSpkiSha256Hex }
            ?: return Reduction(state)
        val address = server.address
        return Reduction(
            state.copy(
                screen = AppScreen.PAIRING,
                address = address,
                addressError = validateAddressInput(address),
                pairingIntent = PairingIntent.RePair(serverSpkiSha256Hex),
                pairing = null,
                scanActive = false,
                scanNotice = null,
                restoredSasInput = "",
            )
        )
    }

    private fun formatSas(raw: String): String {
        val code = SasCode.normalize(raw) ?: return raw
        return SasCode.format(code)
    }
}

/**
 * Project a repair-relevant outcome onto the selected credential for display.
 * The durable owner is the Service; this only mirrors it in the current UI
 * lifetime. A saved Server not involved in the session is untouched.
 */
private fun AppState.projectedRepairState(outcome: SessionOutcome?): List<StoredServer> {
    val intent: Boolean? = when (outcome) {
        is SessionOutcome.Live -> if (
            outcome.phase == LivePhase.READY || outcome.phase == LivePhase.PLAYING
        ) {
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
        null -> null
    }
    val pin = selectedPin()
    if (intent == null || pin == null) return storedServers
    return storedServers.map {
        if (it.serverSpkiSha256Hex == pin) it.copy(needsRepairing = intent) else it
    }
}

/** Reuses the normative `addr` rules (pairing.md §4.2); returns typed state. */
internal fun validateAddressInput(text: String): AddressError? {
    val value = text.trim()
    if (value.isEmpty()) return null
    return try {
        PairingPayload.validateAddress(value)
        null
    } catch (e: AddressValidationException) {
        e.reason
    } catch (_: Exception) {
        AddressError.INVALID_ADDRESS_HINT
    }
}
