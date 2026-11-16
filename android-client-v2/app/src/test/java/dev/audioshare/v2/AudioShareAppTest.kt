package dev.audioshare.v2

import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import android.content.Context
import android.content.pm.ActivityInfo
import androidx.test.core.app.ApplicationProvider
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.Description
import org.junit.runner.RunWith
import org.junit.runners.model.Statement
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * Seam 3 of ticket 21: key Compose screens under Robolectric. Assertions are
 * on user-visible text/state and emitted events, never pixels. The screen is
 * driven by the same pure reducer the app uses.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class AudioShareAppTest {

    private val compose = createComposeRule()

    private val events = mutableListOf<UiEvent>()

    /**
     * `createComposeRule()` launches a bare ComponentActivity. `ui-test-manifest`
     * is only a debug/instrumentation convention and would leak into the
     * production manifest if it were a release dependency, so register the host
     * activity with Robolectric's package manager for this JVM-only test. This
     * rule must run *before* the compose rule's ActivityScenarioRule.
     */
    private val registerHostActivity = TestRule { base: Statement, _: Description ->
        object : Statement() {
            override fun evaluate() {
                val context = ApplicationProvider.getApplicationContext<Context>()
                val info = ActivityInfo().apply {
                    name = ComponentActivity::class.java.name
                    packageName = context.packageName
                    applicationInfo = context.applicationInfo
                    enabled = true
                    exported = false
                }
                shadowOf(context.packageManager).addOrUpdateActivity(info)
                base.evaluate()
            }
        }
    }

    @get:Rule
    val ruleChain: RuleChain = RuleChain.outerRule(registerHostActivity).around(compose)

    private fun start(initial: AppState) {
        var state by mutableStateOf(initial)
        compose.setContent {
            AudioShareTheme {
                AudioShareApp(
                    state = state,
                    versionName = "0.2.0-beta",
                    versionCode = 2,
                ) { event ->
                    events += event
                    state = AppReducer.reduce(state, event).state
                }
            }
        }
    }

    @Test
    fun home_accepts_a_server_address_and_rejects_a_bare_host() {
        start(AppState())
        compose.onNodeWithTag("address-input").performTextReplacement("10.0.2.2")
        compose.onNodeWithText("missing port").assertIsDisplayed()
        compose.onNodeWithTag("address-input").performTextReplacement("10.0.2.2:65531")
        compose.onNodeWithTag("address-input").assertTextContains("10.0.2.2:65531")
        compose.onNodeWithTag("pair-code").assertIsEnabled()
    }

    @Test
    fun home_offers_pairing_choices_and_opens_the_pairing_screen() {
        start(AppState(address = "10.0.2.2:65531", addressError = null))
        compose.onNodeWithText("Pair with code").assertIsDisplayed()
        compose.onNodeWithText("Scan QR code").assertIsDisplayed()
        compose.onNodeWithTag("pair-code").performClick()
        compose.onNodeWithText("Pair your phone").assertIsDisplayed()
        assertTrue(events.contains(UiEvent.StartSasPairing))
    }

    @Test
    fun terminal_error_shows_guidance_and_retry_opens_repairing_pairing() {
        start(
            AppState(
                error = ErrorGuidanceBook.forCode(204),
                sessionPhase = SessionPhase.CLOSED,
                storedServers = listOf(
                    StoredServer("ab".repeat(32), "Pixel", "10.0.2.2:65531")
                ),
            )
        )
        compose.onNodeWithText("This device was revoked").assertIsDisplayed()
        compose.onNodeWithText("Pair again").performScrollTo().assertIsDisplayed()
        compose.onNodeWithTag("retry").performScrollTo().performClick()
        compose.onNodeWithText("Pair your phone").assertIsDisplayed()
    }

    @Test
    fun session_screen_stops_the_session() {
        start(
            AppState(
                screen = AppScreen.SESSION,
                address = "10.0.2.2:65531",
                storedServers = listOf(
                    StoredServer("ab".repeat(32), "Pixel", "10.0.2.2:65531")
                ),
                sessionPhase = SessionPhase.PLAYING,
            )
        )
        compose.onNodeWithTag("session-state").assertTextContains("PLAYING")
        compose.onNodeWithTag("stop").assertIsEnabled().performClick()
        assertTrue(events.contains(UiEvent.StopRequested))
        compose.onNodeWithTag("stop").assertIsNotEnabled()
    }

    @Test
    fun pairing_screen_requires_the_matching_sas_context_before_confirm() {
        start(
            AppState(
                screen = AppScreen.PAIRING,
                address = "10.0.2.2:65531",
                pairing = PairingUi(
                    mode = "sas",
                    phase = "AWAITING_CONFIRMATION",
                    sas = "385 587",
                    fingerprint = "AAAA1111 BBBB2222",
                ),
            )
        )
        compose.onNodeWithText("385 587").performScrollTo().assertIsDisplayed()
        compose.onNodeWithTag("confirm-pairing").performScrollTo().assertIsNotEnabled()
        compose.onNodeWithTag("pairing-sas-input").performScrollTo().performTextReplacement("385588")
        compose.onNodeWithTag("confirm-pairing").assertIsNotEnabled()
        compose.onNodeWithTag("pairing-sas-input").performTextReplacement("385 587")
        compose.onNodeWithTag("confirm-pairing").performScrollTo().assertIsEnabled().performClick()
        assertTrue(events.contains(UiEvent.ConfirmPairing))
    }

    @Test
    fun qr_pairing_can_be_confirmed_without_typing_a_code() {
        start(
            AppState(
                screen = AppScreen.PAIRING,
                address = "10.0.2.2:65531",
                pairing = PairingUi(
                    mode = "qr",
                    phase = "AWAITING_CONFIRMATION",
                    sas = "123 456",
                    fingerprint = "AAAA1111 BBBB2222",
                ),
            )
        )
        compose.onNodeWithTag("confirm-pairing").performScrollTo().assertIsEnabled().performClick()
        assertTrue(events.contains(UiEvent.ConfirmPairing))
    }

    @Test
    fun about_screen_states_the_beta_version_limits_and_v1_warning() {
        start(AppState(screen = AppScreen.ABOUT))
        compose.onNodeWithTag("about-version").assertTextContains("0.2.0-beta", substring = true)
        compose.onNodeWithText("Known limitations").assertIsDisplayed()
        compose.onNodeWithText(
            "The v1 app sends audio without encryption or authentication. Use v1 only on a " +
                "trusted network. This v2 beta is the encrypted, authenticated client; a v2 " +
                "failure never downgrades to v1.",
        ).assertIsDisplayed()
        assertEquals(AppScreen.ABOUT, AppState(screen = AppScreen.ABOUT).screen)
    }
}
