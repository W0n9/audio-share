# WASAPI loopback for the Rust server rewrite — verified API notes

Date: 2026-11-09
Scope: tickets `02-wasapi-loopback-spike` and `05-windows-real-capture`.
Old implementation: `/home/w0n9/workspaces/audio-share/server-core/src/win32/audio_manager_impl.cpp` (authoritative behavior).
Environment: Ubuntu 24.04, Rust 1.98.1, cargo mirror `bfsu` (sparse USTC index). Windows runtime unavailable; Windows **typechecking is available** (std manually installed in the sysroot, see §1.4).

Source abbreviations used below:

- `WASAPI_SRC` = `/home/w0n9/.cargo/registry/src/mirrors.ustc.edu.cn-38d0e5eb5da2abae/wasapi-0.27.0`
- `WINDOWS_SRC` = `/home/w0n9/.cargo/registry/src/mirrors.ustc.edu.cn-38d0e5eb5da2abae/windows-0.62.2`
- `WINDOWS_CORE_SRC` = `/home/w0n9/.cargo/registry/src/mirrors.ustc.edu.cn-38d0e5eb5da2abae/windows-core-0.62.2`
- `CORE_CPP` = `/home/w0n9/workspaces/audio-share/server-core/src/win32/audio_manager_impl.cpp`

Line references are exact for the versions above. The two code sketches in §2.9 and §5.3 were compiled with
`cargo check --target x86_64-pc-windows-msvc` in
`/home/w0n9/.claude/jobs/d82859a4/tmp/wasapi-probe` (full files under `src/bin/`).
They are API-verified but not runtime-tested.

---

## 0. Recommendation

**GO with `wasapi = "0.27.0"`** (optionally pin `=0.27.0`) and `windows = "0.62.2"` as a direct dependency.

- Polling shared-mode loopback (`StreamMode::PollingShared { autoconvert: false, buffer_duration_hns: 10_000_000 }`), matching the old 1-second-buffer poll loop.
- Do **not** use event-driven mode by default: loopback events only work on Windows 10 1703+ (`learn.microsoft.com/.../loopback-recording`), and the old implementation polls. Keep events as a spike comparison only.
- The crate covers every required operation with typed errors; `is_supported` correctly distinguishes S_OK / S_FALSE / `AUDCLNT_E_UNSUPPORTED_FORMAT`; `read_from_device` handles `GetBuffer`/`ReleaseBuffer` and zero-fills silent packets.
- Add `windows = "0.62.2"` directly (it is also `wasapi`'s pinned `windows` version) for `AUDCLNT_E_*` constants and for the rest of the Windows server (firewall, single instance, process metrics). **Never bump `windows` to a different minor** while `wasapi` is in use, or the re-exported `IAudioClient`/`IMMDevice` types stop matching.
- Keep the `windows-rs` skeleton (§5) behind the same internal capture module as a fallback; it compiles, but needs roughly 2–3x the unsafe/boilerplate code.
- Recreate the `DeviceEnumerator`/`IMMDevice`/`IAudioClient` for every start (like the old code). If a client is reused, `Stop` freezes the position and `Start` resumes it; call `Reset` first to drop up to 1 s of stale buffered audio.
- Validate that the selected endpoint is `Direction::Render` before initializing. `wasapi` silently creates a **normal microphone capture** if a capture endpoint ID is passed with `Direction::Capture`; the old code fails with `AUDCLNT_E_WRONG_ENDPOINT_TYPE`.

No-Go triggers for `wasapi` (fall back to `windows-rs`): the spike proves that the crate cannot express the required loopback/format combination, a reproducible bug is found in `is_supported`/`read_from_device`, or the crate becomes unmaintained before cutover. None of these is currently known.

---

## 1. Crate availability and exact versions

### 1.1 Cargo mirror

`~/.cargo/config.toml`:

```toml
[source.crates-io]
replace-with = 'bfsu'

[source.bfsu]
registry = "sparse+https://mirrors.ustc.edu.cn/crates.io-index/"
```

Target-gate the dependency so Linux builds never try to build it:

```toml
[target.'cfg(windows)'.dependencies]
wasapi = "0.27.0"
windows = { version = "0.62.2", features = [ /* see §5.1 */ ] }
```

Probe project: `/home/w0n9/.claude/jobs/d82859a4/tmp/wasapi-probe` (edition 2024, no VCS).

### 1.2 `wasapi` versions

`cargo add wasapi --dry-run` (run 2026-11-09):

```
Updating `bfsu` index
    Adding wasapi v0.27.0 to dependencies
           Features:
           - device-tests
```

Sparse index (`https://mirrors.ustc.edu.cn/crates.io-index/wa/sa/wasapi`) versions: `0.1.0, 0.1.1, 0.1.2, 0.2.0, 0.10.0, 0.12.0 … 0.27.0`. Latest is **0.27.0**; rust-version `1.85`, edition 2024, only feature `device-tests` (`WASAPI_SRC/Cargo.toml:13-14, 43-44`). VCS sha1 `13cbc8042b23ddbf21e468ecd7f873e511675ab0`, repository `https://github.com/HEnquist/wasapi-rs`.

`Cargo.lock` after `cargo add wasapi@0.27.0 && cargo fetch`:

```
name = "wasapi"
version = "0.27.0"
checksum = "58b35dbcb6fae6fbaba9f6f52a22686bb405978ee112ed6eb33d98bd63c97583"
dependencies = ["log", "num-integer", "thiserror", "windows", "windows-core"]
```

`wasapi` pins `windows = "0.62.2"` and `windows-core = "0.62.2"` with features `Foundation`, `Win32_Media_Audio`, `Win32_Media_Audio_Endpoints`, `Win32_Foundation`, `Win32_Devices_FunctionDiscovery`, `Win32_Devices_Properties`, `Win32_UI_Shell_PropertiesSystem`, `Win32_System_Com`, `Win32_System_Com_StructuredStorage`, `Win32_Media_KernelStreaming`, `Win32_Media_Multimedia`, `Win32_System_Threading`, `Win32_System_Variant`, `Win32_Security`, `Win32_Storage_FileSystem`, `Win32_System_IO` (`WASAPI_SRC/Cargo.toml:128-147`).

### 1.3 `windows` fallback versions

Sparse index latest: **0.62.2** (previous 0.62.1, 0.62.0, 0.61.3 …). `cargo add windows --dry-run` also resolves 0.62.2, the same version `wasapi` uses. This is required for type identity of the re-exported COM interfaces (`WASAPI_SRC/src/lib.rs:15,21`).

### 1.4 Windows typechecking in this environment

The environment note is correct: `rustup target list --installed` only prints `x86_64-unknown-linux-gnu`, but the Windows std is present in the sysroot:

```
/home/w0n9/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/x86_64-pc-windows-msvc/lib/*.rlib
```

So use:

```bash
cargo check --target x86_64-pc-windows-msvc
cargo clippy --target x86_64-pc-windows-msvc
```

Do **not** reinstall/remove the target: `rustup target add x86_64-pc-windows-msvc` and `...-gnu` both fail with HTTP 404 against the configured Tsinghua rustup mirror (verified). Linker is absent, so `cargo build`/`test` for Windows will fail at link time; `check`/`clippy` are the supported validation.

`wasapi 0.27.0` and both probe binaries in this note compile for `x86_64-pc-windows-msvc`. The `windows` crate does **not** compile when built for the Linux host target (`windows-future`/`windows-threading` errors), so all Windows code must be under `#[cfg(windows)]` and target-gated dependencies.

---

## 2. Verified `wasapi` 0.27.0 API

### 2.1 COM initialization

```rust
pub fn initialize_mta() -> HRESULT;                    // api.rs:73-75
pub fn initialize_sta() -> HRESULT;                    // api.rs:78-80
pub fn deinitialize();                                 // api.rs:83-85
```

`HRESULT` is `windows_core::HRESULT`; `is_err()` is `self.0 < 0`. `S_FALSE` (already initialized) is success but still requires one matching `CoUninitialize` per successful call. Use a guard whose `Drop` calls `deinitialize()` only when the `CoInitializeEx` result was a success code.

### 2.2 Enumerating render endpoints and default endpoint

```rust
DeviceEnumerator::new() -> WasapiRes<DeviceEnumerator>                 // api.rs:369-373
DeviceEnumerator::get_device_collection(Direction) -> WasapiRes<DeviceCollection> // api.rs:376-386
DeviceEnumerator::get_default_device(Direction) -> WasapiRes<Device>   // api.rs:389-391 (Role::Console)
DeviceEnumerator::get_default_device_for_role(Direction, Role) -> WasapiRes<Device> // api.rs:394-407
DeviceEnumerator::get_device(&str) -> WasapiRes<Device>                // api.rs:410-416
DeviceCollection::get_nbr_devices() -> WasapiRes<u32>                  // api.rs:475-478
DeviceCollection::get_device_at_index(u32) -> WasapiRes<Device>        // api.rs:481-487
Device::get_id() -> WasapiRes<String>                                  // api.rs:696-701
Device::get_friendlyname() -> WasapiRes<String>                        // api.rs:629-632
Device::get_description() -> WasapiRes<String>                         // api.rs:639-641
Device::get_device_format() -> WasapiRes<WaveFormat>                   // api.rs:645-651 (PKEY_AudioEngine_DeviceFormat)
Device::get_direction() -> Direction                                   // api.rs:703-706
Device::get_iaudioclient() -> WasapiRes<AudioClient>                   // api.rs:573-582
```

`get_device_collection(Direction::Render)` uses `EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)` (`api.rs:376-386`), exactly like `CORE_CPP:292-330`. `get_default_device(Direction::Render)` uses role `Console` (`api.rs:389-391`), matching `CORE_CPP:332-350`. `get_id()` returns the same IMMDevice ID string in UTF-8; IDs are ASCII GUID-like strings, so this is config-compatible with the old ANSI conversion.

### 2.3 Activate, mix format, format probing

```rust
AudioClient::get_mixformat() -> WasapiRes<WaveFormat>                        // api.rs:976-997
AudioClient::is_supported(&WaveFormat, ShareMode) -> WasapiRes<Option<WaveFormat>> // api.rs:1018-1079
AudioClient::get_device_period() -> WasapiRes<(i64 /*default*/, i64 /*min*/)> // api.rs:1142-1151
WaveFormat::get_blockalign() -> u32          // waveformat.rs:320-322
WaveFormat::get_bitspersample() -> u16       // waveformat.rs:330-332  (wBitsPerSample = container bits)
WaveFormat::get_validbitspersample() -> u16  // waveformat.rs:335-337  (wValidBitsPerSample)
WaveFormat::get_samplespersec() -> u32       // waveformat.rs:340-342
WaveFormat::get_nchannels() -> u16           // waveformat.rs:345-347
WaveFormat::get_dwchannelmask() -> u32       // waveformat.rs:353-355
WaveFormat::get_subformat() -> WasapiRes<SampleType> // waveformat.rs:358-365
```

`is_supported(Shared)` behavior (verified against the Microsoft docs cited in §9 and the crate source):

| OS result | Crate result |
|---|---|
| `S_OK`, `*ppClosestMatch = NULL` | `Ok(None)` — request accepted as-is |
| `S_FALSE`, `*ppClosestMatch != NULL` | `Ok(Some(closest))` — use the returned format |
| `AUDCLNT_E_UNSUPPORTED_FORMAT` | `Err(WasapiError::Windows(..))` |
| device invalidated / service stopped | `Err(WasapiError::Windows(..))` |

The crate allocates the closest-match structure with `CoTaskMemAlloc`, copies it into `WaveFormat`, and frees it (`api.rs:1047-1075`). The caller does not free it. **Unlike a raw `windows-rs` call, no HRESULT inspection is needed.** Note that `IsFormatSupported` returns raw `HRESULT` in `windows-rs`; `HRESULT::ok()` treats `S_FALSE` as success, so a fallback must inspect the out pointer, not the `Result`.

`get_mixformat` expects `GetMixFormat`'s `WAVEFORMATEX` with `cbSize == 22 && wFormatTag == WAVE_FORMAT_EXTENSIBLE` to be a full `WAVEFORMATEXTENSIBLE`; otherwise it converts a plain `WAVEFORMATEX` via `WaveFormat::from_waveformatex` (`api.rs:980-996`, `waveformat.rs:243-263`). `GetMixFormat` always returns extensible according to Microsoft, but the crate handles both.

### 2.4 Initializing shared-mode loopback

```rust
AudioClient::initialize_client(
    &mut self,
    wavefmt: &WaveFormat,
    direction: Direction,
    stream_mode: StreamMode,
    session_guid: Option<GUID>,
) -> WasapiRes<()>;                                   // api.rs:1268-1351
```

`StreamMode` variants (`api.rs:200-227`):

```rust
PollingShared { autoconvert: bool, buffer_duration_hns: i64 }
PollingExclusive { buffer_duration_hns: i64, period_hns: i64 }
EventsShared { autoconvert: bool, buffer_duration_hns: i64 }
EventsExclusive { period_hns: i64 }
```

Required loopback combination:

1. Endpoint must be a **render** endpoint; `Device::get_iaudioclient()` records `direction = Direction::Render`.
2. Call `initialize_client(&fmt, Direction::Capture, StreamMode::PollingShared { .. } /* or EventsShared */, None)`.
3. The crate sets `AUDCLNT_STREAMFLAGS_LOOPBACK` only for `(self.direction, direction, sharemode) == (Render, Capture, Shared)` (`api.rs:1289-1300`). No public flag parameter exists; `StreamMode` decides shared/exclusive and event/polling.
4. `(Render, Capture, Exclusive)` returns `WasapiError::LoopbackWithExclusiveMode` (`api.rs:1293-1295`, `errors.rs:29-30`).
5. `(Capture, Render, _)` returns `WasapiError::RenderToCaptureDevice` (`api.rs:1296-1298`).
6. `(Capture, Capture, Shared)` is a **normal capture stream** (flags 0). Validate `client.get_direction() == Direction::Render` before initializing to avoid silently recording the microphone.
7. `initialize_shared_audio_stream` (IAudioClient3) explicitly does **not** support loopback or autoconvert (`api.rs:1615-1621, 1630-1641`). Do not use it.

For `autoconvert: true`, the crate ORs `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY` (`api.rs:1301-1313`). For old-behavior parity, use `autoconvert: false`; enabling it lets the engine resample/rechannel instead of returning a closest match.

Shared mode always passes `period = 0` (`api.rs:1321-1332`), as required by Microsoft (`hnsPeriodicity` may be nonzero only in exclusive mode). `buffer_duration_hns = 10_000_000` is the 1 s buffer. The `AudioClient` records the direction/share mode/timing mode/frame size after a successful call (`api.rs:1346-1349`); a second `Initialize` on the same client returns `AUDCLNT_E_ALREADY_INITIALIZED`, so create a fresh client per start.

Event mode additionally requires `set_get_eventhandle()` after `Initialize` and before `Start`; otherwise the audio engine may block for seconds on release (Microsoft docs). The returned `Handle` is `Send + Sync` (`api.rs:2405-2447`), but the `AudioClient` is not.

### 2.5 Capture polling

```rust
AudioClient::get_buffer_size() -> WasapiRes<u32>              // api.rs:1363-1367 (frames)
AudioClient::get_audiocaptureclient() -> WasapiRes<AudioCaptureClient> // api.rs:1433-1440
AudioClient::start_stream() -> WasapiRes<()>                  // api.rs:1406-1409
AudioClient::stop_stream() -> WasapiRes<()>                   // api.rs:1412-1415
AudioClient::reset_stream() -> WasapiRes<()>                  // api.rs:1418-1421

AudioCaptureClient::get_next_packet_size() -> WasapiRes<Option<u32>> // api.rs:2295-2301
AudioCaptureClient::read_from_device(&mut [u8]) -> WasapiRes<(u32 /*frames*/, BufferInfo)> // api.rs:2308-2351
AudioCaptureClient::read_from_device_to_deque(&mut VecDeque<u8>) -> WasapiRes<BufferInfo> // api.rs:2356-2394

pub struct BufferInfo { pub flags: BufferFlags, pub index: u64, pub timestamp: u64 } // api.rs:2207-2235
pub struct BufferFlags {
    pub data_discontinuity: bool,
    pub silent: bool,
    pub timestamp_error: bool,
} // api.rs:2239-2281
```

- `get_next_packet_size()` returns `Ok(None)` only for exclusive mode; in shared mode it returns `Some(frames)`, `0` means "no packet ready" (`api.rs:2291-2301`).
- `read_from_device(&mut buf)` performs `GetBuffer`, copies `frames * block_align` bytes, calls `ReleaseBuffer`, and returns `(frames, BufferInfo)` (`api.rs:2308-2351`).
- If `AUDCLNT_BUFFERFLAGS_SILENT` is set, the crate fills the returned region with zeros; the packet must otherwise be ignored per WASAPI (`api.rs:2340-2345`). This is better than the old code, which broadcasts the raw pointer without checking flags (`CORE_CPP:268-278`).
- `data_discontinuity` means frames were lost before this packet; send the data but count it.
- `timestamp_error` means `BufferInfo.timestamp` is unreliable; data is still valid.
- `BufferInfo.index` is the device position of the first frame. A jump greater than `previous.index + previous.frames` means frames were lost before reading. `index` can reset after a stream restart; track deltas.
- Size the destination slice for the largest possible packet. A packet never exceeds `GetBufferSize()` frames, so `buffer = vec![0u8; buffer_frames * block_align]` is enough. `read_from_device` releases the packet and returns `DataLengthTooShort` if the slice is smaller, which loses that packet.
- Drain in a loop until `get_next_packet_size() == Some(0)`: one engine period may produce several packets.

### 2.6 Stop and reset semantics

- `Stop` freezes the audio clock at the current position; a following `Start` resumes from that position. `Stop` returns `S_FALSE` if already stopped; the crate maps that to `Ok(())` because `HRESULT::ok()` treats any non-negative code as success.
- `Reset` requires a stopped stream; it flushes pending data and resets the clock to 0. Use `Stop` → `Reset` before restarting a reused client, otherwise up to 1 s of pre-stop audio can be delivered again on the next start.
- Recommended: do what the old code does — tear down the whole client and recreate it per session. This also avoids `E_ALREADY_INITIALIZED` after a failed `Initialize` (Microsoft warns the client may not be re-initializable after a failure).
- `stop_stream()` can itself fail with `AUDCLNT_E_DEVICE_INVALIDATED`/`AUDCLNT_E_SERVICE_NOT_RUNNING` during teardown; ignore those on the stop path and always report the original capture error.

### 2.7 `WaveFormat::new` and 24-bit packing

```rust
WaveFormat::new(
    storebits: usize,   // wBitsPerSample (container size)
    validbits: usize,   // wValidBitsPerSample
    sample_type: SampleType, // Float | Int
    samplerate: usize,
    channels: usize,
    channel_mask: Option<u32>,
) -> WaveFormat;        // waveformat.rs:193-240
```

It builds a `WAVEFORMATEXTENSIBLE` with `cbSize = 22`, `wFormatTag = WAVE_FORMAT_EXTENSIBLE`, `SubFormat = KSDATAFORMAT_SUBTYPE_IEEE_FLOAT` for `Float` or `..._PCM` for `Int`, `nBlockAlign = channels * storebits / 8`, `nAvgBytesPerSec = rate * blockalign`, and a default channel mask (`(1 << channels) - 1` for <= 18 channels) when `None` is passed (`waveformat.rs:201-232`).

Important consequences:

- `WaveFormat::new(24, 24, Int, rate, ch, _)` = **packed 24-bit**, 3 bytes per sample, `nBlockAlign = 3 * ch`.
- `WaveFormat::new(32, 24, Int, rate, ch, _)` = 24 valid bits in a 32-bit container, 4 bytes per sample. The old code never builds this, but the OS closest match could.
- `WaveFormat::to_waveformatex()` rejects both 24-bit layouts as ambiguous (`waveformat.rs:279-312`, tests at `waveformat.rs:511-519`). Do not use it for 24-bit formats.

### 2.8 Complete `wasapi` loopback sketch (compile-checked)

Full file: `/home/w0n9/.claude/jobs/d82859a4/tmp/wasapi-probe/src/bin/wasapi_loopback.rs`.
`cargo check --target x86_64-pc-windows-msvc --bin wasapi_loopback` → `Finished` (no errors).

```rust
//! Compile-checked minimal WASAPI shared-mode loopback capture sketch.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use wasapi::{
    deinitialize, initialize_mta, BufferInfo, DeviceEnumerator, Direction, SampleType, ShareMode,
    StreamMode, WaveFormat, WasapiError,
};
use windows::Win32::Media::Audio::{
    AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_RESOURCES_INVALIDATED, AUDCLNT_E_SERVICE_NOT_RUNNING,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding { Invalid = 0, PcmFloat = 1, Pcm8 = 2, Pcm16 = 3, Pcm24 = 4, Pcm32 = 5 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodingPreference { Default, F32, S8, S16, S24, S32 }

#[derive(Clone, Debug)]
pub struct Endpoint { pub id: String, pub name: String }

#[derive(Clone, Copy, Debug)]
pub struct AudioFormat { pub encoding: Encoding, pub channels: u16, pub sample_rate: u32, pub block_align: u32 }

struct ComGuard;
impl Drop for ComGuard { fn drop(&mut self) { deinitialize(); } }

pub fn encoding_of(fmt: &WaveFormat) -> Encoding {
    match fmt.get_subformat() {
        Ok(SampleType::Float) => Encoding::PcmFloat,
        Ok(SampleType::Int) => match fmt.get_bitspersample() {
            8 => Encoding::Pcm8, 16 => Encoding::Pcm16,
            24 => Encoding::Pcm24, 32 => Encoding::Pcm32,
            _ => Encoding::Invalid,
        },
        Err(_) => Encoding::Invalid,
    }
}

pub fn audio_format_of(fmt: &WaveFormat) -> AudioFormat {
    AudioFormat {
        encoding: encoding_of(fmt),
        channels: fmt.get_nchannels(),
        sample_rate: fmt.get_samplespersec(),
        block_align: fmt.get_blockalign(),
    }
}

pub fn requested_format(
    mix: &WaveFormat, pref: EncodingPreference, channels: u16, sample_rate: u32,
) -> Result<WaveFormat, WasapiError> {
    let (storebits, validbits, sample_type) = match pref {
        // Default keeps the endpoint format exactly; old code only mutates
        // the encoding when it is not "default" (CORE_CPP:152-183).
        EncodingPreference::Default => (
            mix.get_bitspersample() as usize,
            mix.get_validbitspersample() as usize,
            mix.get_subformat()?,
        ),
        EncodingPreference::F32 => (32, 32, SampleType::Float),
        EncodingPreference::S8  => (8, 8, SampleType::Int),
        EncodingPreference::S16 => (16, 16, SampleType::Int),
        EncodingPreference::S24 => (24, 24, SampleType::Int), // packed 3 bytes
        EncodingPreference::S32 => (32, 32, SampleType::Int),
    };
    let ch = if channels == 0 { mix.get_nchannels() as usize } else { channels as usize };
    let rate = if sample_rate == 0 { mix.get_samplespersec() as usize } else { sample_rate as usize };
    let mask = if ch == mix.get_nchannels() as usize {
        Some(mix.get_dwchannelmask())
    } else {
        // Old code keeps the mix mask when nChannels changes; that is
        // inconsistent, so pass None and let IsFormatSupported fall back.
        None
    };
    Ok(WaveFormat::new(storebits, validbits, sample_type, rate, ch, mask))
}

pub fn enumerate_render_endpoints() -> Result<Vec<Endpoint>, WasapiError> {
    let enumerator = DeviceEnumerator::new()?;
    let collection = enumerator.get_device_collection(Direction::Render)?;
    let mut out = Vec::with_capacity(collection.get_nbr_devices()? as usize);
    for i in 0..collection.get_nbr_devices()? {
        let device = collection.get_device_at_index(i)?;
        out.push(Endpoint { id: device.get_id()?, name: device.get_friendlyname()? });
    }
    Ok(out)
}

pub fn default_render_endpoint() -> Result<Endpoint, WasapiError> {
    let enumerator = DeviceEnumerator::new()?;
    let device = enumerator.get_default_device(Direction::Render)?;
    Ok(Endpoint { id: device.get_id()?, name: device.get_friendlyname()? })
}

#[derive(Default)]
pub struct Stats {
    pub packets: u64, pub frames: u64, pub silent_packets: u64,
    pub discontinuities: u64, pub timestamp_errors: u64,
    pub max_packet_frames: u32, pub max_packet_interval: Duration,
    pub expected_index: u64, pub index_gaps: u64, pub seen: bool,
}

impl Stats {
    fn on_packet(&mut self, frames: u32, info: &BufferInfo, interval: Duration) {
        self.packets += 1;
        self.frames += u64::from(frames);
        self.max_packet_frames = self.max_packet_frames.max(frames);
        self.max_packet_interval = self.max_packet_interval.max(interval);
        if info.flags.silent { self.silent_packets += 1; }
        if info.flags.data_discontinuity { self.discontinuities += 1; }
        if info.flags.timestamp_error { self.timestamp_errors += 1; }
        if self.seen && info.index > self.expected_index {
            self.index_gaps += info.index - self.expected_index;
        }
        self.expected_index = info.index + u64::from(frames);
        self.seen = true;
    }
}

fn is_invalidated(err: &WasapiError) -> bool {
    matches!(err, WasapiError::Windows(e)
        if e.code() == AUDCLNT_E_DEVICE_INVALIDATED
            || e.code() == AUDCLNT_E_RESOURCES_INVALIDATED
            || e.code() == AUDCLNT_E_SERVICE_NOT_RUNNING)
}

/// Runs one capture session on the calling thread. All COM objects are created,
/// used and dropped on this thread. `stop` is the only cross-thread input.
pub fn run_capture(
    endpoint_id: &str,
    pref: EncodingPreference,
    channels: u16,
    sample_rate: u32,
    stop: Arc<AtomicBool>,
    mut sink: impl FnMut(&[u8], u32, &AudioFormat),
) -> Result<(AudioFormat, Stats), WasapiError> {
    let hr = initialize_mta();
    if hr.is_err() { return Err(WasapiError::Windows(hr.into())); }
    let _com = ComGuard;

    let enumerator = DeviceEnumerator::new()?;
    let device = if endpoint_id.is_empty() || endpoint_id == "default" {
        enumerator.get_default_device(Direction::Render)?
    } else {
        enumerator.get_device(endpoint_id)?
    };
    // Do not skip this check: a capture endpoint would become a normal mic
    // capture instead of an error (see §2.4). Production returns a typed
    // InvalidEndpoint error here.
    debug_assert_eq!(device.get_direction(), Direction::Render);
    let mut client = device.get_iaudioclient()?;
    let mix = client.get_mixformat()?;
    let requested = requested_format(&mix, pref, channels, sample_rate)?;

    let negotiated = match client.is_supported(&requested, ShareMode::Shared)? {
        None => requested,
        Some(closest) => closest,
    };
    let audio_format = audio_format_of(&negotiated);

    let (_default_period, min_period) = client.get_device_period()?;

    client.initialize_client(
        &negotiated,
        Direction::Capture, // Render device + Capture + Shared => LOOPBACK
        StreamMode::PollingShared { autoconvert: false, buffer_duration_hns: 10_000_000 },
        None,
    )?;

    let buffer_frames = client.get_buffer_size()?;
    let capture = client.get_audiocaptureclient()?;
    let block_align = negotiated.get_blockalign() as usize;
    let mut buffer = vec![0u8; buffer_frames as usize * block_align];
    client.start_stream()?;

    // Old code polls at the minimum device period (CORE_CPP:218-220, 239-255).
    let poll = Duration::from_nanos((min_period.max(1) as u64) * 100);
    let mut stats = Stats::default();
    let mut last = Instant::now();

    let loop_result: Result<(), WasapiError> = (|| {
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(poll);
            loop {
                match capture.get_next_packet_size()? {
                    None | Some(0) => break,
                    Some(frames) => {
                        let want = frames as usize * block_align;
                        if want > buffer.len() { buffer.resize(want, 0); }
                        // Releases the packet and zero-fills silent packets.
                        let (got, info) = capture.read_from_device(&mut buffer[..want])?;
                        let interval = last.elapsed();
                        last = Instant::now();
                        stats.on_packet(got, &info, interval);
                        sink(&buffer[..got as usize * block_align], block_align as u32, &audio_format);
                    }
                }
            }
        }
        Ok(())
    })();

    let stop_result = client.stop_stream();
    match loop_result {
        Err(err) => Err(err),
        Ok(()) => stop_result.map(|()| (audio_format, stats)),
    }
}
```

Production differences from this sketch:

- Use a custom error enum (`UnsupportedFormat`, `InvalidEndpoint`, `DeviceInvalidated`, `Wasapi(WasapiError)`) instead of stuffing `hr.into()` into `WasapiError`.
- Return `Result` from the endpoint-direction check rather than an empty `if`.
- Measure per-wake batch intervals, not just per-packet intervals (see §7).
- Send frames through the bounded `FrameSink`; do not block indefinitely (see §6.6).

---

## 3. Loopback specifics

### 3.1 Required mode/flag combination

| Item | Requirement | Evidence |
|---|---|---|
| Endpoint direction | render endpoint (`IMMDevice` from `eRender`) | Microsoft Loopback Recording; `api.rs:573-582` |
| Share mode | shared only; exclusive returns `LoopbackWithExclusiveMode` | `api.rs:1289-1300`; MS docs |
| Stream direction passed to `Initialize` | `Direction::Capture` | `api.rs:1289-1300` |
| Flag | `AUDCLNT_STREAMFLAGS_LOOPBACK` (set internally by the crate) | `api.rs:1290-1292` |
| Buffer | 1 s: `buffer_duration_hns = 10_000_000` | old `CORE_CPP:222-224`; MS docs |
| Periodicity | 0 in shared mode (crate enforces) | `api.rs:1321-1332`; MS docs |
| `IAudioClient3::InitializeSharedAudioStream` | not usable for loopback | `api.rs:1615-1621` |

There is no public loopback flag in the crate; `StreamMode` is the whole control surface. Passing a capture endpoint ID with `Direction::Capture` creates a normal capture stream silently, which is a dangerous difference from the old C++ (`AUDCLNT_E_WRONG_ENDPOINT_TYPE`). Guard with `device.get_direction()`.

### 3.2 Does event-driven mode work?

- Microsoft: event-driven loopback is supported on **Windows 10 1703 and later**. Before 1703, `Initialize` and `SetEventHandle` succeed but the event never fires (`learn.microsoft.com/.../loopback-recording`, `.../iaudioclient-initialize` remarks).
- The crate sets `AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK` for `StreamMode::EventsShared` (`api.rs:1314-1316`) and does not guard by Windows version.
- The crate's device tests exercise loopback only with `StreamMode::PollingShared` (`WASAPI_SRC/tests/device/errors.rs:261-299`), not events.
- Target is "Windows 10+"; Windows 10 1607 LTSC still exists, where event mode would silently deliver nothing. **Use polling.** If the spike wants event mode, test it explicitly and keep polling as the fallback.

### 3.3 Is a 1-second buffer allowed in shared mode?

Yes. Shared mode treats `hnsBufferDuration` as a minimum and has no 5000 ms/2 s cap (those limits are exclusive-mode `AUDCLNT_E_BUFFER_SIZE_ERROR`); the engine allocates at least the requested size, rounded up to whole frames (`IAudioClient::Initialize` docs, Remarks). The old server already uses 1 s (`CORE_CPP:215-224`), which is the strongest evidence. `GetBufferSize()` after `Initialize` returns the actual frame count (typically `sample_rate` frames); log it (§7).

### 3.4 Unsupported-format behavior

- `is_supported(Shared)` returns `Err` for `AUDCLNT_E_UNSUPPORTED_FORMAT`; the old code logs "not supported" and does not start capture (`CORE_CPP:196-211`). New core must return a readable error, not exit the process.
- `is_supported(Shared)` returns `Ok(Some(closest))` for `S_FALSE`; the caller must use the closest format for `Initialize` and for the advertised `AudioFormat`. The old code does exactly this (`CORE_CPP:202-205`, `set_format` at `CORE_CPP:213, 352-381`).
- `AUDCLNT_E_DEVICE_INVALIDATED`, `AUDCLNT_E_RESOURCES_INVALIDATED`, `AUDCLNT_E_SERVICE_NOT_RUNNING` can also come out of `IsFormatSupported`; classify them as restartable device errors.

### 3.5 Requesting each encoding / channel count / sample rate, with fallback

Build a copy of the mix format with the requested fields (crate `WaveFormat::new`, or clone-and-mutate `wave_fmt` like `CORE_CPP:144-191`):

| Preference | storebits / validbits / SampleType |
|---|---|
| Default | keep `mix.get_bitspersample()`, `mix.get_validbitspersample()`, `mix.get_subformat()?` |
| F32 | 32 / 32 / Float |
| S8 | 8 / 8 / Int (WAVE_FORMAT_PCM 8-bit is unsigned) |
| S16 | 16 / 16 / Int |
| S24 | 24 / 24 / Int (packed 3 bytes) |
| S32 | 32 / 32 / Int |

Channels/rate: `0` means "keep the mix format's value"; otherwise set the requested value. Keep the mix `dwChannelMask` only when the channel count is unchanged; otherwise pass `None` (crate default) or a recommended mask, and expect `S_FALSE`.

Expected shared-mode outcomes (to be confirmed by the spike on real endpoints):

- Same rate and channel count as the mix format, different sample representation (f32 ↔ s16/s32/s8): typically `S_OK`, because the engine converts between integer sample sizes and its internal float representation (`Device Formats` docs).
- Different sample rate or channel count: typically `S_FALSE`, with the mix format as the closest match. Without `AUTOCONVERTPCM`, the engine does not resample or rematrix. So requesting 44.1 kHz on a 48 kHz endpoint, or mono on a stereo endpoint, usually falls back to the mix format and must be advertised as such in `GET_FORMAT`.
- Packed s24: runtime-dependent; a fallback to the mix format is acceptable old behavior. Record whether `S_OK`, `S_FALSE`+closest, or `E_UNSUPPORTED_FORMAT`.
- `autoconvert: true` would let different rates/channels succeed via engine SRC/channel matrix (`AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM` docs), but that deviates from old behavior and the spec's "no resampling unless negotiation requires it". Do not enable it for parity; consider it only if the spike proves packed s24 is otherwise impossible and the A/B approves the deviation.

Algorithm:

```rust
let requested = requested_format(&mix, pref, channels, sample_rate)?;
let negotiated = match client.is_supported(&requested, ShareMode::Shared)? {
    None => requested,          // S_OK
    Some(closest) => closest,   // S_FALSE
};
let advertised = audio_format_of(&negotiated); // encoding + channels + sample_rate + block_align
client.initialize_client(&negotiated, Direction::Capture,
    StreamMode::PollingShared { autoconvert: false, buffer_duration_hns: 10_000_000 }, None)?;
```

The advertised `AudioFormat` must come from `negotiated`, never from the requested values.

### 3.6 s24 packing

- Old code: sets `wBitsPerSample = 24`, `wValidBitsPerSample = 24`, `nBlockAlign = 24 * channels / 8 = 3 * channels`, keeps `SubFormat = KSDATAFORMAT_SUBTYPE_PCM` (`CORE_CPP:171-191`). This is **packed 3-byte little-endian** PCM, exactly what Android expects: `ENCODING_PCM_24BIT` maps to `AudioFormat.ENCODING_PCM_24BIT_PACKED` (`android-app/app/src/main/java/io/github/mkckr0/audio_share_app/service/AudioPlayer.kt:196-200`).
- 24 valid bits in a 32-bit container is a different layout (`WaveFormat::new(32, 24, ...)`); it must be advertised as `ENCODING_PCM_32BIT` (old `set_format` maps on `wBitsPerSample`, `CORE_CPP:356-369`) with `block_align = 4 * channels`. Do not advertise `PCM_24BIT` for a 32-bit container unless the bytes are repacked, which the rewrite does not do.
- `WaveFormat::to_waveformatex()` refuses both 24-bit layouts; do not call it.

---

## 4. Shared-mode format semantics and protobuf mapping

### 4.1 What the OS returns

- `GetMixFormat` returns the format the audio engine uses for shared-mode processing; it is always a `WAVEFORMATEXTENSIBLE` per Microsoft, and is always accepted.
- The engine may also accept "similar" formats with the same sample rate and channel count but a different sample representation (integer sample size vs its internal float). The closest match is the engine's suggested supported format; it is allocated with `CoTaskMemAlloc` and must be freed by a raw-API caller.
- Different rates/channels generally produce `S_FALSE` + the mix format as closest match, unless an LFX APO or `AUTOCONVERTPCM` handles conversion.
- The docs state in the `Initialize` remarks that "the loopback data in the capture buffer is in the device format" (`PKEY_AudioEngine_DeviceFormat`). In practice the old server requests non-mix sample representations and relies on `IsFormatSupported`; whether the loopback buffer truly honors a requested non-mix representation is a spike item (UNVERIFIED, §8). `Device::get_device_format()` (`api.rs:645-651`) lets the spike compare mix vs device format.
- A `WaveFormat` from `get_mixformat`/`is_supported` carries the exact `nBlockAlign`, `nAvgBytesPerSec`, `wBitsPerSample`, `wValidBitsPerSample`, `dwChannelMask`, and `SubFormat`. Use these; do not recompute except for requests.

### 4.2 Block align

- `nBlockAlign = nChannels * wBitsPerSample / 8` for the negotiated container (`WaveFormat::new` recomputes this, `waveformat.rs:201`; the OS closest match carries it).
- Examples: s16 stereo = 4; f32 stereo = 8; packed s24 stereo = 6; s32 stereo = 8; s8 mono = 1.
- Use the negotiated format's `get_blockalign()` for:
  - sizing the capture read buffer,
  - UDP MTU payload alignment (architecture: align 1464 down to `block_align`, send one whole frame if `block_align > 1464`),
  - interpreting packet frame counts.
- Add a consistency check in the core: the mapped encoding's expected bytes/sample × channels must equal `nBlockAlign`. If not (e.g. a strange driver reports 24 bits in a 4-byte container), either map to invalid/unsupported or map to the container width; never advertise a `PCM_24BIT` encoding with a 4-byte block align, because the Android client will misparse.

### 4.3 `WAVEFORMATEXTENSIBLE` SubFormat → protobuf `Encoding`

`protos/client.proto:24-29`:

```
ENCODING_INVALID = 0; ENCODING_PCM_FLOAT = 1; ENCODING_PCM_8BIT = 2;
ENCODING_PCM_16BIT = 3; ENCODING_PCM_24BIT = 4; ENCODING_PCM_32BIT = 5;
```

Mapping (old `set_format`, `CORE_CPP:352-381`):

```rust
fn encoding_of(fmt: &WaveFormat) -> Encoding {
    match fmt.get_subformat() {
        Ok(SampleType::Float) => {
            if fmt.get_bitspersample() == 32 { Encoding::PcmFloat } else { Encoding::Invalid }
        }
        Ok(SampleType::Int) => match fmt.get_bitspersample() {
            8  => Encoding::Pcm8,
            16 => Encoding::Pcm16,
            24 => Encoding::Pcm24,
            32 => Encoding::Pcm32,
            _  => Encoding::Invalid,
        },
        Err(_) => Encoding::Invalid,
    }
}
```

Notes:

- `get_subformat()` already normalizes a plain `WAVEFORMATEX` into a `WAVEFORMATEXTENSIBLE` (`waveformat.rs:243-263`). `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT` → `SampleType::Float`; `KSDATAFORMAT_SUBTYPE_PCM` → `SampleType::Int`; anything else → `UnsupportedSubformat` (`waveformat.rs:358-365`).
- Validate `fmt.get_bitspersample() == 32` for Float. Old code advertised `ENCODING_PCM_FLOAT` for any IEEE-float bit depth; a 64-bit float mix format would then be mis-advertised to Android (which expects 32-bit float). Reject unsupported float widths instead.
- 8-bit PCM in WAVEFORMATEX is **unsigned**, and Android's `ENCODING_PCM_8BIT` is also unsigned; send raw bytes. The old code does no conversion.
- 24-bit is packed (see §3.6).
- If the OS returns a format whose `nBlockAlign` disagrees with the mapped encoding, treat it as unsupported (readable error) rather than advertising a wrong layout.

---

## 5. `windows` crate fallback

### 5.1 Exact version and features

Use **`windows = "0.62.2"`** — the same version as `wasapi`'s transitive dependency, latest in the mirror.

Minimal feature set verified by compiling the skeleton below:

```toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.62.2", features = [
  "Win32_Foundation",
  "Win32_Media_Audio",
  "Win32_Media_KernelStreaming",
  "Win32_Media_Multimedia",
  "Win32_System_Com",
  "Win32_System_Com_StructuredStorage",
  "Win32_System_Variant",
  "Win32_UI_Shell_PropertiesSystem",
  "Win32_Devices_FunctionDiscovery",
] }
```

Add these only if used:

- `Win32_System_Threading`, `Win32_Security`: `CreateEventW`/`WaitForMultipleObjects`/`GetProcessTimes`/`GetProcessHandleCount` (event-driven stop, CPU metrics).
- `Win32_Media`: `timeBeginPeriod`/`timeEndPeriod` (timer resolution).
- `Win32_System_ProcessStatus`: `GetProcessMemoryInfo`.

`wasapi` itself pulls these Windows features transitively, but that does not make the modules nameable from your crate; list what you import.

### 5.2 Exact signatures used in the fallback (all verified in `WINDOWS_SRC`)

```rust
// Win32::System::Com
pub unsafe fn CoInitializeEx(Option<*const c_void>, COINIT) -> HRESULT;          // mod.rs:334-337
pub unsafe fn CoUninitialize();                                                   // mod.rs:572-575
pub unsafe fn CoCreateInstance<P1, T>(*const GUID, P1, CLSCTX) -> Result<T>;      // mod.rs:117-125
pub unsafe fn CoTaskMemFree(Option<*const c_void>);                               // mod.rs:552-555

// Win32::Media::Audio
IMMDeviceEnumerator::EnumAudioEndpoints(EDataFlow, DEVICE_STATE) -> Result<IMMDeviceCollection>; // :5588
IMMDeviceEnumerator::GetDefaultAudioEndpoint(EDataFlow, ERole) -> Result<IMMDevice>;             // :5594
IMMDeviceEnumerator::GetDevice<P0: Param<PCWSTR>>(P0) -> Result<IMMDevice>;                      // :5599-5606
IMMDeviceCollection::GetCount() -> Result<u32>;                                                   // :3181
IMMDeviceCollection::Item(u32) -> Result<IMMDevice>;                                              // :5534
IMMDevice::GetId() -> Result<PWSTR>;                                                              // :5390
IMMDevice::OpenPropertyStore(STGM) -> Result<IPropertyStore>;                                     // :5384
IMMDevice::Activate<T: Interface>(CLSCTX, Option<*const PROPVARIANT>) -> Result<T>;                // :5376
IAudioClient::GetMixFormat() -> Result<*mut WAVEFORMATEX>;                                        // :2583
IAudioClient::IsFormatSupported(AUDCLNT_SHAREMODE, *const WAVEFORMATEX,
    Option<*mut *mut WAVEFORMATEX>) -> HRESULT;                                                   // :2580-2581 (raw HRESULT!)
IAudioClient::Initialize(AUDCLNT_SHAREMODE, u32, i64, i64, *const WAVEFORMATEX,
    Option<*const GUID>) -> Result<()>;                                                           // :2559-2561
IAudioClient::GetDevicePeriod(Option<*mut i64>, Option<*mut i64>) -> Result<()>;                  // :2589
IAudioClient::GetBufferSize() -> Result<u32>;                                                      // :2562
IAudioClient::GetService<T: Interface>() -> Result<T>;                                             // :2604
IAudioClient::Start()/Stop()/Reset() -> Result<()>;                                                // :2592-2599
IAudioCaptureClient::GetNextPacketSize() -> Result<u32>;                                           // :2443
IAudioCaptureClient::GetBuffer(*mut *mut u8, *mut u32, *mut u32,
    Option<*mut u64>, Option<*mut u64>) -> Result<()>;                                             // :2437
IAudioCaptureClient::ReleaseBuffer(u32) -> Result<()>;                                             // :2440

// Win32::UI::Shell::PropertiesSystem
IPropertyStore::GetValue(*const PROPERTYKEY) -> Result<PROPVARIANT>;                               // :2456
// Win32::System::Com::StructuredStorage
PropVariantToStringAlloc(*const PROPVARIANT) -> Result<PWSTR>;                                    // :616
PropVariantClear(*mut PROPVARIANT) -> Result<()>;                                                  // :316
// Win32::Devices::FunctionDiscovery
pub const PKEY_Device_FriendlyName: PROPERTYKEY;                                                   // :2002
// Win32::Foundation
pub const S_FALSE: HRESULT = HRESULT(1);                                                           // :9911
// Win32::Media::Audio
AUDCLNT_E_DEVICE_INVALIDATED    = 0x88890004;  // :1456
AUDCLNT_E_RESOURCES_INVALIDATED = 0x88890026;  // :1481
AUDCLNT_E_SERVICE_NOT_RUNNING   = 0x88890010;  // :1482
AUDCLNT_E_UNSUPPORTED_FORMAT    = 0x88890008;  // :1484
AUDCLNT_E_WRONG_ENDPOINT_TYPE   = 0x88890003;  // :1485
AUDCLNT_STREAMFLAGS_LOOPBACK    = 0x00020000;  // :1497
```

GOTCHA: `IAudioClient::IsFormatSupported` is generated as a raw `HRESULT`, not `Result<()>`, precisely because `S_FALSE` matters. Call `.ok()?` to turn failures into errors (this treats S_FALSE as success), then inspect `ppClosestMatch`: non-null means S_FALSE/closest match. If the call fails, the out pointer is null.

### 5.3 Compile-checked fallback skeleton (core functions)

Full file: `/home/w0n9/.claude/jobs/d82859a4/tmp/wasapi-probe/src/bin/windows_fallback.rs`.
`cargo check --target x86_64-pc-windows-msvc --bin windows_fallback` → `Finished` (dead-code warnings only).

```rust
use std::ptr::{null_mut, read};
use windows::core::HSTRING;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::S_FALSE;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceCollection,
    IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_E_DEVICE_INVALIDATED,
    AUDCLNT_E_RESOURCES_INVALIDATED, AUDCLNT_E_SERVICE_NOT_RUNNING,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_BUFFERFLAGS_SILENT,
    DEVICE_STATE_ACTIVE, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0,
    WAVE_FORMAT_PCM,
};
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

struct ComGuard;
impl Drop for ComGuard { fn drop(&mut self) { unsafe { CoUninitialize() }; } }

unsafe fn com_init() -> windows::core::Result<ComGuard> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?; // S_FALSE is success
    Ok(ComGuard)
}

unsafe fn take_format(ptr: *mut WAVEFORMATEX) -> windows::core::Result<WAVEFORMATEXTENSIBLE> {
    let header = unsafe { read(ptr) };
    let fmt = if header.wFormatTag as u32 == WAVE_FORMAT_EXTENSIBLE && header.cbSize >= 22 {
        unsafe { read(ptr.cast::<WAVEFORMATEXTENSIBLE>()) }
    } else {
        let subformat = match header.wFormatTag as u32 {
            WAVE_FORMAT_PCM => KSDATAFORMAT_SUBTYPE_PCM,
            WAVE_FORMAT_IEEE_FLOAT => KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
            _ => return Err(windows::Win32::Foundation::E_INVALIDARG.into()),
        };
        WAVEFORMATEXTENSIBLE {
            Format: header,
            Samples: WAVEFORMATEXTENSIBLE_0 { wValidBitsPerSample: header.wBitsPerSample },
            dwChannelMask: 0,
            SubFormat: subformat,
        }
    };
    unsafe { CoTaskMemFree(Some(ptr.cast())) };
    Ok(fmt)
}

unsafe fn enum_render_endpoints() -> windows::core::Result<Vec<(String, String)>> {
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let collection: IMMDeviceCollection =
        unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)? };
    let count = unsafe { collection.GetCount()? };
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let device: IMMDevice = unsafe { collection.Item(i)? };
        let id_ptr = unsafe { device.GetId()? };
        let id = unsafe { id_ptr.to_string()? };
        unsafe { CoTaskMemFree(Some(id_ptr.0.cast())) };

        let store: IPropertyStore = unsafe { device.OpenPropertyStore(STGM_READ)? };
        let mut prop = unsafe { store.GetValue(&PKEY_Device_FriendlyName)? };
        let name_ptr = unsafe { PropVariantToStringAlloc(&prop)? };
        let name = unsafe { name_ptr.to_string()? };
        unsafe {
            CoTaskMemFree(Some(name_ptr.0.cast()));
            PropVariantClear(&mut prop)?;
        }
        out.push((id, name));
    }
    Ok(out)
}

unsafe fn negotiate(
    client: &IAudioClient,
    requested: &WAVEFORMATEXTENSIBLE,
) -> windows::core::Result<WAVEFORMATEXTENSIBLE> {
    let mut closest: *mut WAVEFORMATEX = null_mut();
    let hr = unsafe {
        client.IsFormatSupported(AUDCLNT_SHAREMODE_SHARED, &requested.Format, Some(&mut closest))
    };
    hr.ok()?; // AUDCLNT_E_UNSUPPORTED_FORMAT => Err; S_FALSE => Ok
    if closest.is_null() { Ok(*requested) } else {
        debug_assert_eq!(hr, S_FALSE);
        unsafe { take_format(closest) }
    }
}

pub fn build_requested(
    mix: &WAVEFORMATEXTENSIBLE, encoding: u8, channels: u16, sample_rate: u32,
) -> WAVEFORMATEXTENSIBLE {
    let mut fmt = *mix;
    if encoding != 0 {
        let (subformat, storebits, validbits) = match encoding {
            1 => (KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, 32u16, 32u16),
            2 => (KSDATAFORMAT_SUBTYPE_PCM, 8, 8),
            3 => (KSDATAFORMAT_SUBTYPE_PCM, 16, 16),
            4 => (KSDATAFORMAT_SUBTYPE_PCM, 24, 24), // packed
            5 => (KSDATAFORMAT_SUBTYPE_PCM, 32, 32),
            _ => (KSDATAFORMAT_SUBTYPE_PCM, mix.Format.wBitsPerSample, unsafe {
                mix.Samples.wValidBitsPerSample
            }),
        };
        fmt.SubFormat = subformat;
        fmt.Format.wBitsPerSample = storebits;
        fmt.Samples.wValidBitsPerSample = validbits;
    }
    if channels != 0 { fmt.Format.nChannels = channels; }
    if sample_rate != 0 { fmt.Format.nSamplesPerSec = sample_rate; }
    fmt.Format.nBlockAlign = fmt.Format.wBitsPerSample * fmt.Format.nChannels / 8;
    fmt.Format.nAvgBytesPerSec = fmt.Format.nSamplesPerSec * fmt.Format.nBlockAlign as u32;
    fmt
}

unsafe fn open_loopback(
    endpoint_id: &str, requested: &WAVEFORMATEXTENSIBLE, buffer_duration_hns: i64,
) -> windows::core::Result<(IAudioClient, IAudioCaptureClient, WAVEFORMATEXTENSIBLE, u32)> {
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let device: IMMDevice = if endpoint_id.is_empty() || endpoint_id == "default" {
        unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole)? }
    } else {
        let wide = HSTRING::from(endpoint_id);
        unsafe { enumerator.GetDevice(&wide)? }
    };
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
    let format = unsafe { negotiate(&client, requested)? };
    unsafe {
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
            buffer_duration_hns, 0, &format.Format, None)?;
    }
    let buffer_frames = unsafe { client.GetBufferSize()? };
    let capture: IAudioCaptureClient = unsafe { client.GetService()? };
    Ok((client, capture, format, buffer_frames))
}

pub fn poll_loop(
    client: &IAudioClient, capture: &IAudioCaptureClient, format: &WAVEFORMATEXTENSIBLE,
    buffer_frames: u32, stop: &std::sync::atomic::AtomicBool,
    device_period_hns: i64, mut sink: impl FnMut(&[u8], u32),
) -> windows::core::Result<()> {
    let block_align = format.Format.nBlockAlign as usize;
    let mut buffer = vec![0u8; buffer_frames as usize * block_align];
    let period = std::time::Duration::from_nanos((device_period_hns.max(1) as u64) * 100);
    unsafe { client.Start()? };
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        std::thread::sleep(period);
        loop {
            let frames_ready = unsafe { capture.GetNextPacketSize()? };
            if frames_ready == 0 { break; }
            let want = frames_ready as usize * block_align;
            if want > buffer.len() { buffer.resize(want, 0); }
            let mut data: *mut u8 = null_mut();
            let mut frames = 0u32;
            let mut flags = 0u32;
            let mut index = 0u64;
            let mut timestamp = 0u64;
            unsafe {
                capture.GetBuffer(&mut data, &mut frames, &mut flags,
                    Some(&mut index), Some(&mut timestamp))?;
            }
            let bytes = frames as usize * block_align;
            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                buffer[..bytes].fill(0);
            } else {
                unsafe { std::ptr::copy_nonoverlapping(data, buffer.as_mut_ptr(), bytes) };
            }
            unsafe { capture.ReleaseBuffer(frames)? };
            sink(&buffer[..bytes], block_align as u32);
        }
    }
    unsafe { client.Stop()? };
    Ok(())
}

pub fn is_invalidated(err: &windows::core::Error) -> bool {
    err.code() == AUDCLNT_E_DEVICE_INVALIDATED
        || err.code() == AUDCLNT_E_RESOURCES_INVALIDATED
        || err.code() == AUDCLNT_E_SERVICE_NOT_RUNNING
}
```

### 5.4 Complexity comparison

| Aspect | `wasapi` 0.27.0 | `windows` 0.62.2 |
|---|---|---|
| Lines for this flow | ~150 (plus crate internals ~2.6k) | ~250–350 |
| Unsafe blocks | only `initialize_mta` result handling / none in capture path | nearly every call |
| Format parsing | `WaveFormat` accessors; `get_mixformat` handles extensible/plain | manual `WAVEFORMATEX`/`WAVEFORMATEXTENSIBLE` parse + `CoTaskMemFree` |
| Closest match | `Ok(Some(WaveFormat))`, memory freed internally | raw HRESULT + out pointer + manual read/free |
| Silence flag | zero-filled by `read_from_device` | manual `flags & SILENT` + zero-fill |
| Errors | `WasapiError` enum (`errors.rs:2-37`) | `windows::core::Error` + HRESULT constants |
| Maintenance | small 0.x project, 95 stars, low bus factor | Microsoft-maintained, huge surface |
| Extra features | device notifications, device format, sessions, data ranges | anything the Win32 API exposes |
| Compile-verified here | yes | yes |
| Fit for this server | exact old-behavior mapping | viable fallback; more code and risk |

Recommendation: ship on `wasapi`; keep the fallback as a thin second implementation behind the same internal `CaptureBackend` seam, not as a second public API.

---

## 6. Threading, COM apartments, and lifetimes

### 6.1 COM objects are `!Send` and `!Sync`

This is a hard constraint, proven by compile probe:

- `windows` COM interfaces are `#[repr(transparent)]` over `IUnknown`, which wraps `NonNull<c_void>` (`WINDOWS_CORE_SRC/src/unknown.rs:11-12`); `NonNull<T>` is `!Send`/`!Sync`.
- There are no `unsafe impl Send/Sync` for `IMMDevice`, `IAudioClient`, or `IAudioCaptureClient` in `WINDOWS_SRC`.
- Probe: `fn assert_send<T: Send>()` + `assert_send::<DeviceEnumerator/Device/AudioClient/AudioCaptureClient>()` produced four `E0277` errors. `Handle` is the exception: the crate explicitly implements `Send + Sync` (`api.rs:2409-2413`), so event handles can be shared.

Consequences:

- Create, use, and drop every COM object on one thread.
- Never store a `DeviceEnumerator`, `Device`, `AudioClient`, or `AudioCaptureClient` in a `CaptureBackend` that must be `Send + Sync` (architecture declares `CaptureBackend: Send + Sync + 'static`). The Windows backend struct must hold only strings/atomics/channels.
- Never hold a COM object across `.await`; an async future containing it becomes `!Send` and cannot be `tokio::spawn`ed.
- Enumeration (`endpoints()`, `default_endpoint()`) must run on a COM-initialized worker thread. Options: a long-lived audio worker thread with a command channel, or a short-lived `std::thread` that calls `CoInitializeEx`/`CoUninitialize` around the call and returns owned strings.
- Do not initialize COM on the Tauri UI thread (often STA) or on arbitrary Tokio worker threads. Use dedicated MTA threads created by the backend.

### 6.2 Per-thread apartment and balancing

```rust
let hr = wasapi::initialize_mta();
if hr.is_err() { return Err(...); }
let _com = ComGuard; // Drop -> wasapi::deinitialize()
```

- `S_OK` and `S_FALSE` are both success; each successful `CoInitializeEx` call, including `S_FALSE`, needs one `CoUninitialize`.
- `RPC_E_CHANGED_MODE` (0x80010106) means the thread already has a different apartment (STA). Do not call `CoUninitialize` for a failed init. Either use the existing apartment (risky if it is STA and you block) or move the work to a fresh thread.
- Keep the guard declared before all COM objects so reverse drop order destroys the objects first, then calls `CoUninitialize`.

### 6.3 Safe stop from another thread

- Recommended polling pattern: `Arc<AtomicBool>` checked every period. The capture thread never touches COM from another thread; the stopper only sets the flag and joins.
- Maximum stop latency is one poll sleep (device minimum period, typically ~10 ms; up to one engine period). That is fine for a 1 s buffer.
- If immediate wake is required (event mode only), return the crate `Handle` (`Send + Sync`) from the capture thread to the controller, create a second stop event with `CreateEventW`, and use `WaitForMultipleObjects` on `[audio_event, stop_event]`. `Handle::as_handle()` exposes the raw handle without giving up ownership (`api.rs:2431-2447`). This is optional; polling is simpler and pre-1703-safe.
- `stop()` should be idempotent: check `swap(true)`, join exactly once, tolerate an already-stopped stream (`Stop` returns `S_FALSE`, which the crate maps to `Ok`).
- Never join the capture thread on the Tauri UI thread or inside an async executor worker without `spawn_blocking`; the join can take a poll period plus sink backpressure.

### 6.4 Device invalidation / hotplug

- `AUDCLNT_E_DEVICE_INVALIDATED` (0x88890004), `AUDCLNT_E_RESOURCES_INVALIDATED` (0x88890026), and `AUDCLNT_E_SERVICE_NOT_RUNNING` (0x88890010) can surface from `GetNextPacketSize`, `GetBuffer`, `Start`, `Stop`, or `IsFormatSupported`.
- `wasapi` wraps them in `WasapiError::Windows(windows::core::Error)`; use `e.code()` and the raw `AUDCLNT_E_*` constants (direct `windows` dependency) as in the sketch.
- On capture error: break the loop, attempt `stop_stream()` (ignore invalidation on teardown), drop all COM objects, and report a restartable error to the supervisor. Do not `exit()` the process (old `exit_on_failed` killed the whole server, `CORE_CPP:477-483`).
- Supervisor policy: re-enumerate after a short backoff; if the configured endpoint ID still exists, reopen it; if it disappeared, either fall back to the default render endpoint or surface a readable "endpoint unavailable" status. The old code exits, so there is no parity constraint beyond "do not crash"; ticket 05 requires repeated endpoint switching to survive.
- Device notifications: `DeviceEnumerator::register_notification_callback(DeviceEventCallbacks)` (`api.rs:418-447`, `events.rs:322-409`) provides added/removed/state/default-device/property callbacks. Callbacks run on a Windows audio thread, must return quickly, and must not call back into the enumerator (`api.rs:428-430`). Use a `std::sync::mpsc`/tokio channel to notify the supervisor. Keep the `DeviceEventRegistration` alive on the thread that owns the enumerator; it is `!Send`.
- Sleep/resume and default-device changes are effectively invalidation events; treat the first error/discontinuity after resume as "restart capture".

### 6.5 Repeated start/stop

- Recreate the enumerator/device/client each session, like the old code (`do_loopback_recording` constructs everything per call, `CORE_CPP:113-239`). This sidesteps `E_ALREADY_INITIALIZED` (also possible after a failed `Initialize`) and stale buffered audio.
- If reusing a client: `Stop` freezes the clock; `Start` resumes from the same position. Call `Reset` while stopped to flush pending data and reset to 0, otherwise the next session can resend up to 1 s of pre-stop audio. `Reset` fails if the stream is not stopped.
- Always drop the old `AudioCaptureClient` before the `AudioClient` (COM refcounts make either order safe, but stopping the stream first is required).
- 100–200 start/stop cycles with 1 s buffers and immediate stops (<1 s after start) are part of the spike; watch handle count, memory, and `E_ALREADY_INITIALIZED`.

### 6.6 Backpressure between capture and network

`FrameSink` in the architecture is `tokio::sync::mpsc::Sender<Vec<u8>>`. From the sync capture thread use `blocking_send`, but:

- Use a bounded channel sized for 2–4 seconds of audio (`rate * blockalign * seconds`) so a slow client cannot grow memory without bound.
- `blocking_send` blocks the capture thread when full; `stop` then waits for capacity. Either check the stop flag before/after send, or use `try_send` and drop/coalesce on full (protocol has no reordering/sequence, so dropping whole packets is the existing behavior under loss).
- If the receiver is dropped, `blocking_send` returns an error; treat it as "server stopping" and break the loop.
- Measure worst-case send time in the spike (with a deliberately slow consumer) and include it in stop-latency numbers.

### 6.7 Sleep granularity and CPU

`std::thread::sleep(10ms)` on Windows is subject to the system timer resolution (historically ~15.6 ms; Windows 10 2004+ changed some behavior, but it is not guaranteed). With a 1 s buffer this causes jitter but not data loss. Options:

- Measure actual overshoot in the spike before optimizing.
- If needed, call `timeBeginPeriod(1)` (`Win32::Media`, feature `Win32_Media`) while capturing and `timeEndPeriod(1)` on stop, or use `CreateWaitableTimerExW(..., CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, ...)` (`Win32::System::Threading`, plus `Win32_Security` for `SECURITY_ATTRIBUTES`). Both affect system power; restore on stop.
- Do not busy-spin; the 1 s buffer makes a missed poll harmless, and the old code's asio timer is not fundamentally different.

---

## 7. Spike measurement plan (ticket 02)

### 7.1 Tool shape

A standalone `tools/wasapi-spike` binary (or `cargo run -p ... --bin wasapi-spike`), Windows-only, sharing the capture code with the core later. Suggested flags:

```
--list                          enumerate render endpoints (ID + friendly name)
--default                       print default render endpoint
--endpoint <id|default>
--encoding default|f32|s8|s16|s24|s32
--channels <0|N>                (0 = endpoint default)
--sample-rate <0|N>             (0 = endpoint default)
--seconds <N>                   capture duration
--mode polling|events           (events is comparison-only; Win10 1703+)
--out <file.raw>                raw PCM output
--jsonl <file.jsonl>            per-packet / per-batch metrics
--cycles <N>                    start/stop cycles for the churn test
```

Record at startup: requested format, `S_OK`/`S_FALSE`/error, negotiated `AudioFormat`, mix format, `Device::get_device_format()`, `GetDevicePeriod` (default/min), `GetBufferSize` (frames), `block_align`, OS build (`RtlGetVersion`/`GetVersionEx`), endpoint ID/name.

### 7.2 Metrics and definitions

Log one JSONL record per poll wake and one per packet (or aggregate per second to keep files small):

| Metric | Definition |
|---|---|
| `poll_interval_us` | wall time between `sleep()` returns; mean/p50/p95/p99/max; count > 2× min period |
| `batch_packets` | packets drained per wake |
| `packet_frames` | `BufferInfo` frame count; distribution and max |
| `packet_bytes` | `frames * block_align` |
| `device_index` / `device_timestamp` | `BufferInfo.index`, `BufferInfo.timestamp` (QPC, 100 ns); true packet cadence = consecutive timestamp deltas / 10_000 µs, while wall arrival cadence uses `Instant` around `read_from_device` |
| `index_gap_frames` | `info.index - expected_index` when positive (frames lost before read) |
| `flags` | `silent`, `data_discontinuity`, `timestamp_error` counts |
| `frames_total` / `elapsed` | effective captured rate = frames/elapsed; compare to negotiated rate (ppm drift) |
| `buffer_ms` | `GetBufferSize() * 1000 / sample_rate`; must be ~1000 ms for the 1 s request |
| `cpu_percent_one_core` | `delta_cpu_seconds / wall_seconds * 100`, where `delta_cpu_seconds = ((kernel+user FILETIME 64-bit deltas) / 10_000_000)`; also `GetThreadTimes` for the capture thread |
| `read_us` | time inside `read_from_device` (copy + ReleaseBuffer), p50/p99/max |
| `sink_us` | time blocked in `blocking_send` |
| `setup_ms` / `teardown_ms` | enumerate→start, stop→all COM dropped |
| `handles` / `working_set` | `GetProcessHandleCount`, `GetProcessMemoryInfo` before/after cycles |
| `glitches` | count of: index gaps > 0, `data_discontinuity`, intervals > 2× min period, sink blocks > 100 ms |

Glitch/audio validation: play a continuous known tone (e.g. 1 kHz sine, 48 kHz, stereo) into the endpoint during capture; analyze the recorded PCM in 10 ms windows: flag windows below −60 dBFS in regions that should be non-silent, and check for discontinuities. This catches data loss that flags do not report. Keep the tone source and playback method fixed for the A/B.

### 7.3 1 s buffer alignment

- After `Initialize`, `GetBufferSize()` returns frames; expected ≈ `sample_rate` (48000 → 48000; 44100 → 44100), possibly rounded up to the engine period. Log `buffer_ms` and accept 950–1050 ms; investigate anything outside.
- `Initialize` rounds a non-integral-frame duration up to whole frames (Microsoft); the actual buffer is at least the request.
- In shared mode there is no `AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED` (that is exclusive + event); still log the actual value because it feeds stop latency and memory (f32 stereo 1 s ≈ 384 KB).
- Poll at `min_period` (old behavior) and record whether the engine period matches the expected ~10 ms (shared-mode default/min varies by endpoint).

### 7.4 Test matrix

Per run, 60 s minimum (10 min for the steady-state subset):

1. **Endpoint enumeration:** `--list` returns active render endpoints; IDs match `IMMDevice::GetId`; names match Sound panel. Default endpoint matches `GetDefaultAudioEndpoint(eRender, eConsole)`.
2. **Encodings:** `default`, `f32`, `s8`, `s16`, `s24`, `s32` at the mix rate/channels. For each, record requested vs negotiated (`S_OK`/`S_FALSE`/error), advertised `AudioFormat`, bytes/sample, block align, and whether the PCM content is sane (tone analysis). Ticket requires all five encodings attempted at least once.
3. **Fallback:** request 44.1 kHz on a 48 kHz endpoint; request mono on a stereo endpoint; request 96 kHz/6 ch; verify `S_FALSE` + closest match and that `GET_FORMAT` would advertise the negotiated values. Also test an invalid encoding/enum value and an invalid endpoint ID (readable error, no panic/exit).
4. **Event vs polling:** 10 min each (event only on Win10 1703+). Compare CPU, poll interval, index gaps, discontinuities. Record build number; note that pre-1703 event mode receives no events.
5. **Start/stop churn:** 100–200 cycles with 1 s buffer, including stops <1 s after start; assert no crash, no `E_ALREADY_INITIALIZED`, handle/memory return to baseline (±small), and repeated runs still deliver audio.
6. **Device invalidation:** disable/unplug the selected endpoint during capture (Sound panel or USB); verify an error is classified as restartable, the process survives, and capture restarts on the default endpoint (or reports a readable status). Repeat endpoint switching while stopped/running.
7. **Sleep/resume:** optional if the host supports it; expect a discontinuity/invalidation and a successful restart.
8. **A/B with old `as-cmd`:** same machine, same tone/source, same 1 s buffer. Capture the old server's UDP PCM with the scripted client and the new spike's raw PCM; compare `AudioFormat` (encoding/channels/rate/block align), frames per second, packet/interval distributions, CPU, and audible/level glitches. This is the parity gate for `wasapi`.

### 7.5 Report and decision

Produce a short measurement report with:

- environment (Windows build, endpoint, mix format),
- per-config requested/negotiated format and result,
- buffer frames/ms,
- poll interval and packet size distribution,
- CPU per config (one-core %),
- glitch/gap/discontinuity counts over the steady-state run,
- start/stop churn results,
- A/B deltas vs old `as-cmd`,
- `wasapi` vs `windows-rs` conclusion and residual risks.

Acceptance guidance (compare against old `as-cmd`, not absolute numbers): no crash in churn/invalidation; all five encodings attempted with honest fallbacks; no frame gaps in a 10-min steady run with an active tone; CPU and glitch counts not materially worse than old `as-cmd`; 1 s buffer confirmed. Treat any unexplained loss of ≥1 packet per minute under steady load as a NO-GO pending root cause.

Map to ticket 02 checkboxes: enumeration, default-endpoint capture to PCM, five encodings + closest-match, callback interval/CPU/glitch and `wasapi` vs `windows-rs` conclusion, 1 s buffer repeated start/stop.

---

## 8. UNVERIFIED / runtime-only

Everything in this section is based on Microsoft docs, the old code, or reasoning; it could not be executed on this Linux host.

1. Whether Windows loopback delivers the **requested non-mix sample representation** (s8/s16/s24/s32) or always the device/mix format. The old server requests it and relies on `IsFormatSupported`; the `Initialize` docs say loopback data is in the device format. Spike must play a known tone and verify bytes (§7.4.2).
2. Whether packed 24-bit (`wBitsPerSample=24`, `nBlockAlign=3*ch`) is accepted by shared-mode loopback on real endpoints, and what closest match is returned if not.
3. Event-driven loopback with a 1 s buffer on Windows 10 1703+ (crate supports the flags; no loopback event test exists in the crate). Pre-1703 event mode is documented broken.
4. Actual `GetBufferSize()`/rounding for a 10,000,000 hns shared-mode request on each endpoint (old code proves it works; exact frames are runtime).
5. Real poll/wake intervals, sleep granularity, CPU, glitches, and drift for the 1 s buffer.
6. `IsFormatSupported` results for mono/stereo and 44.1/48 kHz requests on real endpoints (expect S_FALSE + mix format, not tested).
7. Which call first reports `AUDCLNT_E_DEVICE_INVALIDATED` on unplug, and whether `Stop` succeeds after invalidation; supervisor behavior needs runtime testing.
8. `RPC_E_CHANGED_MODE`/STA interactions with Tauri and Tokio worker threads; use dedicated MTA threads and test in the GUI.
9. Windows 10 pre-1703 event-driven failure (docs only).
10. 8-hour soak, hotplug, sleep/resume, and real v0.3.4 APK acceptance (tickets 05/acceptance; cannot be run here).

---

## 9. Sources and line index

Primary Microsoft docs:

- Loopback Recording — https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording (event-driven loopback from Windows 10 1703; shared-mode only; render endpoint).
- IAudioClient::Initialize — https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-initialize (shared-mode `hnsPeriodicity = 0`; buffer is a minimum; loopback flag; event-driven shared mode; failure codes).
- IAudioClient::IsFormatSupported — https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-isformatsupported (S_OK/S_FALSE/E_UNSUPPORTED_FORMAT; closest-match allocation; shared-mode same-rate/channel rule).
- Device Formats — https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-formats (engine converts sample representation; not rate/channels).
- AUDCLNT_STREAMFLAGS_XXX Constants — https://learn.microsoft.com/en-us/windows/win32/coreaudio/audclnt-streamflags-xxx-constants (LOOPBACK, EVENTCALLBACK, AUTOCONVERTPCM, SRC_DEFAULT_QUALITY).
- IAudioClient::Stop — https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-stop (freezes position; Start resumes).
- IAudioClient::Reset — https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-reset (stopped stream only; flushes pending data, resets position).
- Matthew van Eerde, "How to negotiate an audio format for a WASAPI client" — https://matthewvaneerde.wordpress.com/2017/10/17/how-to-negotiate-an-audio-format-for-a-windows-audio-session-api-wasapi-client (IsFormatSupported vs autoconvert; use closest match in app code).
- NAudio issue #174 — https://github.com/naudio/NAudio/issues/174 (practical note that loopback implementations usually capture the device/mix format; relevant to UNVERIFIED item 1).

Old implementation (`CORE_CPP`):

- COM init/teardown 101-110.
- endpoint selection 119-138; mix format 140-142; format mutation 144-191.
- `IsFormatSupported` 196-211; `set_format` 213 and 352-381.
- device period 218-220; `Initialize` with 1 s buffer 222-224.
- `GetBufferSize` 226-230; `GetService` 232-234; `Start` 236.
- poll loop 239-289 (min period, `GetNextPacketSize` 260-266, `GetBuffer` 268-273, broadcast 275-278, `ReleaseBuffer` 286).
- endpoint enumeration 292-330; default endpoint 332-350; friendly name 383-392.
- `exit_on_failed` 477-483.
- protobuf enum: `/home/w0n9/workspaces/audio-share/protos/client.proto:24-29`.
- Android 24-bit packed mapping: `/home/w0n9/workspaces/audio-share/android-app/app/src/main/java/io/github/mkckr0/audio_share_app/service/AudioPlayer.kt:191-211`.

`wasapi` 0.27.0 (`WASAPI_SRC`):

- lib re-exports `GUID`, `IAudioClient`, `IMMDevice`: `src/lib.rs:15,21`.
- COM init: `src/api.rs:73-85`.
- `Direction`: 88-92; `StreamMode`: 199-227; `SampleType`: 256-261.
- `DeviceEnumerator`: 361-448; `DeviceCollection`: 466-507; `Device`: 543-724.
- `get_idaudioclient`: 572-582; `get_friendlyname`: 628-632; `get_device_format`: 643-651; `get_id`: 695-701.
- `get_mixformat`: 975-997; `is_supported`: 999-1079.
- `get_device_period`: 1141-1151.
- `initialize_client`: 1194-1351; loopback flag: 1289-1300; autoconvert: 1301-1313; event flag: 1314-1316; period/buffer: 1321-1335.
- `set_get_eventhandle`: 1353-1359; `get_buffer_size`: 1361-1367.
- `start_stream`/`stop_stream`/`reset_stream`: 1399-1421.
- `get_audiocaptureclient`: 1432-1440.
- `BufferInfo`: 2207-2235; `BufferFlags`: 2237-2281.
- `AudioCaptureClient`: 2283-2401; `get_next_packet_size`: 2291-2301; `read_from_device`: 2303-2351; deque variant: 2353-2394.
- `Handle`: 2403-2448 (Send/Sync at 2412-2413; `wait_for_event` 2423-2429; `as_handle` 2445-2447).
- `WaveFormat`: `src/waveformat.rs:71-366`; `new`: 187-240; `from_waveformatex`: 242-263; `to_waveformatex`: 265-312; accessors: 319-365; 24-bit tests: 511-519.
- errors: `src/errors.rs:1-37`.
- device notifications: `src/events.rs:322-409`; registration: `src/api.rs:418-464`.
- dependency/version metadata: `Cargo.toml:12-44, 119-147`.

`windows` 0.62.2 (`WINDOWS_SRC`): exact signatures and line numbers are listed in §5.2; all were compiled against.
