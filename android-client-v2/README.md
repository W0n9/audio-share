# android-client-v2

Android client for wire protocol v2. The native crate under `native/` owns the
QUIC transport, auth handshake, control state machine and reconnect glue; the
Kotlin app owns the UI and the reconnect policy.

## Build

```sh
# Default release build: no fixture test identity is compiled in.
ANDROID_HOME=$HOME/Android/Sdk ./gradlew assembleRelease

# E2E / device-test build: adds the TEST-ONLY fixture client identity key.
ANDROID_HOME=$HOME/Android/Sdk ./gradlew assembleRelease -PtestSupport=true
```

`-PtestSupport=true` enables the native crate's `test-support` feature, which
provides the fixture client identity/signing path used by the end-to-end
harness. It is TEST-ONLY. Never ship that configuration: default release
builds omit the feature, and the production identity comes from v2 pairing
(phase 6).

## Native tests

The native crate is its own Cargo workspace:

```sh
cd native
cargo test                  # production configuration
cargo test --features test-support
```

The Android NDK toolchain is required only for the Gradle release build; the
host tests above need no device.
