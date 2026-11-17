package dev.audioshare.v2

/** The four beta screens (beta-spec "Android Client: minimal Compose UI"). */
enum class AppScreen { HOME, PAIRING, SESSION, ABOUT }

/** A paired Server as the UI needs it (public credential data only). */
data class StoredServer(
    val serverSpkiSha256Hex: String,
    val displayName: String?,
    val address: String,
) {
    companion object {
        fun from(record: CredentialRecord) = StoredServer(
            serverSpkiSha256Hex = record.serverSpkiSha256Hex,
            displayName = record.displayName,
            address = record.addressHint.ifEmpty { "${record.host}:${record.port}" },
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
    val message: String? = null,
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

/** Complete UI state; the reducer is pure and Android-free. */
data class AppState(
    val screen: AppScreen = AppScreen.HOME,
    val address: String = "",
    val addressError: String? = null,
    val rememberedAddress: String? = null,
    val storedServers: List<StoredServer> = emptyList(),
    /** The Service-owned single outcome; the UI never derives one. */
    val outcome: SessionOutcome? = null,
    /** Local button-disable while an asynchronous Stop is in flight. */
    val stopRequested: Boolean = false,
    val pairing: PairingUi? = null,
    val scanActive: Boolean = false,
    val scanMessage: String? = null,
) {
    val hasStoredPairing: Boolean get() = storedServers.isNotEmpty()

    /** The error card for a surfaced close, derived from the outcome only. */
    val sessionError: ErrorGuidance?
        get() = (outcome as? SessionOutcome.Ended)
            ?.reason
            ?.let { it as? EndReason.Surfaced }
            ?.let { ErrorGuidanceBook.forCode(it.code) }

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
    data class SessionOutcomeChanged(val outcome: SessionOutcome?) : UiEvent
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
    data object ShowAbout : UiEvent
    data object RequestScan : UiEvent
    data object ScanPermissionDenied : UiEvent
    data object ScanClosed : UiEvent
    data class PairingUriRejected(val message: String) : UiEvent
    data object Retry : UiEvent
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
    ): Boolean = !savedStatePresent && hasCredential && isEligibleOutcome(outcome)

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
            val canStart = state.screen == AppScreen.HOME &&
                state.hasStoredPairing &&
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
                    pairing.copy(message = "The code does not match the one shown on the Server.")
                } else {
                    pairing
                }
                Reduction(state.copy(pairing = next))
            }
        }

        UiEvent.CancelPairing -> Reduction(
            state.copy(
                screen = AppScreen.HOME,
                pairing = null,
                scanActive = false,
                scanMessage = null,
            ),
            AppCommand.CancelPairing,
        )

        // The outcome never chooses the screen. Live/Reconnecting stay wherever
        // the user is; Ended stays in place with different content (ticket 06).
        // PAIRING is therefore never pulled away by a session outcome: pairing
        // terminal state is owned by PairingStateRelay.
        is UiEvent.SessionOutcomeChanged -> Reduction(
            state.copy(
                outcome = event.outcome,
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
            val servers = state.storedServers
                .filterNot { it.serverSpkiSha256Hex == server.serverSpkiSha256Hex } + server
            Reduction(
                state.copy(
                    screen = AppScreen.SESSION,
                    storedServers = servers,
                    stopRequested = false,
                    pairing = null,
                    scanActive = false,
                    scanMessage = null,
                ),
            )
        }

        UiEvent.ShowHome -> Reduction(
            state.copy(screen = AppScreen.HOME, scanActive = false, scanMessage = null)
        )

        UiEvent.ShowPairing -> Reduction(
            state.copy(
                screen = AppScreen.PAIRING,
                pairing = null,
                scanActive = false,
                scanMessage = null,
            )
        )

        UiEvent.ShowAbout -> Reduction(state.copy(screen = AppScreen.ABOUT, scanActive = false))

        UiEvent.RequestScan -> Reduction(
            state.copy(
                screen = AppScreen.PAIRING,
                scanActive = true,
                scanMessage = null,
            )
        )

        UiEvent.ScanPermissionDenied -> Reduction(
            state.copy(
                scanActive = false,
                scanMessage = "Camera permission was denied. Paste the pairing link instead.",
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
                    message = event.message,
                ),
            )
        )

        UiEvent.Retry -> retry(state)
    }

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
                    scanMessage = null,
                )
            )
        }
        if (!state.hasStoredPairing) {
            return Reduction(
                state.copy(
                    screen = AppScreen.HOME,
                    addressError = "No saved pairing. Pair with the Server first.",
                )
            )
        }
        return Reduction(
            state.copy(
                screen = AppScreen.SESSION,
                stopRequested = false,
                pairing = null,
                scanActive = false,
                scanMessage = null,
            ),
            AppCommand.ConnectStored(state.selectedPin()),
        )
    }

    private fun startSas(state: AppState): Reduction {
        val address = state.address.trim()
        if (address.isEmpty()) {
            return Reduction(state.copy(addressError = "Enter the Server address first."))
        }
        val error = validateAddressInput(address)
        if (error != null) return Reduction(state.copy(addressError = error))
        return Reduction(
            state.copy(
                address = address,
                addressError = null,
                screen = AppScreen.PAIRING,
                pairing = PairingUi(mode = "sas", phase = "STARTING"),
                scanActive = false,
                scanMessage = null,
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
                        message = "This is not a valid Audio Share pairing link.",
                    ),
                )
            )
        }
        return Reduction(
            state.copy(
                address = payload.address,
                addressError = null,
                screen = AppScreen.PAIRING,
                pairing = PairingUi(mode = "qr", phase = "STARTING"),
                scanActive = false,
                scanMessage = null,
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
                Reduction(state.copy(pairing = null, scanActive = false))
            }
        }
        val sas = snapshot.sas?.let(::formatSas)
        val guidance = snapshot.errorCode?.let(ErrorGuidanceBook::forCode)
        val message = if (snapshot.isFailed) {
            guidance?.title ?: "Pairing failed. Start a new pairing on the Server and try again."
        } else {
            null
        }
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
            sasInput = previous?.sasInput.orEmpty(),
            errorCode = snapshot.errorCode,
            message = message,
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
                ),
                AppCommand.ConfirmPairing,
            )
        } else {
            Reduction(state.copy(pairing = pairing, scanActive = false))
        }
    }

    private fun retry(state: AppState): Reduction {
        val error = state.sessionError
        if (error?.requiresRepairing == true) {
            return Reduction(
                state.copy(
                    screen = AppScreen.PAIRING,
                    pairing = null,
                )
            )
        }
        if (state.hasStoredPairing) {
            return connect(state)
        }
        return Reduction(state.copy(screen = AppScreen.HOME))
    }

    private fun formatSas(raw: String): String {
        val code = SasCode.normalize(raw) ?: return raw
        return SasCode.format(code)
    }
}

/** Reuses the normative `addr` rules (pairing.md §4.2). */
internal fun validateAddressInput(text: String): String? {
    val value = text.trim()
    if (value.isEmpty()) return null
    return try {
        PairingPayload.validateAddress(value)
        null
    } catch (e: QrPayloadException) {
        e.message ?: "invalid Server address"
    } catch (_: Exception) {
        "invalid Server address"
    }
}
