# Audio Share 0.5.0-beta — release notes draft

Status: published GitHub Release (pre-release) on `W0n9/audio-share`, 2026-11-18.
The annotated tag `v0.5.0-beta` points at `362d023`; the artifacts below are attached as
release assets. F-Droid publication was not performed.
Built from `362d023` on `integration/wire-protocol-v2-beta` (includes the Android
modernization and the ticket-33 Server log events).

This is the Wire Protocol v2 customer beta: an encrypted, authenticated v2 Client and
Server for a controlled trial. It is not a v1 daily-driver replacement.

## Artifacts and SHA-256

Android Client v2 (signed universal APK, arm64-v8a + x86_64):

```
2024ae1423090670f6b46b92277fbed39df40dcc8f0ad74d4b801fa44722677a  audio-share-client-v2-0.2.0-beta.apk
```

Linux Server deb:

```
2b8a4ba246791b5f09a0b123ae7ba2b698c54561eca1d963ea039866678b5f9d  AudioShareServer_0.5.0-beta_amd64.deb
```

Linux Server AppImage:

```
78e91548ab188784df97b40e253b905be12a15e89f5f5e0d7d1a628f80a7b085  AudioShareServer_0.5.0-beta_amd64.AppImage
```

Linux `as-cmd` archive (contains `audio-share-server-cmd/as-cmd`, LICENSE, README):

```
66dd752065fd5ec54cfb429d6132d8866fcffd21cac369d17543cf63171aec5a  audio-share-server-cmd-linux.tar.gz
```

Windows portable Server (`AudioShareServer.exe` + `as-cmd.exe` archive): **BUILT on Linux
with `cargo-xwin`, NOT RUN**. This is a cross-built PE32+ x86-64 artifact, not executed on
Windows or Wine in this pass; Windows acceptance is pending. Hashes:

```
78874511654260e77a6a5c40bcb0c504a71e8dcd81fb458bce22ab0816c6805c  AudioShareServer.exe
d5f9e977702670520547cc27b2633bbb9c443956fb25ababdd234cae4602e9a8  audio-share-server-cmd-windows.zip
```

`as-cmd.exe` needs the Microsoft Visual C++ 2015-2022 Redistributable (x64); the GUI needs
the WebView2 Runtime. The CI workflow still builds the Windows portable Server on
`windows-latest` as the independent check.

**2026-11-18 modernization rebuild.** Ticket 35 chose option B: the tag moved from
`8858c91` to `362d023` and every artifact was rebuilt from that tip. The Client now ships
the modernized Direction A UI (HOME / PAIRING / SESSION / SETTINGS / ABOUT, Light/Dark,
en + zh-CN), `SessionOutcome`-based reconnect handling, and the Server writes the v2
session events from ticket 33 (`pairing.presented/accepted/declined`, `session.accepted`,
`session.closed` with the §6 code, `client.revoked`, `identity.reset`). Previous APK
`c1d1b236…2af58` and Server hashes are superseded. QR pairing remains one-step; SAS still
requires comparing/entering the code on the phone. Install with `adb install -r` only from
a same-signer build; an older v2 beta should be uninstalled first.

**Evidence binding.** JVM/Robolectric (`./gradlew test -PnoNative=true`, JDK 21) was re-run
on `362d023`: 250/250 in both variants PASS. Re-running real-device, Windows, and emulator
acceptance on these exact hashes was **WAIVED** by the maintainer on 2026-11-18 on
code-equivalence grounds: the Android app source is unchanged since `86b2635` (the only
later change is Server/GUI logging from ticket 33), and the Server protocol/media paths are
unchanged. `WAIVED` is **not** a PASS: the statements below about real-device, Windows, and
emulator behaviour were measured on earlier builds and were not repeated on the final
artifacts.

The GitHub Release for this beta is intended to contain the APK, Windows portable
Server, Linux deb/AppImage, `as-cmd`, and the SHA-256 checksums. Only the local artifacts
listed here were produced in this pass.

## What is in the beta

- v2 Android Client with the modernized five-screen Compose UI (HOME, PAIRING, SESSION,
  SETTINGS, ABOUT; Light/Dark; en + zh-CN): Server address, QR/SAS pairing, fingerprint,
  status-first session screen with reconnect attempt count, playback, Stop, About/limitations,
  and a foreground media notification with Stop.
- Server `0.5.0-beta` with a shared Pairing Core, `as-cmd` pairing/administration
  (`pair`, `clients list|revoke`, `identity reset`), and the Server GUI pairing dialog,
  paired-device list, and revoke.
- Encrypted QUIC transport (`audio-share/v2`, ALPN), authenticated control channel,
  single-Client PCM playback, ordered delivery before playback, and explicit close.
- v1 remains enabled by default and byte-compatible.

## v1 compatibility

- **v1 is frozen and still works.** Existing v1 framing, commands, UDP behavior, and
  little-endian layout are unchanged.
- **v1 is UNENCRYPTED and UNAUTHENTICATED. Use v1 only on a trusted network.**
- The released v1 Android APK (v0.3.4) and its F-Droid release remain available. This
  beta does not replace or remove them; the v2 app uses `dev.audioshare.v2`, so it can be
  installed alongside v1.
- A v2 failure never falls back to v1, plaintext, or anonymous encryption. v1 and v2 are
  separate paths.

## Known beta limitations

- One Client per Server. Concurrent phones, multi-Client isolation, and the two-device
  60-minute gate are out of scope.
- No jitter buffer, packet-loss concealment, or clock-drift correction. Audio quality is
  best-effort and has not been measured for the pilot.
- No diagnostics export, no telemetry, and no raw-audio recording.
- No F-Droid distribution and no in-app auto-update. Updates are manual APK/installer
  downloads from the release page.
- No boot start and no background restart after reboot.
- No Server GUI live v2 session/diagnostic view. Identity reset is `as-cmd` only.
- No mDNS discovery, STUN/TURN, NAT hole punching, relays, accounts, browser/iOS clients,
  Opus/FEC/DTX, or application-layer retransmission.
- Windows Server: **PASS (Windows 11 short smoke, 2026-11-17, earlier build; not re-run on
  the final artifacts — WAIVED)**. The portable GUI and `as-cmd.exe` were cross-built on
  Linux with `cargo-xwin` and executed on Windows 11: version/hashes, GUI + as-cmd startup,
  QR one-step pairing, 5-minute playback, Stop → saved-server reconnect, revoke → 204,
  identity reset → 205, and v1 coexistence. The 30-minute playback, lock-screen, and
  network-drop cases were accepted from the earlier real-device run and were not repeated in
  the final short smoke. The final `362d023` Windows executables are built but **NOT RUN**.
- **Server log coverage**: `server.log` now records structured v2 events
  (`pairing.presented/accepted/declined`, `session.accepted`, `session.closed` with the §6
  code, `client.revoked`, `identity.reset`) at info level (ticket 33). The Windows GUI path
  for these lines was **not** re-run on the final build (WAIVED); if a line is missing in a
  field report, use the Android logcat for close-code evidence.
- **Upgrade from an earlier v2 beta**: `adb install -r` over an older v2 beta can retain
  stale credential state and cause a 205 identity-mismatch on "Connect to saved Server".
  Uninstall the old v2 beta first, then install this APK and pair again (ticket 34).

## Platform matrix

| Platform | Status | Notes |
|---|---|---|
| Linux Ubuntu 24.04 x86_64 | PASS (packaging/build) | deb and AppImage rebuilt from `362d023` and checksummed. Interactive pairing/capture UI not exercised headlessly. |
| Windows 11 | PASS (prior build); final exe **NOT RUN** / WAIVED | The 2026-11-17 short smoke ran a pre-modernization exe. The `362d023` `AudioShareServer.exe` is cross-built and well-formed PE32+ but was not executed. |
| Android emulator API 35 x86_64 | EMULATOR (prior commit); final APK not re-run / WAIVED | EMULATOR matrix PASS is bound to `86b2635` and a test-support APK. The final R8 release APK was not installed on an emulator. Real device remains NOT RUN. |
| Android real arm64/API 31 device | PASS (prior builds); final APK **NOT RUN** / WAIVED | Redmi M2007J22C, Android 12/API 31: QR/SAS pairing, 5m09s playback, 3m14s lock, network-drop + Stop, revoke 204, identity reset 205 on earlier builds. The `362d023` signed release APK was not installed on a device. |

Debian note: the deb package version string is `0.5.0-beta`. Debian treats the suffix
after `-` as a package revision, so it does not have semver prerelease ordering. This is
a beta-only artifact; use the AppImage or `as-cmd` archive if exact package ordering
matters.

## Upgrade

Server:

1. Stop the old Server process.
2. Keep the existing `config.ini` and data directory. The v2 identity and pairing records
   live under the data directory (`identity/` and `pairing/clients.json`).
3. Replace the executable, or install the new deb/AppImage. On Linux, the deb depends on
   `libpipewire-0.3-0`.
4. Start the Server. v1 stays enabled; v2 listens on the configured `--v2-port`
   (default 65531). `--v2-port=0` disables v2 without affecting v1.

Android:

1. Install the signed `audio-share-client-v2-0.2.0-beta.apk`. It coexists with the v1 app.
2. If an earlier v2 beta (including a debug-signed test build) of `dev.audioshare.v2` is
   installed, **uninstall it first**. Installing over retained old-beta data can cause a
   205 identity mismatch on the next saved-server reconnect. Uninstalling loses the stored
   pairing; pair again after installing this APK.
3. Pair by QR (preferred) or SAS/fingerprint, then connect.

## Rollback

- Server: stop the beta and reinstall the previous 0.4.x executable/package. The v1
  configuration is unchanged. If the beta created v2 pairing files, they are ignored by
  the old Server; remove them only if you intend to discard those pairings.
- Android: uninstall the beta APK and keep or reinstall the v1 APK. Uninstalling removes
  the beta's stored credential; there is no cross-version credential migration.
- A Server identity reset (`as-cmd identity reset --yes`) invalidates every v2 pairing and
  requires re-pairing. There is no universal recovery backdoor.

## Verify downloads

Compare the downloaded file with the SHA-256 above:

```bash
sha256sum audio-share-client-v2-0.2.0-beta.apk
sha256sum AudioShareServer_0.5.0-beta_amd64.deb
sha256sum AudioShareServer_0.5.0-beta_amd64.AppImage
sha256sum audio-share-server-cmd-linux.tar.gz
```

APK signature (Android build-tools 35.0.0):

```bash
apksigner verify --verbose --print-certs audio-share-client-v2-0.2.0-beta.apk
```

Expected signer certificate SHA-256:

```
90:BE:DC:D1:D8:15:A0:D7:72:89:7E:92:8E:B2:C9:89:4F:6D:1C:A0:38:C5:51:9A:36:DA:6D:B2:7A:A7:A1:3C
```

The APK uses APK Signature Scheme v2 and v3 (v1 disabled; minSdk 26). Package
`dev.audioshare.v2`, versionCode 2, versionName `0.2.0-beta`, native code `arm64-v8a` and
`x86_64`. Explicit test-support/device-test builds are debug-signed and must not be
published; only the APK above carries the beta certificate.

Server version:

```bash
./as-cmd --version   # version: 0.5.0-beta
```

## Feedback

Feedback channel: `<TO BE AGREED WITH PILOT CUSTOMERS>`.

Please include the app version, Server version and platform, pairing method (QR or SAS),
the state/error code shown in the app, and the relevant Server log lines. Do not send
private keys, pairing secrets, QR payloads, SAS codes, or raw audio. No telemetry is
collected.

## Windows acceptance

Windows 11 GUI/headless acceptance steps, artifact transfer, firewall ports, pairing,
playback, revocation, identity reset, and evidence collection are in
[`.scratch/wire-protocol-v2/windows-handoff.md`](wire-protocol-v2/windows-handoff.md).
The Windows binaries are cross-built on Linux and carried to Windows; the Windows host
needs no Rust/Node/Visual Studio toolchain.
