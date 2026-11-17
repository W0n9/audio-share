package dev.audioshare.v2

import android.content.Context
import android.content.pm.ActivityInfo
import androidx.activity.ComponentActivity
import androidx.annotation.StringRes
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.core.app.ApplicationProvider
import org.junit.Rule
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.Description
import org.junit.runners.model.Statement
import org.robolectric.Shadows.shadowOf

/**
 * Shared Robolectric Compose harness for the ticket-18 screen tests. The seam
 * is the same one the app uses: `AudioShareApp(state, onEvent)` plus the pure
 * reducer.
 */
abstract class RobolectricComposeTestBase {
    protected val compose = createAndroidComposeRule<ComponentActivity>()
    protected val events = mutableListOf<UiEvent>()

    protected val context: Context get() = ApplicationProvider.getApplicationContext()

    protected fun str(@StringRes id: Int, vararg args: Any): String =
        context.getString(id, *args)

    /**
     * Bare `ComponentActivity` hosts the content; register it with Robolectric
     * before the compose rule launches its ActivityScenarioRule.
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

    protected fun start(initial: AppState) {
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
}
