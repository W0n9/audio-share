package dev.audioshare.v2

import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicReference

/**
 * CameraX preview + ZXing analysis for the pairing QR. No Google Play
 * Services: CameraX supplies camera2 frames, [QrPayloadDecoder] reads QR
 * luminance. The caller owns the runtime CAMERA permission and the paste-link
 * fallback; this composable only runs once permission is granted.
 */
@Composable
fun QrScanner(
    modifier: Modifier = Modifier,
    onPayload: (String) -> Unit,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val executor = remember { Executors.newSingleThreadExecutor() }
    val previewView = remember {
        PreviewView(context).apply {
            implementationMode = PreviewView.ImplementationMode.COMPATIBLE
            scaleType = PreviewView.ScaleType.FILL_CENTER
        }
    }
    val lastPayload = remember { AtomicReference<String?>(null) }
    val currentOnPayload by rememberUpdatedState(onPayload)

    AndroidView(factory = { previewView }, modifier = modifier)

    LaunchedEffect(previewView, lifecycleOwner, executor) {
        val future = ProcessCameraProvider.getInstance(context)
        future.addListener({
            val provider = try {
                future.get()
            } catch (_: Exception) {
                return@addListener
            }
            val preview = Preview.Builder().build().also {
                it.setSurfaceProvider(previewView.surfaceProvider)
            }
            val analysis = ImageAnalysis.Builder()
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .build()
            analysis.setAnalyzer(executor) { proxy ->
                val text = decodeFrame(proxy)
                if (text != null && text.startsWith("${PairingPayload.SCHEME}://") &&
                    lastPayload.getAndSet(text) != text
                ) {
                    previewView.post { currentOnPayload(text) }
                }
                proxy.close()
            }
            try {
                provider.unbindAll()
                provider.bindToLifecycle(
                    lifecycleOwner,
                    CameraSelector.DEFAULT_BACK_CAMERA,
                    preview,
                    analysis,
                )
            } catch (_: Exception) {
                // No camera (emulator without a virtual scene) is not fatal:
                // the paste-link fallback remains available.
            }
        }, ContextCompat.getMainExecutor(context))
    }

    DisposableEffect(previewView) {
        onDispose {
            executor.shutdown()
            try {
                ProcessCameraProvider.getInstance(context).get().unbindAll()
            } catch (_: Exception) {
            }
        }
    }
}

/** Y-plane luminance → ZXing; shared with the JVM decoder test via the seam. */
internal fun decodeFrame(proxy: ImageProxy): String? = try {
    val plane = proxy.planes.firstOrNull()
    if (plane == null) {
        null
    } else {
        val buffer = plane.buffer
        val data = ByteArray(buffer.remaining())
        buffer.get(data)
        QrPayloadDecoder.decode(data, proxy.width, proxy.height, plane.rowStride)
    }
} catch (_: Exception) {
    null
}
