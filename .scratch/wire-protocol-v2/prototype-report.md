# Prototype report — quinn Android transport validation (ticket 01)

Feature: wire-protocol-v2 · Date: 2026-11-11 · Ticket: `issues/01-validate-quinn-android-transport.md`
Revision 3 — maintainer waived the two hardware-condition gates; verdict GO.

## 1. Verdict

**GO.** All hard gates that could be exercised are PASS with zero FAILs: the transport question is answered affirmatively on a real arm64 Android device — a Xiaomi M2007J22C (cannon), Android 12 / API 31, ran the full path — QUIC handshake with ALPN `audio-share/v2`, pinned-SPKI server identity, credential authentication over the encrypted control stream, subscription, and byte-exact DATAGRAM media consumption through the native ring → direct `ByteBuffer` → `AudioTrack` path — with **zero byte errors and zero control errors over a 160+ second continuous run** (65,486 datagrams, 62.9 MB), including **screen-off playback** and a **clean stop** reported on both ends. Every negative case works on device: wrong SPKI pin rejected at the cryptographic handshake, wrong credential rejected with `AUTH_ERR(BAD_CREDENTIAL)`.

The prototype initially ended BLOCKED on two hardware-availability items; the maintainer explicitly waived both on 2026-11-11 (see ticket Comments):

1. **API 26 physical-device runtime** — waived: no Android 8 device will be used for this validation. Build-level API 26 compliance stands (API-26 bionic linkage, minSdk 26, zero external Android dependencies, no dependency-imposed bump — `android-client/BUILD.md`).
2. **Wi-Fi ↔ cellular migration** — waived: no such scenario exists in the maintainer's environment. The rebind path was still executed on the API 31 device: `Network.bindSocket` returned EPERM (MIUI) and the designed fallback — teardown + bounded backoff reconnect, the spec's "ordinary reconnect remains required" — **passed**. Per the design baseline, migration is a prototype capability gate, not an initial production promise; ordinary reconnect is the shipped requirement and is proven.

This verdict authorizes the transport ADR and the normative protocol specification to proceed (spec §Decision Status, §Normative Specification Gate). It does NOT freeze any wire layout by itself — the throwaway BRIEF.md format remains non-normative.

## 2. Hard gate matrix

| # | Hard gate | Verdict | Environment | Evidence |
|---|---|---|---|---|
| 1 | Real arm64 Android QUIC handshake + arbitrary control stream + DATAGRAM media | **PASS (device)** — Android 12/API 31 arm64; PLAYING, 65k datagrams byte-exact | Xiaomi M2007J22C over tailscale (direct IPv6 path) | `evidence/device-validation.log` CASE 1, `evidence/device-logcat.log` |
| 2 | Android 8/API 26 build, installation, native load, connect, playback-buffer consumption | **PASS (device, API 31)**; API-26-device runtime **waived by maintainer** (build-level API 26 compliance proven: API-26 bionic link, minSdk 26, zero external deps) | device + build machine | `evidence/device-validation.log` CASE 1, `android-client/BUILD.md` |
| 3 | Correct + negative pinned-Server / Client-authorization cases | **PASS (desktop + device)** — wrong pin: `server SPKI does not match pinned identity` at handshake; wrong credential: `auth rejected (code BadCredential)`; unauthenticated SUBSCRIBE rejected (desktop) | device + Linux | `evidence/device-validation.log` CASE 2/3, `evidence/desktop-e2e.log` |
| 4 | ALPN major mismatch rejected; 0-RTT demonstrably disabled | **PASS (desktop)** / on-wire 0-RTT remains config-level evidence (quinn exposes no runtime 0-RTT-state API) | Linux | `evidence/desktop-e2e.log` |
| 5 | Wi-Fi ↔ cellular rebinding | **waived by maintainer** (no such scenario in their environment; no SIM) — rebind path still executed on device: `Network.bindSocket` EPERM (MIUI) → designed fallback teardown+backoff reconnect **PASS** | device | `evidence/device-validation.log` CASE 6 |
| 6 | IPv4/IPv6 + reduced-MTU + dynamic datagram sizing | **PASS (desktop)** — IPv4/IPv6 loopback + LAN; MTU 1280: mds 1162→1192 (DPLPMTUD), 1414 at MTU 1500; device ran over an IPv6 tailscale path (overlay, not raw internet v6) | Linux + veth + device | `veth-mtu1280-mtud-on.log`, `lan-ipv6-happy.log`, `device-validation.log` |
| 7 | Controlled impairment preserves reliable control + unreliable-media semantics | **PASS (desktop)** — loss 2%/reorder/dup/jitter/congestion all as specified | veth + tc netem | `veth-*.log` |
| 8 | Foreground-Service start, screen-off, network loss/recovery, reconnect, clean stop | **PASS (device)** — screen-off playback (active AudioTrack under `dumpsys media.audio_flinger`), wake keeps PLAYING, clean `LOCAL NORMAL` stop both ends; reconnect-under-network-change exercised via the rebind fallback | device | `evidence/device-validation.log` CASE 4/5/6 |
| 9 | Native direct-buffer/ring media path without per-datagram JNI callbacks | **PASS (device)** — one `attachBuffer`, ring + write_pos cursor consumed 62.9 MB; 10 low-frequency JNI functions only | device | `evidence/device-validation.log`, `android-client/BUILD.md` |
| 10 | Stripped arm64 `.so` increment ≈ 5 MiB or smaller | **PASS** — 2,305,128 B (2.20 MiB); APK increment 2,305,482 B; minSdk 26 held | build machine | `android-client/BUILD.md` |
| 11 | Build and runtime results reproducible from recorded commands/versions | **PASS** | build machine + device | README/BUILD.md/BRIEF.md committed; evidence headers carry exact commands |

**Tally: 9 PASS · 0 FAIL · 2 waived by maintainer decision ⇒ GO.**

## 3. Prototype branch and how to run the smallest example

- Branch: `prototype/wire-protocol-v2-transport`, head **`21e1d71`**, pushed to the fork (`W0n9/audio-share`). Merge chain: `ticket/01-transport-server` @ `5484d3b`, `ticket/01-transport-android` @ `8990ef0`, `ticket/01-transport-evidence` @ `34ad884`, `ticket/01-review-fixes` @ `0ecce19`, `ticket/01-device-validation` @ `ce3f457`.
- All code under `prototype/wire-protocol-v2-transport/` (server/, android-client/, scripts/, evidence/, README.md, BRIEF.md). Production dirs untouched.

Smallest desktop example:

```bash
git checkout prototype/wire-protocol-v2-transport
cd prototype/wire-protocol-v2-transport/server
cargo build --release
./target/release/proto-server --bind 127.0.0.1:65531 --spki-out /tmp/spki.hex   # prints PROTO_SERVER_READY ... spki=<64 hex>
./target/release/proto-client --addr 127.0.0.1:65531 --pin "$(cat /tmp/spki.hex)"
```

Scripted 8-case security/lifecycle smoke run: `scripts/run-e2e.sh`. Android build + install: `android-client/BUILD.md`; on-device automation: `am start -S -n dev.prototype.wireproto/.MainActivity -e host H -e port P -e pin PIN --ez noBindHint true --ez autoConnect true`.

## 4. Versions, flags, sizes, resources, impairment commands

| Component | Version |
|---|---|
| Linux kernel (build machine) | 6.8.0-139-generic |
| rustc/cargo | 1.98.1 |
| quinn / quinn-proto / quinn-udp | 0.11.12 / 0.11.19 / 0.5.16 |
| rustls / ring / rcgen | 0.23.46 / 0.17.14 / 0.13.2 |
| tokio / jni | 1.54.1 / 0.21.1 |
| NDK / AGP / Kotlin / Gradle | 27.2.12479018 / 8.7.3 / 2.0.21 / 8.9 |
| compileSdk / targetSdk / minSdk | 35 / 35 / **26** |
| Rust Android target | aarch64-linux-android (direct NDK API-26 clang linker; cargo-ndk NOT used) |
| Validation device | Xiaomi M2007J22C (cannon), Android 12 / API 31, arm64-v8a, MIUI, no SIM, KernelSU root |

- ABI sizes: raw `.so` 8,778,032 B → stripped **2,305,128 B (2.20 MiB)**; release APK with native 2,339,239 B; APK native increment **2,305,482 B** vs 33,757 B empty baseline; debug APK 3,141,687 B. Single-ABI `abiFilters=["arm64-v8a"]` (BUILD.md records why the arm64 increment is unaffected).
- Resources (desktop, 400 datagrams/s loopback): server CPU ~4.7%, client ~6.2%; RSS VmHWM server 8.4 MiB / client 7.5 MiB; zero UDP rcvbuf/sndbuf errors. 25 s clean idle captured (60 s idle killed by quinn's default 30 s `max_idle_timeout` — knob not exposed in prototype CLIs).
- Device stream characteristics (tailscale overlay): ~458 datagrams/s observed, gaps ≈ 7% (overlay reordering/loss), reorder tracked, duplicates 0, byte errors 0, control errors 0; audio seconds delivered slightly exceeded wall-clock (~+14%) — counter/pacing exactness on device not investigated further (see §7).
- Impairment: `scripts/impairment.sh` (idempotent `tc netem`/veth helper; every evidence log header records the exact commands). Profiles: loss 2% → gaps=40/2002, control clean; reorder 25%/50% → 238 reorders, 0 loss; duplicate 5% → +2.2% on wire, **0 app-visible** (QUIC packet-number dedup); jitter 10 ms±5 ms → throttling anomaly documented; congestion 300 kbit/limit 50 → bounded backlog 40 pkts, full-rate recovery; MTU 1280 → mds 1162→1192 with DPLPMTUD, largest datagram 1188, no fragmentation.

## 5. Library limitations, workarounds, findings, inference vs proof

Real-device findings (first JNI/runtime execution of this code):

- **JNI signature bug** (found by device testing, fixed): the exported functions took `env: &mut JNIEnv`; jni 0.21 documents `env: JNIEnv` **by value**. With the reference form every `createSession` call failed with `Null pointer deref in *JNIEnv`. The failure was invisible until the swallowed error string was logged — `guard(...).unwrap_or(0)` had discarded it; the error is now `log::error!`-ed.
- **NetworkCallback bind hint vs VPN overlays** (design finding): pinning the UDP socket to the active network's local address (the migration scaffold's initial-bind hint) breaks routing when the destination is reached through a VPN (tailscale tun): the socket is pinned to wlan0 while the route lives on tun. The app gained a `noBindHint` switch; the migration design must reconcile "bind to the selected Network" with overlay/VPN routes (route lookup, not address naming, decides the egress).
- **`Network.bindSocket` → EPERM** on this MIUI build (device network 217): the rebind path executed and the designed fallback (teardown + bounded backoff with ±20% jitter → reconnect) succeeded end-to-end. Whether EPERM is MIUI-specific or affects stock Android needs one more device.
- libm ULP concern retired: the deterministic waveform (`sin(2π·440·p/48000) as f32` computed in f64) verified **byte-exact across x86_64 and aarch64** — no uniform byte_errors.

quinn/rustls library facts (desktop evidence):

- **quinn `send_datagram` never reports a full buffer** (proto-level `Blocked` unreachable; internal ~1 MiB queue evicts oldest silently). Drop accounting therefore lives in the app-level bounded queue (cap 256) — the honest layer to count at.
- **DPLPMTUD is ON by default**; prototype disables it by default (`--mtud` re-enables). The normative v2 spec must decide MTUD policy explicitly; observed `max_datagram_size()` = 1162 (1200-MTU default).
- **Default `max_idle_timeout` is 30 s** with no keep-alive: idle sessions die at ~30 s (this is also exactly how the mis-routed phone sessions died before the bind-hint fix). Direct design input for Android foreground-service idle behavior.
- **Jittered (variable) delay throttled delivery** to ~333/400 datagrams/s while constant delays ran full rate — suspected congestion-controller/RTT-variance interaction; documented via constant-delay sweep, root cause **not proven**.
- **Packet-level duplication is invisible above QUIC** (packet-number dedup, proven with tcpdump); app-level duplicate handling stays covered by verifier unit tests.
- tokio bounded mpsc empirically holds capacity+1; std IO-safety abort on a `mem::forget`+`from_raw_fd` test roundtrip (rebind tests use `into_raw_fd` ownership transfer).
- IPv6 was initially blocked by the prototype client's own hardcoded `0.0.0.0:0` bind, not by quinn/rustls; fixed, both logs retained.
- **Inference vs proof:** Linux-desktop behavior — proven by committed logs. Real-device behavior — now proven for the primary path (handshake, auth, pinning, control stream, DATAGRAM media, screen-off lifecycle, clean stop) by `evidence/device-validation.log` + `device-logcat.log`. Remaining inference: API-26-device runtime, migration across physical network changes, and anything listed in §7.

## 6. Repair outlook

Not applicable — no FAIL verdicts, and the two hardware-condition items were waived by maintainer decision. The only code-level follow-up a future device could revisit is whether `Network.bindSocket` EPERM is MIUI-specific (relevant only if a migration scenario ever appears; the fallback already covers it).

## 7. Protocol details the prototype did NOT validate

- Runtime on a specifically Android 8/API 26 device (validated device is API 31; API 26 build-level compliance proven; **device test waived by maintainer**).
- Wi-Fi ↔ cellular migration across physical network changes (**scenario waived by maintainer — none exists in their environment**); QUIC path validation across a genuine network swap; whether `Network.bindSocket` EPERM is MIUI-specific.
- The v2 **normative** wire layout, port, error-code registry, Protobuf schema — the throwaway BRIEF.md format is explicitly not normative; no ADR written.
- Production pairing: QR/SAS, Keystore-backed keys, revocation UX, multi-client authorization.
- Opus/FEC/DTX/PLC, drift correction, jitter-buffer tuning, send-ahead window tuning.
- Multi-client isolation, 60-minute scenario, Server soak.
- On-wire 0-RTT behavior beyond configuration audit; on-wire ALPN rejection capture on device (desktop e2e covers it).
- Exact device-side media pacing/counter accounting (audio-seconds vs wall-clock +14% observed, not investigated).

## Outcome

The transport question is answered: **GO** for quinn + rustls/TLS 1.3 + one reliable control stream + QUIC DATAGRAM media, within the accepted native-integration (2.20 MiB `.so`, low-frequency JNI) and API 26 build boundaries. Per spec §Decision Status, this authorizes the **transport ADR, rejected-alternative rationale, and the normative protocol specification** (state-machine tables, schema, media layout, error registry, capability rules, threat model, cross-language golden vectors) as the next phase — none of which exist yet, and none of which are authorized to be inferred from library defaults.
