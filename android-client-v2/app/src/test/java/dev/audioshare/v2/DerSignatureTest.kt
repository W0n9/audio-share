package dev.audioshare.v2

import java.security.KeyPairGenerator
import java.security.Signature
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import org.junit.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue

class DerSignatureTest {
    private fun keyPair() = KeyPairGenerator.getInstance("EC").apply {
        initialize(ECGenParameterSpec("secp256r1"))
    }.generateKeyPair()

    @Test
    fun jca_der_signature_round_trips_through_p1363() {
        val pair = keyPair()
        val context = "raw pairing transcript".toByteArray()
        val signer = Signature.getInstance("SHA256withECDSA")
        signer.initSign(pair.private)
        signer.update(context)
        val der = signer.sign()
        val p1363 = DerSignature.toP1363(der)
        assertEquals(64, p1363.size)

        val verifier = Signature.getInstance("SHA256withECDSA")
        verifier.initVerify(pair.public)
        verifier.update(context)
        assertTrue(verifier.verify(DerSignature.toDer(p1363)))
    }

    @Test
    fun leading_zero_padding_and_short_integers_are_right_aligned() {
        // r = 0x00 0x00 0x01 (padded to 2), s = 0x02 (short)
        val der = byteArrayOf(
            0x30, 0x07,
            0x02, 0x02, 0x00, 0x01,
            0x02, 0x01, 0x02,
        )
        val p1363 = DerSignature.toP1363(der)
        assertEquals(1, p1363[31])
        assertEquals(2, p1363[31 + 32])
        assertTrue(p1363.copyOfRange(0, 31).all { it == 0.toByte() })
    }

    @Test
    fun der_encoding_adds_a_zero_pad_when_the_high_bit_is_set() {
        val p1363 = ByteArray(64)
        p1363[0] = 0x80.toByte()
        p1363[32] = 0x7f
        val der = DerSignature.toDer(p1363)
        // The INTEGER body is 33 bytes, so the length octet after 0x02 is 33.
        assertEquals(0x02, der[2].toInt())
        assertEquals(33, der[3].toInt())
        assertEquals(0, der[4].toInt(), "high-bit integer needs a leading zero")
        assertContentEquals(p1363, DerSignature.toP1363(der))
    }

    @Test
    fun oversized_and_malformed_der_are_rejected() {
        val oversized = byteArrayOf(0x30, 0x25, 0x02, 0x21) + ByteArray(33) { 1 } +
            byteArrayOf(0x02, 0x00)
        assertFailsWith<IllegalArgumentException> { DerSignature.toP1363(oversized) }
        assertFailsWith<IllegalArgumentException> { DerSignature.toP1363(byteArrayOf(0x31)) }
        assertFailsWith<IllegalArgumentException> { DerSignature.toDer(ByteArray(63)) }
    }

    @Test
    fun sec1_compression_uses_y_parity_and_fixed_32_byte_coordinates() {
        val pair = keyPair()
        val compressed = EcKeys.sec1Compressed(pair.public as ECPublicKey)
        assertEquals(33, compressed.size)
        val prefix = compressed[0].toInt()
        val yOdd = (pair.public as ECPublicKey).w.affineY.testBit(0)
        assertEquals(if (yOdd) 0x03 else 0x02, prefix)
    }
}
