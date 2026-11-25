# 01: Validate quinn Android transport

**What to build:** 在独立 throwaway 分支与目录中实现最小 Rust Server ↔ Android Client 原型，验证 quinn 的任意二进制控制流、QUIC DATAGRAM、固定身份认证、动态 MTU、Android 前台 Service、native 媒体热路径和网络迁移；输出逐项 PASS/FAIL/NOT RUN 的 GO/NO-GO 报告。

**Blocked by:** none

**Status:** resolved

**Type:** prototype

## Question to Answer

Can quinn + rustls/TLS 1.3 support this project's v2 transport on a real arm64 Android Client at API 26, without exceeding the accepted native-integration and binary-size boundaries?

The prototype answers a transport question only. It does not freeze the v2 wire layout, implement production pairing, or authorize merging the candidate into Server/Android production code.

## Required Reading

- `.scratch/wire-protocol-v2/spec.md` — accepted design baseline and source links.
- `docs/protocol.md` — frozen v1 behavior that this prototype must not modify.
- `docs/adr/0001-server-rewrite-baseline.md` — v0.4.0 compatibility boundary.
- Android foreground-Service and JNI guidance linked from the design baseline.

## Isolation and Preservation

- Work on a `prototype/wire-protocol-v2-transport` branch out of `main`, in a separate `prototype/wire-protocol-v2-transport/` directory. Keep the prototype as a primary source and record its branch/commit in the report.
- Do not modify production `crates/`, `android-app/`, `gui/`, `protos/`, v1 fixtures, release workflows, or `docs/protocol.md` for the prototype.
- Use small, throwaway implementations and deterministic test patterns. Do not spend the prototype budget on production UI, full QR/SAS pairing, jitter-buffer tuning, Opus, drift correction, or a complete Protobuf schema.
- Record exact commits and versions for Rust, quinn/rustls/ring, NDK, Android Gradle Plugin, dependencies, device, Android build, and kernel. Library defaults observed in the prototype are evidence, not silently adopted protocol decisions.

## Build the Smallest End-to-End Path

### 1. Rust test Server

Implement a command-line Server that:

- Listens on a configurable UDP address/port, separate from v1 `65530`.
- Uses quinn with rustls/TLS 1.3 and ALPN `audio-share/v2`.
- Exposes one reliable bidirectional stream carrying length-delimited, application-defined binary request/response messages.
- Accepts only a configured test authorization credential over the encrypted control stream.
- Sends deterministic sequence/timestamp-bearing test payloads over QUIC DATAGRAM after an explicit subscribe request.
- Reports received control statistics and clean connection-close reasons.
- Uses non-waiting datagram sends and bounded queues; log drops rather than waiting behind stale media.

Completion: a scripted desktop client can prove successful and rejected ALPN, Server identity, Client authorization, subscribe, datagram receipt, and normal/abnormal close cases.

### 2. Android test Client

Implement a minimal foreground media-playback Service and screen that:

- Builds for and runs on Android 8/API 26 and a real arm64 device.
- Loads a native Rust `cdylib`; Kotlin holds an opaque native session handle.
- Verifies the test Server using a pinned SPKI/public key, then presents the test authorization credential over the encrypted control stream.
- Uses low-frequency JNI for connect, subscribe, stop, state, and statistics.
- Receives media into a direct `ByteBuffer` or native ring buffer. The consumer reads buffered data without one JNI/UniFFI/JNA callback per datagram.
- Surfaces sequence gaps, duplicates/reorder where observable, bytes, control errors, and connection state.
- Handles stop, network loss/recovery, screen off, and Service/process lifecycle without claiming playback after termination.

Completion: the real Android Client can connect, reject a wrong Server pin and wrong Client credential, subscribe, consume deterministic media, display counters, and stop cleanly.

### 3. Security and Version Assertions

Automate or provide reproducible steps for:

- Correct pinned Server identity succeeds; an untrusted/self-signed substitute fails.
- Valid Client authorization succeeds; missing/invalid/revoked test authorization fails.
- `audio-share/v2` succeeds; an incompatible ALPN major fails before application media starts.
- 0-RTT is disabled in both configurations. No pairing, authorization, subscribe, or other control operation is accepted as early data.
- Error/log output contains no private key, complete authorization credential, raw test media, or unnecessary internal detail.

Completion: every assertion has a captured result and log/test reference; encrypted transport alone is not counted as Client authorization.

### 4. DATAGRAM, MTU, and Impairment Matrix

Exercise IPv4 and IPv6 where the test network supports them, plus controlled loss, reorder, duplication, jitter, congestion, and reduced MTU. Record the exact impairment profile and tool commands.

Verify and report:

- Reliable ordered control behavior while media datagrams are lost/reordered.
- Application sequence/timestamp observations distinguish media loss and reorder from control failure.
- Current `max_datagram_size()` is queried and respected; the test payload is resized rather than relying on the v1 fixed `1464` calculation.
- A reduced path limit affects subsequent payload sizing without requiring IP fragmentation or sending an oversized datagram.
- Non-waiting send queues stay bounded and stale datagrams do not stall later media indefinitely.
- CPU, memory, packet counters, and observed media timing are captured for a defined idle and streaming interval.

Completion: every matrix cell is PASS, FAIL, or NOT RUN with evidence; an untested IPv6 or impairment environment is never described as passing.

### 5. Android Network Migration

Implement the candidate path using `ConnectivityManager.NetworkCallback`, a socket bound to the selected Android `Network`, and quinn endpoint rebinding.

On a real device, test Wi-Fi to cellular and cellular to Wi-Fi where available:

- Continue the same QUIC connection and media sequence through rebinding without application reconnect.
- Record address/path changes, interruption interval, lost/late media, and any socket/Android API failure.
- Separately test ordinary reconnect when migration is unavailable.

Completion: both migration directions have evidence, or are explicitly NOT RUN with the missing hardware/network condition. Migration success is a prototype gate but does not become a universal production guarantee.

### 6. Release Build and Integration Cost

Produce a stripped release Android build using ABI splits or an App Bundle and report:

- Raw arm64 native `.so` size and its contribution to the release artifact, with the baseline empty-prototype size for comparison.
- Whether the stripped arm64 `.so` increment is within the provisional **approximately 5 MiB** gate.
- Build steps, required NDK/Gradle changes, ProGuard/R8 rules, JNI symbols, and any platform-verifier dependency.
- Native/Kotlin CPU behavior sufficient to show there is no per-packet JNI callback hot path.
- API 26 installation/load result and any dependency-imposed higher API requirement.

Completion: size and build-cost numbers are reproducible. Exceeding 5 MiB or API 26 is a reported gate failure requiring renewed design review, not a silent threshold change.

## Hard GO Gates

A GO recommendation requires **PASS** on every item below:

- [ ] Real arm64 Android QUIC handshake, arbitrary control stream, and DATAGRAM media.
- [ ] Android 8/API 26 build, installation, native load, connect, and playback-buffer consumption.
- [ ] Correct and negative pinned-Server and Client-authorization cases.
- [ ] ALPN major mismatch rejected and 0-RTT demonstrably disabled.
- [ ] Wi-Fi ↔ cellular rebinding continues the same connection in both available test directions.
- [ ] IPv4/IPv6 and reduced-MTU behavior verified, with dynamic datagram sizing.
- [ ] Controlled impairment preserves reliable control and expected unreliable-media semantics.
- [ ] Foreground-Service start, screen-off, network loss/recovery, reconnect, and clean stop behave correctly.
- [ ] Native direct-buffer/ring-buffer media path works without per-datagram JNI callbacks.
- [ ] Stripped arm64 `.so` increment is approximately 5 MiB or smaller.
- [ ] Build and runtime results are reproducible from recorded commands and dependency versions.

If any item is FAIL or NOT RUN, report **NO-GO** or **BLOCKED**, respectively. Do not write a validated transport ADR from a partial result.

## Report and Handoff

Bring a report back to the original feature as `.scratch/wire-protocol-v2/prototype-report.md`. Include:

1. One-paragraph GO/NO-GO/BLOCKED verdict and the evidence that determines it.
2. A row for every hard gate with PASS/FAIL/NOT RUN, device/environment, and evidence link.
3. Exact prototype branch/commit and instructions to run the smallest example.
4. Versions, build flags, ABI sizes, resource measurements, impairment commands, and logs.
5. Library limitations discovered, workarounds attempted, and which conclusions remain inference rather than proof.
6. If NO-GO: whether quinn configuration can repair the failure, or which fallback from the design baseline needs a new decision.
7. A list of protocol details the prototype did **not** validate.

Update this issue's `Status` to `ready-for-human` if implementation is complete but physical-device/network gates remain. Append dated evidence under `## Comments`. Resolve the investigation only when its report answers the transport question; resolution alone never implies GO.

## Comments

### 2026-11-11 — prototype implemented, verdict BLOCKED (device gates outstanding)

Implementation complete on branch `prototype/wire-protocol-v2-transport` @ `e9907e0` (merge of `ticket/01-transport-server`, `ticket/01-transport-android`, `ticket/01-transport-evidence`, `ticket/01-review-fixes`; code review done, all findings fixed). Full report: `.scratch/wire-protocol-v2/prototype-report.md`.

- **Hard-gate tally: 6 PASS · 0 FAIL · 5 NOT RUN ⇒ BLOCKED.** All desktop-runnable gates pass: 8/8 scripted e2e (wrong ALPN/pin/credential/revoked/unauthenticated all rejected), IPv4+IPv6 loopback and LAN, `tc netem` impairment matrix, MTU 1280 DPLPMTUD adaptation (max_datagram_size 1162→1192, 1414 at MTU 1500), bounded queues under congestion with full-rate recovery. Size gate PASS: stripped arm64 `.so` 2,305,128 B = 2.20 MiB (< ~5 MiB), APK increment 2,305,482 B, minSdk 26 held.
- **NOT RUN (why this is ready-for-human):** no Android device was attached (`adb devices` empty) — arm64 handshake on device, API 26 install/native load/AudioTrack consumption, foreground-Service lifecycle/screen-off, Wi-Fi↔cellular migration (rebind code implemented + host-tested, never executed), on-device media-path runtime.
- Tests at close: server workspace 47 (24 proto-core + 22 e2e + config assertion), Android native host 58; `scripts/run-e2e.sh` 8/8 after the review-fix refactor; quinn 0.11.12 / rustls 0.23.46 / NDK 27.2.12479018 / AGP 8.7.3 / Kotlin 2.0.21.
- Notable quinn findings recorded in the report: `send_datagram` full-buffer state unreachable (drop accounting must live app-side), DPLPMTUD on by default, 30 s default idle timeout, variable-delay throttling anomaly, packet-level duplication absorbed by QUIC dedup.
- Next: attach a real arm64 Android 8+ device (+ second network), run the device rows per `prototype/wire-protocol-v2-transport/android-client/BUILD.md`, record results, then flip to GO/NO-GO. Resolution of this investigation does not imply GO.

### 2026-11-11 — device validation executed; BLOCKED narrowed to two hardware conditions

Real-device run on Xiaomi M2007J22C (cannon), Android 12 / API 31, arm64-v8a, over tailscale (direct IPv6 path), server on the build host. Branch updated to `21e1d71` (pushed to fork; `ticket/01-device-validation` @ `ce3f457`). Evidence: `prototype/wire-protocol-v2-transport/evidence/device-validation.log` + `device-logcat.log`; report updated to revision 2.

- **Now PASS on device:** arm64 QUIC handshake + control stream + DATAGRAM media (PLAYING; 65,486 datagrams / 62.9 MB / 163.7 s audio, byte_errors=0, control_errors=0); install + native load + connect + playback consumption (API 31 device); wrong-pin rejected at cryptographic handshake; wrong credential → `AUTH_ERR(BAD_CREDENTIAL)`; screen-off playback (active AudioTrack under `dumpsys media.audio_flinger`); clean stop both ends (`LOCAL NORMAL` / server `media stopped`); rebind fallback (EPERM on `Network.bindSocket` under MIUI → teardown + backoff reconnect succeeded); no-per-datagram-JNI media path.
- **Still NOT RUN (hardware):** API 26 device specifically (validated device is API 31; APK links API-26 bionic, minSdk 26, zero external deps); Wi-Fi↔cellular migration (no SIM in phone). Tally now **9 PASS · 0 FAIL · 2 NOT RUN ⇒ BLOCKED (narrowed)**; no FAIL anywhere.
- **Real bugs/design findings recorded:** (1) JNI exports used `env: &mut JNIEnv` — jni 0.21 requires by-value; every createSession failed until fixed, and the swallowed error string was the only diagnostic (now logged); (2) the NetworkCallback local-address bind hint breaks routing through VPN overlays (socket pinned to wlan0, route on tun) — `noBindHint` switch added, design note for migration; (3) deterministic waveform verified byte-exact across x86_64↔aarch64 (libm ULP concern retired); (4) gradle `buildNative` had no source inputs and skipped cargo rebuilds — fixed.
- Issue stays `ready-for-human`: remaining gates need an Android 8 device and/or a SIM. Resolution of the investigation still does not imply GO.

### 2026-11-11 — maintainer waived the two hardware gates; verdict GO, issue resolved

Maintainer decision (same day): the API 26 physical-device runtime test is **not required**, and there is **no Wi-Fi↔cellular migration scenario** in their environment. With both waivers recorded, the hard-gate tally is **9 PASS · 0 FAIL · 2 waived ⇒ GO**.

- The transport question is answered affirmatively: quinn + rustls/TLS 1.3 + one reliable control stream + QUIC DATAGRAM media works on a real arm64 Android device within the accepted boundaries (2.20 MiB stripped `.so`, low-frequency JNI, API 26 build compliance).
- Ordinary reconnect (the spec's shipped requirement when migration is unavailable) is proven on device: rebind EPERM → teardown + bounded backoff reconnect succeeded.
- The investigation is resolved per tracker convention. This GO authorizes the next phase per spec §Decision Status — transport ADR, normative wire specification, schema, vectors, threat model — as separate future tickets with their own blocking edges; it does not by itself freeze any wire layout.
- Report: `.scratch/wire-protocol-v2/prototype-report.md` (revision 3).
