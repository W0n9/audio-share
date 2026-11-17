package dev.audioshare.v2

import android.content.ComponentName
import android.content.Context
import android.content.pm.ActivityInfo
import androidx.test.core.app.ApplicationProvider
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowPhoneWindow
import kotlin.test.assertEquals
import kotlin.test.assertFalse

/**
 * Stage 3 (ticket 25) targetSdk 36 platform contract, asserted through the
 * built artifact rather than the build file:
 *
 *  - the APK targets API 36 but keeps the API 26 baseline;
 *  - MainActivity opts in to edge-to-edge, so the Scaffold's system-bar insets
 *    (not a decor-fits-window frame) keep content clear of the bars;
 *  - the Activity declares no orientation lock, so large-screen devices may
 *    resize/rotate it (targetSdk 36 ignores orientation restrictions there).
 *
 * Predictive back is covered by the SDK-36 Compose tests: the Activity keeps
 * `enableOnBackInvokedCallback` (manifest) and `BackHandler` still dispatches
 * through OnBackPressedDispatcher.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [36])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class TargetSdkPlatformBehaviorTest {

    private val context: Context get() = ApplicationProvider.getApplicationContext()

    @Test
    fun app_targets_api_36_on_the_api_26_baseline() {
        assertEquals(36, context.applicationInfo.targetSdkVersion)
        assertEquals(26, context.applicationInfo.minSdkVersion)
    }

    @Test
    fun main_activity_draws_edge_to_edge() {
        val activity = Robolectric.buildActivity(MainActivity::class.java).create().get()
        val shadow = Shadows.shadowOf(activity.window) as ShadowPhoneWindow
        assertFalse(
            shadow.getDecorFitsSystemWindows(),
            "enableEdgeToEdge() must let the window draw behind the system bars; " +
                "the Scaffold consumes the insets instead",
        )
    }

    @Test
    fun main_activity_has_no_orientation_lock() {
        val info = context.packageManager.getActivityInfo(
            ComponentName(context, MainActivity::class.java),
            0,
        )
        assertEquals(
            ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED,
            info.screenOrientation,
            "large-screen resize/orientation changes must not be blocked",
        )
    }
}
