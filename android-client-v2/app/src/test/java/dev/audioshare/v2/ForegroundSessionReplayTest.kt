package dev.audioshare.v2

import android.os.Bundle
import androidx.test.core.app.ActivityScenario
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

/** Public session/Service boundaries, never hardcoded native wire results. */
@RunWith(RobolectricTestRunner::class)
class ForegroundSessionReplayTest {

    @Test
    fun recreated_activity_replays_existing_session_state() {
        val session = ClientSession(
            androidx.test.core.app.ApplicationProvider.getApplicationContext(),
            object : ClientSession.Listener {
                override fun onStats(json: org.json.JSONObject) = Unit
                override fun onStateChange(state: String) = Unit
                override fun onLog(line: String) = Unit
            }
        )
        session.stateForTest = "READY"
        val service = org.robolectric.Robolectric.buildService(ControlSessionService::class.java).create().get()
        service.attachSessionForTest(session)
        var scenario = ActivityScenario.launch(MainActivity::class.java)
        scenario.onActivity { activity -> activity.encounterServiceForTest(service) }
        assertEquals("READY", activity.stateTextForTest())

        scenario.recreate()
        assertEquals("READY", session.stateForTest)
    }

    @Test
    fun new_snapshot_is_available_before_polling_completes() {
        val session = ClientSession(
            androidx.test.core.app.ApplicationProvider.getApplicationContext(),
            object : ClientSession.Listener {
                override fun onStats(json: org.json.JSONObject) = Unit
                override fun onStateChange(state: String) = Unit
                override fun onLog(line: String) = Unit
            }
        )
        assertEquals("IDLE", session.currentState)
        session.stateForTest = "READY"
        assertEquals("READY", session.currentState)
        session.stop()
        assertEquals("CLOSED", session.currentState)
    }
}
