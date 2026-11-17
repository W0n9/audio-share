package dev.audioshare.v2

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier

/**
 * Static Material 3 beta theme: default typography, no dynamic color, no
 * in-app theme setting. Light/dark follows the system setting; the XML window
 * theme (`values` + `values-night`) changes with it so launch windows match.
 *
 * [darkTheme] is a seam for Robolectric/Compose tests only.
 */
@Composable
fun AudioShareTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    MaterialTheme(
        colorScheme = if (darkTheme) darkColorScheme() else lightColorScheme(),
    ) {
        Surface(modifier = Modifier.fillMaxSize(), content = content)
    }
}
