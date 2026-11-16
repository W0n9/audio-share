# Evidence policy

How verification results are recorded in this repo. Every claim of verification must use
one of the statuses below; a result may never be promoted to a stronger status than what
was actually run.

## Statuses

| Status | Meaning |
| --- | --- |
| `PASS` | The stated behaviour was verified on the stated platform, with the command, artifact, and output recorded. |
| `FAIL` | The stated behaviour was exercised on the stated platform and did not meet the requirement. |
| `NOT RUN` | The behaviour was not exercised. Absence of failure is not `PASS`. |
| `WAIVED` | The maintainer explicitly accepted the unverified risk. This is not validation and must not be written as `PASS`. |
| `EMULATOR` | The behaviour was verified on an Android emulator. The real device remains `NOT RUN` unless separately recorded. |

## `EMULATOR`

`EMULATOR` is a formal evidence status alongside the four above. Every `EMULATOR` record
must name the AVD (`emulator:<avd-name>`), the API level, the ABI, the app version, the
Server version, and the build flags used (for example
`-PandroidAbi=x86_64 -PtestSupport=true -PdeviceTests=true`). It must state that the real
device is `NOT RUN`.

`EMULATOR` can satisfy:

- functional UI flows, address entry, pairing choice, error display, stop;
- protocol, state-machine, authorization, revocation, and negative-path behaviour;
- Android lifecycle behaviour (foreground Service, ready, rebind, recreate, lock, stop);
- API 26 runtime presence and installability;
- release APK installability, R8, and resource packaging;
- Keystore use with the emulator's software-backed implementation.

`EMULATOR` can never replace:

- audible quality, end-to-end audible latency (p95/p99), underrun behaviour, or
  `AudioTrack` buffer-size findings;
- MIUI/vendor-specific behaviour (for example `Network.bindSocket` EPERM) and real
  Wi-Fi-to-cellular handover;
- the 60-minute two-real-device gate, multi-Client concurrency judged on audio, or Server
  soak judged on audio;
- IPv6 and real dynamic-MTU paths;
- final release-candidate acceptance.

## Rules

- Host stubs, Java/Kotlin compilation, JVM unit tests, and single-end encoder tests are not
  device, emulator, interop, or real-device `PASS`. Record what was actually run.
- Cross-end interop must be labelled `interop` and name both ends; one end's encoder
  replay is not interop.
- Evidence lives under `.scratch/<feature>/evidence/` and stays untracked unless the
  maintainer asks otherwise.
- Every acceptance record lists the device or emulator identity, OS/API level, app and
  Server versions, known limitations, and every item that remains `NOT RUN`.
- A `WAIVED` item may only be changed by maintainer adjudication with new evidence; it is
  never silently upgraded.

## Release stages

The release-stage vocabulary (Beta, Experimental, Stable) is defined in `GLOSSARY.md`.
Beta is a controlled-trial Client release; Experimental is the Server-side protocol status
before the stable gate; Stable requires the full acceptance gate, migration guide, and
rollback path.
