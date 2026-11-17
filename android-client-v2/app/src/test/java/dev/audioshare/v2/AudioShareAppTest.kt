package dev.audioshare.v2

import android.content.Context
import android.content.pm.ActivityInfo
import androidx.activity.ComponentActivity
import androidx.annotation.StringRes
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
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
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * Seam 3 of ticket 21: key Compose screens under Robolectric. Assertions are
 * on user-visible text/state and emitted events, never pixels. Copy is looked
 * up from resources so the same tests hold for en and zh-CN.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class AudioShareAppTest {

    private val compose = createAndroidComposeRule<ComponentActivity>()

    private val events = mutableListOf<UiEvent>()

    private val context: Context get() = ApplicationProvider.getApplicationContext()

    private fun str(@StringRes id: Int): String = context.getString(id)

    private fun reconnect(attempt: Int): String =
        context.resources.getQuantityString(R.plurals.reconnect_attempt, attempt, attempt)

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
                val ctx = ApplicationProvider.getApplicationContext<Context>()
                val info = ActivityInfo().apply {
                    name = ComponentActivity::class.java.name
                    packageName = ctx.packageName
                    applicationInfo = ctx.applicationInfo
                    enabled = true
                    exported = false
                }
                shadowOf(ctx.packageManager).addOrUpdateActivity(info)
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
        compose.onNodeWithTag("address-error")
            .assertIsDisplayed()
            .assertTextContains(str(R.string.address_error_missing_port))
        compose.onNodeWithTag("address-input").performTextReplacement("10.0.2.2:65531")
        compose.onNodeWithTag("address-input").assertTextContains("10.0.2.2:65531")
        compose.onNodeWithTag("pair-code").assertIsEnabled()
    }

    @Test
    fun home_offers_pairing_choices_and_opens_the_pairing_screen() {
        start(AppState(address = "10.0.2.2:65531", addressError = null))
        compose.onNodeWithText(str(R.string.home_pair_code)).performScrollTo().assertIsDisplayed()
        compose.onNodeWithText(str(R.string.home_scan_qr)).performScrollTo().assertIsDisplayed()
        compose.onNodeWithTag("pair-code").performScrollTo().performClick()
        compose.onNodeWithText("Pair your phone").performScrollTo().assertIsDisplayed()
        assertTrue(events.contains(UiEvent.StartSasPairing))
    }

    @Test
    fun terminal_error_shows_guidance_and_retry_opens_repairing_pairing() {
        start(
            AppState(
                screen = AppScreen.SESSION,
                outcome = SessionOutcome.Ended(EndReason.Surfaced(204)),
                storedServers = listOf(
                    StoredServer("ab".repeat(32), "Pixel", "10.0.2.2:65531")
                ),
            )
        )
        compose.onNodeWithText(str(R.string.error_204_title)).assertIsDisplayed()
        compose.onNodeWithText(str(R.string.error_action_pair_again))
            .performScrollTo().assertIsDisplayed()
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
                outcome = SessionOutcome.Live(LivePhase.PLAYING),
            )
        )
        compose.onNodeWithTag("session-state")
            .assertTextContains(str(R.string.session_phase_playing_title))
        compose.onNodeWithTag("stop").assertIsEnabled().performClick()
        assertTrue(events.contains(UiEvent.StopRequested))
        compose.onNodeWithTag("stop").assertIsNotEnabled()
    }

    @Test
    fun reconnecting_is_shown_in_place_with_the_attempt_number() {
        start(
            AppState(
                screen = AppScreen.SESSION,
                address = "10.0.2.2:65531",
                outcome = SessionOutcome.Reconnecting(attempt = 3, lastCloseCode = 601),
            )
        )
        compose.onNodeWithTag("session-state").assertTextContains(reconnect(3))
        compose.onNodeWithTag("stop").assertIsEnabled()
    }

    @Test
    fun ended_user_stop_is_visibly_different_from_a_surfaced_error() {
        start(
            AppState(
                screen = AppScreen.SESSION,
                address = "10.0.2.2:65531",
                outcome = SessionOutcome.Ended(EndReason.UserStop),
            )
        )
        compose.onNodeWithTag("ended-user-stop").assertIsDisplayed()
        compose.onNodeWithTag("connect-again").assertIsDisplayed()
        compose.onNodeWithTag("error-title").assertDoesNotExist()
    }

    @Test
    fun ended_surfaced_shows_the_error_card_without_the_user_stop_copy() {
        start(
            AppState(
                screen = AppScreen.SESSION,
                address = "10.0.2.2:65531",
                outcome = SessionOutcome.Ended(EndReason.Surfaced(601)),
            )
        )
        compose.onNodeWithTag("error-title").assertIsDisplayed()
        compose.onNodeWithText(str(R.string.error_601_title)).assertIsDisplayed()
        compose.onNodeWithTag("ended-user-stop").assertDoesNotExist()
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
    fun qr_pairing_shows_the_automatic_confirmation_transition_without_a_confirm_button() {
        start(
            AppState(
                screen = AppScreen.PAIRING,
                address = "10.0.2.2:65531",
                pairing = PairingUi(
                    mode = "qr",
                    phase = "AWAITING_CONFIRMATION",
                    sas = "123 456",
                    fingerprint = "AAAA1111 BBBB2222",
                    autoConfirmSent = true,
                ),
            )
        )
        compose.onNodeWithTag("qr-auto-confirm").performScrollTo().assertIsDisplayed()
        compose.onNodeWithText(str(R.string.pairing_awaiting_qr_title)).assertIsDisplayed()
        compose.onNodeWithTag("pairing-fingerprint")
            .assertTextContains("AAAA1111 BBBB2222")
        compose.onNodeWithTag("confirm-pairing").assertDoesNotExist()
        compose.onNodeWithTag("pairing-sas-input").assertDoesNotExist()
        compose.onNodeWithText("123 456").performScrollTo().assertIsDisplayed()
    }

    @Test
    fun qr_pairing_shows_that_the_server_still_needs_to_accept_after_the_auto_confirm() {
        start(
            AppState(
                screen = AppScreen.PAIRING,
                address = "10.0.2.2:65531",
                pairing = PairingUi(
                    mode = "qr",
                    phase = "CONFIRMING",
                    fingerprint = "AAAA1111 BBBB2222",
                    autoConfirmSent = true,
                ),
            )
        )
        compose.onNodeWithTag("pairing-confirming").performScrollTo().assertIsDisplayed()
        compose.onNodeWithText(str(R.string.pairing_confirming_title)).assertIsDisplayed()
        compose.onNodeWithTag("confirm-pairing").assertDoesNotExist()
    }

    @Test
    fun an_invalid_qr_blocks_and_offers_an_explicit_scan_again_action() {
        val app = ApplicationProvider.getApplicationContext<android.app.Application>()
        shadowOf(app).grantPermissions(android.Manifest.permission.CAMERA)
        start(
            AppState(
                screen = AppScreen.PAIRING,
                address = "10.0.2.2:65531",
                pairing = PairingUi(
                    mode = "qr",
                    phase = "FAILED",
                    errorCode = 201,
                ),
            )
        )
        compose.onNodeWithText(str(R.string.error_201_title))
            .performScrollTo().assertIsDisplayed()
        compose.onNodeWithText(str(R.string.error_201_guidance))
            .performScrollTo().assertIsDisplayed()
        compose.onNodeWithTag("scan-again").performScrollTo().assertIsDisplayed().performClick()
        assertTrue(events.contains(UiEvent.RequestScan))
    }

    @Test
    fun home_opens_settings_which_lists_the_paired_server_and_version() {
        start(
            AppState(
                address = "10.0.2.2:65531",
                storedServers = listOf(
                    StoredServer("ab".repeat(32), "Pixel", "10.0.2.2:65531")
                ),
            )
        )
        compose.onNodeWithTag("settings").performClick()
        compose.onNodeWithTag("settings-title").assertIsDisplayed()
        compose.onNodeWithText("Pixel").assertIsDisplayed()
        compose.onNodeWithTag("settings-version")
            .assertTextContains("0.2.0-beta", substring = true)
        assertTrue(events.contains(UiEvent.ShowSettings))
        compose.onNodeWithTag("settings-about").performScrollTo().performClick()
        assertTrue(events.contains(UiEvent.ShowAbout))
    }

    @Test
    fun settings_without_a_pairing_shows_an_empty_state() {
        start(AppState(screen = AppScreen.SETTINGS))
        compose.onNodeWithTag("settings-empty").assertIsDisplayed()
    }

    @Test
    fun system_back_from_pairing_cancels_and_returns_home() {
        start(
            AppState(
                screen = AppScreen.PAIRING,
                address = "10.0.2.2:65531",
                pairing = PairingUi(mode = "sas", phase = "AWAITING_CONFIRMATION"),
            )
        )
        compose.runOnUiThread {
            compose.activity.onBackPressedDispatcher.onBackPressed()
        }
        compose.onNodeWithTag("home-title").assertIsDisplayed()
        assertTrue(events.contains(UiEvent.BackPressed))
        assertFalse(events.contains(UiEvent.StopRequested), "Back is not Stop")
    }

    @Test
    fun system_back_from_session_returns_home_without_stopping_playback() {
        start(
            AppState(
                screen = AppScreen.SESSION,
                address = "10.0.2.2:65531",
                outcome = SessionOutcome.Live(LivePhase.PLAYING),
            )
        )
        compose.runOnUiThread {
            compose.activity.onBackPressedDispatcher.onBackPressed()
        }
        compose.onNodeWithTag("home-title").assertIsDisplayed()
        assertTrue(events.contains(UiEvent.BackPressed))
        assertFalse(events.contains(UiEvent.StopRequested), "playback keeps running")
    }

    @Test
    fun system_back_at_home_is_left_to_the_system_to_finish_the_activity() {
        start(AppState())
        val hasEnabledCallbacks = compose.runOnUiThread {
            compose.activity.onBackPressedDispatcher.hasEnabledCallbacks()
        }
        assertFalse(
            hasEnabledCallbacks,
            "the HOME BackHandler is disabled so the Activity finishes normally",
        )
    }

    @Test
    fun about_screen_states_the_beta_version_limits_and_v1_warning() {
        start(AppState(screen = AppScreen.ABOUT))
        compose.onNodeWithTag("about-version").assertTextContains("0.2.0-beta", substring = true)
        compose.onNodeWithText(str(R.string.about_known_limitations)).assertIsDisplayed()
        compose.onNodeWithText(str(R.string.about_v1_warning)).assertIsDisplayed()
        assertEquals(AppScreen.ABOUT, AppState(screen = AppScreen.ABOUT).screen)
    }
}
