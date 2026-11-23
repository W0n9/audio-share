# android-client-v2

Android client for wire protocol v2. The native crate under `native/` owns the
QUIC transport, auth handshake, control state machine and reconnect glue; the
Kotlin app owns the UI and the reconnect policy.

## Build

```sh
# Default release build: no fixture test identity is compiled in.
# Default ABI is arm64-v8a (real devices).
ANDROID_HOME=$HOME/Android/Sdk ./gradlew assembleRelease

# Release artifacts: one signed APK per ABI (the release workflow runs both).
ANDROID_HOME=$HOME/Android/Sdk ./gradlew assembleRelease -PandroidAbis=arm64-v8a
ANDROID_HOME=$HOME/Android/Sdk ./gradlew assembleRelease -PandroidAbis=x86_64

# A universal APK is no longer produced; -PandroidAbis takes a single ABI.

# E2E / device-test build: adds the TEST-ONLY fixture client identity key.
ANDROID_HOME=$HOME/Android/Sdk ./gradlew assembleRelease -PtestSupport=true
```

`-PandroidAbi=x86_64` remains supported for emulator-only development.
`-PtestSupport=true` enables the native crate's `test-support` feature, which
provides the fixture client identity/signing path used by the end-to-end
harness. It is TEST-ONLY. Never ship that configuration: default release
builds omit the feature, and the production identity comes from v2 pairing
(phase 6).

## Release signing

Production release APKs are signed with the dedicated beta keystore; they no
longer fall back to the debug key. Credentials must stay outside the
repository. Gradle reads either environment variables:

```sh
export AUDIO_SHARE_KEYSTORE_FILE=/path/outside/repo/audio-share-beta.jks
export AUDIO_SHARE_KEYSTORE_PASSWORD='...'
export AUDIO_SHARE_KEY_ALIAS=audio-share-beta
export AUDIO_SHARE_KEY_PASSWORD='...'
```

or a properties file containing `storeFile`, `storePassword`, `keyAlias`, and
`keyPassword`, named by `AUDIO_SHARE_KEYSTORE_PROPERTIES` or
`-PreleaseKeystoreProperties=...`. The default external candidate is
`$HOME/.cache/audio-share-beta-keys/keystore.properties`.

`keystore.properties`, `*.jks`, and `*.keystore` are gitignored. Without
credentials a production `assembleRelease` stays unsigned and fails at
`packageRelease` with guidance. Explicit `-PtestSupport=true` and/or
`-PdeviceTests=true` builds are signed with the debug key so test artifacts can
never carry the customer certificate.

## Native tests

The native crate is its own Cargo workspace:

```sh
cd native
cargo test                  # production configuration
cargo test --features test-support
```

The Android NDK toolchain is required only for the Gradle release build; the
host tests above need no device.

## 16 KB page-size alignment

Release native libraries are linked with `-Wl,-z,max-page-size=16384` and
`-Wl,-z,common-page-size=16384` for both ABIs (see
`native/.cargo/config.toml`). Verify a release build non-interactively with:

```sh
ANDROID_HOME=$HOME/Android/Sdk ./scripts/check-elf-alignment.sh \
  app/build/generated/jniLibs/arm64-v8a/libaudio_share_client_v2_native.so \
  app/build/generated/jniLibs/x86_64/libaudio_share_client_v2_native.so \
  app/build/outputs/apk/release/app-release.apk
```

The script uses the NDK 27.2 `llvm-readelf` and build-tools 36.0.0 `zipalign`
(override with `--ndk-dir` / `--zipalign`). It asserts every `LOAD` segment is
at least 16 KB-aligned, the `GNU_RELRO` end is 0x4000-aligned (or no writable
data follows it), and `zipalign -c -P 16 4` passes. It exits non-zero unless
every input prints `ALIGNED`.

## Foreground-service device regression

The ticket-11 device harness observes the real Activity and the public
`SessionController` outcome listener/snapshot seam: `setOutcomeListener`
replays the current `SessionOutcome` on registration, and `outcomeSnapshot()`
restores it after an Activity recreation. Native READY is never simulated.
The test APK uses only Android platform instrumentation: no production
dependencies are added, and the API 26 baseline is unchanged.

```sh
ANDROID_HOME=$HOME/Android/Sdk ./gradlew \
  assembleRelease assembleReleaseAndroidTest \
  -PtestSupport=true -PdeviceTests=true
adb -s "$SERIAL" install -r app/build/outputs/apk/release/app-release.apk
adb -s "$SERIAL" install -r \
  app/build/outputs/apk/androidTest/release/app-release-androidTest.apk

python3 -I scripts/service-device-test.py --serial "$SERIAL" \
  --config "$PUBLIC_CONFIG" --server-log "$SERVER_LOG" \
  --test rebind --output "$RESULT_FILE"
```

`PUBLIC_CONFIG` is JSON with the isolated Server's `host`, integer `port`,
`pinnedSpki` (SPKI SHA-256 hex), and `serverKey` (full Ed25519 public key hex).
Never copy or commit the Server's private key. The Server must authorize the
checked-in fixture's P-256 **public key** in the isolated test configuration;
the test APK's fixture identity is compiled in only with explicit
`-PtestSupport=true`.

Cases are `ready`, `rebind`, `recreate`, `lock`, and `stop`. `rebind` drives a
real session to READY, then asserts that registering a new listener receives
READY **synchronously**; waiting for the next poll does not pass. `recreate`
and `lock` also require `--server-log` to compare connection counts around
READY, so a reconnect cannot mask a lifecycle defect. `stop` checks cleanup
through the public snapshot and Android's actual foreground flag; the Server's
701 close code is covered by device evidence.

MIUI may ask the user to confirm USB installation of the test APK and may
prevent background Activity launches. The harness opens the UI explicitly with
`adb am start`; it does not change those permissions and does not use root.
Before each case it wakes the screen and requests dismiss-keyguard; a password
lock still needs manual unlocking, and an unbound Activity while the screen is
off is not treated as a native-state failure. `lock` turns the screen off and
on for real and tries to restore the screen even if the case fails.

`-PdeviceTests=true` keeps only the few Binder APIs consumed by the separate
test APK; R8 stays enabled. A default `assembleRelease` uses none of those keep
rules and contains no fixture private key. Never publish these test builds.
