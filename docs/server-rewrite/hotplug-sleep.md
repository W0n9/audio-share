# Hotplug and sleep/resume behavior (Linux/PipeWire, ticket 15)

Status: hotplug behavior measured headlessly; automatic capture recovery is
**not implemented and not claimed**. Sleep/resume was **not executed** on this
host. This page records what was run, what the code does, and the exact manual
procedure for the remaining release-gate checks.

## What the code does on target loss

`audio-share-core::capture::linux::PipeWireCapture` ends the capture session
when the target `Audio/Sink` disappears (registry `global_remove`) or when the
realtime callback stalls for 5 s after streaming started. It drops the frame
sender, and the server's broadcast loop ends. The server deliberately keeps
running with its previously negotiated, immutable `AudioFormat`: TCP
`GET_FORMAT` still answers and UDP id binding still works, but no PCM is
produced until the server is stopped and started again. The reason for the
capture end is reported on stderr.

There is **no supervisor** that restarts capture. `as-cmd` has no control
channel to receive a new format; the GUI exposes Start/Stop. The documented
recovery model is therefore operator Stop/Start.

### Is a minimal same-format supervisor feasible?

It is feasible but not a one-line change, and it is not included in ticket 15:

- The current `ServerHandle` owns the only `CaptureSession` and its frame
  channel; a supervisor would need to own the session, keep a long-lived frame
  queue feeding the broadcast loop, and recreate the session with backoff when
  the frame channel closes.
- The published `Arc<AudioFormat>` is immutable and clients cache it from
  `GET_FORMAT`. If the recreated session negotiates a different format, the
  supervisor must close all sessions and surface an error to the GUI/CLI;
  there is no status/error surface for that today. Only a same-format restart
  can preserve the session contract.
- Shutdown must remain bounded and idempotent while a restart is in flight
  (capture `start` can block up to 5 s during negotiation).

A follow-up ticket should add a supervisor with a public status/error event,
retry backoff, and a session-close path on format change. Until then, tests
must not claim automatic recovery.

## Headless test procedure and results

`scripts/hotplug-recovery.sh` runs inside `scripts/pw-test-session.sh` with a
440 Hz tone:

1. Start `target/release/as-cmd` on the null sink (`-e $PW_TEST_ENDPOINT_SERIAL`).
2. Probe: PCM flows and the 440 Hz tone is present.
3. `pw-cli destroy <sink-registry-id>`.
4. Probe with `--min-datagrams 0`: server process alive, zero datagrams.
5. Recreate an `Audio/Sink` with the same name and `priority.session=1000`.
6. Probe again with the *old* server: still zero datagrams (no automatic
   recovery), which proves the honest claim above.
7. Stop the server (SIGINT, exit 0) and start a fresh server on the new sink
   serial; probe: tone returns. This is operator Stop/Start recovery.

Headless observation (Ubuntu 24.04.5, PipeWire 1.0.5, null sink, **release**
binaries, `scripts/hotplug-recovery.sh --skip-build --out /tmp/t15-hotplug-release`):

```text
hotplug: sink=TestSink serial=37
pass baseline capture
pass sink destroyed (registry id 36)
pass server stayed alive, no PCM after target loss
pass sink recreated (new serial 63)
pass no automatic recovery (documented: no capture supervisor)
pass operator Stop/Start recovered capture
hotplug: PASS
```

`/tmp/t15-hotplug-release/hotplug-summary.json` records
`automatic_recovery_implemented: false`, `sleep_resume_executed: false`, the
old/new object serials, and `server_exit_after_target_loss: 0`.

Run it yourself:

```bash
scripts/hotplug-recovery.sh --out /tmp/hotplug
# machine-readable evidence:
jq . /tmp/hotplug/hotplug-summary.json
```

Real-hardware hotplug (unplug/replug a USB/HDMI output, or switch the default
sink) still needs a real desktop and is a release-gate item. The expectation is
the same: capture ends, the server stays up, and the operator uses Stop/Start.

## Sleep/resume: NOT EXECUTED

This host has no controllable suspend path (`systemctl suspend` is unavailable
in the session and suspending the shared machine would disrupt other work).
Sleep/resume therefore remains a human check:

1. Start the Server on a real Linux desktop with a real output device and a
   phone connected/playing.
2. `systemctl suspend` (or close the laptop lid), wait ~30 s, resume.
3. Observe: does PipeWire reconnect the capture stream automatically? Does the
   phone keep playing silence, disconnect, or reconnect?
4. If audio does not resume, press Stop then Start in the GUI (or restart
   `as-cmd`) and verify audio returns with the same negotiated format.
5. Record the sink ids/format before and after resume, the server log lines,
   and the phone behavior. On Windows, repeat with modern standby/S3.

Until this is recorded, the ticket-15 checkbox "device hotplug and sleep
recovery" stays unchecked, and automatic recovery is explicitly not claimed.
