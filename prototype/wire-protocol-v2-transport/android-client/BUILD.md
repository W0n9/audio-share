# android-client build report — ticket 01 (wire-protocol-v2 prototype)

Throwaway prototype. Everything here is evidence for the ticket's item 6
("Release Build and Integration Cost"), not production guidance.

## What was built

Standalone gradle project at `prototype/wire-protocol-v2-transport/android-client/`:

- `app/` — Kotlin module (minSdk 26, compileSdk 35, targetSdk 35, framework
  APIs only — no androidx/compose). One `MainActivity` (connection inputs,
  state, counters, log), one foreground `MediaPlaybackService`
  (`mediaPlayback` type), `ProtoSession` (JNI lifecycle, direct-ByteBuffer
  consumer, bounded exponential backoff with ±20% jitter,
  `ConnectivityManager.NetworkCallback` migration scaffold).
- `native/` — Rust crate `audio-share-proto-native` built as `cdylib`
  `libaudio_share_proto_native.so`: quinn client + rustls with SPKI-pinned
  custom verifier, ALPN `audio-share/v2`, control frame codec + client state
  machine, deterministic-media verification (sequence gaps / duplicates /
  reorder / byte-exact waveform), tokio runtime, network-change rebind
  (`Network.bindSocket` fd handoff → quinn `Endpoint::rebind`), and the
  overwrite-oldest ring fed by the native datagram loop into the attached
  direct ByteBuffer. The wire contract itself (frame layout, media header,
  waveform formula, SPKI helpers) is **single-sourced on `proto-core`** via a
  plain path dependency on `../../server/proto-core` — the crate still opts
  out of the root cargo workspace; no wire constant is declared twice.

## Exact versions (this build host, 2026-11-11)

| Component | Version |
|---|---|
| Rust toolchain | 1.98.1 (2026-09-01) |
| Rust target | `aarch64-linux-android` (same toolchain version, installed via `rustup target add`) |
| quinn | 0.11.12 (quinn-proto 0.11.19) |
| rustls | 0.23.46 (rustls-pki-types 1.16.0, ring provider 0.17.14) |
| tokio | 1.54.1 (multi-thread runtime, 2 workers) |
| jni crate | 0.21.1 |
| rcgen (dev-dep, host tests only) | 0.13.2 |
| android_logger | 0.14.1 (Android target only) |
| NDK | 27.2.12479018 |
| Android Gradle Plugin | 8.7.3 |
| Kotlin (AGP plugin) | 2.0.21 |
| Gradle (wrapper, copied from `android-app/`) | 8.9 |
| compileSdk / targetSdk | 35 |
| minSdk | **26** (prototype gate) |
| Kernel (build host) | Linux 6.8 x86_64 |
| Java | 17 (temurin/u24.04 distro build) |

`ANDROID_HOME` is not set globally on this machine — it is exported per shell /
by the gradle task (`export ANDROID_HOME=/home/w0n9/Android/Sdk`).

## Reproducing the native library

```sh
export ANDROID_HOME=/home/w0n9/Android/Sdk
rustup target add aarch64-linux-android
cd prototype/wire-protocol-v2-transport/android-client/native
cargo test                                   # 58 host tests, all green
cargo build --release --target aarch64-linux-android
```

Toolchain wiring: **direct NDK clang linker config in
`native/.cargo/config.toml`** (cargo-ndk was NOT used — recorded per scope):

- `target.aarch64-linux-android.linker` =
  `$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android26-clang`
  (API-26 wrapper — the produced binary links the exact minSdk bionic).
- `[env] CC_aarch64_linux_android` / `AR_aarch64_linux_android` point at the
  same clang wrapper and `llvm-ar` — needed because `ring`'s cc-rs build
  script looks for a bare `aarch64-linux-android-clang` that the NDK does not
  ship. Without this the build fails with
  `cc-rs: failed to find tool "aarch64-linux-android-clang"`.
- The absolute paths are machine-local to this build host; on another host,
  edit the two paths in `native/.cargo/config.toml`.

## Reproducing the APK

```sh
export ANDROID_HOME=/home/w0n9/Android/Sdk
cd prototype/wire-protocol-v2-transport/android-client
./gradlew assembleRelease                  # with native lib (R8 minified, debug-signed)
./gradlew assembleRelease -PnoNative       # empty baseline (native tasks skipped)
./gradlew clean                            # removes generated jniLibs + apk outputs
```

- The release build type is **debug-signed** (no release keystore exists for a
  prototype) so `assembleRelease` produces an installable APK directly.
- cargo runs as a gradle `Exec` task (`buildNative`) before
  `merge*JniLibFolders`; the output is copied into
  `app/build/generated/jniLibs/arm64-v8a/` and stripped **in place with the
  NDK's `llvm-strip`** (task `stripNative`).
- The generated jniLibs dir lives under `build/` so `clean` and `-PnoNative`
  baseline builds can never see a stale `.so` in a source directory. Binaries
  are gitignored; sources and this document are committed.

## Measured sizes (arm64-v8a only, `abiFilters = ["arm64-v8a"]`)

Re-measured 2026-11-11 after the wire contract was single-sourced on
proto-core and the rebind path was added; deltas vs. the first measurement
were +16,152 B raw / +1,720 B stripped (+0.07% — within noise of the added
rebind code) and the gate verdict is unchanged.

| Artifact | Bytes | KiB | MiB |
|---|---:|---:|---:|
| `libaudio_share_proto_native.so` raw (debug-info symbols, not stripped) | 8,778,032 | 8572 | 8.37 |
| `libaudio_share_proto_native.so` stripped (NDK `llvm-strip`) | 2,305,128 | 2251 | 2.20 |
| APK release **with** native lib | 2,339,239 | 2284 | 2.23 |
| APK release **baseline** (no native lib) | 33,757 | 33.0 | 0.032 |
| **APK increment (native contribution)** | **2,305,482** | **2251** | **2.20** |
| APK debug with native lib (R8 off) | 3,141,687 | 3068 | 3.00 |
| APK debug baseline (R8 off) | 831,313 | 812 | 0.79 |

**~5 MiB gate: PASS on this evidence.** The stripped arm64 `.so` increment is
2.20 MiB (44% of the provisional 5 MiB gate). The whole release APK including
the native library is 2.23 MiB. ABI approach: single-ABI filter rather than
splits; the per-arm64 contribution is identical for splits/AAB.

Rough .so size composition (from dependency weight at `opt-level="z"`,
`lto=fat`, `codegen-units=1`): the bulk is quinn+rustls+ring (TLS + QUIC),
tokio runtime, and panic/format machinery. No rcgen/rcgen-related code ships
(dev-dependency only); no serde/protobuf.

## R8 / ProGuard rules required

`app/proguard-rules.pro`:

```
-keep class dev.prototype.wireproto.ProtoNative { *; }
-keepattributes SourceFile,LineNumberTable
```

The `-keep` is load-bearing: JNI resolves symbols by name at `loadLibrary`
time, so R8 must not rename/remove the entry-point class. Nothing else needed
(the app has no reflection or serialization beyond `org.json`, which ships in
the platform). R8 minify + resource shrink both pass with these rules.

## JNI symbols (exported by the stripped .so)

```
Java_dev_prototype_wireproto_ProtoNative_attachBuffer
Java_dev_prototype_wireproto_ProtoNative_connect
Java_dev_prototype_wireproto_ProtoNative_createSession
Java_dev_prototype_wireproto_ProtoNative_destroy
Java_dev_prototype_wireproto_ProtoNative_getState
Java_dev_prototype_wireproto_ProtoNative_getStats
Java_dev_prototype_wireproto_ProtoNative_rebind
Java_dev_prototype_wireproto_ProtoNative_stop
Java_dev_prototype_wireproto_ProtoNative_subscribe
Java_dev_prototype_wireproto_ProtoNative_unsubscribe
```

All ten are low-frequency: session lifecycle, stats polling, and one call
per network change (`rebind`). Media bytes cross JNI exactly once — as one
`ByteBuffer.allocateDirect(4 MiB)` attached via `attachBuffer`; the native
ring writes into it and Kotlin reads it with the monotonic `buffer.write_pos`
cursor from `getStats` JSON. There is no per-datagram callback, no UniFFI, no
per-packet JNI traffic.

## minSdk / dependency platform constraints

- Gate: API 26 — **holds**. The crate links against the API-26 NDK clang
  wrapper; the app declares `minSdk = 26`.
- No dependency imposed a higher minSdk. The app deliberately has zero
  external Android dependencies (framework APIs only), so the only compiled
  third-party artifact is `kotlin-stdlib` (bundled by the Kotlin plugin), which
  does not raise minSdk.
- `rustls`/`quinn`/`ring`/`tokio` are pure-Rust or C-free here (ring uses its
  own asm, compiled by the API-26 clang wrapper above) — no cmake, no
  autotools, no additional NDK components required.

## Host test evidence (`cargo test` in `native/`)

58 tests, all green, zero warnings; modules and what each guards:

- `codec` (11): frame round-trips for all 7 message types, BE layout,
  streaming decode across push boundaries, 64 KiB cap, zero/oversized/unknown
  type/malformed-payload rejection — all against the **shared proto-core
  codec**, plus two cross-end contract tests (proto-core-encodes →
  client-decodes and client-encodes → proto-core-parses).
- `control` (11): AUTH_OK→READY, AUTH_ERR terminal, media-before-auth
  protocol error, duplicate AUTH, SUBSCRIBE-only-in-READY gating,
  MEDIA_START reconfiguration, CLOSE from any state, no processing after
  close.
- `media` (14): 20-byte BE header parse, gap/reorder/duplicate accounting,
  instance-change reset, deterministic waveform byte verification
  (incl. corruption detection at a computed sample position), misaligned
  payload rejection, config-generation mismatch counting.
- `ring` (7): contiguous appends, wraparound, reader-lap detection by cursor
  arithmetic, oversize chunk drop + `dropped_bytes`, cursor consistency over
  10k writes.
- `verify` (6): SPKI pin extraction/sha256 via proto-core matches rcgen's
  `public_key_der()`, pin hex round-trip, pinned-cert acceptance, wrong-key
  rejection, garbage DER rejection. (No QUIC connection needed — pure
  verifier logic against rcgen-generated certs.)
- `session` (3): network-change rebind primitive (endpoint rebinds onto a raw
  fd transferred with `into_raw_fd`, local address follows), rebind failure
  before the endpoint exists and on a stopped session without touching the fd.
- `stats` (5): golden JSON shape, null sections, close object, escaping,
  null `highest_seq` before first datagram.

## What is NOT tested (device/network gates)

Everything below compiles but has no executable evidence from this host:

- QUIC handshake against a real server, ALPN negotiation behavior on-wire,
  0-RTT absence on the wire (config never enables it — quinn/rustls reject
  early data unless explicitly enabled, which this prototype never does).
- `session.rs` / `media_pump` runtime behavior (control loop, datagram loop,
  ring feeding under real traffic).
- All ten JNI functions at runtime (no device; `adb devices` empty). The
  rebind primitive's endpoint-level logic IS host-tested (see `session`
  tests above); the JVM fd handoff and the live-connection migration are not.
- Foreground service lifecycle, `startForeground` typed paths, screen-off,
  process death (`START_NOT_STICKY` is configured but unobserved).
- Backoff reconnects under real network loss; NetworkCallback
  onAvailable/onLost firing; `Network.bindSocket` fd handoff;
  `Endpoint::rebind` against a live QUIC connection on a real network change.
- Playback consumption via `AudioTrack`.

## Network migration: what is implemented vs. device-validated (honesty section)

Implemented (code on both ends, end-to-end behavior **NOT RUN** on a device):

- `ConnectivityManager.NetworkCallback` (INTERNET capability filter) with
  `onAvailable` / `onLost` handlers; the active network's local address
  (IPv4 preferred) is captured and passed to native `connect()` as a UDP
  socket bind hint for the initial connection.
- **Network-change rebind (implemented, replacing the old scaffold):** on
  `onAvailable` Kotlin creates a `DatagramSocket`, route-pins it to the new
  `Network` via `Network.bindSocket(socket)`, wraps it with
  `ParcelFileDescriptor.fromDatagramSocket(...)` and passes the raw fd to the
  new JNI function `ProtoNative.rebind(handle, fd)`. Native rebuilds a
  `std::net::UdpSocket` from the raw fd (`FromRawFd`) and calls quinn's
  `Endpoint::rebind()`, which moves the endpoint — and all live connections —
  onto the new socket. FD ownership transfers to native on call (success or
  failure); the JVM side never closes the socket afterwards.
- On rebind failure (or before any endpoint exists): the pre-existing
  teardown + bounded exponential backoff with jitter (500 ms → 30 s, ±20%)
  reconnect is the fallback. `onLost` always tears down (the route is gone).

Host-tested (in `session.rs`): the rebind primitive itself — a bare endpoint
rebinds onto an `into_raw_fd`-transferred UDP fd and reports the new local
address; rebind without a stored endpoint and rebind on a stopped session fail
without touching the fd.

NOT RUN (needs a device; `adb devices` empty on this host):

- an actual Wi-Fi↔cellular (or Wi-Fi↔Wi-Fi) migration while media flows:
  callback firing, `Network.bindSocket` + fd handoff, `Endpoint::rebind`
  under live QUIC traffic, continuity of the media stream across the swap;
- whether QUIC path validation/`Response` migration actually completes on the
  new network without a connection loss (the endpoint keeps connections, but
  a full path change may still drop them — that is exactly what the device
  run must answer).

## Build failures encountered (all fixed, recorded per scope)

1. `current package believes it's in a workspace` — repo-root cargo workspace
   pulled the crate in; fixed with an explicit empty `[workspace]` in the
   crate manifest (prototype isolation).
2. `cc-rs: failed to find tool "aarch64-linux-android-clang"` — fixed with
   `CC_aarch64_linux_android`/`AR_aarch64_linux_android` in `.cargo/config.toml`
   `[env]` (NDK ships only API-suffixed wrappers).
3. Gradle `Exec` working directory resolved to the app module dir, so cargo
   walked up and started building the repo-root workspace; fixed by resolving
   `rootProject.projectDir.resolve("native")`.
4. First `-PnoNative` baseline APK was byte-identical to the native APK
   because the stripped `.so` had been copied into `src/main/jniLibs` (not
   removed by `clean`); fixed by generating jniLibs under `build/` and
   re-capturing the baseline after a clean.

## Library defaults observed (evidence, not decisions)

- rustls 0.23 + ring provider: TLS 1.3 only unless TLS 1.2 is enabled
  explicitly — the client config here enables no versions explicitly and gets
  1.3-only defaults; on-wire assertion remains a device/e2e gate.
- quinn 0.11 defaults: datagrams enabled when the server advertises
  `max_datagram_size` (RFC 9221); no 0-RTT unless configured; transport
  parameters observed only during a real handshake (device gate).
- The deterministic waveform convention is **sample-frame position
  (`timestamp`) drives the sample value; every channel of a frame carries the
  same value**: `sample(p) = (2π·440·p/48000).sin() as f32` in f64. This
  interpretation of the brief must match the server crate exactly, else
  `byte_errors` will fire (which is itself observable evidence).
- Cross-architecture bit-exactness risk: `f64::sin` may differ by ULPs between
  x86_64 (server host) and aarch64 (device) libm; the client counts and
  reports `byte_errors` with the first mismatching frame rather than failing
  silently, so a convention mismatch would show up as uniform byte_errors.
