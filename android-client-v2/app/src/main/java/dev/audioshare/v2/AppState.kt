package dev.audioshare.v2

/** The four beta screens (beta-spec "Android Client: minimal Compose UI"). */
enum class AppScreen { HOME, PAIRING, SESSION, ABOUT }

/** User-visible session state; [NONE] means no session has been requested. */
enum class SessionPhase { NONE, CONNECTING, AUTHENTICATING, READY, PLAYING, CLOSED }

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
) {
    val isStarting: Boolean get() = phase == "STARTING"
    val isAwaitingConfirmation: Boolean get() = phase == "AWAITING_CONFIRMATION"
    val isComplete: Boolean get() = phase == "COMPLETE"
    val isFailed: Boolean get() = phase == "FAILED"

    /**
     * The SAS is a human comparison, never an automatic one: in SAS mode
     * Confirm is only valid when the code the user read on the Server
     * normalizes to the code this Client derived. QR mode is already bound to
     * the Server identity by the payload pin (pairing.md §6.1), so the QR
     * path stays one action and Confirm does not require typing the code.
     */
    val sasMatches: Boolean
        get() {
            val expected = sas?.let(SasCode::normalize) ?: return false
            val entered = SasCode.normalize(sasInput) ?: return false
            return expected == entered
        }

    val canConfirm: Boolean
        get() = mode == "qr" || sasMatches
}

/** Complete UI state; the reducer is pure and Android-free. */
data class AppState(
    val screen: AppScreen = AppScreen.HOME,
    val address: String = "",
    val addressError: String? = null,
    val rememberedAddress: String? = null,
    val storedServers: List<StoredServer> = emptyList(),
    val sessionPhase: SessionPhase = SessionPhase.NONE,
    val connectRequested: Boolean = false,
    val stopping: Boolean = false,
    val pairing: PairingUi? = null,
    val error: ErrorGuidance? = null,
    val scanActive: Boolean = false,
    val scanMessage: String? = null,
) {
    val hasStoredPairing: Boolean get() = storedServers.isNotEmpty()

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
    data class SessionStateChanged(val nativeState: String) : UiEvent
    data class SessionError(val guidance: ErrorGuidance?) : UiEvent
    data object StopRequested : UiEvent
    data class StartupLoaded(
        val rememberedAddress: String?,
        val credentials: List<CredentialRecord>,
    ) : UiEvent
    data class CredentialStored(val record: CredentialRecord) : UiEvent
    data object ShowHome : UiEvent
    data object ShowPairing : UiEvent
    data object ShowAbout : UiEvent
    data object RequestScan : UiEvent
    data object ScanPermissionDenied : UiEvent
    data object ScanClosed : UiEvent
    data class PairingUriRejected(val message: String) : UiEvent
    data object Retry : UiEvent
    data object DismissError : UiEvent
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
 * a fresh Activity launch, only with a saved credential, and only when the
 * Service is not already carrying a session (Activity recreation must not
 * start a second connection).
 */
object AutoConnectPolicy {
    fun shouldAutoConnect(
        savedStatePresent: Boolean,
        hasCredential: Boolean,
        serviceState: String,
    ): Boolean {
        if (savedStatePresent || !hasCredential) return false
        return serviceState.isEmpty() || serviceState == "IDLE" || serviceState == "CLOSED"
    }
}

object AppReducer {
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
                state.sessionPhase in setOf(SessionPhase.NONE, SessionPhase.CLOSED) &&
                state.hasStoredPairing
            if (canStart) connect(state) else Reduction(state)
        }

        UiEvent.StartSasPairing -> startSas(state)

        is UiEvent.StartQrPairing -> startQr(state, event.uri)

        is UiEvent.PairingSnapshotChanged -> Reduction(onPairingSnapshot(state, event.snapshot))

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
                connectRequested = false,
                stopping = false,
                scanActive = false,
                scanMessage = null,
            ),
            AppCommand.CancelPairing,
        )

        is UiEvent.SessionStateChanged -> Reduction(onNativeState(state, event.nativeState))

        is UiEvent.SessionError -> Reduction(onSessionError(state, event.guidance))

        UiEvent.StopRequested -> Reduction(
            // An explicit Stop is not a failure: dismiss any banner left over
            // from an earlier transport blip before the CLOSED replay lands.
            state.copy(stopping = true, error = null),
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
            val server = StoredServer.from(event.record)
            val servers = state.storedServers
                .filterNot { it.serverSpkiSha256Hex == server.serverSpkiSha256Hex } + server
            Reduction(
                state.copy(
                    screen = AppScreen.SESSION,
                    storedServers = servers,
                    sessionPhase = SessionPhase.CONNECTING,
                    connectRequested = true,
                    stopping = false,
                    pairing = null,
                    error = null,
                ),
                AppCommand.ConnectStored(server.serverSpkiSha256Hex),
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
                error = null,
            )
        )

        UiEvent.ShowAbout -> Reduction(state.copy(screen = AppScreen.ABOUT, scanActive = false))

        UiEvent.RequestScan -> Reduction(
            state.copy(
                screen = AppScreen.PAIRING,
                scanActive = true,
                scanMessage = null,
                error = null,
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

        UiEvent.DismissError -> Reduction(state.copy(error = null))
    }

    private fun connect(state: AppState): Reduction {
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
                sessionPhase = SessionPhase.CONNECTING,
                connectRequested = true,
                stopping = false,
                pairing = null,
                error = null,
                scanActive = false,
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
                connectRequested = false,
                stopping = false,
                error = null,
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
                connectRequested = false,
                stopping = false,
                error = null,
                scanActive = false,
                scanMessage = null,
            ),
            AppCommand.BeginQr(uri.trim()),
        )
    }

    private fun onPairingSnapshot(state: AppState, snapshot: PairingSnapshot?): AppState {
        if (snapshot == null) {
            // The Service clears its pairing relay on connect/cancel; the
            // screen stays available for another attempt.
            return if (state.pairing?.isComplete == true) state
            else state.copy(pairing = null, scanActive = false)
        }
        val sas = snapshot.sas?.let(::formatSas)
        val guidance = snapshot.errorCode?.let(ErrorGuidanceBook::forCode)
        val message = if (snapshot.isFailed) {
            guidance?.title ?: "Pairing failed. Start a new pairing on the Server and try again."
        } else {
            null
        }
        val pairing = PairingUi(
            mode = snapshot.mode,
            phase = snapshot.phase,
            sas = sas,
            fingerprint = snapshot.fingerprint,
            sasInput = state.pairing?.sasInput.orEmpty(),
            errorCode = snapshot.errorCode,
            message = message,
        )
        return state.copy(pairing = pairing, scanActive = false)
    }

    private fun onNativeState(state: AppState, nativeState: String): AppState {
        val phase = mapSessionPhase(nativeState, state.connectRequested)
        val closed = phase == SessionPhase.CLOSED
        val live = phase == SessionPhase.READY || phase == SessionPhase.PLAYING
        val wasStopping = state.stopping
        val screen = when {
            state.screen == AppScreen.ABOUT -> AppScreen.ABOUT
            state.screen == AppScreen.PAIRING -> AppScreen.PAIRING
            closed -> AppScreen.HOME
            state.connectRequested && state.screen == AppScreen.HOME -> AppScreen.SESSION
            else -> state.screen
        }
        return state.copy(
            screen = screen,
            sessionPhase = phase,
            connectRequested = if (closed) false else state.connectRequested,
            stopping = if (closed) false else state.stopping,
            // A recovered session makes an earlier "Connection lost" card
            // stale; a user-requested close is never an error.
            error = if (live || (closed && wasStopping)) null else state.error,
        )
    }

    private fun onSessionError(state: AppState, guidance: ErrorGuidance?): AppState {
        if (guidance == null) return state
        // A close that completes a user Stop is not an error, even if the
        // Service still carries the close code from an earlier interruption.
        if (state.stopping) return state
        return state.copy(
            screen = if (state.screen == AppScreen.ABOUT) AppScreen.ABOUT else AppScreen.HOME,
            error = guidance,
            sessionPhase = SessionPhase.CLOSED,
            connectRequested = false,
            stopping = false,
            pairing = null,
            scanActive = false,
        )
    }

    private fun retry(state: AppState): Reduction {
        val error = state.error
        if (error?.requiresRepairing == true) {
            return Reduction(
                state.copy(
                    screen = AppScreen.PAIRING,
                    pairing = null,
                    error = null,
                    sessionPhase = SessionPhase.CLOSED,
                )
            )
        }
        if (state.hasStoredPairing) {
            return connect(state.copy(error = null))
        }
        return Reduction(state.copy(error = null, screen = AppScreen.HOME))
    }

    private fun formatSas(raw: String): String {
        val code = SasCode.normalize(raw) ?: return raw
        return SasCode.format(code)
    }
}

/** IDLE is the native pre-handshake state; a requested connect is CONNECTING. */
internal fun mapSessionPhase(nativeState: String, connectRequested: Boolean): SessionPhase =
    when (nativeState) {
        "CONNECTING" -> SessionPhase.CONNECTING
        "AUTHENTICATING", "NEGOTIATING" -> SessionPhase.AUTHENTICATING
        "READY" -> SessionPhase.READY
        "PLAYING" -> SessionPhase.PLAYING
        "CLOSED", "CLOSING" -> SessionPhase.CLOSED
        "IDLE" -> if (connectRequested) SessionPhase.CONNECTING else SessionPhase.NONE
        else -> if (connectRequested) SessionPhase.CONNECTING else SessionPhase.NONE
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
