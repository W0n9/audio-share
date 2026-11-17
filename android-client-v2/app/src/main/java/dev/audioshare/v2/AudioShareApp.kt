package dev.audioshare.v2

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat

/**
 * The whole beta UI is a pure function of [AppState]; it emits [UiEvent]s and
 * never touches the session implementation. The Activity owns binding and
 * command execution.
 */
@Composable
fun AudioShareApp(
    state: AppState,
    versionName: String,
    versionCode: Int,
    onEvent: (UiEvent) -> Unit,
) {
    Column(modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp)) {
        when (state.screen) {
            AppScreen.HOME -> HomeScreen(state, onEvent)
            AppScreen.PAIRING -> PairingScreen(state, onEvent)
            AppScreen.SESSION -> SessionScreen(state, onEvent)
            AppScreen.ABOUT -> AboutScreen(versionName, versionCode, onEvent)
        }
    }
}

@Composable
private fun HomeScreen(state: AppState, onEvent: (UiEvent) -> Unit) {
    Text("Audio Share (beta)", style = MaterialTheme.typography.headlineSmall)
    Text(
        "Encrypted audio from your Server, on your phone.",
        style = MaterialTheme.typography.bodyMedium,
    )
    Spacer(Modifier.height(16.dp))

    SessionStatus(state)

    Spacer(Modifier.height(16.dp))
    OutlinedTextField(
        value = state.address,
        onValueChange = { onEvent(UiEvent.AddressChanged(it)) },
        label = { Text("Server address") },
        placeholder = { Text("host:port") },
        singleLine = true,
        isError = state.addressError != null,
        modifier = Modifier.fillMaxWidth().testTag("address-input"),
    )
    if (state.addressError != null) {
        Text(
            state.addressError,
            color = MaterialTheme.colorScheme.error,
            style = MaterialTheme.typography.bodySmall,
        )
    }

    Spacer(Modifier.height(16.dp))
    if (state.hasStoredPairing) {
        Button(
            onClick = { onEvent(UiEvent.ConnectRequested) },
            modifier = Modifier.fillMaxWidth().testTag("connect"),
        ) {
            Text("Connect to saved Server")
        }
        Spacer(Modifier.height(8.dp))
    }

    val scan = rememberScanStarter(onEvent)
    Button(
        onClick = { onEvent(UiEvent.StartSasPairing) },
        enabled = canStartPairing(state),
        modifier = Modifier.fillMaxWidth().testTag("pair-code"),
    ) {
        Text("Pair with code")
    }
    Spacer(Modifier.height(8.dp))
    OutlinedButton(
        onClick = scan,
        modifier = Modifier.fillMaxWidth().testTag("scan-qr"),
    ) {
        Text("Scan QR code")
    }
    if (state.scanMessage != null) {
        Text(state.scanMessage, style = MaterialTheme.typography.bodySmall)
    }

    Spacer(Modifier.height(16.dp))
    ErrorCard(state, onEvent)

    Spacer(Modifier.height(24.dp))
    TextButton(onClick = { onEvent(UiEvent.ShowAbout) }) { Text("About this beta") }
}

@Composable
private fun PairingScreen(state: AppState, onEvent: (UiEvent) -> Unit) {
    Text("Pair your phone", style = MaterialTheme.typography.headlineSmall)
    Text("Use the QR code or pairing code shown on the Server.", style = MaterialTheme.typography.bodyMedium)
    Spacer(Modifier.height(16.dp))

    if (state.scanActive) {
        QrScanner(
            modifier = Modifier.fillMaxWidth().height(280.dp),
            onPayload = { onEvent(UiEvent.StartQrPairing(it)) },
        )
        Text(
            "Point the camera at the QR code on the Server.",
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.padding(vertical = 8.dp),
        )
        OutlinedButton(onClick = { onEvent(UiEvent.ScanClosed) }, modifier = Modifier.fillMaxWidth()) {
            Text("Close camera")
        }
    } else {
        val scan = rememberScanStarter(onEvent)
        Button(onClick = scan, modifier = Modifier.fillMaxWidth().testTag("scan-qr")) {
            Text("Scan QR code")
        }
        if (state.scanMessage != null) {
            Text(state.scanMessage, style = MaterialTheme.typography.bodySmall)
        }
    }

    Spacer(Modifier.height(12.dp))
    HorizontalDivider()
    Spacer(Modifier.height(12.dp))

    var pastedLink by rememberSaveable { mutableStateOf("") }
    OutlinedTextField(
        value = pastedLink,
        onValueChange = { pastedLink = it },
        label = { Text("Paste pairing link") },
        singleLine = false,
        modifier = Modifier.fillMaxWidth().testTag("paste-link"),
    )
    Spacer(Modifier.height(8.dp))
    OutlinedButton(
        onClick = { onEvent(UiEvent.StartQrPairing(pastedLink.trim())) },
        enabled = pastedLink.isNotBlank(),
        modifier = Modifier.fillMaxWidth().testTag("use-pasted-link"),
    ) {
        Text("Use pasted link")
    }

    Spacer(Modifier.height(12.dp))
    Button(
        onClick = { onEvent(UiEvent.StartSasPairing) },
        enabled = canStartPairing(state),
        modifier = Modifier.fillMaxWidth().testTag("start-code-pairing"),
    ) {
        Text("Pair with a code")
    }

    val pairing = state.pairing
    if (pairing != null) {
        Spacer(Modifier.height(16.dp))
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.padding(16.dp)) {
                Text(pairingTitle(pairing), style = MaterialTheme.typography.titleMedium)
                when {
                    pairing.isBlockingInvitationFailure -> {
                        Spacer(Modifier.height(8.dp))
                        val guidance = pairing.guidance
                        if (guidance != null) {
                            Text(guidance.guidance, style = MaterialTheme.typography.bodyMedium)
                        } else if (pairing.message != null) {
                            Text(pairing.message, color = MaterialTheme.colorScheme.error)
                        }
                        if (pairing.mode == "qr") {
                            Text(
                                "The invitation cannot be reused. The Server must show a fresh one.",
                                style = MaterialTheme.typography.bodySmall,
                            )
                        } else {
                            Text(
                                "Start a new pairing on the Server before trying again.",
                                style = MaterialTheme.typography.bodySmall,
                            )
                        }
                        if (pairing.requiresExplicitRescan) {
                            Spacer(Modifier.height(8.dp))
                            val rescan = rememberScanStarter(onEvent)
                            Button(
                                onClick = rescan,
                                modifier = Modifier.fillMaxWidth().testTag("scan-again"),
                            ) {
                                Text("Scan again")
                            }
                            Text(
                                "The camera stays closed until you choose Scan again.",
                                style = MaterialTheme.typography.bodySmall,
                            )
                        }
                    }

                    pairing.isAwaitingConfirmation && pairing.mode == "qr" -> {
                        // QR mode is one action: the scan already bound the
                        // Server identity, so the reducer sends PairingConfirm
                        // without a Confirm tap (ticket 30 D5). The SAS and
                        // fingerprint stay visible as optional operator aids.
                        Spacer(Modifier.height(8.dp))
                        Text(
                            "Pairing with ${pairing.fingerprint ?: "the Server"}…",
                            style = MaterialTheme.typography.bodyMedium,
                            modifier = Modifier.testTag("qr-auto-confirm"),
                        )
                        Text(
                            "The QR code verified the Server identity. This phone is confirming " +
                                "automatically; the Server still needs to accept this device.",
                            style = MaterialTheme.typography.bodySmall,
                        )
                        Spacer(Modifier.height(8.dp))
                        Text(
                            "Pairing code (optional check)",
                            style = MaterialTheme.typography.labelMedium,
                        )
                        Text(
                            pairing.sas ?: "—",
                            fontSize = 30.sp,
                            fontWeight = FontWeight.Bold,
                            textAlign = TextAlign.Center,
                            modifier = Modifier.fillMaxWidth().testTag("pairing-sas"),
                        )
                        Spacer(Modifier.height(8.dp))
                        Text("Server fingerprint", style = MaterialTheme.typography.labelMedium)
                        Text(
                            pairing.fingerprint ?: "—",
                            style = MaterialTheme.typography.bodySmall,
                            modifier = Modifier.testTag("pairing-fingerprint"),
                        )
                    }

                    pairing.isAwaitingConfirmation -> {
                        Spacer(Modifier.height(8.dp))
                        Text("Code on this phone", style = MaterialTheme.typography.labelMedium)
                        Text(
                            pairing.sas ?: "—",
                            fontSize = 30.sp,
                            fontWeight = FontWeight.Bold,
                            textAlign = TextAlign.Center,
                            modifier = Modifier.fillMaxWidth().testTag("pairing-sas"),
                        )
                        Spacer(Modifier.height(8.dp))
                        OutlinedTextField(
                            value = pairing.sasInput,
                            onValueChange = { onEvent(UiEvent.SasInputChanged(it)) },
                            label = { Text("Enter the code shown on the Server") },
                            singleLine = true,
                            modifier = Modifier.fillMaxWidth().testTag("pairing-sas-input"),
                        )
                        if (pairing.sasInput.isNotEmpty() && !pairing.sasMatches) {
                            Text(
                                "The codes do not match.",
                                color = MaterialTheme.colorScheme.error,
                                style = MaterialTheme.typography.bodySmall,
                            )
                        }
                        Spacer(Modifier.height(8.dp))
                        Text("Server fingerprint", style = MaterialTheme.typography.labelMedium)
                        Text(
                            pairing.fingerprint ?: "—",
                            style = MaterialTheme.typography.bodySmall,
                            modifier = Modifier.testTag("pairing-fingerprint"),
                        )
                        Spacer(Modifier.height(8.dp))
                        Button(
                            onClick = { onEvent(UiEvent.ConfirmPairing) },
                            enabled = pairing.canConfirm,
                            modifier = Modifier.fillMaxWidth().testTag("confirm-pairing"),
                        ) {
                            Text("Confirm pairing")
                        }
                    }

                    pairing.isConfirming -> {
                        Spacer(Modifier.height(8.dp))
                        Text(
                            "Confirmed, waiting for the Server to accept.",
                            style = MaterialTheme.typography.bodyMedium,
                            modifier = Modifier.testTag("pairing-confirming"),
                        )
                        Text(
                            "The pairing completes only after the Server operator accepts this " +
                                "device. " +
                                if (pairing.mode == "qr") {
                                    "If they decline, the QR code must be refreshed."
                                } else {
                                    "If they decline, start a new pairing."
                                },
                            style = MaterialTheme.typography.bodySmall,
                        )
                        if (pairing.sas != null) {
                            Spacer(Modifier.height(8.dp))
                            Text(
                                "Pairing code (optional check)",
                                style = MaterialTheme.typography.labelMedium,
                            )
                            Text(
                                pairing.sas,
                                fontSize = 30.sp,
                                fontWeight = FontWeight.Bold,
                                textAlign = TextAlign.Center,
                                modifier = Modifier.fillMaxWidth().testTag("pairing-sas"),
                            )
                        }
                        if (pairing.fingerprint != null) {
                            Spacer(Modifier.height(8.dp))
                            Text("Server fingerprint", style = MaterialTheme.typography.labelMedium)
                            Text(
                                pairing.fingerprint,
                                style = MaterialTheme.typography.bodySmall,
                                modifier = Modifier.testTag("pairing-fingerprint"),
                            )
                        }
                    }

                    pairing.isComplete -> {
                        Text(
                            "Saving the pairing. The app will connect automatically.",
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }

                    pairing.message != null -> {
                        Text(pairing.message, color = MaterialTheme.colorScheme.error)
                    }
                }
            }
        }
    }

    Spacer(Modifier.height(16.dp))
    ErrorCard(state, onEvent)

    Spacer(Modifier.height(8.dp))
    OutlinedButton(
        onClick = { onEvent(UiEvent.CancelPairing) },
        modifier = Modifier.fillMaxWidth().testTag("cancel-pairing"),
    ) {
        Text("Cancel")
    }
    TextButton(onClick = { onEvent(UiEvent.ShowHome) }) { Text("Back to Home") }
}

@Composable
private fun SessionScreen(state: AppState, onEvent: (UiEvent) -> Unit) {
    Text("Session", style = MaterialTheme.typography.headlineSmall)
    Spacer(Modifier.height(12.dp))
    SessionStatus(state)
    Spacer(Modifier.height(8.dp))
    Text(
        if (state.address.isBlank()) "Server: not set" else "Server: ${state.address}",
        style = MaterialTheme.typography.bodyMedium,
    )
    Spacer(Modifier.height(16.dp))
    Button(
        onClick = { onEvent(UiEvent.StopRequested) },
        enabled = state.sessionPhase != SessionPhase.NONE &&
            state.sessionPhase != SessionPhase.CLOSED &&
            !state.stopping,
        modifier = Modifier.fillMaxWidth().testTag("stop"),
    ) {
        Text("Stop")
    }
    Spacer(Modifier.height(12.dp))
    ErrorCard(state, onEvent)
    Spacer(Modifier.height(12.dp))
    TextButton(onClick = { onEvent(UiEvent.ShowHome) }) { Text("Back to Home") }
}

@Composable
private fun AboutScreen(versionName: String, versionCode: Int, onEvent: (UiEvent) -> Unit) {
    Text("About this beta", style = MaterialTheme.typography.headlineSmall)
    Spacer(Modifier.height(8.dp))
    Text(
        "Version $versionName ($versionCode)",
        style = MaterialTheme.typography.titleMedium,
        modifier = Modifier.testTag("about-version"),
    )
    Spacer(Modifier.height(16.dp))

    Text("Known limitations", style = MaterialTheme.typography.titleSmall)
    Bullet("No jitter buffer, concealment, or clock-drift correction. Audio quality is best-effort.")
    Bullet("One Client per Server. Concurrent phones are not supported.")
    Bullet("No diagnostics export, no telemetry, and no raw-audio recording.")
    Spacer(Modifier.height(12.dp))

    Text("About v1", style = MaterialTheme.typography.titleSmall)
    Text(
        "The v1 app sends audio without encryption or authentication. Use v1 only on a " +
            "trusted network. This v2 beta is the encrypted, authenticated client; a v2 " +
            "failure never downgrades to v1.",
        style = MaterialTheme.typography.bodyMedium,
    )
    Spacer(Modifier.height(12.dp))

    Text("Updates", style = MaterialTheme.typography.titleSmall)
    Text(
        "Updates are manual. Download the new APK from the GitHub Release page and verify " +
            "its SHA-256 checksum before installing. There is no in-app updater.",
        style = MaterialTheme.typography.bodyMedium,
    )
    Spacer(Modifier.height(12.dp))

    Text("Privacy", style = MaterialTheme.typography.titleSmall)
    Text(
        "This beta has no telemetry and does not record raw audio.",
        style = MaterialTheme.typography.bodyMedium,
    )
    Spacer(Modifier.height(20.dp))
    TextButton(onClick = { onEvent(UiEvent.ShowHome) }) { Text("Back") }
}

@Composable
private fun SessionStatus(state: AppState) {
    val label = when (state.sessionPhase) {
        SessionPhase.NONE -> "Not connected"
        else -> state.sessionPhase.name
    }
    Row(horizontalArrangement = Arrangement.Start) {
        Text("State: ", style = MaterialTheme.typography.bodyMedium)
        Text(
            label,
            style = MaterialTheme.typography.bodyMedium,
            fontWeight = FontWeight.SemiBold,
            modifier = Modifier.testTag("session-state"),
        )
    }
}

@Composable
private fun ErrorCard(state: AppState, onEvent: (UiEvent) -> Unit) {
    val error = state.error ?: return
    Spacer(Modifier.height(8.dp))
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp)) {
            Text(
                error.title,
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.testTag("error-title"),
            )
            Text(error.guidance, style = MaterialTheme.typography.bodyMedium)
            if (error.fallbackToV1) {
                Text("This app never falls back to v1.", style = MaterialTheme.typography.bodySmall)
            }
            Spacer(Modifier.height(8.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    onClick = { onEvent(UiEvent.Retry) },
                    modifier = Modifier.testTag("retry"),
                ) {
                    Text(if (error.requiresRepairing) "Pair again" else "Retry")
                }
                TextButton(onClick = { onEvent(UiEvent.DismissError) }) { Text("Dismiss") }
            }
        }
    }
}

@Composable
private fun Bullet(text: String) {
    Text("• $text", style = MaterialTheme.typography.bodyMedium)
}

private fun canStartPairing(state: AppState): Boolean =
    state.address.isNotBlank() && state.addressError == null

private fun pairingTitle(pairing: PairingUi): String = when (pairing.phase) {
    "STARTING" -> "Connecting to the Server…"
    "AWAITING_CONFIRMATION" ->
        if (pairing.mode == "qr") "Pairing with the Server…" else "Compare the pairing code"
    "CONFIRMING" -> "Confirmed, waiting for the Server to accept"
    "COMPLETE" -> "Pairing complete"
    "FAILED" -> pairing.guidance?.title ?: "Pairing failed"
    else -> pairing.phase
}

/**
 * Runtime CAMERA permission (no GMS). When granted, the reducer opens the
 * scanner; when denied, the paste-link fallback stays available.
 */
@Composable
private fun rememberScanStarter(onEvent: (UiEvent) -> Unit): () -> Unit {
    val context = LocalContext.current
    val launcher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { granted ->
        onEvent(if (granted) UiEvent.RequestScan else UiEvent.ScanPermissionDenied)
    }
    return remember(context, launcher) {
        {
            val granted = ContextCompat.checkSelfPermission(
                context, Manifest.permission.CAMERA,
            ) == PackageManager.PERMISSION_GRANTED
            if (granted) {
                onEvent(UiEvent.RequestScan)
            } else {
                launcher.launch(Manifest.permission.CAMERA)
            }
        }
    }
}
