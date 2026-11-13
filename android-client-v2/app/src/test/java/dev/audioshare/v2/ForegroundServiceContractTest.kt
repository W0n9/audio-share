package dev.audioshare.v2

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import java.io.File

class ForegroundServiceContractTest {
    @Test
    fun service_owns_session_instead_of_activity() {
        val manifest = File("src/main/AndroidManifest.xml").readText()
        assert(manifest.contains("android.permission.FOREGROUND_SERVICE"))
        assert(manifest.contains("android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK"))
        assert(manifest.contains("android:foregroundServiceType=\"mediaPlayback\""))
        val service = File("src/main/java/dev/audioshare/v2/ControlSessionService.kt").readText()
        assert(service.contains("override fun onBind"))
        assert(service.contains("session?.stop()"))
        assert(service.contains("media playback is not implemented"))
        val activity = File("src/main/java/dev/audioshare/v2/MainActivity.kt").readText()
        assert(activity.contains("unbindService(connection)"))
        assertFalse(activity.contains("session?.stop()"))
    }
}
