# Ticket 09 evidence index

Environment: base commit `9c706dd`, branch `ticket/09-phase3-cross-validation`; device Xiaomi M2007J22C (Android 12 / API 31, arm64), serial `100.117.189.121:5555`; host `100.94.170.124`.

## Commands

```sh
# server (this worktree)
AUDIO_SHARE_CONFIG=/tmp/ticket09/data/config.ini \
  ./target/release/as-cmd -b 0.0.0.0:65530 --v2-port 65531 --fake-capture -V \
  > /tmp/ticket09/server.log 2>&1 &

# client build (contains the ticket-09 single-worker bypass)
cd android-client-v2 && ANDROID_HOME=$HOME/Android/Sdk ./gradlew assembleRelease

# client launch
adb -s 100.117.189.121:5555 shell am start -n dev.audioshare.v2/.MainActivity \
  -e host 100.94.170.124 --ei port 65531 \
  -e pinnedSpki ee5bae520923ab3971b6a68dc0fd774fa9b27dc3f1eac5fb576985e5e9f79c94 \
  -e serverKey 04a9754252f678feb693284407a03ce37edad729114cf8b4267e144aa858810a \
  -e clientName "Ticket 09 Phone" --ez bindHint false

# host live-QUIC negatives
cargo test -p v2-transport --test session_driver -- --nocapture

# v1 regression
./target/release/protocol-harness replay --fixture fixtures/protocol/v0.3.4/linux/f32-48k-stereo
cargo test --workspace
```

## Map

| Directory | Content |
|---|---|
| `01-e2e-positive/` | READY screenshots, server/client logs; `server-after-stop.log` records the 701 close after Activity recreation |
| `02-idle/` | READY after >80 s idle |
| `03-lock/` | screen-off and READY-after-unlock screenshots |
| `04-network/` | Wi-Fi cycle log, client/server logs, READY before/after outage |
| `05-negatives/` | wrong pin, unknown identity, live revocation before/after, revoked new connection, TM-5 device rate limit, host live-QUIC suite |
| `06-resources/` | `dumpsys cpuinfo`, `/proc/<pid>/status`, server RSS |
| `07-v1-regression/` | six replay logs, live v1 replay while v2 READY, workspace test log |
| `08-replay/` | Rust and Android native replay test logs |

Screenshots are raw `adb exec-out screencap -p` PNGs (1080x2340). Server timestamps in logs are UTC; device logcat timestamps are local (UTC+8).

Note: the APK used here carries the temporary `.worker_threads(2)` → `.worker_threads(1)` bypass in `android-client-v2/native/src/session.rs:89-95`; see the phase-3 report.
