package dev.audioshare.v2

import android.os.Bundle

/**
 * The minimal config-change bundle (ticket 06 §4): screen, address, SAS input,
 * and the re-pair context only. Session outcome, pairing snapshot, credentials,
 * errors, and the camera state always replay from the Service, never from disk.
 *
 * `AppState` stays a plain data class; this object is the only place that
 * touches [Bundle].
 */
internal object UiStatePersistence {
    private const val KEY_SCREEN = "dev.audioshare.v2.ui.screen"
    private const val KEY_ADDRESS = "dev.audioshare.v2.ui.address"
    private const val KEY_SAS_INPUT = "dev.audioshare.v2.ui.sasInput"
    private const val KEY_REPAIR_PIN = "dev.audioshare.v2.ui.repairPin"

    fun save(outState: Bundle, state: AppState) {
        outState.putString(KEY_SCREEN, state.screen.name)
        outState.putString(KEY_ADDRESS, state.address)
        outState.putString(KEY_SAS_INPUT, state.pairing?.sasInput.orEmpty())
        // The RePair cause/instruction must survive Activity recreation; an
        // empty pin is the ordinary First pairing page.
        outState.putString(
            KEY_REPAIR_PIN,
            (state.pairingIntent as? PairingIntent.RePair)?.serverSpkiSha256Hex.orEmpty(),
        )
    }

    /** `null` on a cold start (no saved bundle). */
    fun restore(savedInstanceState: Bundle?): AppState? {
        if (savedInstanceState == null) return null
        val screen = savedInstanceState.getString(KEY_SCREEN)
            ?.let { name -> AppScreen.values().firstOrNull { it.name == name } }
            ?: AppScreen.HOME
        val repairPin = savedInstanceState.getString(KEY_REPAIR_PIN).orEmpty()
        return AppState(
            screen = screen,
            address = savedInstanceState.getString(KEY_ADDRESS).orEmpty(),
            restoredSasInput = savedInstanceState.getString(KEY_SAS_INPUT).orEmpty(),
            pairingIntent = if (repairPin.isEmpty()) {
                PairingIntent.First
            } else {
                PairingIntent.RePair(repairPin)
            },
        )
    }
}
