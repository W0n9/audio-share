package dev.audioshare.v2

import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import java.util.Base64
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

/**
 * The QR camera path's decode seam: ZXing's own encoder produces a real QR
 * matrix, the production luminance decoder reads it back, and the result must
 * parse as a normative `audioshare-pair://p1` payload. This is an independent
 * oracle (encoder in tests, decoder in production), not a round-trip stub.
 */
class QrPayloadDecoderTest {

    @Test
    fun zxing_qr_round_trips_into_a_valid_pairing_payload() {
        val encoder = Base64.getUrlEncoder().withoutPadding()
        val key = ByteArray(32) { (it * 7).toByte() }
        val nonce = ByteArray(16) { (255 - it).toByte() }
        val uri = "audioshare-pair://p1?pm=2&addr=10.0.2.2:65531" +
            "&key=${encoder.encodeToString(key)}&nonce=${encoder.encodeToString(nonce)}"

        val matrix = QRCodeWriter().encode(
            uri,
            BarcodeFormat.QR_CODE,
            240,
            240,
            mapOf(EncodeHintType.MARGIN to 2, EncodeHintType.CHARACTER_SET to "UTF-8"),
        )
        val luminance = ByteArray(matrix.width * matrix.height) { index ->
            val x = index % matrix.width
            val y = index / matrix.width
            if (matrix.get(x, y)) 0 else 255.toByte()
        }

        val decoded = QrPayloadDecoder.decode(luminance, matrix.width, matrix.height)
        assertEquals(uri, decoded)

        val payload = PairingPayload.parse(decoded!!)
        assertEquals("10.0.2.2:65531", payload.address)
        assertEquals("10.0.2.2", payload.host)
        assertEquals(65531, payload.port)
    }

    @Test
    fun blank_frames_have_no_payload() {
        assertNull(QrPayloadDecoder.decode(ByteArray(120 * 120) { 255.toByte() }, 120, 120))
    }
}
