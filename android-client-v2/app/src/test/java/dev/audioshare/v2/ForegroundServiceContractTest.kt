package dev.audioshare.v2

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue
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
        val activity = File("src/main/java/dev/audioshare/v2/MainActivity.kt").readText()
        assert(activity.contains("unbindService(connection)"))
        assertFalse(activity.contains("session?.stop()"))
    }

    /** ticket 14: the Service really is the media-playback owner now. */
    @Test
    fun service_owns_the_playback_path_too() {
        val service = File("src/main/java/dev/audioshare/v2/ControlSessionService.kt").readText()
        assertFalse(
            service.contains("media playback is not implemented"),
            "M4 activates playback; the notification must not claim otherwise",
        )

        val session = File("src/main/java/dev/audioshare/v2/ClientSession.kt").readText()
        assertTrue(session.contains("MediaPlayer(NativeMediaBridge)"))
        assertTrue(
            session.contains("it.attach(h)"),
            "the ring is attached once per native session, before connect()",
        )

        val player = File("src/main/java/dev/audioshare/v2/MediaPlayer.kt").readText()
        assertTrue(
            player.contains("recordPlayback"),
            "the player reports what AudioTrack actually accepted (§9)",
        )
        assertTrue(player.contains("AudioTrack"), "playback is a real AudioTrack")
    }

    /** The media ring is a single direct buffer; media never crosses JNI. */
    @Test
    fun media_ring_is_a_direct_buffer_read_by_cursor() {
        val ring = File("src/main/java/dev/audioshare/v2/MediaRingBuffer.kt").readText()
        assertTrue(ring.contains("allocateDirect"))
        assertTrue(ring.contains("fun read(readPosition: Long"))
        assertTrue(ring.contains("fun lapped("))
        val native = File("src/main/java/dev/audioshare/v2/ClientNative.kt").readText()
        assertTrue(native.contains("attachMediaRing"))
        assertTrue(native.contains("ringPosition"))
        assertTrue(native.contains("mediaFormatJson"))
        assertTrue(native.contains("recordPlayback"))
    }

    /** Sample formats without a direct AudioTrack encoding are refused loudly. */
    @Test
    fun unsupported_platform_sample_formats_have_no_silent_fallback() {
        val player = File("src/main/java/dev/audioshare/v2/MediaPlayer.kt").readText()
        assertTrue(
            player.contains("S24 packed 3-byte has no direct AudioTrack encoding"),
            "S24LE has no AudioTrack encoding; the mapping must say so",
        )
        assertFalse(
            player.contains("ENCODING_PCM_24BIT"),
            "AudioFormat has no 24-bit encoding on API 26",
        )
        assertEquals(1, player.split("val encoding = when (sampleFormat)").size - 1)
    }
}
