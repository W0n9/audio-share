package dev.audioshare.v2

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

private val FORGET_PIN = "ab".repeat(32)

private fun forgettableRecord() = CredentialRecord(
    serverSpkiSha256Hex = FORGET_PIN,
    serverPublicKeyHex = "cd".repeat(32),
    addressHint = "10.0.2.2:65531",
    keyAlias = "v2-key",
    pairedAt = "2026-11-16T00:00:00Z",
    softwareBacked = true,
    clientId = "client-1",
    displayName = "Pixel",
    host = "10.0.2.2",
    port = 65531,
)

/** Records call order so the credential delete cannot be hidden by a key failure. */
private class OrderRecordingStore(private val record: CredentialRecord?) : CredentialStore {
    private var current = record
    val calls = mutableListOf<String>()

    override fun upsert(record: CredentialRecord) {
        current = record
    }

    override fun all(): List<CredentialRecord> = listOfNotNull(current)

    override fun find(serverSpkiSha256Hex: String): CredentialRecord? =
        current?.takeIf { it.serverSpkiSha256Hex == serverSpkiSha256Hex }

    override fun delete(serverSpkiSha256Hex: String) {
        calls += "delete:$serverSpkiSha256Hex"
        current = null
    }
}

class ForgetServerTest {

    @Test
    fun forget_deletes_the_credential_then_its_identity_key() {
        val store = OrderRecordingStore(forgettableRecord())
        val order = mutableListOf<String>()
        val forget = ForgetServer(
            credentials = store,
            deleteIdentity = { alias -> order += "key:$alias" },
        )

        val removed = forget.forget(FORGET_PIN)

        assertEquals(forgettableRecord(), removed)
        assertNull(store.find(FORGET_PIN))
        assertEquals(listOf("delete:$FORGET_PIN"), store.calls)
        assertEquals(listOf("key:v2-key"), order)
    }

    @Test
    fun a_key_deletion_failure_never_undoes_or_masks_the_credential_delete() {
        val store = OrderRecordingStore(forgettableRecord())
        val logged = mutableListOf<String>()
        val forget = ForgetServer(
            credentials = store,
            deleteIdentity = { throw IllegalStateException("keystore unavailable") },
            log = { logged += it },
        )

        val result = runCatching { forget.forget(FORGET_PIN) }

        assertTrue(result.isSuccess, "a key error must not abort the Forget")
        assertNull(store.find(FORGET_PIN), "the credential delete stands")
        assertEquals(listOf("delete:$FORGET_PIN"), store.calls)
        assertTrue(logged.single().contains("keystore unavailable"))
    }

    @Test
    fun forgetting_an_unknown_server_deletes_nothing() {
        val store = OrderRecordingStore(null)
        var keyCalls = 0
        val forget = ForgetServer(
            credentials = store,
            deleteIdentity = { keyCalls += 1 },
        )

        assertNull(forget.forget(FORGET_PIN))
        assertEquals(emptyList(), store.calls)
        assertEquals(0, keyCalls)
    }
}

/**
 * The real Android adapter: a software-backed identity is two app-private
 * files; a Keystore alias is an entry. Delete must handle both without
 * requiring the credential to remember which one it used.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [36])
class ClientIdentityKeyDeletionTest {

    @Test
    fun delete_removes_a_software_backed_identity() {
        val context: Context = ApplicationProvider.getApplicationContext()
        SoftwareIdentitySigner.create(context, "v2-forget-test")
        assertTrue(
            SoftwareIdentitySigner.load(context, "v2-forget-test") != null,
            "the identity exists before Forget",
        )

        ClientIdentity.delete(context, "v2-forget-test")

        assertNull(
            SoftwareIdentitySigner.load(context, "v2-forget-test"),
            "both key files are gone",
        )
    }

    @Test
    fun delete_is_idempotent_for_an_absent_alias() {
        val context: Context = ApplicationProvider.getApplicationContext()
        ClientIdentity.delete(context, "v2-never-created")
        ClientIdentity.delete(context, "v2-never-created")
    }
}
