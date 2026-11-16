package dev.audioshare.v2

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyProperties
import java.io.File
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.ProviderException
import java.security.PublicKey
import java.security.Signature
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import java.security.spec.PKCS8EncodedKeySpec
import java.security.spec.X509EncodedKeySpec

/**
 * The Kotlin side of the native identity-signing seam (pairing.md §10.1).
 *
 * The native crate calls [publicKeySec1] and [sign] over JNI. [sign] receives
 * the RAW transcript bytes and must apply exactly one SHA-256 (the one inside
 * `SHA256withECDSA`) and return fixed 64-byte `r || s` (P1363).
 */
interface ClientSigner {
    /** The 33-byte SEC1 compressed P-256 public key. */
    fun publicKeySec1(): ByteArray

    /** ECDSA-P256-SHA256 over the raw bytes; 64-byte `r || s`. */
    fun sign(rawContext: ByteArray): ByteArray
}

/** DER ⇄ P1363 signature conversion for the JCA `SHA256withECDSA` output. */
object DerSignature {
    fun toP1363(der: ByteArray): ByteArray {
        require(der.size >= 2 && der[0] == 0x30.toByte()) { "missing DER SEQUENCE" }
        val (seqLen, seqHeader) = derLength(der, 1)
        val body = der.copyOfRange(1 + seqHeader, der.size)
        require(seqLen == body.size) { "DER sequence length mismatch" }
        val (r, rEnd) = derInteger(body, 0)
        val (s, sEnd) = derInteger(body, rEnd)
        require(sEnd == body.size) { "trailing DER bytes" }
        val out = ByteArray(64)
        copyInteger(r, out, 0)
        copyInteger(s, out, 32)
        return out
    }

    fun toDer(p1363: ByteArray): ByteArray {
        require(p1363.size == 64) { "signature must be 64 bytes, got ${p1363.size}" }
        val r = derIntegerBytes(p1363.copyOfRange(0, 32))
        val s = derIntegerBytes(p1363.copyOfRange(32, 64))
        val body = byteArrayOf(0x02) + encodeLength(r.size) + r +
            byteArrayOf(0x02) + encodeLength(s.size) + s
        return byteArrayOf(0x30) + encodeLength(body.size) + body
    }

    private fun derLength(buf: ByteArray, offset: Int): Pair<Int, Int> {
        require(offset < buf.size) { "missing DER length" }
        val first = buf[offset].toInt() and 0xff
        if (first < 0x80) return first to 1
        val count = first and 0x7f
        require(count in 1..2 && offset + count < buf.size) { "unsupported DER length" }
        var length = 0
        for (i in 1..count) length = (length shl 8) or (buf[offset + i].toInt() and 0xff)
        return length to (1 + count)
    }

    private fun derInteger(buf: ByteArray, offset: Int): Pair<ByteArray, Int> {
        require(offset + 2 <= buf.size && buf[offset] == 0x02.toByte()) {
            "expected DER INTEGER"
        }
        val (length, header) = derLength(buf, offset + 1)
        val start = offset + 1 + header
        val end = start + length
        require(end <= buf.size) { "truncated DER INTEGER" }
        var value = buf.copyOfRange(start, end)
        if (value.size > 1 && value[0] == 0x00.toByte()) value = value.copyOfRange(1, value.size)
        require(value.size <= 32) { "DER integer wider than 32 bytes" }
        return value to end
    }

    private fun copyInteger(value: ByteArray, out: ByteArray, offset: Int) {
        require(value.size <= 32) { "integer does not fit 32 bytes" }
        value.copyInto(out, offset + 32 - value.size)
    }

    private fun derIntegerBytes(value: ByteArray): ByteArray {
        var start = 0
        while (start + 1 < value.size && value[start] == 0x00.toByte()) start++
        val stripped = value.copyOfRange(start, value.size)
        val needsPad = (stripped[0].toInt() and 0x80) != 0
        return if (needsPad) byteArrayOf(0x00) + stripped else stripped
    }

    private fun encodeLength(length: Int): ByteArray = when {
        length < 0x80 -> byteArrayOf(length.toByte())
        length <= 0xff -> byteArrayOf(0x81.toByte(), length.toByte())
        else -> byteArrayOf(0x82.toByte(), (length shr 8).toByte(), length.toByte())
    }
}

/** SEC1 compressed encoding of an EC public key. */
object EcKeys {
    fun sec1Compressed(publicKey: ECPublicKey): ByteArray {
        val x = fixed32(publicKey.w.affineX.toByteArray())
        val y = fixed32(publicKey.w.affineY.toByteArray())
        val prefix = if (y[31].toInt() and 1 == 1) 0x03 else 0x02
        return byteArrayOf(prefix.toByte()) + x
    }

    private fun fixed32(value: ByteArray): ByteArray {
        if (value.size == 32) return value
        if (value.size > 32) return value.copyOfRange(value.size - 32, value.size)
        return ByteArray(32 - value.size) + value
    }
}

/** The result of provisioning or loading a Client identity. */
data class ClientIdentityResult(
    val signer: ClientSigner,
    val alias: String,
    /** The documented degradation (pairing.md §10.1): no Keystore key. */
    val softwareBacked: Boolean,
    /** Best-effort diagnostic: the Keystore key lives in secure hardware. */
    val hardwareBacked: Boolean,
)

/**
 * Provisions the Client identity in Android Keystore, falling back to
 * StrongBox-less Keystore and finally to the documented app-private software
 * key. The software key is a real P-256 identity marked `software_backed`; it
 * is never a test fixture.
 */
object ClientIdentity {
    internal const val ANDROID_KEYSTORE = "AndroidKeyStore"
    private const val EC_CURVE = "secp256r1"

    fun load(context: Context, alias: String): ClientIdentityResult {
        SoftwareIdentitySigner.load(context, alias)?.let { signer ->
            return ClientIdentityResult(
                signer = signer,
                alias = alias,
                softwareBacked = true,
                hardwareBacked = false,
            )
        }
        if (!keystoreContains(alias)) {
            throw IllegalStateException("no stored Client identity for alias $alias")
        }
        val signer = KeystoreSigner(alias)
        return ClientIdentityResult(
            signer = signer,
            alias = alias,
            softwareBacked = false,
            hardwareBacked = signer.isHardwareBacked(),
        )
    }

    /** Load an existing identity or create one, preferring Keystore. */
    fun create(context: Context, alias: String): ClientIdentityResult {
        runCatching { return load(context, alias) }
        val strongBox = tryGenerateKeystore(alias, preferStrongBox = true)
        if (strongBox) {
            val signer = KeystoreSigner(alias)
            return ClientIdentityResult(signer, alias, softwareBacked = false, hardwareBacked = signer.isHardwareBacked())
        }
        val plain = tryGenerateKeystore(alias, preferStrongBox = false)
        if (plain) {
            val signer = KeystoreSigner(alias)
            return ClientIdentityResult(signer, alias, softwareBacked = false, hardwareBacked = signer.isHardwareBacked())
        }
        val signer = SoftwareIdentitySigner.create(context, alias)
        return ClientIdentityResult(signer, alias, softwareBacked = true, hardwareBacked = false)
    }

    private fun keystoreContains(alias: String): Boolean = try {
        val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        keyStore.containsAlias(alias) && keyStore.getKey(alias, null) != null
    } catch (_: Exception) {
        false
    }

    private fun tryGenerateKeystore(alias: String, preferStrongBox: Boolean): Boolean {
        return try {
            val builder = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_SIGN)
                .setAlgorithmParameterSpec(ECGenParameterSpec(EC_CURVE))
                .setDigests(KeyProperties.DIGEST_SHA256)
                .setUserAuthenticationRequired(false)
            if (preferStrongBox && Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                builder.setIsStrongBoxBacked(true)
            }
            val generator = KeyPairGenerator.getInstance(
                KeyProperties.KEY_ALGORITHM_EC,
                ANDROID_KEYSTORE,
            )
            generator.initialize(builder.build())
            generator.generateKeyPair()
            true
        } catch (_: ProviderException) {
            // StrongBoxUnavailableException (API 28+) is a ProviderException;
            // fall back to ordinary Keystore on every API level.
            false
        } catch (_: Exception) {
            false
        }
    }
}

/** Android Keystore-backed signer: the private key is non-exportable. */
internal class KeystoreSigner(private val alias: String) : ClientSigner {
    private val privateKey: PrivateKey
    private val sec1: ByteArray

    init {
        val keyStore = KeyStore.getInstance(ClientIdentity.ANDROID_KEYSTORE).apply { load(null) }
        privateKey = keyStore.getKey(alias, null) as PrivateKey
        val publicKey = keyStore.getCertificate(alias)?.publicKey
            ?: throw IllegalStateException("Keystore alias $alias has no certificate")
        sec1 = EcKeys.sec1Compressed(publicKey as ECPublicKey)
    }

    override fun publicKeySec1(): ByteArray = sec1.copyOf()

    override fun sign(rawContext: ByteArray): ByteArray {
        val signature = Signature.getInstance("SHA256withECDSA")
        signature.initSign(privateKey)
        // Exactly one hash: SHA256withECDSA hashes the RAW context. Never
        // pre-hash and never use NONEwithECDSA.
        signature.update(rawContext)
        return DerSignature.toP1363(signature.sign())
    }

    fun isHardwareBacked(): Boolean = try {
        val factory = KeyFactory.getInstance(privateKey.algorithm, ClientIdentity.ANDROID_KEYSTORE)
        val info = factory.getKeySpec(privateKey, KeyInfo::class.java) as KeyInfo
        info.isInsideSecureHardware
    } catch (_: Exception) {
        false
    }
}

/**
 * The documented software fallback (pairing.md §10.1): a real P-256 key in
 * app-private storage. The pairing record is marked `software_backed`.
 */
internal class SoftwareIdentitySigner private constructor(
    private val privateKey: PrivateKey,
    private val publicKey: PublicKey,
) : ClientSigner {
    private val sec1 = EcKeys.sec1Compressed(publicKey as ECPublicKey)

    override fun publicKeySec1(): ByteArray = sec1.copyOf()

    override fun sign(rawContext: ByteArray): ByteArray {
        val signature = Signature.getInstance("SHA256withECDSA")
        signature.initSign(privateKey)
        signature.update(rawContext)
        return DerSignature.toP1363(signature.sign())
    }

    companion object {
        fun create(context: Context, alias: String): SoftwareIdentitySigner {
            val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC)
            generator.initialize(ECGenParameterSpec("secp256r1"))
            val pair = generator.generateKeyPair()
            val (privateFile, publicFile) = files(context, alias)
            writePrivate(privateFile, pair.private.encoded)
            writePrivate(publicFile, pair.public.encoded)
            return SoftwareIdentitySigner(pair.private, pair.public)
        }

        fun load(context: Context, alias: String): SoftwareIdentitySigner? {
            val (privateFile, publicFile) = files(context, alias)
            if (!privateFile.isFile || !publicFile.isFile) return null
            return try {
                val keyFactory = KeyFactory.getInstance(KeyProperties.KEY_ALGORITHM_EC)
                val privateKey = keyFactory.generatePrivate(
                    PKCS8EncodedKeySpec(privateFile.readBytes()),
                )
                val publicKey = keyFactory.generatePublic(
                    X509EncodedKeySpec(publicFile.readBytes()),
                )
                SoftwareIdentitySigner(privateKey, publicKey)
            } catch (_: Exception) {
                null
            }
        }

        private fun files(context: Context, alias: String): Pair<File, File> {
            val safe = alias.replace(Regex("[^A-Za-z0-9._-]"), "_")
            val base = File(context.filesDir, "client-identity-$safe")
            return File(base.path + ".priv") to File(base.path + ".pub")
        }

        private fun writePrivate(file: File, bytes: ByteArray) {
            file.outputStream().use { it.write(bytes) }
            // App-private storage; make the intent explicit (0600 where the
            // filesystem honours POSIX modes).
            file.setReadable(false, false)
            file.setWritable(false, false)
            file.setReadable(true, true)
            file.setWritable(true, true)
        }
    }
}
