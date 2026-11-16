package dev.audioshare.v2

/**
 * Applies reducer commands to the [SessionController] seam. This is the only
 * place the UI forwards side effects, which keeps it JVM-testable with a fake
 * controller and keeps Android out of the command logic.
 */
class SessionCommandExecutor(private val controller: SessionController) {
    private var expectedStop = false

    fun execute(command: AppCommand) {
        when (command) {
            AppCommand.None -> Unit
            is AppCommand.BeginQr -> controller.beginQrPairing(command.uri)
            is AppCommand.BeginSas -> controller.beginSasPairing(command.address)
            AppCommand.ConfirmPairing -> controller.confirmPairing()
            AppCommand.CancelPairing -> controller.cancelPairing()
            is AppCommand.ConnectStored -> {
                expectedStop = false
                controller.connectStored(command.serverSpkiSha256Hex)
            }
            AppCommand.Stop -> {
                expectedStop = true
                controller.disconnect()
            }
        }
    }

    /**
     * Guidance for a CLOSED transition. A user-requested Stop must not
     * surface a stale error code from the previous session.
     */
    fun guidanceOnClosed(): ErrorGuidance? {
        val stopped = expectedStop
        expectedStop = false
        return if (stopped) null else controller.lastErrorGuidance()
    }
}
