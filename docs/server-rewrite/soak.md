# Soak test — new Server (ticket 15)

Status: the default **8-hour release soak was executed on 2026-11-09/10 and
PASSED** (2 clients, no crash, 0 heartbeat misses, no gap above 100 ms). The
earlier **12-minute release soak** is kept below for comparison.

## What `scripts/soak.sh` runs

- Release `as-cmd` with the real Linux PipeWire capture backend, inside a
  disposable headless PipeWire + WirePlumber session with a null `Audio/Sink`
  and a looped 440 Hz tone (the same setup as the acceptance matrix).
- N scripted clients (`protocol-harness probe`) that perform the v0.3.4 client
  flow, reply to heartbeats, and optionally reconnect inside the same process
  (default: every 30 minutes).
- Every sample interval it records server RSS (KB), server CPU (% averaged by
  `ps` over the process lifetime), summed client RSS, and whether the server
  and each client are alive.
- At the end it aggregates per-client PCM bytes, datagrams, sessions,
  reconnects, heartbeats/misses, datagram-arrival gaps (50/100/250 ms buckets
  as a glitch proxy), the max gap, tone presence, and throughput against
  `sample_rate * block_align * elapsed`.
- Logs are rotated: server stdout/stderr is piped through a rotating writer;
  `monitor.csv` is reopened on every append so rotation is safe; `server.log`
  is renamed when it exceeds the threshold (as-cmd keeps its descriptor, so
  later lines continue in the rotated file until the process restarts — this
  is documented rather than hidden).
- The script exits nonzero on a server crash, a non-zero server exit, any
  client probe failure, or any heartbeat miss.

Usage:

```bash
# default: 8 hours, 2 clients, reconnect every 30 minutes, sample every 5 s
scripts/soak.sh --out /tmp/soak-8h

# shortened run
scripts/soak.sh --duration 12m --clients 2 --reconnect-every 240 \
  --sample-seconds 5 --out /tmp/soak-short
```

Requires `cargo`, PipeWire/WirePlumber, `pw-cli`/`pw-play`/`pw-dump`,
`dbus-run-session`, `python3`, and `jq`; no sudo and no user services.

## Executed shortened run (2026-11-09)

Environment: Ubuntu 24.04.5 x86_64, release binaries built from this branch,
Rust 1.98.1, libpipewire 1.0.5, WirePlumber 0.4.17, headless null sink with a
440 Hz tone.

Exact command:

```bash
cargo build --release -p as-cmd -p protocol-harness
scripts/soak.sh --skip-build --duration 12m --clients 2 \
  --reconnect-every 240 --sample-seconds 5 --out /tmp/t15-soak-release2
```

Results (`/tmp/t15-soak-release2/soak-summary.json` and `monitor.csv`; the
same fixed-monitor script also produced `/tmp/t15-soak-release` beforehand,
explained in the harness note below):

| Metric | Result |
|---|---|
| Requested / actual duration | 720 s / 722 s |
| Clients | 2/2 probes ok |
| Server exit | 0 (graceful SIGINT at the end), no crash |
| Server RSS min / mean / max | 11524 / 15558.8 / 15612 KiB |
| Server CPU mean / max | 3.304 % / 28.5 % |
| Samples | 141 |
| PCM bytes (both clients) | 548,339,712 |
| Datagrams (both clients) | 401,616 |
| Sessions / reconnects | 6 / 4 |
| Heartbeats / misses | 470 / 0 |
| Datagram gaps >50 / >100 / >250 ms | 53 / 0 / 0 |
| Max datagram gap | 65 ms |
| Throughput ratio (min client) | 0.9914 |
| Tone present for all clients | yes |
| Verdict | PASS |

Interpretation:

- The minimum RSS is the startup allocation (11,524 KiB); after startup the
  server stayed at 15,558.8 KiB mean and 15,612 KiB max, a ~53 KiB band with
  no growth trend. CPU stayed in the low single digits (mean 3.304 %, with a
  28.5 % startup sample) on an idle 36-core host. These are observations for
  this host; the 8-hour run below confirms the same band holds over the full
  duration.
- Client reconnects were intentional and counted; the 3 s reconnect delay
  lowers the aggregate throughput ratio slightly (0.9914), which is expected
  client behavior, not packet loss.
- A "glitch" here is a gap between UDP datagrams above the named threshold.
  The run recorded 53 gaps in the 50-100 ms bucket, none above 100 ms, and a
  65 ms maximum. These must be compared against a baseline before calling
  them a regression; they are reported, not hidden.
- Harness note: an earlier run with the same workload
  (`/tmp/t15-soak-release`) reported `client_process_failure: true` together
  with `pass: true`, `clients_ok: 2` and `client_errors: []`. That flag was a
  monitor race: the final sample ran after both clients had finished at the
  duration and before the elapsed check, so "alive < clients" was misread as
  an early failure. The pass verdict never depended on the flag. `soak.sh` now
  checks the duration before client liveness; the rerun above records
  `client_process_failure: false`. Do not treat the earlier run as a client
  crash: both probes exited 0 with `ok: true`.
- Log rotation was exercised separately with
  `scripts/soak.sh --skip-build --duration 10s --clients 1 --reconnect-every 0
  --sample-seconds 1 --rotate-mb 0 --out /tmp/t15-soak-rotate`
  (18 rotated files, summary PASS); the 12-minute run's logs are only a few
  KiB and stayed below the default 16 MiB threshold, so rotation did not
  trigger there.

## Executed 8-hour run (2026-11-09 → 2026-11-10)

Environment: the same host and setup as the shortened run (Ubuntu 24.04.5
x86_64, release binaries built from `integration/server-rewrite` at `c6e0277`;
the later commits on this branch touch only tests, CI, and docs), Rust 1.98.1,
libpipewire 1.0.5, WirePlumber 0.4.17, headless null sink with a 440 Hz tone.

Exact command:

```bash
scripts/soak.sh --duration 8h --clients 2 --reconnect-every 1800 \
  --sample-seconds 5 --out /home/w0n9/workspaces/audio-share-artifacts/soak-8h
```

Results (`soak-summary.json`, `monitor.csv`, and both `client-*.json`):

| Metric | Result |
|---|---|
| Requested / actual duration | 28,800 s / 28,803 s (8 h 0 m 3 s) |
| Clients | 2/2 probes ok |
| Server exit | 0 (graceful SIGINT at the end), no crash |
| Server RSS min / mean / max | 11,500 / 15,601.5 / 15,620 KiB |
| Server CPU mean / max | 3.651 % / 28.5 % |
| Samples | 5,578 |
| PCM bytes (both clients) | 22,083,813,376 (~22.08 GB) |
| Datagrams (both clients) | 16,174,668 |
| Sessions / reconnects | 32 / 30 |
| Heartbeats / misses | 19,140 / 0 |
| Datagram gaps >50 / >100 / >250 ms | 1,336 / 0 / 0 |
| Max datagram gap | 66 ms |
| Throughput ratio (min client) | 0.9984 |
| Tone present for all clients | yes (440 Hz detected, amplitude ≈ 0.249) |
| Alignment / max datagram | `alignment_ok: true`, `max_datagram_ok: true` for both clients |
| Verdict | PASS |

Interpretation:

- RSS: startup minimum 11,500 KiB; the run plateaued at 15,601.5 KiB mean and
  15,620 KiB max over 5,578 samples, the same ~15.6 MB band as the shortened
  run. No growth trend over 8 hours.
- CPU: mean 3.651 %, max 28.5 % (the startup sample); comparable to the
  12-minute run.
- Heartbeats: 19,140 over 32 sessions, 0 misses.
- Glitches: 1,336 datagram-arrival gaps in the 50–100 ms bucket and none above
  100 ms; the maximum was 66 ms (short run: 65 ms). This is the same order as
  the shortened run and is reported as an observation, not a regression claim
  against an unrecorded baseline.
- Throughput: the minimum client reached 0.9984 of the expected
  `sample_rate * block_align * elapsed`; the 30 intentional reconnects account
  for the remainder.
- Both clients detected the 440 Hz tone (amplitude 0.2487), reported
  `alignment_ok: true`, `max_datagram_ok: true`, empty `errors`, and
  16 sessions / 15 reconnects each.
- Evidence: `/home/w0n9/workspaces/audio-share-artifacts/soak-8h/`
  (`soak-summary.json`, `monitor.csv`, `client-0.json`, `client-1.json`,
  `server.log`, `server-stream.log`, `tone-48000.wav`).

Acceptance criteria met: `pass: true`, `server_crashed: false`,
`server_exit: 0`, both clients ok, `heartbeat_misses: 0`, tone present, and no
datagram gap above 100 ms.

## Limits

- Null sink and synthetic tone only; no real hardware, no phone, no
  suspend/resume on the Linux soak host. Real-device playback was later
  executed on one Android 12 phone and is recorded in
  `android-v034-acceptance.md`: the single-phone matrix and a **passing
  60-minute playback** (12/12 probes, 440 Hz, 0 heartbeat misses, one
  auto-reconnect, no app crash) were run. That run also exposed (and led to
  the fix of) the Windows UDP broadcast stall on a slow Wi-Fi peer. A
  two-Android-client run, suspend/resume, and real-hardware hotplug remain
  open.
- The probe captures at most 8 MiB of PCM for tone analysis; counters cover
  the whole run, but tone presence is checked over the first ~21 s of the
  first session (enough to prove the capture path after startup).
- `ps` reports CPU averaged over process lifetime; short spikes between
  samples can be missed.
