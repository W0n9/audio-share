package dev.audioshare.v2

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.foundation.layout.fillMaxSize

/** Minimal beta theme: system Material3 defaults, no branding or telemetry. */
@Composable
fun AudioShareTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = lightColorScheme()) {
        Surface(modifier = Modifier.fillMaxSize(), content = content)
    }
}
