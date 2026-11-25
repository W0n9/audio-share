# PipeWire capture spike — findings

Ticket: `issues/03-pipewire-capture-spike.md` (and feeds `06-linux-real-capture.md`)
Date: 2026-11-09
Environment: Ubuntu 24.04.5 LTS, Rust 1.98.1, libpipewire 1.0.5, wireplumber 0.4.17,
pipewire/wireplumber/pw-cli/pw-play/pw-record/dbus-run-session installed,
`libpipewire-0.3-dev` 1.0.5, clang/libclang 18.

Scratch code (throwaway, outside the repo):
- `/home/w0n9/.claude/jobs/d82859a4/tmp/pwspike/` — Cargo project, safe-crate spike (`src/main.rs`), loop-reuse proof (`src/bin/reuse.rs`), raw fallback (`src/bin/sys_fallback.rs`)
- `/home/w0n9/.claude/jobs/d82859a4/tmp/pwspike-harness.sh` — headless PipeWire/wireplumber driver
- `/home/w0n9/.claude/jobs/d82859a4/tmp/tone.wav` — 440 Hz stereo 48 kHz 16-bit source
- `/home/w0n9/.claude/jobs/d82859a4/tmp/analyze_tone.py` — Goertzel/peak verification of captured PCM

Legend: **[verified]** = compiled and executed in this environment; **[unverified]** = reasoned
from source/documented API, not executed here.

## TL;DR / recommendation

**Go with the safe `pipewire` crate 0.10.1.** It covers every requirement in ticket 03:
Audio/Sink enumeration with `priority.session` default selection, monitor capture via
`target.object` (name or `object.serial`), F32LE plus S16LE/S24LE/S32LE/U8, exact
rate/channel negotiation, clean stop from another thread, and repeated start/stop.
`pipewire-sys` is a viable fallback but is not needed; it requires manual interface-vtable
dispatch and manual buffer re-queueing.

## 1. Crate availability and build

`cargo add --dry-run` against the configured sparse mirror (`~/.cargo/config.toml` →
`sparse+https://mirrors.ustc.edu.cn/crates.io-index/`) resolved:

| crate | version | notes |
|---|---|---|
| `pipewire` | **0.10.1** | not yanked, `rust-version = 1.80`, MIT, repo `https://gitlab.freedesktop.org/pipewire/pipewire-rs` |
| `pipewire-sys` | **0.10.1** | not yanked, `links = "pipewire-0.3"`, bindgen at build time |
| `libspa` | **0.10.1** | safe SPA wrapper, re-exported as `pw::spa` |
| `libspa-sys` | **0.10.1** | FFI SPA; needed directly only for the fallback |

This confirms the research doc's `pipewire` 0.10.1 (`docs/research/server-migration-rust-vs-go.md:36,187`).

Exact commands used:

```bash
cargo new pwspike --bin
cd pwspike
cargo add pipewire@0.10.1 --features v1_0_0
cargo add ringbuf@0.6.0                 # spike-only lock-free handoff
cargo add pipewire-sys@0.10.1 libspa-sys@0.10.1   # fallback skeleton only
cargo build
```

- Enable `v1_0_0` for the Ubuntu 24.04 baseline (libpipewire 1.0.5). The feature chain
  enables `v0_3_44`, which the upstream example uses for `TARGET_OBJECT` (the key constant
  itself is always available). Do not enable `v1_2_0` on the 1.0.5 baseline — it exposes
  APIs newer than the installed headers.
- Build-time system deps: `pkg-config`, `libpipewire-0.3-dev` (provides `libpipewire-0.3`
  and `libspa-0.2` for `system-deps`), and libclang for `pipewire-sys`'s bindgen build
  script. On a minimal Ubuntu 24.04 CI image install `libpipewire-0.3-dev libclang-dev
  pkg-config`.
- Registry source root used below:
  `~/.cargo/registry/src/mirrors.ustc.edu.cn-38d0e5eb5da2abae/`
  (crate dirs `pipewire-0.10.1/`, `libspa-0.10.1/`, `pipewire-sys-0.10.1/`,
  `libspa-sys-0.10.1/`).

## 2. Verified API map (pipewire 0.10.1 + libspa 0.10.1)

All names below are the exact Rust item names in this version.

| Purpose | API | Source |
|---|---|---|
| init (idempotent) | `pw::init()`, `unsafe pw::deinit()` (once, at process exit) | `pipewire-0.10.1/src/lib.rs:180-193` |
| loop | `pw::main_loop::MainLoopRc::new(None)`, `.loop_()`, `.run()`, `.quit()`, `.downgrade()` | `src/main_loop/rc.rs:35`, `src/main_loop/mod.rs:35,43,49` |
| context | `pw::context::ContextRc::new(&loop, None)`, `.connect_rc(None)` | `src/context/rc.rs:48,80` |
| core | `CoreRc`, `core.sync(seq)`, `core.get_registry_rc()`, `PW_ID_CORE` | `src/core/mod.rs:32,63,83`; `src/core/rc.rs:56` |
| registry | `registry.add_listener_local().global(...).global_remove(...).register()` | `src/registry/mod.rs:67,194,219,289` |
| registry object | `GlobalObject { id, permissions, type_, version, props: Option<&DictRef> }` | `src/registry/mod.rs:289` |
| dict | `DictRef::get(&str) -> Option<&str>` | `libspa-0.10.1/src/utils/dict.rs:90` |
| stream | `StreamRc::new(core.clone(), name, props)`, `.connect(...)`, `.disconnect()`, `.node_id()`, `.name()`, `.state()` | `src/stream/rc.rs:39`; `src/stream/mod.rs:188,271,325-353` |
| stream listener | `.add_local_listener_with_user_data(D)` → `.state_changed` `.param_changed` `.process` `.register()` | `src/stream/mod.rs:164,686,773,860,895` |
| flags | `StreamFlags::{AUTOCONNECT, MAP_BUFFERS, RT_PROCESS, ...}` | `src/stream/mod.rs:947` |
| buffer | `stream.dequeue_buffer() -> Option<Buffer>`; `Buffer::datas_mut()`; `Data::data()`, `Data::chunk()`; `Chunk::{offset,size}`; Drop auto-queues | `src/stream/mod.rs:252`; `src/buffer.rs:28,79-83`; `libspa-0.10.1/src/buffer/mod.rs:84,98,143,151` |
| properties | `pw::properties::properties!` macro, `PropertiesBox::insert` | `src/properties/box_.rs:174-186`; `src/properties/mod.rs:67` |
| stop channel | `pw::channel::channel()`; `Sender<T>` (Send, clonable), `Receiver::attach(loop, cb)` | `src/channel.rs:143,198,87` |
| audio format | `AudioFormat::{U8,S16LE,S24LE,S24_32LE,S32LE,F32LE,...}` | `libspa-0.10.1/src/param/audio/mod.rs:22-47` |
| audio info | `AudioInfoRaw::{new,set_format,set_rate,set_channels,parse,format,rate,channels}`, `From<AudioInfoRaw> for Vec<Property>` | `libspa-0.10.1/src/param/audio/raw.rs:23,36,52,60,82,104` |
| pod build | `PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(Object{...}))` | `libspa-0.10.1/src/pod/serialize.rs:268` |
| time / latency | `Stream::time() -> Time` (`v0_3_50+` uses `pw_stream_get_time_n`) | `src/stream/mod.rs:64,377-394` |
| keys | `pw::keys::{MEDIA_TYPE,MEDIA_CATEGORY,MEDIA_ROLE,APP_NAME,NODE_NAME,TARGET_OBJECT,MEDIA_CLASS,OBJECT_SERIAL,NODE_NICK,NODE_DESCRIPTION,PRIORITY_SESSION,STREAM_CAPTURE_SINK}` | `src/keys.rs:66,120,183,186,189,451,463,532` |
| upstream examples | `examples/audio-capture.rs`, `examples/roundtrip.rs`, `examples/pw-mon.rs` | `pipewire-0.10.1/examples/` |

Threading contract from the crate docs: PipeWire objects do **not** implement `Send`/`Sync`;
spawn a `MainLoop` in another thread and communicate with `std::sync::mpsc` + `pipewire::channel`
(`src/lib.rs:123-132`).

## 3. Verified: enumerate Audio/Sink + select default

Complete minimal, verified code (adapted from `src/main.rs::enumerate_endpoints`):

```rust
use pipewire as pw;
use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;

#[derive(Clone)]
struct Endpoint {
    id: u32,
    serial: String,
    priority: i64,
    display_name: String,
}

fn list_endpoints() -> Result<Vec<Endpoint>, Box<dyn Error>> {
    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;

    let out: Rc<RefCell<Vec<Endpoint>>> = Rc::new(RefCell::new(Vec::new()));
    let pending = core.sync(0)?; // AsyncSeq, returned to us for the done match

    let ml_weak = main_loop.downgrade();
    let _core_listener = core
        .add_listener_local()
        .done(move |id, seq| {
            if id == pw::core::PW_ID_CORE && seq == pending {
                if let Some(ml) = ml_weak.upgrade() {
                    ml.quit();
                }
            }
        })
        .register();

    let _registry_listener = registry
        .add_listener_local()
        .global({
            let out = out.clone();
            move |obj| {
                if obj.type_ != pw::types::ObjectType::Node {
                    return;
                }
                let Some(props) = obj.props else { return };
                if props.get(*pw::keys::MEDIA_CLASS) != Some("Audio/Sink") {
                    return;
                }
                let Some(serial) = props.get(*pw::keys::OBJECT_SERIAL) else { return };
                let priority = props
                    .get(*pw::keys::PRIORITY_SESSION)
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(-1);
                let nick = props.get(*pw::keys::NODE_NICK);
                let desc = props.get(*pw::keys::NODE_DESCRIPTION);
                let name = props.get(*pw::keys::NODE_NAME);
                let display_name = nick.or(desc).or(name).unwrap_or(serial).to_owned();
                out.borrow_mut().push(Endpoint {
                    id: obj.id,
                    serial: serial.to_owned(),
                    priority,
                    display_name,
                });
            }
        })
        .register();

    main_loop.run(); // registry events arrive before our sync's done, then done quits
    Ok(out.borrow().clone())
}

// Old-server default rule: default_priority starts at -1 and only a strictly
// greater priority replaces it. If every sink has priority <= -1, there is no
// default (empty string) in the C++ implementation.
fn pick_default(eps: &[Endpoint]) -> Option<&Endpoint> {
    let mut best_prio = -1i64;
    let mut best = None;
    for ep in eps {
        if ep.priority > best_prio {
            best_prio = ep.priority;
            best = Some(ep);
        }
    }
    best
}
```

Verified behavior (release binary, headless harness):

```
# two null sinks, priorities 500 and 2000
id=36 serial=37 priority=500  class=Audio/Sink name=Some("TestSink")
id=39 serial=41 priority=2000 class=Audio/Sink name=Some("TestSinkHigh")
default: serial=41 priority=2000 display="TestSinkHigh"
```

Properties:
- `media.class` is exactly the string `Audio/Sink`.
- Endpoint ID for the Wire Protocol / `target.object` is `object.serial` (a decimal string),
  **not** the registry/global `id`. In our runs `id=36` had `object.serial=37`. The old C++
  does the same (`server-core/src/linux/audio_manager_impl.cpp:369-375`).
- `node.nick` → `node.description` → `node.name` display-name fallback matches the old code
  (`...:375`). Setting `node.nick=NickName` produced `display="NickName"` [verified].
- `priority.session` arrives as a **string** in the SPA dict; parse it. Missing → `-1`.
- Registry enumeration works with only the `pipewire` daemon (no session manager) [verified].

Parity caveat: our spike initially picked the first sink even when all priorities were `-1`.
The old server would pick **none** in that case because it initializes `default_priority = -1`
and requires `priority > -1` (`audio_manager_impl.cpp:378-381,425-428`), leaving
`get_default_endpoint()` empty and the start path failing. Decision needed for the rewrite:
replicate the old rule exactly (recommended for parity), or deliberately improve it (pick the
highest even when negative). Real desktop sinks normally have a `priority.session` from
wireplumber.

## 4. Verified: monitor capture stream

Complete minimal, verified pattern (from `src/main.rs::capture_worker`). The key points:
construct all PipeWire objects **inside** the loop thread; send PCM out through a
lock-free SPSC ring; stop through `pipewire::channel`.

```rust
use pipewire as pw;
use pw::spa;
use pw::spa::param::audio::{AudioFormat, AudioInfoRaw};
use pw::spa::param::format::{MediaSubtype, MediaType};
use pw::spa::param::format_utils;
use pw::spa::param::ParamType;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Object, Pod, Value};
use pw::spa::utils::{Direction, SpaTypes};
use ringbuf::traits::Producer; // push_slice
use std::error::Error;
use std::io::Cursor;

struct UserData {
    // RT-safe outputs only: lock-free producer + atomics.
    producer: ringbuf::HeapProd<u8>,
    callbacks: std::sync::Arc<std::sync::atomic::AtomicU64>,
    format: std::sync::Arc<std::sync::Mutex<Option<(AudioFormat, u32, u32)>>>,
}

// The ring is created by the caller so the consumer stays outside the PipeWire
// thread; only the producer is moved in here.
fn capture_thread(
    target_serial_or_name: String,
    requested: AudioFormat,
    producer: ringbuf::HeapProd<u8>,
    stop_rx: pw::channel::Receiver<()>,
) -> Result<(), Box<dyn Error>> {
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    let core = context.connect_rc(None)?;

    let props = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::APP_NAME => "Audio Share Server",
        *pw::keys::NODE_NAME => "Audio Share Server",
        *pw::keys::TARGET_OBJECT => target_serial_or_name.as_str(),
    };
    // Optional; not required when TARGET_OBJECT points at a sink (verified both ways):
    // props.insert(*pw::keys::STREAM_CAPTURE_SINK, "true");

    let stream = pw::stream::StreamRc::new(core.clone(), "audio-share-server", props)?;
    let ud = UserData { producer, callbacks: Default::default(), format: Default::default() };

    let listener = stream
        .add_local_listener_with_user_data(ud)
        .state_changed(|stream, _ud, old, new| {
            if let pw::stream::StreamState::Error(e) = &new {
                eprintln!("stream error {old:?} -> Error({e})");
            }
            if new == pw::stream::StreamState::Streaming {
                eprintln!("streaming node_id={}", stream.node_id());
            }
        })
        .param_changed(|_stream, ud, id, param| {
            if id != ParamType::Format.as_raw() { return; }
            let Some(param) = param else { return; };
            let Ok((media_type, subtype)) = format_utils::parse_format(param) else { return };
            if media_type != MediaType::Audio || subtype != MediaSubtype::Raw { return; }
            let mut info = AudioInfoRaw::new();
            if info.parse(param).is_err() { return; }
            *ud.format.lock().unwrap() =
                Some((info.format(), info.rate(), info.channels()));
            // block_align = bytes_per_sample(info.format()) * info.channels()
        })
        .process(|stream, ud| {
            ud.callbacks.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // Buffer is auto-queued when dropped (src/buffer.rs:79-83).
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let datas = buffer.datas_mut();
            if datas.is_empty() { return; }
            let d = &mut datas[0];
            let off = d.chunk().offset() as usize;
            let size = d.chunk().size() as usize;
            let Some(data) = d.data() else { return };
            if off + size > data.len() { return; }
            let _pushed = ud.producer.push_slice(&data[off..off + size]);
        })
        .register()?;

    // EnumFormat: single entry; rate/channels = 0 means "let the graph decide".
    let mut info = AudioInfoRaw::new();
    info.set_format(requested);
    // info.set_rate(48_000); info.set_channels(2); // optional, verified conversion works
    let obj = Object {
        type_: SpaTypes::ObjectParamFormat.as_raw(),
        id: ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    };
    let values: Vec<u8> =
        PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(obj))?.0.into_inner();
    let mut params = [Pod::from_bytes(&values).expect("valid pod")];

    stream.connect(
        Direction::Input,
        None,
        pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS,
        &mut params,
    )?;

    let ml_weak = main_loop.downgrade();
    let stream_ctrl = stream.clone();
    let _attached = stop_rx.attach(main_loop.loop_(), move |_| {
        let _ = stream_ctrl.disconnect(); // ignore already-disconnected
        if let Some(ml) = ml_weak.upgrade() { ml.quit(); }
    });

    main_loop.run(); // blocks until stop_rx delivers or the stream dies

    // Drop order in this scope: _attached, listener, stream, core, context, main_loop.
    // That is the required order; Rust's reverse-declaration drop gives it to you.
    Ok(())
}
```

Main-thread side of the ring + stop channel:

```rust
use ringbuf::traits::Split;

let rb = ringbuf::HeapRb::<u8>::new(4 * 1024 * 1024);
let (producer, mut consumer) = rb.split(); // consumer stays in the network/tokio side
let (stop_tx, stop_rx) = pw::channel::channel::<()>();
let handle = std::thread::spawn(move || {
    capture_thread(target, AudioFormat::F32LE, producer, stop_rx)
});
// ... drain `consumer` (lock-free) into the encoder/UDP path ...
let _ = stop_tx.send(());   // Sender is Send + Clone; safe to call from any thread/task
handle.join().unwrap();
```

Verified facts:
- `TARGET_OBJECT` accepts both the `object.serial` decimal string and `node.name` [verified].
- A capture stream targeted at an `Audio/Sink` receives that sink's **monitor** audio; playing
  a 440 Hz tone with `pw-play --target TestSink tone.wav` produced the same 440 Hz content in
  every captured format (Goertzel 440 ≈ 0.042 amplitude, 1000 Hz ≈ 0) [verified].
- `STREAM_CAPTURE_SINK=true` is **not** needed when `TARGET_OBJECT` is set. With no target and
  `STREAM_CAPTURE_SINK=true`, wireplumber selected the default sink monitor and captured the
  tone [verified]. With no target and no property it also captured the only sink's monitor in
  our setup [verified, environment-specific].
- `node_id()` is only meaningful while connected: after disconnect it returns `PW_ID_ANY`
  (`0xFFFFFFFF`, printed as `4294967295`). Read it in `state_changed(..., Streaming, ...)`
  [verified].
- `Buffer` drop automatically calls `pw_stream_queue_buffer`; do not queue manually with the
  safe API [source: `src/buffer.rs:79-83`].
- Use `chunk().offset()` + `chunk().size()`, not the whole `data()` slice; `data()` returns
  `maxsize` bytes starting at the mapping base [source: `libspa/src/buffer/mod.rs:84`]. Offset
  was 0 in our runs, but the old C++ handles nonzero offsets, so keep the same logic.

## 5. Measured results (all verified)

Driver: headless harness below. Tone: `ffmpeg -f lavfi -i sine=frequency=440:sample_rate=48000:duration=12 -ac 2 -c:a pcm_s16le tone.wav`.

### 5.1 Format matrix (48 kHz, 2 ch, null sink driver quantum 2048 frames)

| requested (Encoding) | negotiated | bytes/sample | block_align | chunk bytes | frames/chunk |
|---|---|---|---|---|---|
| F32LE | `AudioFormat::F32LE` | 4 | 8 | 16384 | 2048 |
| S16LE | `AudioFormat::S16LE` | 2 | 4 | 8192 | 2048 |
| S24LE | `AudioFormat::S24LE` | **3** | **6** | **12288** | 2048 |
| S32LE | `AudioFormat::S32LE` | 4 | 8 | 16384 | 2048 |
| U8 (s8 mapping) | `AudioFormat::U8` | 1 | 2 | 4096 | 2048 |
| S24_32LE (experiment) | `AudioFormat::S24_32LE` | 4 | 8 | 16384 | 2048 |

All six formats captured the 440 Hz tone with identical peak/RMS (peak ≈ 0.0884 of full
scale, Goertzel 440 ≈ 0.0423). 3-second F32 run: 70 callbacks, 1,146,880 bytes, no zero-size
or null buffers, no ring overflow.

### 5.2 Rate / channel negotiation (exact, resampled by PipeWire)

| requested | negotiated | block_align | observed chunk bytes |
|---|---|---|---|
| 44.1 kHz, 1 ch | 44100 / 1 | 4 | 7412–7528 |
| 44.1 kHz, 2 ch | 44100 / 2 | 8 | ~15043 avg |
| 48 kHz, 1 ch | 48000 / 1 | 4 | 8192 |
| 22.05 kHz, 2 ch | 22050 / 2 | 8 | 7296–7528 |
| 0 / 0 (native) | 48000 / 2 | 8 | 16384 |

PipeWire's adapter inserts the needed resampler/channel mixer. Chunk sizes are no longer a
fixed quantum at non-native rates, but every chunk was a multiple of `block_align` (checked
7412/4, 7528/4, 7296/8, 7528/8). MTU fragmentation must therefore use `block_align`, never a
fixed quantum.

### 5.3 CPU / memory

Release build, F32LE 48 kHz stereo, 10 s capture:
- 234 callbacks, 3,833,856 bytes, callback span 9941 ms (23.5 callbacks/s)
- user 0.01 s + sys 0.06 s = 0.07 s CPU over 10.03 s wall → ~0.7 % CPU
- max RSS 12,384 KB
- zero-size callbacks 0, null buffers 0, ring overflow 0

Debug build same test: 0.31 s CPU (~3 %), RSS 12,096 KB. Spike includes the ring consumer
draining to a file; it does not include encoding/UDP.

### 5.4 Threading

- `RT_PROCESS` on: `param_changed`/`state_changed` ran on `ThreadId(2)`; `process` ran on
  `ThreadId(3)` (PipeWire's realtime data thread). **Different threads.**
- `RT_PROCESS` off (`--no-rt`): both on `ThreadId(2)`.

### 5.5 Lifecycle / stop behavior

- 5 consecutive start/stop cycles (1 s each): all negotiated, captured, exited cleanly.
- Stop 1 ms after spawn (while `Connecting`): states `Unconnected -> Connecting ->
  Unconnected`, 0 callbacks, clean exit.
- Stop at 50 ms / 200 ms: clean `Streaming -> Paused -> Unconnected`.
- Two `Stop` messages queued: clean; the second `disconnect()` errors and is ignored.
- Sink switch (capture A F32 → capture B S16 → capture A S24): no crash, correct formats.
- Destroying the target sink mid-stream (1.5 s into a 4 s capture): callbacks stopped
  immediately (~1.54 s of audio captured) and **no stream `state_changed` event was emitted
  within the following 2.5 s**; the stream stayed `Streaming` from the client's point of view.
  The server must watch the registry (`global_remove`) / have a data-liveness watchdog and
  restart or fail the session.
- Invalid `TARGET_OBJECT` (nonexistent name): `connect` succeeds, PipeWire autoconnects to a
  fallback node, negotiation succeeds, and the stream captures data with no error (no
  playback source existed in that run, so the content was not analyzed). There is no error
  surfaced. Validate the endpoint against the registry and re-check on hotplug.
- No wireplumber (only `pipewire` daemon): enumeration works; capture reaches `Connecting ->
  Paused -> Unconnected`, never negotiates and never gets a format. A session manager is
  required for AUTOCONNECT capture linking.

### 5.6 Reusing one connection/loop (matches old C++ architecture)

`src/bin/reuse.rs` [verified]: one `MainLoopRc`/`ContextRc`/`CoreRc`/`RegistryRc` was used to
(a) enumerate sinks via a registry listener + `core.sync(0)` and `run()` until `done` called
`quit()`, then (b) create a `StreamRc` on the same core and call `run()` again for capture,
stopping via the channel. Output:

```
enumerated 1 sinks: [("34", 1000)]
reuse: format=AudioFormat::F32LE rate=48000 channels=2
reuse capture: callbacks=46 bytes=753664
```

So the old server's one-loop-per-process pattern is portable. For a permanently running
loop, do not `quit()` for a list request: either keep a registry cache up to date and answer
immediately, or send `core.sync(seq)` and reply in the `done(id, seq)` callback without
quitting the loop [design recommendation, unverified].

## 6. Null-sink / headless test harness

Important gotcha: the `pw-cli create-node` command suggested in the ticket text has malformed
SPA-JSON. `audio.position=FL,FR priority.session=1000` is parsed as `audio.position=FL`
followed by a key `FR` = `priority.session`, so the sink has **no** `priority.session` and a
malformed position. Use `audio.position=[FL,FR]` (verified: then both props appear).

Correct create command:

```bash
pw-cli create-node adapter '{ factory.name=support.null-audio-sink \
  node.name=TestSink node.description=TestSink media.class=Audio/Sink \
  object.linger=1 audio.position=[FL,FR] priority.session=1000 }'
```

Minimal reusable harness (`/home/w0n9/.claude/jobs/d82859a4/tmp/pwspike-harness.sh`):

```bash
#!/bin/bash
# Usage: dbus-run-session -- pwspike-harness.sh [--play WAV] [--priority N] [--no-sink] -- command...
set -u
PLAY=""; PRIORITY=1000; NO_SINK=0
while [ $# -gt 0 ]; do
  case "$1" in
    --play) PLAY="$2"; shift 2 ;;
    --priority) PRIORITY="$2"; shift 2 ;;
    --no-sink) NO_SINK=1; shift ;;
    --) shift; break ;;
    *) break ;;
  esac
done
BASE=${BASE:-/tmp}
RT=$(mktemp -d "$BASE/pwrt.XXXXXX"); chmod 700 "$RT"
export XDG_RUNTIME_DIR="$RT" PIPEWIRE_RUNTIME_DIR="$RT" PIPEWIRE_DEBUG="${PIPEWIRE_DEBUG:-1}"

PW_PID=0; WP_PID=0; PLAY_PID=0
cleanup() {
  [ "$PLAY_PID" != 0 ] && kill "$PLAY_PID" 2>/dev/null
  [ "$WP_PID" != 0 ] && kill "$WP_PID" 2>/dev/null
  [ "$PW_PID" != 0 ] && kill "$PW_PID" 2>/dev/null
  wait 2>/dev/null; rm -rf "$RT"
}
trap cleanup EXIT INT TERM

pipewire >"$RT/pipewire.log" 2>&1 & PW_PID=$!
for _ in $(seq 1 100); do [ -S "$RT/pipewire-0" ] && break; sleep 0.05; done
[ -S "$RT/pipewire-0" ] || { echo "no pipewire socket"; tail -50 "$RT/pipewire.log"; exit 1; }

wireplumber >"$RT/wireplumber.log" 2>&1 & WP_PID=$!
for _ in $(seq 1 100); do pw-cli info 0 >/dev/null 2>&1 && break; sleep 0.05; done

if [ "$NO_SINK" = 0 ]; then
  pw-cli create-node adapter "{ factory.name=support.null-audio-sink node.name=TestSink node.description=TestSink media.class=Audio/Sink object.linger=1 audio.position=[FL,FR] priority.session=$PRIORITY }" >/dev/null 2>&1
  found=0
  for _ in $(seq 1 100); do
    pw-cli ls Node 2>/dev/null | grep -q 'node.name = "TestSink"' && { found=1; break; }
    sleep 0.05
  done
  [ "$found" = 1 ] || { echo "TestSink did not appear"; pw-cli ls Node; exit 1; }
fi

if [ -n "$PLAY" ]; then
  pw-play --target TestSink "$PLAY" >"$RT/pw-play.log" 2>&1 & PLAY_PID=$!
  sleep 0.3   # let the playback stream link before capture starts
fi

"$@"; RC=$?
echo "harness: command exited rc=$RC"
[ -s "$RT/pw-play.log" ] && { echo "--- pw-play log ---"; cat "$RT/pw-play.log"; }
exit "$RC"
```

Usage examples:

```bash
# WAV source
ffmpeg -hide_banner -loglevel error -y -f lavfi \
  -i "sine=frequency=440:sample_rate=48000:duration=12" -ac 2 -c:a pcm_s16le tone.wav

# enumerate
dbus-run-session -- ./pwspike-harness.sh -- ./pwspike list

# capture F32LE with audio, write PCM
dbus-run-session -- ./pwspike-harness.sh --play tone.wav -- \
  timeout 20s ./pwspike capture --target TestSink --format f32le --seconds 3 --out f32.pcm

# two sinks / default selection
dbus-run-session -- ./pwspike-harness.sh --no-sink -- bash -c '
  pw-cli create-node adapter "{ factory.name=support.null-audio-sink node.name=A node.description=A media.class=Audio/Sink object.linger=1 audio.position=[FL,FR] priority.session=500 }"
  pw-cli create-node adapter "{ factory.name=support.null-audio-sink node.name=B node.description=B media.class=Audio/Sink object.linger=1 audio.position=[FL,FR] priority.session=2000 }"
  sleep 0.5; ./pwspike list'
```

Tips:
- Always wrap the harness with `timeout Ns` in automation; a missing session manager or a
  wrong target can otherwise leave the stream waiting.
- `dbus-run-session` gives wireplumber a private session bus. `XDG_RUNTIME_DIR` must be a
  fresh 0700 directory to avoid attaching to the developer's real PipeWire.
- Enumeration tests can skip wireplumber (`--no-sink` plus a manual sink, or no session
  manager at all) [verified]; capture tests need it.
- Teardown order: kill pw-play, wireplumber, pipewire, `wait`, then remove the runtime dir.
- Node serials are allocated dynamically; resolve `object.serial` at test time, don't
  hard-code it.

## 7. Format mapping details

### s8 → U8
Old server maps `encoding_s8` to `SPA_AUDIO_FORMAT_U8` (`audio_manager_impl.cpp:129-131`).
The proto name is `ENCODING_PCM_8BIT`; Android maps it to `AudioFormat.ENCODING_PCM_8BIT`
(unsigned 8-bit, `android-app/.../AudioPlayer.kt:194`). So the Rust mapping is
`Encoding::Pcm8Bit => AudioFormat::U8` (verified U8 capture; content is centered at 128).

### s24 packing: S24_LE is packed 24-bit, **not** a 32-bit container
- `libspa::param::audio::AudioFormat::S24LE` maps to `SPA_AUDIO_FORMAT_S24_LE`
  (`libspa-0.10.1/src/param/audio/mod.rs:35`), a distinct enum value from
  `S24_32LE` (`:27`).
- Empirical proof: for a 2048-frame stereo quantum, S24LE chunks were **12,288 bytes =
  2048 × 2 × 3**; S24_32LE chunks were **16,384 bytes = 2048 × 2 × 4** [verified].
- This matches the old server's `block_align = 24/8 × channels` (`audio_manager_impl.cpp:267-287`)
  and Android's `ENCODING_PCM_24BIT_PACKED` mapping (`AudioPlayer.kt:195-197`). Keep
  `bytes_per_sample(S24LE) = 3` in the new core.

### block_align
`block_align = bytes_per_sample(negotiated_format) × negotiated_channels`, computed after
`param_changed`. Verified values: U8/S8=1, S16=2, S24LE=3, S24_32LE/S32LE/F32LE=4 bytes per
sample. Do not derive it from the requested encoding; use the negotiated format (PipeWire can
substitute a closest format — though in all our tests it honored the request exactly).

### Reading back negotiated rate/channels
In `param_changed` for `ParamType::Format`, parse with
`format_utils::parse_format(param)` (must be `MediaType::Audio` + `MediaSubtype::Raw`) then
`AudioInfoRaw::parse(param)`, then `info.format()`, `info.rate()`, `info.channels()`
(`libspa-0.10.1/src/param/audio/raw.rs:36-66,82-85`). A `None` param means the format was
cleared. Publish the result atomically/shared; do not re-read mutable state from the process
callback without synchronization.

### Quantum / chunk size
The graph chooses the quantum; it is not fixed by the API. The same F32LE stereo stream
produced 16384-byte chunks (2048 frames) while the null sink was driven by playback, and
8192-byte chunks (1024 frames) in another run. Under resampling, chunks can vary within a run
(7412–7528 bytes). Treat each `process` buffer as arbitrary length divisible by `block_align`;
never assume a constant `quantum`.

## 8. Threading, RT restrictions, lifetime

- All `MainLoop`/`Context`/`Core`/`Registry`/`Stream` objects are `!Send`/`!Sync`; create and
  drop them inside the dedicated thread. Cross the thread boundary only with
  `std::sync::mpsc` and `pipewire::channel::{Sender,Receiver}`. `Sender<T>` is `Send` (and
  `Clone`); `Receiver<T>` is attached to the loop with `receiver.attach(loop, callback)` and
  must stay alive until after `run()` returns [verified compile/run].
- `pipewire::channel` is unbounded (`VecDeque`) and writes one byte to a pipe only when the
  queue was empty (`src/channel.rs:155-177`). Sending Stop is non-blocking in practice; still
  don't call it from the RT process callback.
- `RT_PROCESS` means `process` runs on PipeWire's realtime data thread [verified thread IDs].
  There it must not allocate, lock (contended), log, do file/network I/O, or block. The spike
  hands data to the network side through a lock-free SPSC ring (`ringbuf`) and uses atomics
  for counters. This is the recommended production handoff (e.g. `ringbuf`/`rtrb` + a tokio
  consumer). Do not `Mutex<Vec<u8>>` + `write()` from `process`.
- Without `RT_PROCESS`, `process` runs on the loop thread [verified]; allocations are legal
  but any slow work stalls registry/state events and the stream. Choose RT + ring for
  production, or RT-off only if the callback is a very fast copy.
- Crate API caveat: the listener passes `&mut D` to `state_changed`/`param_changed`/`process`.
  With `RT_PROCESS` those callbacks were observed on different threads. PipeWire serializes
  format changes around streaming in practice, but do not rely on `&mut D` as a
  cross-thread-exclusive resource; keep cross-callback state in `Arc<Atomic*>`/a ring, and
  treat user data as per-callback scratch.
- Stop: from the control thread send a command through `pipewire::channel`; in the loop
  thread call `stream.disconnect()` then `main_loop.quit()`. This produced clean
  `Streaming -> Paused -> Unconnected` transitions and safe repeated stops (verified,
  including two queued stops and a stop during `Connecting`).
- Drop order: attached receiver → stream listener → stream → core → context → main loop.
  Reverse-declaration drop in one scope gives this automatically; `StreamRc` also holds
  `CoreRc`/`ContextRc`/loop internally, so even out-of-order drops are kept alive
  (`src/stream/rc.rs:20-30`, `src/core/rc.rs:16-24`, `src/context/rc.rs:16-24`).
- `pw::init()` is idempotent via `OnceLock`; `pw::deinit()` is `unsafe`, must be called at
  most once at process exit with no PipeWire threads alive. Don't call it per start/stop.
- `Stream::time()` (feature `v0_3_50`) exposes `pw_time` (now/rate/ticks/delay/queued), the
  equivalent of the old C++ trace timer — useful for the 8 h soak/xrun instrumentation.
  [API present; not exercised in this spike.]
- Start/stop idempotence: per-session thread + channel stop is idempotent in practice; a
  second Stop after quit has no receiver effect, and calling `disconnect()` twice returns an
  ignored error [verified].

## 9. `pipewire-sys` fallback assessment

The fallback is viable but not necessary. `src/bin/sys_fallback.rs` compiles against
`pipewire-sys` 0.10.1 + `libspa-sys` 0.10.1 and its `list` mode was run successfully in the
headless harness [verified]. Findings:

- `pipewire-sys` bindgen does **not** wrap the C static-inline proxy helpers
  (`pw_core_get_registry`, `pw_core_sync`, `pw_registry_add_listener`, ...). You must dispatch
  through the SPA interface vtable yourself. Skeleton:

```rust
use libspa_sys as spa;
use pipewire_sys as pw;
use std::os::raw::c_void;

unsafe fn iface_parts<T>(obj: *mut T) -> (*const c_void, *mut c_void) {
    let iface = obj.cast::<spa::spa_interface>();
    ((*iface).cb.funcs, (*iface).cb.data)
}

// core proxy:
let (funcs, core_obj) = iface_parts(core);
let methods = funcs.cast::<pw::pw_core_methods>();
let registry = ((*methods).get_registry.unwrap())(core_obj, pw::PW_VERSION_REGISTRY, 0);
let seq = ((*methods).sync.unwrap())(core_obj, pw::PW_ID_CORE, 0);
// IMPORTANT: seq is not 0; in our run it was 1073741827 (0x40000003). Compare the done
// callback's seq against this returned value, exactly like the C++ roundtrip.
```

- Registry listener: zeroed `spa::spa_hook`, `pw_registry_events { version:
  PW_VERSION_REGISTRY_EVENTS, global: Some(cb), ..zeroed }`, then
  `((*registry_methods).add_listener.unwrap())(registry_obj, &mut hook, &events, data)`.
  `spa_dict_lookup` works directly on `*const spa_dict` with keys like
  `pw::PW_KEY_MEDIA_CLASS: &[u8; 12] = b"media.class\0"`.
- Capture: `pw_stream_new_simple`/`pw_stream_connect`/`pw_stream_dequeue_buffer` are real
  symbols, but you must call `pw_stream_queue_buffer` manually (no Drop), manage the
  `pw_stream_events` function pointers, and build the pod with
  `spa_pod_builder_init` + `spa_format_audio_raw_build` from `libspa-sys` (both are exported
  by its `wrap_static_fns` build).
- Build still needs `libpipewire-0.3-dev`, pkg-config, and libclang (bindgen).
- Cost vs safe crate: much more `unsafe`, manual lifetime/hook management, easy to miss
  buffer requeueing. Use only if the safe crate hits a wall; it did not.

## 10. Unverified / open items

- Only the `support.null-audio-sink` virtual sink was available; real ALSA/USB sink monitor
  ports, device names/nick formats, and `priority.session` values on real hardware were not
  tested.
- Hot-unplug was observed for only ~2.5 s; whether PipeWire eventually emits an `Error` or
  another state after a longer interval is unknown. Registry-based detection is still needed
  regardless.
- Long-run stability (8 h soak), suspend/resume, and repeated hotplug are not covered by this
  spike.
- Non-F32/resampled captures were short (1–2 s); long-run drift/xrun behavior is unmeasured.
- The Android client was not exercised; format/byte compatibility is argued from the old C++
  mapping and Android's `ENCODING_PCM_24BIT_PACKED`/`ENCODING_PCM_8BIT` mappings.
- Chunk `offset != 0` was not observed (the code handles it).
- The `priority.session <= -1` "no default" parity decision is not implemented in the spike's
  `list` helper (see §3).
- Multi-client broadcast, encoding, MTU fragmentation, and 1 s buffering are outside this
  spike.
- Wire Protocol parity harness numbers are from the old `as-cmd` workflow, not this spike.

## 11. Recommendation

**Go.** Adopt `pipewire = "0.10.1"` with feature `v1_0_0` for the Linux core:
- Enumerate and select exactly as the old server does (object.serial + priority.session;
  decide the `-1` parity quirk explicitly).
- One dedicated PipeWire thread per capture session (verified clean start/stop), or one
  long-lived loop with command channel (loop reuse verified); hand PCM to tokio through a
  lock-free ring.
- Map Encoding → `AudioFormat`: F32LE (default), U8 (s8), S16LE, S24LE (packed 3-byte),
  S32LE. Derive block_align from the negotiated format × channels.
- Add registry `global_remove`/watchdog handling because stream state does not report target
  loss promptly, and validate targets because invalid targets silently fall back.
- Keep `pipewire-sys` as a documented fallback only; the safe crate meets ticket 03.

## 12. Reference index

Crate sources (root `~/.cargo/registry/src/mirrors.ustc.edu.cn-38d0e5eb5da2abae/`):
- `pipewire-0.10.1/examples/audio-capture.rs:29-192` — main-loop/context/stream/capture example
- `pipewire-0.10.1/examples/roundtrip.rs:14-53` — `core.sync(0)` + registry roundtrip
- `pipewire-0.10.1/src/lib.rs:123-132,180-193` — threading contract, init/deinit
- `pipewire-0.10.1/src/main_loop/mod.rs:35-53`; `src/main_loop/rc.rs:35`
- `pipewire-0.10.1/src/context/rc.rs:48,80`; `src/core/mod.rs:32,63,83,305`; `src/core/rc.rs:56`
- `pipewire-0.10.1/src/registry/mod.rs:67,194,219,289`
- `pipewire-0.10.1/src/stream/mod.rs:151-271,325-353,686,773,860,895,947`; `src/stream/rc.rs:39`
- `pipewire-0.10.1/src/buffer.rs:28,79-83`
- `pipewire-0.10.1/src/channel.rs:87,143,155-177,198`
- `pipewire-0.10.1/src/keys.rs:66,120,183,186,189,451,463,532`
- `libspa-0.10.1/src/param/audio/mod.rs:22-47,86`; `src/param/audio/raw.rs:23-104`
- `libspa-0.10.1/src/buffer/mod.rs:84,98,143,151`
- `libspa-0.10.1/src/pod/serialize.rs:268`; `src/utils/dict.rs:90`
- `pipewire-sys-0.10.1/build.rs` (bindgen); `libspa-sys-0.10.1/Cargo.toml` (`wrap_static_fns`)

Repo:
- `server-core/src/linux/audio_manager_impl.cpp:100-443` — authoritative old Linux behavior
- `server-core/src/main.cpp:54-62` — `-l` prints `*` for `get_default_endpoint()`
- `android-app/.../AudioPlayer.kt:190-200` — Android encoding mapping (8-bit unsigned,
  24-bit packed)
- `protos/client.proto:23-34` — frozen encoding enum / AudioFormat
- `docs/research/server-migration-rust-vs-go.md:36,187,385` — prior crate assessment
