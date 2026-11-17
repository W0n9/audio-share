package dev.audioshare.v2

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedCard
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat

/**
 * The whole beta UI is a pure function of [AppState]; it emits [UiEvent]s and
 * never touches the session implementation. The Activity owns binding and
 * command execution.
 *
 * Direction A shell (ticket 18): one static hero and one primary action per
 * screen; tiny top bar with Settings/About icon actions; no tab bar and no
 * Navigation Compose. PAIRING/SESSION layout is intentionally left to tickets
 * 19/20, but any copy they share with the shell resolves through resources.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AudioShareApp(
    state: AppState,
    versionName: String,
    versionCode: Int,
    onEvent: (UiEvent) -> Unit,
) {
    // Back is navigation only (ticket 06 §5). HOME deliberately has no enabled
    // handler: the system finishes the Activity, and MainActivity.onDestroy
    // sends ACTION_STOP only for a real finish (so two Backs from SESSION stop
    // the session, as intended).
    BackHandler(enabled = state.screen != AppScreen.HOME) {
        onEvent(UiEvent.BackPressed)
    }
    Scaffold(
        topBar = { ShellTopBar(state.screen, onEvent) },
    ) { contentInsets ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(contentInsets)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp, vertical = 16.dp),
        ) {
            when (state.screen) {
                AppScreen.HOME -> HomeScreen(state, onEvent)
                AppScreen.PAIRING -> PairingScreen(state, onEvent)
                AppScreen.SESSION -> SessionScreen(state, onEvent)
                AppScreen.ABOUT -> AboutScreen(versionName, versionCode, onEvent)
                AppScreen.SETTINGS -> SettingsScreen(state, versionName, versionCode, onEvent)
            }
        }
    }
}

/**
 * Small app bar. Settings/About are one tap from the shell screens; PAIRING
 * and SESSION keep their content-level exits so an in-flight exchange or live
 * session is never silently stranded by navigation (tickets 19/20 own those
 * screens).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ShellTopBar(screen: AppScreen, onEvent: (UiEvent) -> Unit) {
    val showActions = screen == AppScreen.HOME ||
        screen == AppScreen.ABOUT ||
        screen == AppScreen.SETTINGS
    TopAppBar(
        title = { Text(stringResource(R.string.app_name)) },
        colors = TopAppBarDefaults.topAppBarColors(
            containerColor = MaterialTheme.colorScheme.surface,
        ),
        actions = {
            if (showActions) {
                IconButton(
                    onClick = { onEvent(UiEvent.ShowSettings) },
                    modifier = Modifier.size(48.dp).testTag("settings"),
                ) {
                    Icon(
                        painter = painterResource(R.drawable.ic_settings),
                        contentDescription = stringResource(R.string.action_settings),
                    )
                }
                IconButton(
                    onClick = { onEvent(UiEvent.ShowAbout) },
                    modifier = Modifier.size(48.dp).testTag("about"),
                ) {
                    Icon(
                        painter = painterResource(R.drawable.ic_info),
                        contentDescription = stringResource(R.string.action_about),
                    )
                }
            }
        },
    )
}

@Composable
private fun HomeScreen(state: AppState, onEvent: (UiEvent) -> Unit) {
    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        HeroPulse()
        Spacer(Modifier.height(12.dp))
        Text(
            stringResource(R.string.home_title),
            style = MaterialTheme.typography.headlineMedium,
            textAlign = TextAlign.Center,
            modifier = Modifier.testTag("home-title"),
        )
        Spacer(Modifier.height(4.dp))
        Text(
            stringResource(
                if (state.hasStoredPairing) {
                    R.string.home_subtitle_paired
                } else {
                    R.string.home_subtitle_unpaired
                },
            ),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(16.dp))
    }

    SessionStatus(state)
    Spacer(Modifier.height(16.dp))

    val server = state.storedServers.firstOrNull()
    if (server != null) {
        OutlinedCard(
            modifier = Modifier.fillMaxWidth().testTag("home-server-card"),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth().padding(16.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text(
                        server.displayName ?: stringResource(R.string.home_paired_server),
                        style = MaterialTheme.typography.titleMedium,
                    )
                    Text(
                        server.address,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Text(
                    stringResource(R.string.home_paired_badge),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.primary,
                )
            }
        }
        Spacer(Modifier.height(16.dp))
    }

    OutlinedTextField(
        value = state.address,
        onValueChange = { onEvent(UiEvent.AddressChanged(it)) },
        label = { Text(stringResource(R.string.server_address_label)) },
        placeholder = { Text(stringResource(R.string.server_address_placeholder)) },
        singleLine = true,
        isError = state.addressError != null,
        modifier = Modifier.fillMaxWidth().testTag("address-input"),
    )
    if (state.addressError != null) {
        Text(
            stringResource(state.addressError.messageRes()),
            color = MaterialTheme.colorScheme.error,
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier
                .padding(top = 4.dp)
                .semantics { liveRegion = LiveRegionMode.Assertive }
                .testTag("address-error"),
        )
    }

    Spacer(Modifier.height(16.dp))
    if (state.hasStoredPairing) {
        PrimaryButton(
            text = stringResource(R.string.home_connect),
            onClick = { onEvent(UiEvent.ConnectRequested) },
            tag = "connect",
        )
        Spacer(Modifier.height(8.dp))
        OutlinedButton(
            onClick = { onEvent(UiEvent.StartSasPairing) },
            enabled = canStartPairing(state),
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("pair-code"),
        ) {
            Text(stringResource(R.string.home_pair_new))
        }
        Spacer(Modifier.height(8.dp))
        TextButton(
            onClick = rememberScanStarter(onEvent),
            modifier = Modifier.heightIn(min = 48.dp).testTag("scan-qr"),
            contentPadding = androidx.compose.foundation.layout.PaddingValues(
                horizontal = 16.dp,
                vertical = 8.dp,
            ),
        ) {
            Text(stringResource(R.string.home_scan_qr_instead))
        }
    } else {
        PrimaryButton(
            text = stringResource(R.string.home_pair_code),
            onClick = { onEvent(UiEvent.StartSasPairing) },
            enabled = canStartPairing(state),
            tag = "pair-code",
        )
        Spacer(Modifier.height(8.dp))
        OutlinedButton(
            onClick = rememberScanStarter(onEvent),
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("scan-qr"),
        ) {
            Text(stringResource(R.string.home_scan_qr))
        }
    }
    if (state.scanNotice != null) {
        Text(
            stringResource(state.scanNotice.messageRes()),
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier
                .padding(top = 8.dp)
                .semantics { liveRegion = LiveRegionMode.Assertive }
                .testTag("scan-message"),
        )
    }
}

@Composable
private fun PrimaryButton(
    text: String,
    onClick: () -> Unit,
    tag: String,
    enabled: Boolean = true,
) {
    Button(
        onClick = onClick,
        enabled = enabled,
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag(tag),
    ) {
        Text(text)
    }
}

/** Static Direction-A hero: concentric pulses, no infinite animation. */
@Composable
private fun HeroPulse() {
    val primary = MaterialTheme.colorScheme.primary
    Canvas(modifier = Modifier.size(140.dp)) {
        val radius = size.minDimension / 2f
        drawCircle(primary.copy(alpha = 0.10f), radius = radius)
        drawCircle(primary.copy(alpha = 0.18f), radius = radius * 0.72f)
        drawCircle(primary.copy(alpha = 0.28f), radius = radius * 0.46f)
        drawCircle(primary, radius = radius * 0.22f)
    }
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
        OutlinedButton(
            onClick = { onEvent(UiEvent.ScanClosed) },
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp),
        ) {
            Text("Close camera")
        }
    } else {
        val scan = rememberScanStarter(onEvent)
        Button(
            onClick = scan,
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("scan-qr"),
        ) {
            Text(stringResource(R.string.home_scan_qr))
        }
        if (state.scanNotice != null) {
            Text(
                stringResource(state.scanNotice.messageRes()),
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier
                    .padding(top = 8.dp)
                    .semantics { liveRegion = LiveRegionMode.Assertive }
                    .testTag("scan-message"),
            )
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
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("use-pasted-link"),
    ) {
        Text("Use pasted link")
    }

    Spacer(Modifier.height(12.dp))
    Button(
        onClick = { onEvent(UiEvent.StartSasPairing) },
        enabled = canStartPairing(state),
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("start-code-pairing"),
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
                        Text(
                            stringResource(
                                ErrorGuidanceResources.guidanceRes(pairing.errorCode),
                            ),
                            style = MaterialTheme.typography.bodyMedium,
                        )
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
                                modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)
                                    .testTag("scan-again"),
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
                        SasText(pairing.sas)
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
                        SasText(pairing.sas)
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
                            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)
                                .testTag("confirm-pairing"),
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
                            SasText(pairing.sas)
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

                    // Copy is resolved from the code, never from Kotlin state.
                    pairing.isFailed && pairing.notice == null -> {
                        Text(
                            stringResource(
                                ErrorGuidanceResources.guidanceRes(pairing.errorCode),
                            ),
                            color = MaterialTheme.colorScheme.error,
                        )
                    }

                    pairing.notice != null -> {
                        Text(
                            stringResource(pairing.notice.messageRes()),
                            color = MaterialTheme.colorScheme.error,
                        )
                    }
                }
            }
        }
    }

    Spacer(Modifier.height(8.dp))
    OutlinedButton(
        onClick = { onEvent(UiEvent.CancelPairing) },
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("cancel-pairing"),
    ) {
        Text("Cancel")
    }
    TextButton(
        onClick = { onEvent(UiEvent.ShowHome) },
        modifier = Modifier.heightIn(min = 48.dp),
    ) {
        Text(stringResource(R.string.session_back_home))
    }
}

/** SAS is announced digit by digit (a11y list item 2). */
@Composable
private fun SasText(sas: String?) {
    val spoken = sas?.filter(Char::isDigit)?.map(Char::toString)?.joinToString(" ")
    Text(
        sas ?: "—",
        fontSize = 30.sp,
        fontWeight = FontWeight.Bold,
        textAlign = TextAlign.Center,
        modifier = Modifier
            .fillMaxWidth()
            .testTag("pairing-sas")
            .then(
                if (spoken.isNullOrEmpty()) {
                    Modifier
                } else {
                    Modifier.semantics { contentDescription = spoken }
                },
            ),
    )
}

@Composable
private fun SessionScreen(state: AppState, onEvent: (UiEvent) -> Unit) {
    Text(stringResource(R.string.session_title), style = MaterialTheme.typography.headlineSmall)
    Spacer(Modifier.height(12.dp))
    SessionStatus(state)
    Spacer(Modifier.height(8.dp))
    Text(
        if (state.address.isBlank()) {
            stringResource(R.string.session_server_unset)
        } else {
            stringResource(R.string.session_server_value, state.address)
        },
        style = MaterialTheme.typography.bodyMedium,
    )
    Spacer(Modifier.height(16.dp))

    when (val outcome = state.outcome) {
        is SessionOutcome.Live,
        is SessionOutcome.Reconnecting -> {
            // Reconnection is mid-session: Stop stays the only primary action
            // and is merely disabled while the asynchronous Stop is in flight.
            PrimaryButton(
                text = stringResource(R.string.session_stop),
                onClick = { onEvent(UiEvent.StopRequested) },
                enabled = !state.stopRequested,
                tag = "stop",
            )
        }

        is SessionOutcome.Ended -> when (val reason = outcome.reason) {
            // Two visibly different terminal presentations (ticket 06/20):
            // a user Stop is silent; a surfaced close is an error card.
            EndReason.UserStop -> {
                Text(
                    stringResource(R.string.session_stopped_by_user),
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.testTag("ended-user-stop"),
                )
                Spacer(Modifier.height(8.dp))
                PrimaryButton(
                    text = stringResource(R.string.session_connect_again),
                    onClick = { onEvent(UiEvent.ConnectRequested) },
                    tag = "connect-again",
                )
            }

            is EndReason.Surfaced -> {
                val guidance = ErrorGuidanceBook.forCode(reason.code)
                    ?: ErrorGuidance(
                        code = reason.code ?: 0,
                        name = "SESSION_ENDED",
                        retryable = true,
                        requiresRepairing = false,
                        automaticRetry = false,
                    )
                ErrorCard(guidance, onEvent)
            }
        }

        // A Connect was accepted but the Service has not published yet.
        null -> PrimaryButton(
            text = stringResource(R.string.session_stop),
            onClick = {},
            enabled = false,
            tag = "stop",
        )
    }

    Spacer(Modifier.height(12.dp))
    TextButton(
        onClick = { onEvent(UiEvent.ShowHome) },
        modifier = Modifier.heightIn(min = 48.dp),
    ) {
        Text(stringResource(R.string.session_back_home))
    }
}

@Composable
private fun SettingsScreen(
    state: AppState,
    versionName: String,
    versionCode: Int,
    onEvent: (UiEvent) -> Unit,
) {
    // Ticket 18 keeps SETTINGS read-only: no switches, delete, or rename.
    Text(
        stringResource(R.string.settings_title),
        style = MaterialTheme.typography.headlineSmall,
        modifier = Modifier.testTag("settings-title"),
    )
    Spacer(Modifier.height(16.dp))

    Text(stringResource(R.string.settings_paired_server), style = MaterialTheme.typography.titleSmall)
    val server = state.storedServers.firstOrNull()
    if (server == null) {
        Spacer(Modifier.height(8.dp))
        Text(
            stringResource(R.string.settings_empty_body),
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.testTag("settings-empty"),
        )
        Spacer(Modifier.height(16.dp))
        PrimaryButton(
            text = stringResource(R.string.settings_pair_phone),
            onClick = { onEvent(UiEvent.ShowHome) },
            tag = "settings-pair",
        )
    } else {
        Spacer(Modifier.height(8.dp))
        OutlinedCard(
            modifier = Modifier.fillMaxWidth().testTag("settings-server"),
        ) {
            Column(
                modifier = Modifier.fillMaxWidth().padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                LabeledValue(
                    label = stringResource(R.string.settings_server_name),
                    value = server.displayName ?: "—",
                )
                LabeledValue(
                    label = stringResource(R.string.settings_server_address),
                    value = server.address,
                )
                LabeledValue(
                    label = stringResource(R.string.settings_server_fingerprint),
                    value = server.serverSpkiSha256Hex,
                    valueTag = "settings-fingerprint",
                )
            }
        }
    }
    Spacer(Modifier.height(16.dp))

    Text(
        stringResource(R.string.version_label, versionName, versionCode),
        style = MaterialTheme.typography.bodyMedium,
        modifier = Modifier.testTag("settings-version"),
    )
    Spacer(Modifier.height(16.dp))

    TextButton(
        onClick = { onEvent(UiEvent.ShowAbout) },
        modifier = Modifier.heightIn(min = 48.dp).testTag("settings-about"),
    ) {
        Text(stringResource(R.string.settings_about_link))
    }
    TextButton(
        onClick = { onEvent(UiEvent.ShowHome) },
        modifier = Modifier.heightIn(min = 48.dp),
    ) {
        Text(stringResource(R.string.action_back))
    }
}

@Composable
private fun LabeledValue(label: String, value: String, valueTag: String? = null) {
    Column(Modifier.fillMaxWidth()) {
        Text(
            label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            value,
            style = MaterialTheme.typography.bodyMedium,
            modifier = if (valueTag != null) Modifier.testTag(valueTag) else Modifier,
        )
    }
}

@Composable
private fun AboutScreen(versionName: String, versionCode: Int, onEvent: (UiEvent) -> Unit) {
    Text(stringResource(R.string.about_title), style = MaterialTheme.typography.headlineSmall)
    Spacer(Modifier.height(8.dp))
    Text(
        stringResource(R.string.version_label, versionName, versionCode),
        style = MaterialTheme.typography.titleMedium,
        modifier = Modifier.testTag("about-version"),
    )
    Spacer(Modifier.height(16.dp))

    SectionTitle(stringResource(R.string.about_known_limitations))
    Bullet(stringResource(R.string.about_limit_jitter))
    Bullet(stringResource(R.string.about_limit_one_client))
    Bullet(stringResource(R.string.about_limit_diagnostics))
    Spacer(Modifier.height(12.dp))

    SectionTitle(stringResource(R.string.about_v1_title))
    Text(
        stringResource(R.string.about_v1_warning),
        style = MaterialTheme.typography.bodyMedium,
    )
    Spacer(Modifier.height(12.dp))

    SectionTitle(stringResource(R.string.about_updates_title))
    Text(
        stringResource(R.string.about_updates_body),
        style = MaterialTheme.typography.bodyMedium,
    )
    Spacer(Modifier.height(12.dp))

    SectionTitle(stringResource(R.string.about_privacy_title))
    Text(
        stringResource(R.string.about_privacy_body),
        style = MaterialTheme.typography.bodyMedium,
    )
    Spacer(Modifier.height(20.dp))
    TextButton(
        onClick = { onEvent(UiEvent.ShowHome) },
        modifier = Modifier.heightIn(min = 48.dp),
    ) {
        Text(stringResource(R.string.action_back))
    }
}

@Composable
private fun SectionTitle(text: String) {
    Text(text, style = MaterialTheme.typography.titleSmall)
}

@Composable
private fun SessionStatus(state: AppState) {
    val label = when (val outcome = state.outcome) {
        null -> stringResource(R.string.session_not_connected)
        is SessionOutcome.Live -> outcome.phase.name
        is SessionOutcome.Reconnecting -> pluralStringResource(
            R.plurals.reconnect_attempt,
            outcome.attempt,
            outcome.attempt,
        )
        is SessionOutcome.Ended -> when (outcome.reason) {
            EndReason.UserStop -> stringResource(R.string.session_outcome_stopped)
            is EndReason.Surfaced -> stringResource(R.string.session_outcome_ended)
        }
    }
    Row(horizontalArrangement = Arrangement.Start) {
        Text(
            stringResource(R.string.session_state_prefix),
            style = MaterialTheme.typography.bodyMedium,
        )
        Text(
            label,
            style = MaterialTheme.typography.bodyMedium,
            fontWeight = FontWeight.SemiBold,
            modifier = Modifier.testTag("session-state"),
        )
    }
}

@Composable
private fun ErrorCard(error: ErrorGuidance, onEvent: (UiEvent) -> Unit) {
    Spacer(Modifier.height(8.dp))
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.errorContainer,
            contentColor = MaterialTheme.colorScheme.onErrorContainer,
        ),
    ) {
        Column(Modifier.padding(16.dp)) {
            Text(
                stringResource(ErrorGuidanceResources.titleRes(error.code)),
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier
                    .semantics { liveRegion = LiveRegionMode.Assertive }
                    .testTag("error-title"),
            )
            Text(
                stringResource(ErrorGuidanceResources.guidanceRes(error.code)),
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.semantics { liveRegion = LiveRegionMode.Assertive },
            )
            if (error.fallbackToV1) {
                Text(
                    stringResource(R.string.error_no_v1_fallback),
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            Spacer(Modifier.height(8.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    onClick = { onEvent(UiEvent.Retry) },
                    modifier = Modifier.heightIn(min = 48.dp).testTag("retry"),
                ) {
                    Text(
                        stringResource(
                            if (error.requiresRepairing) {
                                R.string.error_action_pair_again
                            } else {
                                R.string.error_action_retry
                            },
                        ),
                    )
                }
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

@Composable
private fun pairingTitle(pairing: PairingUi): String = when (pairing.phase) {
    "STARTING" -> "Connecting to the Server…"
    "AWAITING_CONFIRMATION" ->
        if (pairing.mode == "qr") "Pairing with the Server…" else "Compare the pairing code"
    "CONFIRMING" -> "Confirmed, waiting for the Server to accept"
    "COMPLETE" -> "Pairing complete"
    "FAILED" -> stringResource(ErrorGuidanceResources.titleRes(pairing.errorCode))
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
