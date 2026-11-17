package dev.audioshare.v2

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
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
    Column(Modifier.fillMaxWidth()) {
        Text(
            stringResource(R.string.pairing_title),
            style = MaterialTheme.typography.headlineSmall,
            modifier = Modifier.testTag("pairing-title"),
        )
        Text(
            stringResource(R.string.pairing_subtitle),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(16.dp))

        val pairing = state.pairing
        when {
            state.scanActive -> PairingScanner(state, onEvent)
            pairing == null -> PairingEntries(state, onEvent)
            else -> PairingExchange(pairing, onEvent)
        }
    }
}

/** Direction A: the not-started entries are only shown when nothing is in flight. */
@Composable
private fun PairingEntries(state: AppState, onEvent: (UiEvent) -> Unit) {
    // One primary action: Scan QR. Paste-link and SAS stay secondary.
    PrimaryButton(
        text = stringResource(R.string.pairing_scan_qr),
        onClick = rememberScanStarter(onEvent),
        tag = "scan-qr",
    )
    ScanNoticeText(state.scanNotice)

    Spacer(Modifier.height(16.dp))
    var pastedLink by rememberSaveable { mutableStateOf("") }
    OutlinedTextField(
        value = pastedLink,
        onValueChange = { pastedLink = it },
        label = { Text(stringResource(R.string.pairing_paste_label)) },
        singleLine = false,
        modifier = Modifier.fillMaxWidth().testTag("paste-link"),
    )
    Spacer(Modifier.height(8.dp))
    OutlinedButton(
        onClick = { onEvent(UiEvent.StartQrPairing(pastedLink.trim())) },
        enabled = pastedLink.isNotBlank(),
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("use-pasted-link"),
    ) {
        Text(stringResource(R.string.pairing_use_pasted_link))
    }

    Spacer(Modifier.height(12.dp))
    OutlinedButton(
        onClick = { onEvent(UiEvent.StartSasPairing) },
        enabled = canStartPairing(state),
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("start-code-pairing"),
    ) {
        Text(stringResource(R.string.pairing_start_code))
    }
    PairingBackHome(onEvent)
}

@Composable
private fun PairingScanner(state: AppState, onEvent: (UiEvent) -> Unit) {
    Box(
        modifier = Modifier.fillMaxWidth().height(280.dp).testTag("qr-scanner"),
    ) {
        QrScanner(
            modifier = Modifier.fillMaxSize(),
            onPayload = { onEvent(UiEvent.StartQrPairing(it)) },
        )
    }
    Text(
        stringResource(R.string.pairing_scan_instruction),
        style = MaterialTheme.typography.bodySmall,
        modifier = Modifier.padding(vertical = 8.dp),
    )
    ScanNoticeText(state.scanNotice)
    OutlinedButton(
        onClick = { onEvent(UiEvent.ScanClosed) },
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("close-camera"),
    ) {
        Text(stringResource(R.string.pairing_close_camera))
    }
    PairingBackHome(onEvent)
}

@Composable
private fun ScanNoticeText(notice: ScanNotice?) {
    if (notice == null) return
    Text(
        stringResource(notice.messageRes()),
        style = MaterialTheme.typography.bodySmall,
        modifier = Modifier
            .padding(top = 8.dp)
            .liveRegionAssertive()
            .testTag("scan-message"),
    )
}

/** The single in-flight pairing card: one status, at most one primary action. */
@Composable
private fun PairingExchange(pairing: PairingUi, onEvent: (UiEvent) -> Unit) {
    when {
        pairing.isStarting -> PairingStarting(onEvent)
        pairing.isAwaitingConfirmation && pairing.mode == "qr" ->
            PairingQrAwaiting(pairing, onEvent)
        pairing.isAwaitingConfirmation -> PairingSasAwaiting(pairing, onEvent)
        pairing.isConfirming -> PairingConfirming(pairing, onEvent)
        pairing.isComplete -> PairingComplete(onEvent)
        pairing.isFailed -> PairingFailed(pairing, onEvent)
        else -> PairingStarting(onEvent)
    }
}

@Composable
private fun PairingStarting(onEvent: (UiEvent) -> Unit) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp)) {
            Text(
                stringResource(R.string.pairing_starting_title),
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.liveRegionAssertive().testTag("pairing-status"),
            )
            Spacer(Modifier.height(12.dp))
            LinearProgressIndicator(
                modifier = Modifier.fillMaxWidth().testTag("pairing-progress"),
            )
            Spacer(Modifier.height(12.dp))
            Text(
                stringResource(R.string.pairing_starting_body),
                style = MaterialTheme.typography.bodyMedium,
            )
        }
    }
    Spacer(Modifier.height(12.dp))
    PairingCancel(onEvent)
    PairingBackHome(onEvent)
}

@Composable
private fun PairingQrAwaiting(pairing: PairingUi, onEvent: (UiEvent) -> Unit) {
    // QR mode is one action: the reducer already sent the single PairingConfirm
    // after the scan (ticket 30 D5). Never render a Confirm button here.
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp)) {
            Text(
                stringResource(R.string.pairing_awaiting_qr_title),
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.liveRegionAssertive().testTag("pairing-status"),
            )
            Spacer(Modifier.height(8.dp))
            Text(
                stringResource(R.string.pairing_awaiting_qr_confirmed),
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.liveRegionAssertive().testTag("qr-auto-confirm"),
            )
            Spacer(Modifier.height(8.dp))
            Text(
                stringResource(R.string.pairing_awaiting_qr_body),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            PairingSasAndFingerprint(pairing, optionalCode = true)
        }
    }
    Spacer(Modifier.height(12.dp))
    PairingCancel(onEvent)
    PairingBackHome(onEvent)
}

@Composable
private fun PairingSasAwaiting(pairing: PairingUi, onEvent: (UiEvent) -> Unit) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp)) {
            Text(
                stringResource(R.string.pairing_awaiting_sas_title),
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.liveRegionAssertive().testTag("pairing-status"),
            )
            Spacer(Modifier.height(8.dp))
            Text(
                stringResource(R.string.pairing_awaiting_sas_body),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(12.dp))
            Text(
                stringResource(R.string.pairing_code_on_phone),
                style = MaterialTheme.typography.labelMedium,
            )
            SasText(pairing.sas)
            Spacer(Modifier.height(12.dp))
            val mismatch = pairing.sasInput.isNotEmpty() && !pairing.sasMatches
            OutlinedTextField(
                value = pairing.sasInput,
                onValueChange = { onEvent(UiEvent.SasInputChanged(it)) },
                label = { Text(stringResource(R.string.pairing_enter_code)) },
                singleLine = true,
                isError = mismatch,
                modifier = Modifier.fillMaxWidth().testTag("pairing-sas-input"),
            )
            if (mismatch) {
                Text(
                    stringResource(R.string.pairing_notice_sas_mismatch),
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier
                        .padding(top = 4.dp)
                        .liveRegionAssertive()
                        .testTag("pairing-sas-error"),
                )
            }
            PairingFingerprint(pairing.fingerprint)
            Spacer(Modifier.height(12.dp))
            Button(
                onClick = { onEvent(UiEvent.ConfirmPairing) },
                enabled = pairing.canConfirm,
                modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)
                    .testTag("confirm-pairing"),
            ) {
                Text(stringResource(R.string.pairing_confirm))
            }
        }
    }
    Spacer(Modifier.height(12.dp))
    PairingCancel(onEvent)
    PairingBackHome(onEvent)
}

@Composable
private fun PairingConfirming(pairing: PairingUi, onEvent: (UiEvent) -> Unit) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp)) {
            Text(
                stringResource(R.string.pairing_confirming_title),
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.liveRegionAssertive().testTag("pairing-confirming"),
            )
            Spacer(Modifier.height(8.dp))
            Text(
                stringResource(
                    if (pairing.mode == "qr") {
                        R.string.pairing_confirming_body_qr
                    } else {
                        R.string.pairing_confirming_body_sas
                    },
                ),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            PairingSasAndFingerprint(pairing, optionalCode = true)
        }
    }
    Spacer(Modifier.height(12.dp))
    PairingCancel(onEvent)
    PairingBackHome(onEvent)
}

@Composable
private fun PairingComplete(onEvent: (UiEvent) -> Unit) {
    // Ticket 17: the Service persists and starts the session, then pushes a
    // one-shot PairingCompleted. This screen only reports the handoff; there
    // is no credential polling and no UI-initiated connect.
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp)) {
            Text(
                stringResource(R.string.pairing_complete_title),
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.liveRegionAssertive().testTag("pairing-status"),
            )
            Spacer(Modifier.height(12.dp))
            LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
            Spacer(Modifier.height(12.dp))
            Text(
                stringResource(R.string.pairing_complete_body),
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.testTag("pairing-complete"),
            )
        }
    }
    PairingBackHome(onEvent)
}

@Composable
private fun PairingFailed(pairing: PairingUi, onEvent: (UiEvent) -> Unit) {
    val code = pairing.errorCode
    val titleRes = if (code == null) {
        R.string.pairing_failed_title
    } else {
        ErrorGuidanceResources.titleRes(code)
    }
    val guidanceRes = pairing.notice?.messageRes()
        ?: ErrorGuidanceResources.guidanceRes(code)
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.errorContainer,
            contentColor = MaterialTheme.colorScheme.onErrorContainer,
        ),
    ) {
        Column(Modifier.padding(16.dp)) {
            Text(
                stringResource(titleRes),
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.liveRegionAssertive().testTag("pairing-error-title"),
            )
            if (code != null) {
                Text(
                    stringResource(R.string.pairing_error_code, code),
                    style = MaterialTheme.typography.labelMedium,
                    modifier = Modifier
                        .padding(top = 4.dp)
                        .testTag("pairing-error-code"),
                )
            }
            Spacer(Modifier.height(8.dp))
            Text(
                stringResource(guidanceRes),
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.liveRegionAssertive().testTag("pairing-error-guidance"),
            )
            PairingFingerprint(pairing.fingerprint)
            if (pairing.isBlockingInvitationFailure) {
                Spacer(Modifier.height(8.dp))
                Text(
                    stringResource(
                        if (pairing.mode == "qr") {
                            R.string.pairing_failed_invitation_qr
                        } else {
                            R.string.pairing_failed_invitation_sas
                        },
                    ),
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.testTag("pairing-invitation-hint"),
                )
            }
        }
    }
    Spacer(Modifier.height(12.dp))
    when {
        // 201/206 end the invitation. QR recovery is an explicit tap; the
        // camera is never reopened by a snapshot or by recomposition.
        pairing.isBlockingInvitationFailure && pairing.mode == "qr" -> {
            PrimaryButton(
                text = stringResource(R.string.pairing_scan_again),
                onClick = rememberScanStarter(onEvent),
                tag = "scan-again",
            )
            Text(
                stringResource(R.string.pairing_camera_stays_closed),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(top = 8.dp),
            )
        }

        // SAS failures never scan; Try again returns to the in-place entries
        // so the user can restart the exchange on both ends.
        else -> PrimaryButton(
            text = stringResource(R.string.pairing_try_again),
            onClick = { onEvent(UiEvent.ShowPairing) },
            tag = "pairing-try-again",
        )
    }
    PairingBackHome(onEvent)
}

@Composable
private fun PairingSasAndFingerprint(pairing: PairingUi, optionalCode: Boolean) {
    if (pairing.sas != null) {
        Spacer(Modifier.height(8.dp))
        Text(
            stringResource(
                if (optionalCode) R.string.pairing_sas_optional else R.string.pairing_code_on_phone,
            ),
            style = MaterialTheme.typography.labelMedium,
        )
        SasText(pairing.sas)
    }
    PairingFingerprint(pairing.fingerprint)
}

@Composable
private fun PairingFingerprint(fingerprint: String?) {
    if (fingerprint == null) return
    Spacer(Modifier.height(8.dp))
    Text(
        stringResource(R.string.pairing_server_fingerprint),
        style = MaterialTheme.typography.labelMedium,
    )
    Text(
        fingerprint,
        style = MaterialTheme.typography.bodySmall,
        modifier = Modifier.testTag("pairing-fingerprint"),
    )
}

@Composable
private fun PairingCancel(onEvent: (UiEvent) -> Unit) {
    OutlinedButton(
        onClick = { onEvent(UiEvent.CancelPairing) },
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("cancel-pairing"),
    ) {
        Text(stringResource(R.string.pairing_cancel))
    }
}

@Composable
private fun PairingBackHome(onEvent: (UiEvent) -> Unit) {
    TextButton(
        onClick = { onEvent(UiEvent.ShowHome) },
        modifier = Modifier.heightIn(min = 48.dp).testTag("pairing-back-home"),
    ) {
        Text(stringResource(R.string.pairing_back_home))
    }
}

private fun Modifier.liveRegionAssertive(): Modifier =
    semantics { liveRegion = LiveRegionMode.Assertive }

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
