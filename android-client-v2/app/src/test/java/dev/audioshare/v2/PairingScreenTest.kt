package dev.audioshare.v2

import android.Manifest
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
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private fun awaitingSasSnapshot(sas: String = "418 902") = PairingSnapshot(
    mode = "sas",
    phase = "AWAITING_CONFIRMATION",
    sas = sas,
    fingerprint = "AAAA1111 BBBB2222 CCCC3333 DDDD4444 EEEE5555 FFFF6666 77778888 99990000",
    serverPublicKeyHex = "ee".repeat(32),
    serverSpkiSha256Hex = "ff".repeat(32),
    clientId = null,
    displayName = "Pixel",
    errorCode = null,
)

private fun awaitingQrSnapshot(sas: String = "418 902") =
    awaitingSasSnapshot(sas).copy(mode = "qr", fingerprint = "AAAA1111 BBBB2222")

private fun completedRecord() = CredentialRecord(
    serverSpkiSha256Hex = "ab".repeat(32),
    serverPublicKeyHex = "cd".repeat(32),
    addressHint = "10.0.2.2:65531",
    keyAlias = "v2-test",
    pairedAt = "2026-11-16T00:00:00Z",
    softwareBacked = false,
    clientId = "client-1",
    displayName = "Pixel",
    host = "10.0.2.2",
    port = 65531,
)

private fun pairingState(pairing: PairingUi?) = AppState(
    screen = AppScreen.PAIRING,
    address = "10.0.2.2:65531",
    pairing = pairing,
)

/**
 * Ticket 19 seam: the pairing screen is a pure projection of [AppState].
 * These tests cover the six Direction-A states plus the accessibility and
 * localization contract; native pairing behaviour stays with tickets 30/32.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [36])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class PairingScreenTest : RobolectricComposeTestBase() {

    // ---- 1. not started -----------------------------------------------------

    @Test
    fun not_started_shows_the_three_pairing_entries() {
        start(pairingState(null))

        compose.onNodeWithTag("pairing-title")
            .assertIsDisplayed()
            .assertTextContains(str(R.string.pairing_title))
        compose.onNodeWithText(str(R.string.pairing_subtitle)).assertIsDisplayed()
        compose.onNodeWithTag("scan-qr")
            .performScrollTo()
            .assertIsDisplayed()
            .assertTextContains(str(R.string.pairing_scan_qr))
        compose.onNodeWithTag("paste-link").performScrollTo().assertIsDisplayed()
        compose.onNodeWithTag("use-pasted-link")
            .performScrollTo()
            .assertIsDisplayed()
            .assertIsNotEnabled()
        compose.onNodeWithTag("start-code-pairing")
            .performScrollTo()
            .assertIsDisplayed()
            .assertIsEnabled()
            .assertTextContains(str(R.string.pairing_start_code))
    }

    // ---- 2. scanning --------------------------------------------------------

    @Test
    fun scanning_shows_the_camera_view_and_a_close_affordance() {
        start(pairingState(null).copy(scanActive = true))

        compose.onNodeWithTag("qr-scanner").assertIsDisplayed()
        compose.onNodeWithText(str(R.string.pairing_scan_instruction)).assertIsDisplayed()
        compose.onNodeWithTag("close-camera")
            .performScrollTo()
            .assertIsDisplayed()
            .assertHeightIsAtLeast(48.dp)
            .assertWidthIsAtLeast(48.dp)
            .assertTextContains(str(R.string.pairing_close_camera))
        compose.onNodeWithTag("scan-qr").assertDoesNotExist()
        compose.onNodeWithTag("start-code-pairing").assertDoesNotExist()
    }

    // ---- 3. STARTING --------------------------------------------------------

    @Test
    fun starting_shows_a_single_status_and_cancel_action() {
        start(pairingState(PairingUi(mode = "sas", phase = "STARTING")))

        compose.onNodeWithTag("pairing-status")
            .assertIsDisplayed()
            .assertTextContains(str(R.string.pairing_starting_title))
        compose.onNodeWithText(str(R.string.pairing_starting_body)).assertIsDisplayed()
        compose.onNodeWithTag("cancel-pairing")
            .performScrollTo()
            .assertIsDisplayed()
            .assertTextContains(str(R.string.pairing_cancel))
        compose.onNodeWithTag("scan-qr").assertDoesNotExist()
        compose.onNodeWithTag("start-code-pairing").assertDoesNotExist()
    }

    // ---- 4. AWAITING_CONFIRMATION ------------------------------------------

    @Test
    fun qr_auto_confirms_exactly_once_and_shows_the_waiting_copy() {
        start(pairingState(PairingUi(mode = "qr", phase = "STARTING")))

        send(UiEvent.PairingSnapshotChanged(awaitingQrSnapshot()))
        compose.waitForIdle()

        assertEquals(
            1,
            commands.count { it == AppCommand.ConfirmPairing },
            "the QR scan is the Client's one action",
        )
        compose.onNodeWithTag("qr-auto-confirm")
            .performScrollTo()
            .assertIsDisplayed()
            .assertTextContains(str(R.string.pairing_awaiting_qr_confirmed))
        compose.onNodeWithTag("pairing-sas")
            .performScrollTo()
            .assertContentDescriptionEquals("4 1 8 9 0 2")
        compose.onNodeWithTag("confirm-pairing").assertDoesNotExist()
        compose.onNodeWithTag("pairing-sas-input").assertDoesNotExist()

        // A replayed snapshot must not queue a second confirmation.
        send(UiEvent.PairingSnapshotChanged(awaitingQrSnapshot()))
        compose.waitForIdle()
        assertEquals(1, commands.count { it == AppCommand.ConfirmPairing })
    }

    @Test
    fun sas_never_auto_confirms_and_requires_the_matching_code() {
        start(pairingState(PairingUi(mode = "sas", phase = "STARTING")))

        send(UiEvent.PairingSnapshotChanged(awaitingSasSnapshot()))
        compose.waitForIdle()

        assertFalse(commands.contains(AppCommand.ConfirmPairing), "SAS needs an explicit tap")
        compose.onNodeWithTag("pairing-status")
            .assertTextContains(str(R.string.pairing_awaiting_sas_title))
        compose.onNodeWithTag("pairing-sas")
            .assertContentDescriptionEquals("4 1 8 9 0 2")
        compose.onNodeWithTag("confirm-pairing")
            .performScrollTo()
            .assertIsNotEnabled()

        compose.onNodeWithTag("pairing-sas-input")
            .performScrollTo()
            .performTextReplacement("418903")
        compose.onNodeWithTag("pairing-sas-error")
            .assertIsDisplayed()
            .assertTextContains(str(R.string.pairing_notice_sas_mismatch))
        compose.onNodeWithTag("confirm-pairing").assertIsNotEnabled()

        compose.onNodeWithTag("pairing-sas-input").performTextReplacement("418902")
        compose.onNodeWithTag("confirm-pairing")
            .performScrollTo()
            .assertIsEnabled()
            .performClick()
        assertTrue(events.contains(UiEvent.ConfirmPairing))
        assertTrue(commands.contains(AppCommand.ConfirmPairing))
    }

    // ---- 5. FAILED ----------------------------------------------------------

    @Test
    fun qr_201_blocks_with_code_guidance_and_an_explicit_rescan() {
        grantCamera()
        start(
            pairingState(
                PairingUi(mode = "qr", phase = "FAILED", errorCode = 201),
            )
        )

        assertBlockingFailure(
            code = 201,
            title = str(R.string.error_201_title),
            guidance = str(R.string.error_201_guidance),
        )
        compose.onNodeWithTag("pairing-error-title").assert(
            SemanticsMatcher.expectValue(
                SemanticsProperties.LiveRegion,
                LiveRegionMode.Assertive,
            ),
        )
        // The failed exchange never reopens the camera by itself.
        compose.onNodeWithTag("qr-scanner").assertDoesNotExist()
        compose.onNodeWithText(str(R.string.pairing_camera_stays_closed))
            .performScrollTo()
            .assertIsDisplayed()

        compose.onNodeWithTag("scan-again").performScrollTo().performClick()
        assertTrue(events.contains(UiEvent.RequestScan))
        compose.onNodeWithTag("qr-scanner").assertIsDisplayed()
    }

    @Test
    fun qr_206_blocks_and_waits_for_the_user_to_scan_again() {
        grantCamera()
        start(
            pairingState(
                PairingUi(mode = "qr", phase = "FAILED", errorCode = 206),
            )
        )

        assertBlockingFailure(
            code = 206,
            title = str(R.string.error_206_title),
            guidance = str(R.string.error_206_guidance),
        )
        compose.onNodeWithTag("qr-scanner").assertDoesNotExist()
        compose.onNodeWithText(str(R.string.pairing_camera_stays_closed))
            .performScrollTo()
            .assertIsDisplayed()
    }

    @Test
    fun a_general_failure_shows_code_guidance_and_try_again_in_place() {
        start(
            pairingState(
                PairingUi(mode = "sas", phase = "FAILED", errorCode = 203),
            )
        )

        assertBlockingFailure(
            code = 203,
            title = str(R.string.error_203_title),
            guidance = str(R.string.error_203_guidance),
        )
        compose.onNodeWithTag("pairing-try-again")
            .performScrollTo()
            .assertIsDisplayed()
            .assertTextContains(str(R.string.pairing_try_again))
            .performClick()

        assertTrue(events.contains(UiEvent.ShowPairing))
        // "In place" means the not-started entries return; the screen does not change.
        compose.onNodeWithTag("scan-qr").performScrollTo().assertIsDisplayed()
        compose.onNodeWithTag("start-code-pairing").performScrollTo().assertIsDisplayed()
    }

    @Test
    fun an_invalid_link_failure_has_no_numeric_code_and_offers_try_again() {
        start(
            pairingState(
                PairingUi(
                    mode = "qr",
                    phase = "FAILED",
                    notice = PairingNotice.INVALID_PAIRING_LINK,
                ),
            )
        )

        compose.onNodeWithTag("pairing-error-title")
            .assertTextContains(str(R.string.pairing_failed_title))
        compose.onNodeWithText(str(R.string.pairing_notice_invalid_link))
            .performScrollTo()
            .assertIsDisplayed()
        compose.onNodeWithTag("pairing-error-code").assertDoesNotExist()
        compose.onNodeWithTag("pairing-try-again").performScrollTo().assertIsDisplayed()
    }

    @Test
    fun a_sas_invitation_failure_offers_try_again_not_a_scan() {
        start(
            pairingState(
                PairingUi(mode = "sas", phase = "FAILED", errorCode = 201),
            )
        )

        compose.onNodeWithText(str(R.string.pairing_failed_invitation_sas))
            .performScrollTo()
            .assertIsDisplayed()
        compose.onNodeWithTag("scan-again").assertDoesNotExist()
        compose.onNodeWithTag("pairing-try-again")
            .performScrollTo()
            .assertIsDisplayed()
            .assertTextContains(str(R.string.pairing_try_again))
            .performClick()
        assertTrue(events.contains(UiEvent.ShowPairing))
    }

    // ---- 6. COMPLETE --------------------------------------------------------

    @Test
    fun complete_shows_saving_and_handoff_feedback_and_pairing_completed_navigates() {
        start(
            pairingState(
                PairingUi(mode = "sas", phase = "COMPLETE"),
            )
        )

        compose.onNodeWithTag("pairing-complete")
            .performScrollTo()
            .assertIsDisplayed()
        compose.onNodeWithTag("pairing-status")
            .assertTextContains(str(R.string.pairing_complete_title))
        compose.onNodeWithText(str(R.string.pairing_complete_body))
            .performScrollTo()
            .assertIsDisplayed()

        send(UiEvent.PairingCompleted(completedRecord()))
        compose.waitForIdle()
        assertTrue(
            compose.onAllNodesWithTag("session-state").fetchSemanticsNodes().isNotEmpty(),
            "the one-shot PairingCompleted handoff moves the screen to SESSION",
        )
    }

    // ---- a11y / i18n --------------------------------------------------------

    @Test
    fun pairing_status_and_error_are_assertive_live_regions() {
        start(pairingState(PairingUi(mode = "sas", phase = "STARTING")))
        compose.onNodeWithTag("pairing-status").assert(
            SemanticsMatcher.expectValue(
                SemanticsProperties.LiveRegion,
                LiveRegionMode.Assertive,
            ),
        )
    }

    @Test
    fun pairing_action_targets_are_at_least_48dp() {
        start(pairingState(null))
        assertTargetsAtLeast48("scan-qr", "use-pasted-link", "start-code-pairing")
    }

    @Test
    fun in_flight_pairing_action_targets_are_at_least_48dp() {
        start(
            pairingState(
                PairingUi(
                    mode = "sas",
                    phase = "AWAITING_CONFIRMATION",
                    sas = "418 902",
                ),
            )
        )
        assertTargetsAtLeast48("confirm-pairing", "cancel-pairing", "pairing-back-home")
    }

    @Test
    fun failed_pairing_action_targets_are_at_least_48dp() {
        grantCamera()
        start(
            pairingState(
                PairingUi(mode = "qr", phase = "FAILED", errorCode = 201),
            )
        )
        assertTargetsAtLeast48("scan-again", "pairing-back-home")
    }

    @Test
    fun general_failure_action_target_is_at_least_48dp() {
        start(
            pairingState(
                PairingUi(mode = "sas", phase = "FAILED", errorCode = 203),
            )
        )
        assertTargetsAtLeast48("pairing-try-again", "pairing-back-home")
    }

    @Test
    fun pairing_focus_order_is_title_then_status_then_primary_action() {
        start(pairingState(PairingUi(mode = "sas", phase = "AWAITING_CONFIRMATION", sas = "418 902")))
        val title = compose.onNodeWithTag("pairing-title").fetchSemanticsNode().boundsInRoot
        val status = compose.onNodeWithTag("pairing-status").fetchSemanticsNode().boundsInRoot
        val primary = compose.onNodeWithTag("confirm-pairing").fetchSemanticsNode().boundsInRoot
        assertTrue(title.top < status.top, "title precedes status")
        assertTrue(status.top < primary.top, "status precedes the primary action")
    }

    @Test
    fun pairing_failure_focus_order_is_title_then_error_then_primary_action() {
        start(
            pairingState(
                PairingUi(mode = "sas", phase = "FAILED", errorCode = 203),
            )
        )
        val title = compose.onNodeWithTag("pairing-title").fetchSemanticsNode().boundsInRoot
        val error = compose.onNodeWithTag("pairing-error-title").fetchSemanticsNode().boundsInRoot
        val guidance = compose.onNodeWithTag("pairing-error-guidance")
            .fetchSemanticsNode().boundsInRoot
        val primary = compose.onNodeWithTag("pairing-try-again")
            .fetchSemanticsNode().boundsInRoot
        assertTrue(title.top < error.top, "title precedes the error")
        assertTrue(error.top < guidance.top, "error title precedes guidance")
        assertTrue(guidance.top < primary.top, "error content precedes the primary action")
    }

    @Test
    fun the_pairing_screen_survives_two_hundred_percent_font_scale() {
        RuntimeEnvironment.setFontScale(2f)
        try {
            start(
                pairingState(
                    PairingUi(
                        mode = "sas",
                        phase = "AWAITING_CONFIRMATION",
                        sas = "418 902",
                        fingerprint = "AAAA1111 BBBB2222",
                    ),
                )
            )

            compose.onNodeWithTag("pairing-title").assertIsDisplayed()
            compose.onNodeWithTag("pairing-status").performScrollTo().assertIsDisplayed()
            compose.onNodeWithTag("pairing-sas").performScrollTo().assertIsDisplayed()
            compose.onNodeWithTag("confirm-pairing").performScrollTo().assertIsDisplayed()
        } finally {
            RuntimeEnvironment.setFontScale(1f)
        }
    }

    private fun assertTargetsAtLeast48(vararg tags: String) {
        for (tag in tags) {
            compose.onNodeWithTag(tag)
                .performScrollTo()
                .assertHeightIsAtLeast(48.dp)
                .assertWidthIsAtLeast(48.dp)
        }
    }

    private fun assertBlockingFailure(code: Int, title: String, guidance: String) {
        compose.onNodeWithTag("pairing-error-title")
            .performScrollTo()
            .assertIsDisplayed()
            .assertTextContains(title)
        compose.onNodeWithTag("pairing-error-code")
            .performScrollTo()
            .assertIsDisplayed()
            .assertTextContains(str(R.string.pairing_error_code, code))
        compose.onNodeWithTag("pairing-error-guidance")
            .performScrollTo()
            .assertIsDisplayed()
            .assertTextContains(guidance)
    }

    private fun grantCamera() {
        val app = ApplicationProvider.getApplicationContext<android.app.Application>()
        shadowOf(app).grantPermissions(Manifest.permission.CAMERA)
    }
}

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [36], qualifiers = "zh-rCN")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class PairingChineseLocaleTest : RobolectricComposeTestBase() {

    @Test
    fun pairing_copy_comes_from_the_zh_cn_resources() {
        start(
            pairingState(
                PairingUi(
                    mode = "sas",
                    phase = "AWAITING_CONFIRMATION",
                    sas = "418 902",
                    fingerprint = "AAAA1111 BBBB2222",
                ),
            )
        )

        compose.onNodeWithTag("pairing-title").assertTextContains("配对手机")
        compose.onNodeWithTag("pairing-status").assertTextContains("核对配对码")
        compose.onNodeWithTag("confirm-pairing").assertTextContains("确认配对")
    }
}
