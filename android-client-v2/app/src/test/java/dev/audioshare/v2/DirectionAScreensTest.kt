package dev.audioshare.v2

import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertContentDescriptionEquals
import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertWidthIsAtLeast
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.unit.dp
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import kotlin.test.assertTrue

private fun storedServer() = StoredServer(
    serverSpkiSha256Hex = "ab".repeat(32),
    displayName = "Pixel",
    address = "10.0.2.2:65531",
)

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [36])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class HomeScreenTest : RobolectricComposeTestBase() {

    @Test
    fun no_credentials_hides_the_server_card_and_connect_action() {
        start(AppState())
        compose.onNodeWithTag("home-title").assertIsDisplayed()
        compose.onNodeWithTag("address-input").performScrollTo().assertIsDisplayed()
        compose.onNodeWithTag("pair-code").performScrollTo().assertIsDisplayed()
        compose.onNodeWithTag("scan-qr").performScrollTo().assertIsDisplayed()
        compose.onNodeWithTag("home-server-card").assertDoesNotExist()
        compose.onNodeWithTag("connect").assertDoesNotExist()
    }

    @Test
    fun a_saved_server_shows_the_card_and_connect_primary_action() {
        start(
            AppState(
                address = "10.0.2.2:65531",
                storedServers = listOf(storedServer()),
            )
        )
        compose.onNodeWithTag("home-server-card").performScrollTo().assertIsDisplayed()
        compose.onNodeWithText("Pixel").performScrollTo().assertIsDisplayed()
        compose.onNodeWithTag("connect").performScrollTo().assertIsDisplayed().assertIsEnabled()
        compose.onNodeWithTag("pair-code").assertTextContains(str(R.string.home_pair_new))
        compose.onNodeWithTag("scan-qr")
            .assertTextContains(str(R.string.home_scan_qr_instead))
    }

    @Test
    fun an_invalid_address_renders_an_inline_localizable_error_and_disables_pairing() {
        start(AppState())
        compose.onNodeWithTag("address-input").performTextReplacement("10.0.2.2")
        compose.onNodeWithTag("address-error")
            .assertIsDisplayed()
            .assertTextContains(str(R.string.address_error_missing_port))
        compose.onNodeWithTag("pair-code").assertIsNotEnabled()
    }

    @Test
    fun camera_permission_denial_is_shown_inline_on_home() {
        start(AppState(scanNotice = ScanNotice.CAMERA_PERMISSION_DENIED))
        compose.onNodeWithTag("scan-message")
            .performScrollTo()
            .assertIsDisplayed()
            .assertTextContains(str(R.string.scan_permission_denied))
    }
}

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [36])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class AboutScreenTest : RobolectricComposeTestBase() {

    @Test
    fun about_shows_version_limitations_v1_warning_updates_and_privacy() {
        start(AppState(screen = AppScreen.ABOUT))
        compose.onNodeWithText(str(R.string.about_title)).assertIsDisplayed()
        compose.onNodeWithTag("about-version")
            .assertTextContains("0.2.0-beta", substring = true)
        compose.onNodeWithText(str(R.string.version_label, "0.2.0-beta", 2))
            .assertIsDisplayed()

        compose.onNodeWithText(str(R.string.about_known_limitations))
            .performScrollTo().assertIsDisplayed()
        for (id in listOf(
            R.string.about_limit_jitter,
            R.string.about_limit_one_client,
            R.string.about_limit_diagnostics,
        )) {
            compose.onNodeWithText(str(id), substring = true)
                .performScrollTo().assertIsDisplayed()
        }
        for (id in listOf(
            R.string.about_v1_title,
            R.string.about_v1_warning,
            R.string.about_updates_title,
            R.string.about_updates_body,
            R.string.about_privacy_title,
            R.string.about_privacy_body,
        )) {
            compose.onNodeWithText(str(id)).performScrollTo().assertIsDisplayed()
        }
    }
}

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [36])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class SettingsScreenTest : RobolectricComposeTestBase() {

    @Test
    fun paired_settings_is_read_only_and_lists_server_details_and_version() {
        val pin = "cd".repeat(32)
        start(
            AppState(
                screen = AppScreen.SETTINGS,
                storedServers = listOf(
                    StoredServer(pin, "Pixel", "10.0.2.2:65531"),
                ),
            )
        )
        compose.onNodeWithTag("settings-title").assertIsDisplayed()
        compose.onNodeWithTag("settings-server").assertIsDisplayed()
        compose.onNodeWithText("Pixel").assertIsDisplayed()
        compose.onNodeWithTag("settings-fingerprint").assertTextContains(pin)
        compose.onNodeWithTag("settings-version")
            .assertTextContains("0.2.0-beta", substring = true)
        compose.onNodeWithTag("settings-about")
            .performScrollTo().assertIsDisplayed().performClick()
        assertTrue(events.contains(UiEvent.ShowAbout))
    }

    @Test
    fun empty_settings_explains_the_state_and_offers_pairing() {
        start(AppState(screen = AppScreen.SETTINGS))
        compose.onNodeWithTag("settings-empty")
            .assertIsDisplayed()
            .assertTextContains(str(R.string.settings_empty_body))
        compose.onNodeWithTag("settings-pair")
            .assertIsDisplayed()
            .assertTextContains(str(R.string.settings_pair_phone))
            .performClick()
        assertTrue(events.contains(UiEvent.ShowHome))
    }
}

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [36])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class AccessibilitySemanticsTest : RobolectricComposeTestBase() {

    @Test
    fun icon_only_shell_actions_have_content_descriptions() {
        start(AppState())
        compose.onNodeWithContentDescription(str(R.string.action_settings)).assertIsDisplayed()
        compose.onNodeWithContentDescription(str(R.string.action_about)).assertIsDisplayed()
    }

    @Test
    fun shell_and_primary_targets_are_at_least_48dp() {
        start(AppState(address = "10.0.2.2:65531", storedServers = listOf(storedServer())))
        for (tag in listOf("settings", "about", "connect", "pair-code", "scan-qr")) {
            compose.onNodeWithTag(tag)
                .assertHeightIsAtLeast(48.dp)
                .assertWidthIsAtLeast(48.dp)
        }
    }

    @Test
    fun address_error_and_scan_notice_are_assertive_live_regions() {
        start(AppState())
        compose.onNodeWithTag("address-input").performTextReplacement("10.0.2.2")
        compose.onNodeWithTag("address-error").assert(
            SemanticsMatcher.expectValue(
                SemanticsProperties.LiveRegion,
                LiveRegionMode.Assertive,
            ),
        )
    }

    @Test
    fun scan_notice_is_an_assertive_live_region() {
        start(AppState(scanNotice = ScanNotice.CAMERA_PERMISSION_DENIED))
        compose.onNodeWithTag("scan-message").assert(
            SemanticsMatcher.expectValue(
                SemanticsProperties.LiveRegion,
                LiveRegionMode.Assertive,
            ),
        )
    }

    @Test
    fun surfaced_error_card_text_is_an_assertive_live_region() {
        start(
            AppState(
                screen = AppScreen.SESSION,
                address = "10.0.2.2:65531",
                outcome = SessionOutcome.Ended(EndReason.Surfaced(601)),
            )
        )
        compose.onNodeWithTag("error-title").assert(
            SemanticsMatcher.expectValue(
                SemanticsProperties.LiveRegion,
                LiveRegionMode.Assertive,
            ),
        )
    }

    @Test
    fun pairing_sas_is_announced_digit_by_digit() {
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
        compose.onNodeWithTag("pairing-sas").assertContentDescriptionEquals("3 8 5 5 8 7")
    }

    @Test
    fun home_focus_order_is_title_then_outcome_then_error_then_primary_action() {
        start(AppState())
        compose.onNodeWithTag("address-input").performTextReplacement("10.0.2.2")
        val title = compose.onNodeWithTag("home-title").fetchSemanticsNode().boundsInRoot
        val outcome = compose.onNodeWithTag("session-outcome").fetchSemanticsNode().boundsInRoot
        val error = compose.onNodeWithTag("address-error").fetchSemanticsNode().boundsInRoot
        val primary = compose.onNodeWithTag("pair-code").fetchSemanticsNode().boundsInRoot
        assertTrue(title.top < outcome.top, "title precedes outcome")
        assertTrue(outcome.top < error.top, "outcome/error follows the title")
        assertTrue(error.top < primary.top, "the primary action follows the error")
    }
}

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [36], qualifiers = "zh-rCN")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ChineseLocaleSmokeTest : RobolectricComposeTestBase() {

    @Test
    fun home_renders_chinese_copy_from_the_system_locale() {
        start(AppState())
        compose.onNodeWithText("准备好了吗").assertIsDisplayed()
        compose.onNodeWithContentDescription("设置").assertIsDisplayed()
        compose.onNodeWithText("使用配对码").assertIsDisplayed()
    }
}
