package dev.audioshare.v2

import android.Manifest
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.mutableStateOf

/**
 * Beta UI host. It only knows the [SessionController] Binder seam: no JNI,
 * QUIC, pairing transcript, or Keystore call lives in this class.
 *
 * The Service owns the session; this Activity binds for state replay, applies
 * the pure [AppReducer], and forwards the resulting [AppCommand].
 */
class MainActivity : ComponentActivity() {
    private val state = mutableStateOf(AppState())
    private val prefs by lazy { SharedPreferencesUiPreferences(this) }
    private val handler = Handler(Looper.getMainLooper())

    private var controller: SessionController? = null
    private var bound = false
    private var hadSavedState = false
    private var initialized = false
    private var harnessStarted = false
    private var credentialPolling = false

    private val notificationPermission = registerForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { /* The foreground Service still runs; the notification is suppressed. */ }

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, binder: IBinder?) {
            val service = (binder as? ControlBinder)?.service ?: return
            controller = service
            service.setOutcomeListener { outcome ->
                runOnUiThread { dispatch(UiEvent.SessionOutcomeChanged(outcome)) }
            }
            service.setPairingListener { snapshot -> runOnUiThread { onPairingSnapshot(snapshot) } }
            initialize(service)
        }

        override fun onServiceDisconnected(name: ComponentName?) {
            controller = null
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        hadSavedState = savedInstanceState != null
        setContent {
            AudioShareTheme {
                AudioShareApp(
                    state = state.value,
                    versionName = BuildConfig.VERSION_NAME,
                    versionCode = BuildConfig.VERSION_CODE,
                    onEvent = ::dispatch,
                )
            }
        }
        if (BuildConfig.TEST_SUPPORT && hasHarnessConfiguration()) {
            // TEST-SUPPORT ONLY: the historic adb-extras lifecycle harness.
            // Auto-connect is suppressed so the harness owns the only session.
            harnessStarted = tryTestSupportSession()
        }
    }

    override fun onStart() {
        super.onStart()
        if (!bound) {
            bound = bindService(
                Intent(this, ControlSessionService::class.java),
                connection,
                Context.BIND_AUTO_CREATE,
            )
        }
    }

    override fun onStop() {
        if (bound) {
            unbindService(connection)
            bound = false
            controller = null
        }
        super.onStop()
    }

    override fun onDestroy() {
        handler.removeCallbacksAndMessages(null)
        credentialPolling = false
        if (isFinishing && !isChangingConfigurations) {
            startService(Intent(this, ControlSessionService::class.java).apply {
                action = ControlSessionService.ACTION_STOP
            })
        }
        super.onDestroy()
    }

    // ---- binding / startup -------------------------------------------------

    private fun initialize(service: SessionController) {
        val firstBind = !initialized
        initialized = true
        dispatch(UiEvent.StartupLoaded(prefs.lastAddress(), service.credentials()))
        val outcome = service.outcomeSnapshot()
        if (outcome != null) dispatch(UiEvent.SessionOutcomeChanged(outcome))

        if (firstBind) {
            requestNotificationPermissionIfNeeded()
            val credentials = service.credentials()
            if (!harnessStarted && AutoConnectPolicy.shouldAutoConnect(
                    savedStatePresent = hadSavedState,
                    hasCredential = credentials.isNotEmpty(),
                    outcome = outcome,
                )
            ) {
                dispatch(UiEvent.AutoConnectStarted)
            }
        }
    }

    private fun requestNotificationPermissionIfNeeded() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        val granted = checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        if (!granted) notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
    }

    private fun hasHarnessConfiguration(): Boolean =
        intent?.getStringExtra("host").orEmpty().isNotEmpty()

    private fun tryTestSupportSession(): Boolean = try {
        Class.forName("dev.audioshare.v2.TestSupportHarness")
            .getMethod("startActivitySession", android.app.Activity::class.java)
            .invoke(null, this) as Boolean
    } catch (_: Throwable) {
        false
    }

    // ---- reducer plumbing --------------------------------------------------

    private fun dispatch(event: UiEvent) {
        val reduction = AppReducer.reduce(state.value, event)
        state.value = reduction.state
        execute(reduction.command)
        if (event is UiEvent.AddressChanged &&
            event.text.isNotBlank() &&
            reduction.state.addressError == null
        ) {
            prefs.saveLastAddress(event.text)
        }
    }

    /**
     * The reducer's commands map one-to-one onto the [SessionController] seam.
     * There is no Activity-scoped state between the two: stop attribution and
     * outcome classification belong to the Service/session owner.
     */
    private fun execute(command: AppCommand) {
        val controller = controller ?: return
        when (command) {
            AppCommand.None -> Unit
            is AppCommand.BeginQr -> controller.beginQrPairing(command.uri)
            is AppCommand.BeginSas -> controller.beginSasPairing(command.address)
            AppCommand.ConfirmPairing -> controller.confirmPairing()
            AppCommand.CancelPairing -> controller.cancelPairing()
            is AppCommand.ConnectStored -> controller.connectStored(command.serverSpkiSha256Hex)
            AppCommand.Stop -> controller.disconnect()
        }
    }

    private fun onPairingSnapshot(snapshot: PairingSnapshot?) {
        dispatch(UiEvent.PairingSnapshotChanged(snapshot))
        if (snapshot?.isComplete == true) awaitCredential(snapshot.serverSpkiSha256Hex)
    }

    /**
     * `PairingResult(OK)` persists the credential just after the COMPLETE
     * snapshot, so connect only once the record is actually in the store.
     */
    private fun awaitCredential(pin: String?) {
        if (credentialPolling) return
        credentialPolling = true
        val deadline = SystemClock.uptimeMillis() + 5_000
        val poll = object : Runnable {
            override fun run() {
                val service = controller
                if (service == null) {
                    credentialPolling = false
                    return
                }
                val record = if (pin != null) {
                    service.credentials().firstOrNull { it.serverSpkiSha256Hex == pin }
                } else {
                    service.credentials().lastOrNull()
                }
                if (record == null && SystemClock.uptimeMillis() < deadline) {
                    handler.postDelayed(this, 100)
                    return
                }
                credentialPolling = false
                if (record != null) dispatch(UiEvent.CredentialStored(record))
            }
        }
        handler.post(poll)
    }
}
