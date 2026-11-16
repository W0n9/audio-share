package dev.audioshare.v2

import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.MultiFormatReader
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.Result
import com.google.zxing.common.HybridBinarizer

/**
 * Pure QR decode seam for the CameraX analyzer. It takes raw luminance bytes
 * (not a Bitmap) so the same code is exercised by a JVM test that encodes a
 * payload with ZXing's own writer. Returns null for frames without a QR code.
 */
object QrPayloadDecoder {
    private val hints: Map<DecodeHintType, Any> = mapOf(
        DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE),
        DecodeHintType.TRY_HARDER to true,
    )

    fun decode(
        luminance: ByteArray,
        width: Int,
        height: Int,
        rowStride: Int = width,
    ): String? {
        if (width <= 0 || height <= 0 || rowStride < width) return null
        if (luminance.size < rowStride * height) return null
        return try {
            val source = PlanarYUVLuminanceSource(
                luminance, rowStride, height, 0, 0, width, height, false,
            )
            val bitmap = BinaryBitmap(HybridBinarizer(source))
            val result: Result = MultiFormatReader().apply { setHints(hints) }.decode(bitmap)
            result.text
        } catch (_: Exception) {
            // NotFoundException is the normal "no QR in this frame" outcome;
            // malformed/unsupported frames are equally non-fatal.
            null
        }
    }
}
