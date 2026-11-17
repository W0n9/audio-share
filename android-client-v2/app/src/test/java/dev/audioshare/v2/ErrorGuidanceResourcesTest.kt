package dev.audioshare.v2

import android.content.Context
import android.content.res.Configuration
import androidx.test.core.app.ApplicationProvider
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.util.Locale
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue

/**
 * The code -> resource seam (ticket 18 item 3): every registered §6 code has
 * its own `error_<code>_title` / `error_<code>_guidance`, and unknown/null
 * codes use the documented `error_generic_*` fallback. English copy keeps the
 * recovery facts the old Kotlin strings carried.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class ErrorGuidanceResourcesTest {

    private val context: Context get() = ApplicationProvider.getApplicationContext()

    @Test
    fun every_registered_code_maps_to_non_blank_specific_copy() {
        for (guidance in ErrorGuidanceBook.all()) {
            val code = guidance.code
            assertNotEquals(
                R.string.error_generic_title,
                ErrorGuidanceResources.titleRes(code),
                "code $code must not use the generic title",
            )
            assertNotEquals(
                R.string.error_generic_guidance,
                ErrorGuidanceResources.guidanceRes(code),
                "code $code must not use the generic guidance",
            )
            assertTrue(context.getString(ErrorGuidanceResources.titleRes(code)).isNotBlank())
            assertTrue(context.getString(ErrorGuidanceResources.guidanceRes(code)).isNotBlank())
        }
    }

    @Test
    fun unknown_and_null_codes_use_the_documented_generic_fallback() {
        for (code in listOf(null, 0, 9999)) {
            assertEquals(R.string.error_generic_title, ErrorGuidanceResources.titleRes(code))
            assertEquals(R.string.error_generic_guidance, ErrorGuidanceResources.guidanceRes(code))
        }
        assertTrue(context.getString(R.string.error_generic_title).isNotBlank())
        assertTrue(context.getString(R.string.error_generic_guidance).isNotBlank())
    }

    @Test
    fun every_address_error_maps_to_non_blank_copy() {
        for (error in AddressError.values()) {
            assertTrue(context.getString(error.messageRes()).isNotBlank(), "missing copy for $error")
        }
        assertTrue(context.getString(AddressError.MISSING_PORT.messageRes()).contains("port"))
    }

    @Test
    fun english_copy_keeps_the_recovery_facts() {
        val rejected = context.getString(R.string.error_201_guidance)
        assertTrue(rejected.contains("fresh QR code"), "201 must direct the user to a new invitation")
        assertTrue(
            rejected.contains("accept this device", ignoreCase = true),
            "the Server Accept gate stays visible after a decline",
        )

        assertEquals("Pairing timed out", context.getString(R.string.error_206_title))
        val timedOut = context.getString(R.string.error_206_guidance)
        assertTrue(timedOut.contains("ten-minute"))
        assertTrue(timedOut.contains("new pairing"))
        assertTrue(timedOut.contains("fresh QR code"))

        assertEquals("This device was revoked", context.getString(R.string.error_204_title))
        assertEquals("Connection lost", context.getString(R.string.error_601_title))
    }

    @Test
    fun chinese_copy_is_localized_not_the_english_default() {
        val config = Configuration(context.resources.configuration).apply {
            setLocale(Locale.SIMPLIFIED_CHINESE)
        }
        val zh = context.createConfigurationContext(config)
        assertEquals("配对被拒绝", zh.getString(R.string.error_201_title))
        assertEquals("设置", zh.getString(R.string.action_settings))
        assertNotEquals(
            context.getString(R.string.about_v1_warning),
            zh.getString(R.string.about_v1_warning),
        )
    }
}
