package dev.audioshare.v2

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import kotlin.test.assertTrue

/**
 * Ticket 20, SESSION screen Direction A: four Live phases, Reconnecting in
 * place, and the two visually distinct Ended presentations. Assertions use
 * resource lookups and semantics, so the same expectations hold for en and
 * zh-CN. Ticket 08 scenarios #1-#5 are covered here or in
 * [AppStateReducerTest].
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [36])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class SessionScreenDirectionATest : RobolectricComposeTestBase() {

    private fun live(
        outcome: SessionOutcome.Live,
        @androidx.annotation.StringRes titleRes: Int,
        @androidx.annotation.StringRes bodyRes: Int,
        progress: Boolean = false,
    ) {
        startSession(outcome)
        compose.onNodeWithTag("session-title").assertIsDisplayed()
        compose.onNodeWithTag("session-state").assertTextEquals(str(titleRes))
        compose.onNodeWithTag("session-hero-body").assertTextEquals(str(bodyRes))
        compose.onNodeWithTag("stop").performScrollTo().assertIsEnabled()
        if (progress) {
            compose.onNodeWithTag("session-progress").performScrollTo().assertIsDisplayed()
        } else {
            compose.onNodeWithTag("session-progress").assertDoesNotExist()
        }
    }

    private fun startSession(outcome: SessionOutcome?) {
        start(
            AppState(
                screen = AppScreen.SESSION,
                address = "10.0.2.2:65531",
                storedServers = listOf(
                    StoredServer("ab".repeat(32), "Pixel", "10.0.2.2:65531"),
                ),
                outcome = outcome,
            )
        )
    }

    private fun reconnect(attempt: Int): String =
        context.resources.getQuantityString(R.plurals.reconnect_attempt, attempt, attempt)

    @Test
    fun connecting_shows_the_connecting_hero_and_progress() {
        live(
            SessionOutcome.Live(LivePhase.CONNECTING),
            R.string.session_phase_connecting_title,
            R.string.session_phase_connecting_body,
            progress = true,
        )
    }

    @Test
    fun authenticating_shows_the_verifying_hero_and_progress() {
        live(
            SessionOutcome.Live(LivePhase.AUTHENTICATING),
            R.string.session_phase_authenticating_title,
            R.string.session_phase_authenticating_body,
            progress = true,
        )
    }

    @Test
    fun ready_shows_the_connected_hero_without_progress() {
        live(
            SessionOutcome.Live(LivePhase.READY),
            R.string.session_phase_ready_title,
            R.string.session_phase_ready_body,
        )
    }

    @Test
    fun playing_shows_the_playing_hero_copy_and_only_stop_as_primary_action() {
        live(
            SessionOutcome.Live(LivePhase.PLAYING),
            R.string.session_phase_playing_title,
            R.string.session_phase_playing_body,
        )
        // Ticket 08 #5 at the UI seam: no second Connect action is offered
        // while a live session already owns the Service.
        compose.onNodeWithTag("connect").assertDoesNotExist()
        compose.onNodeWithTag("connect-again").assertDoesNotExist()
        compose.onNodeWithTag("retry").assertDoesNotExist()
        compose.onNodeWithTag("stop").assertHeightIsAtLeast(48.dp)
    }

    @Test
    fun reconnecting_stays_in_session_with_the_attempt_and_no_error_card() {
        startSession(SessionOutcome.Reconnecting(attempt = 3, lastCloseCode = 601))
        compose.onNodeWithTag("session-title").assertIsDisplayed()
        compose.onNodeWithTag("session-state").assertTextEquals(reconnect(3))
        compose.onNodeWithTag("session-hero-body")
            .assertTextEquals(str(R.string.session_reconnecting_body))
        compose.onNodeWithTag("session-reconnect-code")
            .assertTextContains("601", substring = true)
        compose.onNodeWithTag("stop").performScrollTo().assertIsEnabled()
        compose.onNodeWithTag("session-progress").performScrollTo().assertIsDisplayed()
        compose.onNodeWithTag("error-title").assertDoesNotExist()
        compose.onNodeWithTag("error-guidance").assertDoesNotExist()
        compose.onNodeWithTag("session-error-card").assertDoesNotExist()
        compose.onNodeWithTag("connect-again").assertDoesNotExist()
        compose.onNodeWithTag("retry").assertDoesNotExist()
        compose.onNodeWithTag("session-state").assert(
            SemanticsMatcher.expectValue(
                SemanticsProperties.LiveRegion,
                LiveRegionMode.Assertive,
            ),
        )
    }

    @Test
    fun ended_user_stop_is_calm_and_connect_again_emits_connect_requested() {
        startSession(SessionOutcome.Ended(EndReason.UserStop))
        compose.onNodeWithTag("ended-user-stop")
            .assertTextEquals(str(R.string.session_stopped_by_user))
        compose.onNodeWithTag("session-hero-body")
            .assertTextEquals(str(R.string.session_ended_user_body))
        compose.onNodeWithTag("connect-again")
            .performScrollTo().assertIsDisplayed()
            .assertHeightIsAtLeast(48.dp)
        compose.onNodeWithTag("error-title").assertDoesNotExist()
        compose.onNodeWithTag("session-error-card").assertDoesNotExist()
        compose.onNodeWithTag("retry").assertDoesNotExist()
        compose.onNodeWithTag("ended-user-stop").assert(
            SemanticsMatcher.expectValue(
                SemanticsProperties.LiveRegion,
                LiveRegionMode.Assertive,
            ),
        )

        compose.onNodeWithTag("connect-again").performClick()
        assertTrue(events.contains(UiEvent.ConnectRequested))
    }

    @Test
    fun ended_surfaced_shows_code_guidance_and_retry_without_the_user_stop_copy() {
        startSession(SessionOutcome.Ended(EndReason.Surfaced(601)))
        compose.onNodeWithTag("session-state")
            .assertTextEquals(str(R.string.session_ended_surfaced_title))
        compose.onNodeWithTag("session-hero-body")
            .assertTextEquals(str(R.string.session_ended_surfaced_body))
        compose.onNodeWithTag("session-error-card").assertIsDisplayed()
        compose.onNodeWithTag("error-title")
            .assertTextEquals(str(R.string.error_601_title))
        compose.onNodeWithTag("error-guidance")
            .assertTextEquals(str(R.string.error_601_guidance))
        compose.onNodeWithTag("session-error-code").assertTextContains("601", substring = true)
        compose.onNodeWithTag("retry")
            .performScrollTo().assertIsDisplayed()
            .assertTextContains(str(R.string.error_action_retry))
            .assertHeightIsAtLeast(48.dp)
        compose.onNodeWithTag("ended-user-stop").assertDoesNotExist()
        compose.onNodeWithTag("connect-again").assertDoesNotExist()

        for (tag in listOf("error-title", "error-guidance")) {
            compose.onNodeWithTag(tag).assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.LiveRegion,
                    LiveRegionMode.Assertive,
                ),
            )
        }

        compose.onNodeWithTag("retry").performClick()
        assertTrue(events.contains(UiEvent.Retry))
    }

    @Test
    fun ended_surfaced_requiring_repair_offers_pair_again_and_opens_pairing() {
        startSession(SessionOutcome.Ended(EndReason.Surfaced(204)))
        compose.onNodeWithTag("error-title").assertTextEquals(str(R.string.error_204_title))
        compose.onNodeWithTag("retry").assertTextContains(str(R.string.error_action_pair_again))
        compose.onNodeWithTag("connect-again").assertDoesNotExist()

        compose.onNodeWithTag("retry").performScrollTo().performClick()
        assertTrue(events.contains(UiEvent.Retry))
        compose.onNodeWithText(str(R.string.pairing_title)).performScrollTo().assertIsDisplayed()
    }

    @Test
    fun session_focus_order_is_title_then_status_or_error_then_primary_action() {
        startSession(SessionOutcome.Live(LivePhase.PLAYING))
        val title = compose.onNodeWithTag("session-title").fetchSemanticsNode().boundsInRoot
        val status = compose.onNodeWithTag("session-state").fetchSemanticsNode().boundsInRoot
        val primary = compose.onNodeWithTag("stop").fetchSemanticsNode().boundsInRoot
        assertTrue(title.top < status.top, "title precedes the status")
        assertTrue(status.top < primary.top, "the status precedes the primary action")
    }

    @Test
    fun surfaced_focus_order_is_title_then_status_then_error_then_retry() {
        startSession(SessionOutcome.Ended(EndReason.Surfaced(601)))
        val title = compose.onNodeWithTag("session-title").fetchSemanticsNode().boundsInRoot
        val status = compose.onNodeWithTag("session-state").fetchSemanticsNode().boundsInRoot
        val error = compose.onNodeWithTag("error-title").fetchSemanticsNode().boundsInRoot
        val retry = compose.onNodeWithTag("retry").fetchSemanticsNode().boundsInRoot
        assertTrue(title.top < status.top, "title precedes the status")
        assertTrue(status.top < error.top, "the error follows the status")
        assertTrue(error.top < retry.top, "the primary action follows the error")
    }

    @Test
    fun session_layout_survives_twice_the_font_scale() {
        compose.setContent {
            val density = LocalDensity.current
            CompositionLocalProvider(
                LocalDensity provides Density(density.density, fontScale = 2f),
            ) {
                AudioShareTheme {
                    AudioShareApp(
                        state = AppState(
                            screen = AppScreen.SESSION,
                            address = "10.0.2.2:65531",
                            outcome = SessionOutcome.Live(LivePhase.PLAYING),
                        ),
                        versionName = "0.2.0-beta",
                        versionCode = 2,
                    ) { }
                }
            }
        }
        for (tag in listOf("session-title", "session-state", "session-hero-body", "stop")) {
            compose.onNodeWithTag(tag).performScrollTo().assertIsDisplayed()
        }
    }

    @Test
    fun a_surfaced_null_code_uses_the_generic_guidance_and_retry() {
        // Unknown/null codes must never leave SESSION without an explanation.
        startSession(SessionOutcome.Ended(EndReason.Surfaced(null)))
        compose.onNodeWithTag("error-title")
            .assertTextEquals(str(R.string.error_generic_title))
        compose.onNodeWithTag("error-guidance")
            .assertTextEquals(str(R.string.error_generic_guidance))
        compose.onNodeWithTag("session-error-code").assertDoesNotExist()
        compose.onNodeWithTag("retry").assertTextContains(str(R.string.error_action_retry))
        compose.onNodeWithTag("connect-again").assertDoesNotExist()
    }
}

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [36], qualifiers = "zh-rCN")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class SessionChineseLocaleTest : RobolectricComposeTestBase() {

    @Test
    fun session_renders_chinese_copy_from_the_system_locale() {
        start(
            AppState(
                screen = AppScreen.SESSION,
                address = "10.0.2.2:65531",
                outcome = SessionOutcome.Live(LivePhase.PLAYING),
            )
        )
        compose.onNodeWithTag("session-title").assertTextEquals("会话")
        compose.onNodeWithTag("session-state").assertTextEquals("正在播放")
        compose.onNodeWithTag("session-hero-body").assertTextEquals("正在从服务器流式播放音频。")
        compose.onNodeWithTag("stop").performScrollTo().assertTextContains("停止")
    }
}
