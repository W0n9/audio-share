package dev.audioshare.v2

import org.junit.Test
import java.io.File
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * Source-scan contract test for the facts a JVM test cannot otherwise observe:
 * the manifest/OS surface and the packaging identity.
 *
 * Ticket 26 keeps only structure/seam invariants here. Implementation shape
 * and notification/copy literals moved to behaviour tests: session lifecycle
 * ordering is covered by `SessionLifecycleDriverTest`, service orchestration by
 * `SessionOrchestratorTest`, and rendered screens by the Robolectric Compose
 * tests. Do not add assertions about private call sites or Kotlin string
 * literals back to this file.
 */
class ForegroundServiceContractTest {
    private fun source(path: String) = File("src/main/$path").readText()

    @Test
    fun manifest_declares_the_media_playback_service_and_permissions() {
        val manifest = source("AndroidManifest.xml")
        assertTrue(manifest.contains("android.permission.FOREGROUND_SERVICE"))
        assertTrue(manifest.contains("android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK"))
        assertTrue(manifest.contains("android:foregroundServiceType=\"mediaPlayback\""))
        assertTrue(manifest.contains("android.permission.CAMERA"))
        assertTrue(manifest.contains("android.hardware.camera\" android:required=\"false\""))
        assertTrue(manifest.contains("android.hardware.camera.any\" android:required=\"false\""))
    }

    /** Auto-connect exists, but nothing starts the app after reboot. */
    @Test
    fun no_boot_start_receiver() {
        val manifest = source("AndroidManifest.xml")
        assertFalse(manifest.contains("RECEIVE_BOOT_COMPLETED"))
        assertFalse(manifest.contains("android.intent.action.BOOT_COMPLETED"))
        assertFalse(manifest.contains("<receiver"))
    }

    /** Ticket 21 acceptance: coexist with v1 and ship the current client version. */
    @Test
    fun application_id_and_version() {
        val gradle = File("build.gradle.kts").readText()
        assertTrue(gradle.contains("applicationId = \"dev.audioshare.v2\""))
        assertTrue(gradle.contains("versionCode = 5"))
        assertTrue(gradle.contains("versionName = \"0.6.2\""))
    }

    /**
     * Seam invariant: the UI binds the `SessionController` Binder only. It may
     * see interface data types but must never reach the native/transport or
     * Keystore implementations directly.
     */
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
}
