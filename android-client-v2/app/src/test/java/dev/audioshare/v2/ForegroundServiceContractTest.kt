package dev.audioshare.v2

import org.junit.Test
import java.io.File
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * Boundary/contract tests for the beta UI. These are behavior-boundary checks
 * at source level (the seams AGP and the OS consume), not widget tests; the
 * Compose screens are covered behaviorally in AudioShareAppTest.
 */
class ForegroundServiceContractTest {
    private fun source(path: String) = File("src/main/$path").readText()

    @Test
    fun service_owns_session_instead_of_activity() {
        val manifest = source("AndroidManifest.xml")
        assertTrue(manifest.contains("android.permission.FOREGROUND_SERVICE"))
        assertTrue(manifest.contains("android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK"))
        assertTrue(manifest.contains("android:foregroundServiceType=\"mediaPlayback\""))
        assertTrue(manifest.contains("android.permission.CAMERA"))
        assertTrue(manifest.contains("android.hardware.camera\" android:required=\"false\""))

        val service = source("java/dev/audioshare/v2/ControlSessionService.kt")
        assertTrue(service.contains("override fun onBind"))
        assertTrue(service.contains("class ControlSessionService : Service(), SessionController"))
        // Ticket 21: the Service depends on the SessionEngineFactory seam. The
        // single ClientSession construction site is the hand-written factory;
        // stop attribution (the old `session?.stop(reason)`) is now asserted
        // behaviourally in SessionOrchestratorTest.
        assertEquals(
            1,
            Regex("""ClientSession\(""").findAll(service).count(),
            "the engine factory is the only ClientSession construction site",
        )
        assertTrue(service.contains("SessionEngineFactory"))
        assertTrue(service.contains("orchestrator.stop(reason)"))
        assertFalse(service.contains("session?.stop("))

        val activity = source("java/dev/audioshare/v2/MainActivity.kt")
        assertTrue(activity.contains("unbindService(connection)"))
        assertFalse(activity.contains("session?.stop()"))
        // Activity.finish keeps dispatching ACTION_STOP; the lifecycle
        // Instrumentation relies on that (service-device-test.py stop case).
        assertTrue(activity.contains("ControlSessionService.ACTION_STOP"))
    }

    /** Ticket 21: the UI binds the Binder seam only, never JNI/QUIC/Keystore. */
    @Test
    fun ui_uses_only_the_session_controller_seam() {
        val uiSources = listOf(
            "java/dev/audioshare/v2/MainActivity.kt",
            "java/dev/audioshare/v2/AudioShareApp.kt",
            "java/dev/audioshare/v2/AppState.kt",
            "java/dev/audioshare/v2/UiPreferences.kt",
        ).map(::source)
        val forbidden = listOf(
            "ClientNative",
            "ClientSession",
            "ClientIdentity",
            "ClientSigner",
            "KeystoreSigner",
            "AndroidKeyStore",
            "KeyStore",
            "createSession",
            "createAuthSession",
            "createPairingSession",
            "quinn",
            "DatagramSocket",
        )
        for ((index, text) in uiSources.withIndex()) {
            for (token in forbidden) {
                assertFalse(
                    text.contains(token),
                    "UI source #$index must not reach the session implementation directly ($token)",
                )
            }
        }
        // The pairing/credential types are interface data, not implementation.
        assertTrue(source("java/dev/audioshare/v2/MainActivity.kt").contains("SessionController"))
    }

    /**
     * Ticket 21: the foreground notification identifies a media session and
     * carries the same ACTION_STOP as the in-app Stop button.
     */
    @Test
    fun foreground_notification_offers_stop_and_opens_the_app() {
        val service = source("java/dev/audioshare/v2/ControlSessionService.kt")
        assertTrue(service.contains("PendingIntent.getService"))
        assertTrue(service.contains("setAction(ACTION_STOP)"))
        assertTrue(service.contains("PendingIntent.getActivity"))
        assertTrue(service.contains("setContentIntent(contentIntent)"))
        assertTrue(service.contains("Notification.MediaStyle"))
        assertTrue(service.contains("setMediaSession"))
        assertTrue(service.contains("CATEGORY_TRANSPORT"))
        assertTrue(service.contains("FLAG_IMMUTABLE"))
        assertTrue(service.contains("FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK"))
    }

    /** Ticket 21 acceptance: coexist with v1 and ship the beta version. */
    @Test
    fun beta_application_id_and_version() {
        val gradle = File("build.gradle.kts").readText()
        assertTrue(gradle.contains("applicationId = \"dev.audioshare.v2\""))
        assertTrue(gradle.contains("versionCode = 2"))
        assertTrue(gradle.contains("versionName = \"0.2.0-beta\""))
    }

    /** Auto-connect exists, but nothing starts the app after reboot. */
    @Test
    fun no_boot_start_receiver() {
        val manifest = source("AndroidManifest.xml")
        assertFalse(manifest.contains("RECEIVE_BOOT_COMPLETED"))
        assertFalse(manifest.contains("android.intent.action.BOOT_COMPLETED"))
        assertFalse(manifest.contains("<receiver"))
    }

    /** ticket 14: the Service really is the media-playback owner now. */
    @Test
    fun service_owns_the_playback_path_too() {
        val service = source("java/dev/audioshare/v2/ControlSessionService.kt")
        assertFalse(
            service.contains("media playback is not implemented"),
            "M4 activates playback; the notification must not claim otherwise",
        )

        val session = source("java/dev/audioshare/v2/ClientSession.kt")
        // Ticket 22: the Android adapter still constructs the platform player;
        // attach-before-connect is asserted behaviourally in
        // SessionLifecycleDriverTest because the driver owns that ordering.
        assertTrue(session.contains("MediaPlayer(NativeMediaBridge)"))

        val player = source("java/dev/audioshare/v2/MediaPlayer.kt")
        assertTrue(
            player.contains("recordPlayback"),
            "the player reports what AudioTrack actually accepted (§9)",
        )
        assertTrue(player.contains("AudioTrack"), "playback is a real AudioTrack")
    }

    /** The media ring is a single direct buffer; media never crosses JNI. */
    @Test
    fun media_ring_is_a_direct_buffer_read_by_cursor() {
        val ring = source("java/dev/audioshare/v2/MediaRingBuffer.kt")
        assertTrue(ring.contains("allocateDirect"))
        assertTrue(ring.contains("fun read(readPosition: Long"))
        assertTrue(ring.contains("fun lapped("))
        val native = source("java/dev/audioshare/v2/ClientNative.kt")
        assertTrue(native.contains("attachMediaRing"))
        assertTrue(native.contains("ringPosition"))
        assertTrue(native.contains("mediaFormatJson"))
        assertTrue(native.contains("recordPlayback"))
    }

    /** Sample formats without a direct AudioTrack encoding are refused loudly. */
    @Test
    fun unsupported_platform_sample_formats_have_no_silent_fallback() {
        val player = source("java/dev/audioshare/v2/MediaPlayer.kt")
        assertTrue(
            player.contains("S24 packed 3-byte has no direct AudioTrack encoding"),
            "S24LE has no AudioTrack encoding; the mapping must say so",
        )
        assertFalse(
            player.contains("ENCODING_PCM_24BIT"),
            "AudioFormat has no 24-bit encoding on API 26",
        )
        // The supported-format table is centralized and names the Rust gate as
        // authoritative; the mapping behaviour itself lives in MediaPlaybackTest.
        assertTrue(
            player.contains("control.rs"),
            "the Kotlin table must point at the authoritative Rust gate",
        )
        assertTrue(player.contains("SUPPORTED_SAMPLE_FORMATS"))
        assertEquals(
            1,
            player.split("fun encodingFor(sampleFormat: Int)").size - 1,
            "one centralized supported-format table",
        )
    }
}
