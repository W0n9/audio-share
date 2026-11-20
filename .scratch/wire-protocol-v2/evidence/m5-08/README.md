# M5-08 — Read-only root-cause diagnosis: pump drops, client trims/re-anchor, underrun counters

- Ticket: `.scratch/wire-protocol-v2/issues/m5-08-pump-drop-trim-underrun-investigation.md`
- Baseline: `integration/m5` @ `800c40c`
- Evidence: `.scratch/wire-protocol-v2/evidence/m5-windows-evidence-2026-11-20/`
  (`server-C-extended.log`, `client-A/B/C-extended.log`, `client-all.log`, `aflinger-A/B/C.txt`)
- Method: code reading + arithmetic on the recorded counters. No repo file modified, no test run
  against the repo. One standalone Python simulation of `MediaPacer` was run under `/tmp`
  (`/tmp/m508/sim.py`), importing nothing.

---

## 0. Executive summary

| # | Signal | Root cause | Confidence |
|---|---|---|---|
| A | Server `pending_frames` at the 500 ms pump bound; `pump_dropped_chunks` +1 / ~5 s; `queue_dropped_newest=0` | **`MediaPacer` systematically under-refills its token bucket**: every `try_consume` call truncates the fractional frame accrual and then moves `last_refill` forward, so the discarded fraction is never carried. With ~200 calls/s the pacer delivers ~0.2 % less than 48 000 frames/s, exactly the observed drop rate. Secondary: the pacer has no catch-up, so any transient backlog is permanent. Separately, the §7.4 text formula is **not implemented at all**; the literal `capture_position − last_sent_position ≤ 100 ms` *is* violated (~480 ms) in this state. | **High** (~0.8) for under-refill; High that §7.4 formula is violated; Medium for a capture-device clock contribution |
| B | `underrun=5576` fills in case A; 5 576 × 10 ms would be 55.8 s of silence | **Gating on MIUI's `AudioTrack.underrunCount` is invalid.** AudioFlinger at the same time shows the *mixer* raw underrun counters `partial=0 empty=0` and the track buffer `FrmRdy=2015 / FrmCnt=2052` (≈98 % full), while the per-track `Underruns` column reads **3 508 242**. The gate converts each bogus counter advance into a 10 ms silence write. Partly coupled to ring starvation/trim (see C), but the event *count* is not a stall count. | **High** (~0.85) that 5 576 is an artifact; Medium that some fills correspond to real short stalls; per-fill accepted frames unknown |
| C | Soft trims 63 k frames / 28 events (5 min), 619 k / 13 min; re-anchor 43 k / 155 k; `drift_ppm` 36 then a single 499, later ±300–660 swings | Not a sustained 0.4 % rate mismatch. Trims are **burst-driven** (jitter-buffer hole release + late/partial playout + 20 ms hysteresis against a 60 ms target), and the ppm estimate is **window-starved**: it is reset by every soft trim, re-anchor, server-side timestamp re-base, and *every* platform-underrun increment (10×/s during the storm). The 499 is a short/quantised-window artifact, not physical drift. | **High** (~0.8) that trims are burst/hysteresis-driven, not rate-driven; **High** (~0.85) that 499 is an estimator artifact; Medium on the exact burst source split (network loss vs. playout quantization) |

No accepted spec value (60/200 ms, ±100 ppm, 100 ms) needs to change to fix A/B/C. Three items
need maintainer adjudication because they are about *interpretation* or about the M4 500 ms
backlog decision (§7.4 / MAX_PENDING), listed in §D.

---

## A. Server: pending pinned at ~480 ms, 1 pre-packetize drop / ~5 s, encoded queue never full

### A.1 What the code decides

1. The pre-packetize bound and the "drop newest" decision — `crates/v2-transport/src/media_pump.rs:138-143`:

```rust
let max_frames = u64::from(self.format.sample_rate) * MAX_PENDING_MS / 1_000; // 24000
let incoming = (chunk.pcm.len() / frame_bytes) as u64;
if self.pending_frames() + incoming > max_frames {
    self.dropped_chunks += 1;
    return Ok(());
}
```

`MAX_PENDING_MS = 500` (`media_pump.rs:48`). `pending_frames()` (`media_pump.rs:102-113`) is
the number of accepted-not-yet-packetised frames. The encoded queue is separate and drops the
*newest* only at 256 datagrams (`send_queue.rs:15, 45-56`), which is why
`queue_dropped_newest=0`: the network/queue side is healthy; the loss is entirely pre-packetize.

2. The only §7.4 enforcement is a wall-clock token bucket — `crates/v2-transport/src/send_queue.rs:140-162`:

```rust
Some(last) => {
    if now > last {
        let elapsed = now.saturating_duration_since(last);
        let accrued = elapsed.as_nanos() * u128::from(self.sample_rate) / 1_000_000_000;
        let accrued = u64::try_from(accrued).unwrap_or(u64::MAX);
        self.available = self.available.saturating_add(accrued).min(self.window_frames);
        self.last_refill = Some(now);          // <-- fractional frames discarded here
    }
}
```

3. `take_sendable` is called from the media loop once per loop iteration — `crates/v2-transport/src/driver.rs:117-173`:
   a `tokio::select!` over `MEDIA_TICK = 10 ms` **or** one `chunks.recv()`, then drains the
   channel with `try_recv()`, then calls `pump.take_sendable(now, …)`. A capture-chunk wake-up
   and a timer tick can therefore both produce a `take_sendable` call, and a tick left ready by
   a chunk branch is consumed immediately on the next iteration (near-zero `elapsed`).

### A.2 Root cause H1 — fractional refill truncation (High)

Each `try_consume` computes `floor(elapsed_ns × 48000 / 1e9)` and then sets
`last_refill = now`. The fractional part (0 ≤ f < 1 frame) is dropped, and the next call
restarts from the new `now`, so it is never recovered. Under sustained backlog the pump calls
this hundreds of times per second, so the loss is `Σ f ≈ 0.5 frame × calls/s`:

| total pacer calls/s (100 timer ticks + extra wake-ups) | simulated delivered rate (one run) | deficit | drops/5 s at ~480-frame chunks |
|---|---|---|---|
| 200 (100 ticks + 100 chunk wake-ups) | ≈28 739 k / 28 800 k frames per 600 s | **≈−2 100 ppm** | ≈1.06 |
| 300 (100 ticks + 200 chunk wake-ups) | ≈28 713 k / 28 800 k | **≈−3 000 ppm** | ≈1.52 |

(Run-to-run variation is ≈±10 ppm; the mean loss is the analytic `0.5 frame × calls/s`.)

*Simulation in `/tmp/m508/sim.py`; it models the exact `send_queue.rs` arithmetic (including the
`min(window)` cap) with 10 ms ticks plus uniformly-jittered extra wake-ups. The random-dt runs
lose ≈0.5 frame per call as predicted; with exactly 10 ms dt and no extra calls the loss is 0,
i.e. this is a *cadence/jitter* bug, not an always-on constant.

Observed drop rate (`server-C-extended.log`): first line `pump_dropped_chunks=1` at
`02:58:21.877`, last `=176` at `03:12:30.247` → **175 increments / 848.4 s = 0.206 / s**.
With chunks of ~480 frames (the `pending_frames` values differ by multiples of 480:
22 694 / 23 040 / 23 520, and ~480-frame deltas appear throughout), that is
**≈99 frames/s ≈ 2 060 ppm** of sustained excess production vs. consumption. The simulation's
~200-calls/s case gives ≈−2 100 ppm ≈ 101 fps ≈ **0.212 chunks/s** — within a few percent of the
observed 0.206/s. This is a very tight quantitative match and is the strongest evidence for H1.

Scale check: even timer ticks alone lose up to ~0.5 frame per tick ≈ **1 000 ppm**, an order of
magnitude larger than §8's ±100 ppm rate-match budget. The pacer's send clock literally cannot be
more accurate than its call cadence, so the server can never reliably track a 48 000 fps capture
clock while it truncates on every call.

Why the 5 s cadence: it is the *diagnostic* cadence (`DropReporter`, `media_pump.rs:277-304`),
not the drop cadence; one drop per ~4.85 s is what a ~2 000 ppm deficit produces. The reported
`pending_frames` is *drop-time biased*: `DropReporter::poll` only emits when a counter changed
(`media_pump.rs:291-294`), so every logged `pending_frames` is measured at an instant a drop just
happened. It does not prove pending is pinned between drops; it proves it reaches the bound at
each drop.

**Classification against the ticket's four options:** primarily **pacer token-rate accounting**
(the truncating refill); **loop cadence** is the multiplier — more `take_sendable` calls per
second lose more fractions, so the deficit grows with wake-up rate; **pacer/timestamp semantics**
are a separate conformance problem (the pacer enforces a wall-clock burst, not the §7.4
capture-lag formula — see A.4); a real **capture-clock offset** is possible but is not needed to
explain the measured rate.

### A.3 Secondary contributors

- **H2 — no catch-up (High as a design gap, Medium as the cause of this run).** Refill is capped
  at one window (`send_queue.rs:148-151`), and the only way to drain backlog faster than the
  nominal media rate is the initial 100 ms burst. Any transient stall (Windows scheduler, a long
  `send_datagram` burst, a delayed tokio tick) permanently adds to `pending`; the pacer can never
  repay it. This explains why the backlog *reached* ~500 ms and stayed there, and why the system
  is fragile on a loaded desktop (browser + VS Code were open per `RESULT.txt`). It does not by
  itself explain a *continued* 0.2 drop/s once at the bound; H1 does.
- **H3 — capture-device clock (Medium).** WASAPI loopback is clocked by the render endpoint
  (`capture/windows/backend.rs:406-464`). If the endpoint is genuinely ~2 000 ppm fast relative to
  `Instant`, H1 is not needed to explain the drops. A consumer device at +2 000 ppm is unusual but
  not impossible. H1 and H3 are distinguishable by measurement (see A.6), and they are not
  mutually exclusive.
- **H4 — earlier growth (Low).** Playback began 10:48:39; first drop 10:58:21 (9 m 42 s). At a
  constant 100 fps deficit, 24 000 frames take only 240 s, so the deficit was smaller early (fewer
  wake-ups / a less loaded machine) or `pending` did not start at zero. Not a separate bug.

### A.4 Does `capture_position − last_sent_position` exceed 100 ms? (Yes — and the code never checks it)

`MediaPump::pending_frames()` is, by construction, `newest accepted capture position −
packetizer.next_timestamp()`, i.e. exactly the spec's `capture_position − last_sent_position`
(plus any capture frames dropped at the bound and not yet replaced). At the drop instants it is
22 694–23 827 frames = **473–496 ms** (`server-C-extended.log:11-180`). So the literal §7.4
formula (`docs/wire-protocol-v2/spec.md:499`:
`capture_position − last_sent_position ≤ 100 ms`) **is violated** in this state.

But nothing in the implementation compares that quantity to 100 ms:

- `MediaPacer` only bounds *how much media may be released per unit of wall time* (`send_queue.rs:140-162`);
- the pump only bounds total backlog at 500 ms (`media_pump.rs:138-143`);
- the comment at `media_pump.rs:43-48` asserts "the separate §7.4 send-ahead invariant (≤ 100 ms)
  is enforced by `MediaPacer`, not by this buffer bound" — that assertion is false as written:
  the pacer cannot observe capture position.

The M5-05 evidence already recorded the same interpretation
(`.scratch/wire-protocol-v2/evidence/m5-05/README.md:19-25`: "`MAX_PENDING_MS` (500 ms) is
unchanged and is backlog latency, not the §7.4 send-ahead window"; the guard test is
upper-bound-only and "never went RED"). In other words the project has been enforcing
"≤ 100 ms of burst per 100 ms of wall time" and treating the spec's capture-lag formula as
aspirational. That is a **spec-semantics adjudication item**, not just a bug: under the literal
formula a 500 ms backlog is non-conformant; under the burst reading it is allowed. Either way,
the *current* behaviour is a functional problem because it silently discards ~10 ms of capture
audio every ~5 s (the client correctly re-bases on the timestamp jump and inserts no silence —
`android-client-v2/native/src/jitter.rs:321-331` — so the loss is inaudible-ish but real).

### A.5 Minimal fix (server) and risk

1. **Fix the token accounting (low risk, no spec-value change).** Do not throw away the fractional
   accrual. Either keep a `refill_remainder: u128` of sub-frame nano-units and carry it, or compute
   `total = (now − epoch) × sr` from a fixed epoch and only convert the *consumed* amount. Add a
   regression test that calls `try_consume` at 1/3/7 ms cadences for 10 s and asserts
   `consumed_frames` = 480 000 ± 1. (There is currently no such test: `media_queue.rs:95-124` only
   checks the window cap and two same-instant consumes.)
2. **Add bounded catch-up or a target-aware decimation (medium risk, adjudication).** To honour
   the literal §7.4 formula, when `pending > 100 ms` the pump needs either (a) a bounded catch-up
   budget (send up to one extra window per second; keeps all audio, needs a clear statement that
   this does not violate the wall-clock burst reading), or (b) drop **oldest** down to 100 ms
   (drops audio, directly contradicts the M4 500 ms "survive a stall" rationale). This touches an
   accepted value/decision → **maintainer adjudication**.
3. **Instrument (low risk).** Log `pacer.consumed_frames()` and `pending_frames()` on the 5 s
   tick even when no drop occurred, and log the pacer call count. This turns A.3 into a direct
   measurement (see A.6). No protocol change; `consumed_frames()` already exists
   (`send_queue.rs:132-135`) but is only used in a test.

### A.6 Falsifiable checks

- **Pacer call rate:** add a counter around `take_sendable` and log calls/s. H1 predicts
  ≈200 calls/s on this rig; H3 predicts the count is irrelevant and the deficit persists at
  ≤100 calls/s.
- **Consumed-rate vs wall clock:** log `consumed_frames` at t0 and t1 (≥60 s apart, with
  `pending > 0` throughout). H1 predicts `(consumed1−consumed0)/(t1−t0) ≈ 47 900 fps` (−2 000 ppm);
  H3 predicts ≈48 000 fps while capture still outruns it.
- **Capture rate:** in a Windows build, sample `MediaTap`'s `next_frame` (or the last
  `MediaChunk.end_frame`) against `Instant` over 10 min. This directly measures the device rate in
  ppm and splits H1 from H3.
- **Pending not at the bound:** a separate periodic log (not only on drop) will show whether
  pending saw-tooths or is truly pinned.

---

## B. Client: 5 576 `underrun` fills in case A

### B.1 What the code decides

- Gate — `android-client-v2/app/src/main/java/dev/audioshare/v2/MediaPlayer.kt:415-427`:

```kotlin
if (platformUnderruns <= lastPlatformUnderruns) {
    return UnderrunGate(false, lastPlatformUnderruns)
}
val fill = hasPlayed && frameBytes > 0 && availableBytes < frameBytes
return UnderrunGate(fill, platformUnderruns)
```

- `platformUnderruns` is `AudioTrack.underrunCount` (`MediaPlayer.kt:187`).
- Fill size — `MediaPlayer.kt:258, 380-392`: `LOOP_SLEEP_MS = 10`, so
  `frames = 48000 × 10 / 1000 = 480 frames` (10 ms); it writes that silence to the sink and calls
  `native.recordUnderrun(session, acceptedFrames)`. The native side increments the `Underrun`
  concealment *event* counter for every call with `acceptedFrames > 0`
  (`native/src/media_session.rs:597-607, 623-631`), and adds `acceptedFrames` to the signed
  `concealed_frames` field.
- The fill branch is only taken when the read branch is not (`MediaPlayer.kt:599-615`), i.e. the
  app ring has `< 1 frame` available.

### B.2 Root cause H1 — the platform counter is not an underrun signal (High)

`client-A.log` storm shape:

| time | underrun | Δ |
|---|---|---|
| 10:50:12.166 | 206 | (storm already on) |
| 10:50:17.416 | 452 | +246 / 5 s |
| 10:50:22.666 | 698 | +246 / 5 s |
| … | … | ≈ +246 / 5 s = **49 / s** |
| 10:52:51.665 | 5 473 | |
| 10:52:56.666 | 5 573 | +100 |
| 10:58:47.165 | 5 600 | +27 in 6 min ≈ 0.075 / s |

So the counter runs at ~49 events/s for ~2.5 min, then essentially stops. A real 49/s underrun
rate with 10 ms fills would be ~490 ms of silence per second — half of playback — which nobody
would describe as "没有卡顿" (`RESULT.txt` case A).

The same-run AudioFlinger dump (`aflinger-C.txt`) shows:

- line 223: `Normal mixer raw underrun counters: partial=0 empty=0`
- lines 225-226: the app's track row:
  `287 yes 11044 3721 260 A … 48000 … Server FrmCnt FrmRdy … 2052 2015 A Underruns=3508242 …`
  → `FrmRdy/FrmCnt = 2015/2052` (98 % full) and a per-track Underruns value of **3 508 242**
  (≈2 658 "events"/s over the 22 min track lifetime — impossible as real underruns).
- lines 539-541: `numTracks=1 writeErrors=0 underruns=2`, `mixPeriod=5.33 ms latency=42.67 ms`.

This is the Q3 behaviour already documented
(`.scratch/wire-protocol-v2/m5-proposal.md:72`, `map.md:64`,
`evidence/m5-buffer-experiment/results.md:45-46,99-100`: teardown alone yields thousands to
50 240; 30/40 ms tiers recorded +1 286/+1 779 with clean subjective playback). The counter is not
a stall count, yet `underrunGate` treats *every advance* as one fill.

**What each fill means in inserted silence:** worst case 480 frames × 5 576 = 2 676 480 frames =
**55.8 s** at 48 kHz. Actual is `concealed_frames / 5576` (frames per fill), which is a §9 field
already computed
(`media_session.rs:559`) but **not printed** by the `playback.concealments` log
(`native/src/session.rs:1128-1138` prints only the per-kind event counts). This is the single
most important missing number. Because the AudioTrack is ~98 % full at dump time, most writes
probably accept only the free space (tens of frames), not all 480; that would reconcile a high
event count with a tolerable audible result. The periodic `AudioTrack: isLongTimeZeroData …
N Seconds` lines (10:50:09–10:51:02, N up to 23) show that *some* zero data was accepted, so the
fills are not purely phantom — they just are not 5 576 × 10 ms.

### B.3 H2 — coupled real starvation (Medium)

The ring-empty condition plus the platform events can also be a *secondary* real stall: when the
jitter buffer waits up to `target_ms` (60 ms) for a missing sequence slot
(`native/src/jitter.rs:317-363`, threshold at 339-345), the app ring can drain for ~60 ms while
the AudioTrack holds only 2 052 frames (42 ms) of buffer. Each such wedge can produce one or more
real underruns and then a 60 ms batch release that spikes the app ring (see C). But this cannot
turn ~570 gap events (gap count over 50 min, `client-all.log`) into 5 576 underrun *events*; the
count inflation is H1.

### B.4 Reliable signal to gate underrun fill

Use a signal that reflects *the reader being starved*, not a vendor counter:

1. **Sustained ring emptiness:** require `availableBytes < frameBytes` for N consecutive loop
   iterations while `hasPlayed` (N ≈ 2–3, i.e. 20–30 ms). Between paced datagrams the ring is
   normally empty for a few ms, so N must be > one datagram period; 20–30 ms is a safe start and
   is tunable without touching any §8 number.
2. **Playhead stagnation (stronger):** sample `AudioTrack.getPlaybackHeadPosition()`; if it does
   not advance over more than one AudioTrack buffer period while the play state is PLAYING and
   data has been submitted, the platform truly stalled. This is vendor-independent.
3. Keep `underrunCount` only as a §9 diagnostic field (it already is), and sanity-cap or
   rate-limit it; never use its absolute delta as the fill trigger.
4. When a fill is applied, log `acceptedFrames` per event (or aggregate into
   `concealed_frames`) so B is measurable next run.

### B.5 Minimal fix and risk

- Replace the gate with (1) and/or (2) above. No spec value changes: §8 says "underrun → silence"
  and "every intervention counted", and the *definition* of the trigger is implementation.
  Risk: medium-low; it changes concealment counts and may stop inserting benign silence, so it
  needs a device listening check.
- Add `concealed_frames` (and `late_dropped`, `cumulative_lost`) to the 5 s L1 log line. Zero
  behavioural risk; §9 already carries the fields.
- If fills are still wanted as a stall masker, also reset/notify the drift estimator from
  `record_underrun` (it currently adjusts `position_adjust_frames` but does not reset the drift
  window, unlike `record_playback_at`; `media_session.rs:463-471, 623-631`).
- Note the interaction: every accepted fill advances the playout timeline without consuming ring
  frames, which increases the app-ring backlog and can trigger a soft trim (C). Fixing B should
  reduce C too.

---

## C. Client: soft trims, re-anchors, and the 499 ppm sample

### C.1 Counts and implied rates

From the logs:

| window | soft frames / events | re-anchor frames / events | implied "rate" of dropped frames |
|---|---|---|---|
| A, 5 m 06 s | 63 263 / 28 | 43 207 / 3 | 63 263 / 306 s ≈ **207 fps ≈ 4 300 ppm** |
| Ext, 10:58:52→11:12:10 (798 s) | +534 524 / +205 | +112 443 / +11 | 534 524 / 798 ≈ **670 fps ≈ 14 000 ppm**; + re-anchor ≈ 141 fps → **~17 000 ppm combined** |
| Whole run to 11:38 | 1 133 499 / 465 | 394 203 / 37 | ≈ 378 fps soft + 131 fps re-anchor ≈ **0.9–1 %** |

These are *not* steady rates: they are the sum of episodic deletions divided by elapsed time.
If they were a real capture-vs-playback rate difference, the app ring would grow monotonically at
0.4–1.7 %, and after each trim it would sit at the 60 ms target and re-exceed the 80 ms threshold
within a few seconds. Instead the reported `buffer_current_ms` is 27–60 ms and *often below* the
60 ms target (45, 33, 56, 50, 40, 28, 27 — `client-A/B.log`, `client-all.log`), with long quiet
stretches and then a ~47–54 ms trim. A sustained receiver-slow mismatch cannot leave the ring
below target between trims. Conclusion: **trims are burst-driven, not rate-driven (High).**

### C.2 Why delivery is bursty

The trim decision is `MediaRingBuffer.trim` (`MediaRingBuffer.kt:73-99`): trim down to the 60 ms
target whenever backlog exceeds `target + 20 ms` (hysteresis, `MediaPlayer.kt:277`). The reader
removes the *oldest* audio. The writer is the native jitter buffer. The jitter buffer deliberately
holds later datagrams while a sequence hole is open and releases them all at once once the hole is
declared (`jitter.rs:317-363`: wait until `span ≥ target`, fill the hole with silence, then
`while`-commit all contiguous queued frames). A single lost/reordered datagram therefore
(a) stalls write-out for up to 60 ms and (b) produces a ~60 ms burst into the app ring. With a
60 ms target and a 20 ms hysteresis, such a burst lands at ~60–120 ms of backlog → trim of
~20–60 ms. The observed per-event trims (63 263/28 ≈ 2 260 frames ≈ 47 ms; 534 524/205 ≈ 2 608
frames ≈ 54 ms) match this shape almost exactly. The `gap` counter (22→570 over the run, ~1 per
5 s) confirms recurring sequence holes / jitter-buffer wedges as the trigger source.

The platform/fill interaction (B.5) is a second burst source: accepted underrun silence advances
playout without consuming ring frames, so the app ring temporarily grows by the inserted amount
(bounded by the 42 ms AudioTrack buffer). With a 60 ms target, +42 ms > 80 ms hysteresis → a trim.

**Trim budget (why `concealed_frames` matters).** In the extended window the client dropped
534 524 soft + 112 443 re-anchor frames in 798 s. During the same window the underrun counter rose
only by 625 events; at the maximum 480 frames/fill that is ≤ 300 000 frames, i.e. at most ~56 %
of the soft-trim volume even in the best case. If fills were typically partially accepted (the
AudioTrack was 98 % full), they explain far less, and the remainder is either a genuine
capture-vs-playback offset or jitter-buffer burst trimming. Logging `concealed_frames` (and
`cumulative_lost`) closes this budget and tells us which. Do **not** infer a 0.4–1.7 % device
rate from the trim counters alone.

### C.3 The drift samples, and why 499 ppm is an artifact (High)

`DriftEstimator` is a least-squares slope over a 10 s rolling window
(`native/src/drift.rs:94-162`). It is **not robust** in this workload:

- it is reset by every soft trim (`media_session.rs:640-653`), every re-anchor (`669-680`), every
  server-side timestamp re-base (`898-910`), and by `record_playback_at` whenever the platform
  underrun counter increased since the previous 100 ms report (`463-471`). During the 49/s storm
  the latter resets it **10×/s**; after the storm, occasional +1 counter increments and a soft
  trim every ~10 s reset it roughly every 5–10 s. A full 10 s window therefore often does not
  exist, which is why only **11 `playback.drift` lines appear in ~50 min** (`client-all.log`).
- when a window does complete, it is short and its samples are quantised by the playback loop:
  the reader submits up to `scratch.size` = 4 096 bytes = **512 frames = 10.7 ms per iteration**
  (`MediaPlayer.kt:561, 599-609`) and the 100 ms report samples `submitted_position`
  (`media_session.rs:541-566, 576-584`). A single ~5–10 ms playout step near a window edge is
  enough to produce hundreds of ppm in a 10 s least-squares fit. The sample at 10:56:06 came
  ~10.25 s after the previous soft-trim reset (10:55:56, `client-B.log:86` vs the trim
  transition), i.e. the estimator had exactly one just-over-10 s window with no outlier
  filtering — this is a textbook short-window artifact.
- the later samples are decisive: `drift_ppm = -315, 662, 348, -449, -569, -119, 347` at
  11:29:16–11:32:05 (`client-all.log:56175-58432`), swinging sign and several hundred ppm
  within 5 s. A physical 10 s clock difference cannot do that; a reset/quantisation-confounded
  estimator can.

So: **the 499 is not evidence of 0.4 % physical drift, and the 36/-80 sample is not a clean
rate measurement either.** The controller's ±100 ppm clamp is not the reason the buffer moves;
the buffer moves because of bursts and trims, which the estimator is not measuring.

There is also a **direct hardware-rate measurement** in the same AudioFlinger dump: the FastMixer
block reports `localSR(47996.9, 5.05908e-07)` (`aflinger-C.txt:542`, next to
`:539 numTracks=1 writeErrors=0 underruns=2 overruns=0`) — the playback device's measured local
sample rate is **47 996.9 Hz ≈ −65 ppm** from 48 kHz. The other output threads in the same dump
report `localSR` 47999.8 (−4 ppm), 48002 (+42 ppm) and 47998.7 (−27 ppm). All are **within the
accepted ±100 ppm budget**, not 0.4 %. The FastMixer mix-cycle stats (`mean=5.33 ms
max=10.53 ms stddev=0.45 ms`) are also healthy. This independently confirms that the
multi-thousand-ppm figures implied by the trim counters are not a physical clock rate.

One genuine design tension remains: the controller only commands **integer Hz**
(`playbackRateHz`, `MediaPlayer.kt:23-28`: ±100 ppm → ±5 Hz at 48 kHz, quantised to
20.8 ppm steps), and the estimator's noise is far larger than that step. Until the estimator
is clean, the controller is effectively driven by noise.

### C.4 Minimal fix and risk

1. **Make the drift window robust (medium-low risk, no spec value).** Do not clear the whole
   window on every discontinuity; either (a) keep the samples and subtract the known position
   offsets (`position_adjust_frames` is already maintained), or (b) exclude the affected interval
   rather than reset. Require a minimum number of samples and a span ≥ 10 s before publishing;
   optionally median-filter/require 2–3 consistent windows. Log `span_ms` and `sample_count` with
   each ppm. (§8 still says 10 s rolling window; this is implementation quality, not a value
   change.)
2. **Make the soft trim burst-aware (medium risk).** Keep 60/200 ms (accepted). Change only the
   *hysteresis/decision*: require backlog > target+hysteresis for ≥ 2 consecutive observations
   (e.g. ~50 ms) before trimming, and/or raise the hysteresis to ~30–40 ms so a single 60 ms
   jitter-buffer release is absorbed rather than trimmed. Soft trim is already a documented
   deviation from §8's four interventions (`MediaPlayer.kt:267-277`), so this does not need an
   accepted-value change — but it trades latency for drop count and needs a listening A/B.
3. **Reduce the burst at the source (medium, ties to B).** Do not hold all later datagrams for the
   full 60 ms before filling a hole when the hole is known to be a *server-side timestamp re-base*
   (sequence contiguous) — that case already inserts no silence; keep it immediate. For true
   network holes, consider releasing later media with a short bounded hold rather than the full
   target. This is a jitter-buffer policy change and needs evidence/adjudication.
4. Fix B first: the platform-counter-driven fills are a removable burst/offset source, and the
   trims should drop with them. Re-measure before changing trim thresholds.

---

## D. Ranked minimal fixes (no code changed)

| Rank | Fix | Scope | Risk | Touches accepted value / needs adjudication |
|---|---|---|---|---|
| 1 | Carry the fractional pacer refill (`MediaPacer`) so the bucket delivers exactly `sample_rate`; add a multi-cadence regression test | server | Low | No. Pure bug fix; 100 ms implementation only (no value change) |
| 2 | Gate underrun fill on sustained ring emptiness / playhead stagnation instead of `AudioTrack.underrunCount`; log `concealed_frames` per fill | client | Low–Medium | No. §8 trigger definition is implementation; concealment accounting unchanged |
| 3 | Robust drift estimator: don't reset the whole 10 s window on every underrun/trim; publish only full, sufficiently sampled windows; log span/count | client | Medium | No. §8's 10 s window unchanged |
| 4 | Burst-aware soft trim (require sustained excess before trimming; optionally raise hysteresis) | client | Medium | Partly: target 60 / hard 200 are accepted and must stay; hysteresis is not a spec value. Needs listening A/B |
| 5 | Server instrumentation: periodic `pending_frames` + `pacer.consumed_frames` + call rate (not only on drop) | server | Low | No |
| 6 | §7.4 conformance decision: implement `capture_position − last_sent_position ≤ 100 ms` via bounded catch-up or drop-oldest-to-100 ms | server | Medium–High | **Yes — maintainer adjudication.** Touches the 100 ms interpretation and the M4 500 ms `MAX_PENDING_MS` decision |
| 7 | Jitter-buffer hole-hold policy (short bounded hold vs. full target before a gap fill) | client | Medium | **Yes — needs evidence/adjudication**; changes receiver behaviour and gap/concealment semantics |

Recommended order: 1 + 5 (prove A), 2 + 3 (stop the artifact and make drift measurable),
re-measure, then 4, then adjudicate 6/7 with data.

Server side, `MAX_DATAGRAMS_PER_TICK` / `MEDIA_TICK` / `SEND_AHEAD_MS` need no change; the encoded
queue (`queue_dropped_newest=0`) and the capture tap are not implicated.

---

## E. What still needs measurement

1. **Per-fill accepted frames** — add `concealed_frames` (delta) to the 5 s `playback.concealments`
   line, or a per-fill debug line. This converts "5 576 events" into seconds of silence.
2. **Real underruns** — sample `AudioTrack.getPlaybackHeadPosition()` / dumpsys
   `FrmRdy/FrmCnt/Underruns` at 1 s cadence during the same run. Expect mixer `empty=0`,
   `FrmRdy≈FrmCnt`; confirm whether any real stalls exist.
3. **Gate-off baseline** — run with the platform counter ignored (ring-empty debounce only) and
   compare underrun events / trims. Expect the event count to collapse; if it does, H1 in B is
   proven.
4. **Pacer call rate and consumed rate** — server 5 s log of `consumed_frames`, `pending_frames`,
   `Instant`, and a call counter. Expect ~200 calls/s and ~47.9 k fps under H1.
5. **Capture device rate** — Windows 10 min sample of `MediaTap.next_frame` vs `Instant` (or the
   last `MediaChunk.end_frame`); report ppm. This is the only way to separate H1 from H3.
6. **Sampled (not drop-time) pending** — show whether pending saw-tooths or is pinned; the current
   log is biased to drop instants.
7. **True client playback rate** — `getPlaybackHeadPosition` vs wall clock over ≥60 s with no
   trims/underruns (or a Linux/PipeWire soak with the same instrumentation). Needed to know
   whether any real ±100 ppm correction is warranted.
8. **Network loss** — log `cumulative_lost`, `late_dropped`, `gap` with timestamps alongside the
   trims; this sizes the jitter-buffer-burst contribution to C.
9. **Idle-machine control** — repeat with the desktop otherwise idle (the acceptance run had
   browser + VS Code open, `RESULT.txt` NOT RUN for the idle control). H1 predicts a lower call
   rate and slower backlog growth.

---

## F. Appendix — key log excerpts

Server (`server-C-extended.log`):

```
[2026-11-20 02:58:21.877] ... media.drops session_id=2 instance=1 pump_dropped_chunks=1 queue_dropped_newest=0 pending_frames=23174
[2026-11-20 02:58:26.879] ... pump_dropped_chunks=2 queue_dropped_newest=0 pending_frames=22694
...
[2026-11-20 03:12:30.247] ... pump_dropped_chunks=176 queue_dropped_newest=0 pending_frames=23347
```
(175 increments / 848.4 s; pending 22 560–23 827 frames = 470–496 ms.)

Client underrun storm (`client-A.log`):

```
[10:50:12.166] playback.concealments gap=0 underrun=206  drift=0 reanchor=0
[10:50:17.416] playback.concealments gap=0 underrun=452  drift=0 reanchor=0   (+246)
[10:50:22.666] playback.concealments gap=0 underrun=698  drift=0 reanchor=0   (+246)
...
[10:52:51.665] playback.concealments gap=12 underrun=5473 drift=0 reanchor=2
[10:52:56.666] playback.concealments gap=15 underrun=5573 drift=0 reanchor=3
[10:58:47.165] playback.concealments gap=49 underrun=5600 drift=0 reanchor=3   (+27 in 6 min)
```

AudioTrack / AudioFlinger (`client-A.log`, `aflinger-C.txt`):

```
[10:48:39.704] AudioTrack: createTrack_l ... frameCount=2052, mSampleRate=48000
[10:48:39.752] AudioSink.open: minBuffer=16416 bytes=16416 frames=2052 ms=42
[10:50:07.184] AudioTrack: restartIfDisabled: ... disabled due to previous underrun, restarting
[10:50:09.936] AudioTrack: isLongTimeZeroData zero date time 1 Seconds   (…up to 23 Seconds)
...
aflinger: Normal mixer raw underrun counters: partial=0 empty=0
aflinger: 287 yes 11044 3721 260 A … 48000 … FrmCnt=2052 FrmRdy=2015 A Underruns=3508242 …
aflinger: numTracks=1 writeErrors=0 underruns=2, mixPeriod=5.33 ms latency=42.67 ms
```

Drift (`client-B.log`, `client-all.log`):

```
[10:54:03.666] playback.drift drift_ppm=36  correction_ppm=-80  buffer_current_ms=60 target=60
[10:56:06.916] playback.drift drift_ppm=499 correction_ppm=100  buffer_current_ms=56 target=60
[11:29:16.166] playback.drift drift_ppm=-315 correction_ppm=-60 buffer_current_ms=50
[11:29:21.416] playback.drift drift_ppm=662  correction_ppm=100 buffer_current_ms=40
[11:29:31.915] playback.drift drift_ppm=-449 correction_ppm=-100 buffer_current_ms=28
```

---

## G. Code references (line numbers at `800c40c`)

- `crates/v2-transport/src/media_pump.rs:43-48` (M4 500 ms rationale / pacer claim),
  `:48` (`MAX_PENDING_MS`), `:102-113` (`pending_frames`), `:122-149` (`accept`),
  `:138-143` (bound + drop), `:170-227` (`take_sendable`), `:201` (`try_consume`),
  `:204-219` (timestamp re-anchor), `:277-304` (`DropReporter`).
- `crates/v2-transport/src/send_queue.rs:15,18` (queue/window), `:45-56` (drop newest),
  `:105-162` (`MediaPacer`), `:143-152` (truncating refill), `:156-160` (consume).
- `crates/v2-transport/src/driver.rs:80-84` (`MEDIA_TICK`, `MAX_DATAGRAMS_PER_TICK`),
  `:117-173` (media loop / `take_sendable`), `:176-189` (`DropReporter` wiring).
- `crates/audio-share-core/src/capture/media.rs:162-169` (subscriber channel capacity 8),
  `:175-211` (`publish`; a full subscriber drops the chunk being published = drop-newest, and the
  counter is not reachable from v2-transport, per the M5-05 known-gap note).
- `crates/audio-share-core/src/capture/windows/backend.rs:406-464` (WASAPI poll loop, one publish
  per packet), `:361-403` (min period, 1 s buffer).
- `crates/audio-share-core/src/server/mod.rs:610-614` (publish before v1 checks).
- `android-client-v2/app/src/main/java/dev/audioshare/v2/MediaPlayer.kt:187` (`underruns`),
  `:258` (`LOOP_SLEEP_MS=10`), `:277` (`SOFT_TRIM_HYSTERESIS_MS=20`),
  `:330-349` (`applyReadPosition`), `:380-392` (`fillUnderrun`), `:415-427` (`underrunGate`),
  `:435-439` (`readyForPlayback`), `:599-615` (read vs fill), `:642-643` (sleep).
- `android-client-v2/app/src/main/java/dev/audioshare/v2/MediaRingBuffer.kt:73-99` (trim /
  soft threshold / re-anchor).
- `android-client-v2/native/src/jitter.rs:317-363` (drain, hole wait, gap fill),
  `:321-331` (timestamp re-base → no silence).
- `android-client-v2/native/src/media_session.rs:463-471` (reset on underrun increase),
  `:539-569` (diagnostics; `concealed_frames` at 559), `:576-584` (`submitted_position`),
  `:597-607` (`record_concealment`), `:623-631` (`record_underrun`), `:640-653`
  (`record_soft_trim`), `:669-680` (`record_trim`), `:898-910` (re-base resets drift/jitter).
- `android-client-v2/native/src/drift.rs:38-53` (constants), `:89-91` (`reset`),
  `:94-119` (`observe` window), `:124-162` (`ppm` least squares), `:242-288` (controller).
- `docs/wire-protocol-v2/spec.md:34` (D12), `:496-502` (§7.4), `:525-539` (§8), §9 diagnostics.
- `.scratch/wire-protocol-v2/evidence/m5-05/README.md:19-25` (500 ms backlog vs §7.4;
  upper-bound-only guard test), `:107-124` (known gaps / timestamp-gap lesson).
