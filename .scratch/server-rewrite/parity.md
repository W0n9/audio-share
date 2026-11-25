# Audio Share Server Rewrite — Exhaustive Parity Reference

Baseline: upstream **v0.3.4**. Repo: `/home/w0n9/workspaces/audio-share`. The working tree is
byte-identical to tag `v0.3.4` for `server-core/`, `server-mfc/`, `protos/`, `android-app/`,
`.github/`, `README.md`, `VERSION`, `scripts/`, `metadata/` (verified with
`git diff --stat v0.3.4 -- <paths>` → empty).

Verification labels used below:

- **[run]** — executed against `/home/w0n9/workspaces/audio-share-notes/server-rewrite/bin/as-cmd-0.3.4-linux`
  (the built v0.3.4 Linux binary) on 2026-11-09.
- **[code]** — verified by reading the cited source only (MFC/GUI and Windows paths could not be run here).
- **[derived]** — deduced from source plus protocol observation, but not directly observed.
- **[unverified]** — could not be confirmed in this environment.
- **[new]** — a requirement from the rewrite spec, not old behavior.

Test environment for runtime probes: Linux, PipeWire 1.0.5 + WirePlumber, a `support.null-audio-sink`
created with `pw-cli create-node adapter`, all inside `dbus-run-session` with a private
`XDG_RUNTIME_DIR`. The machine's default LAN address during the probes was `<lan-ip>`, which
appears in transcripts; that value is machine-specific.

---

## 0. Source map

| Area | Files |
|---|---|
| CLI entry | `server-core/src/main.cpp` |
| Audio abstraction | `server-core/src/audio_manager.hpp`, `audio_manager.cpp` |
| Linux capture | `server-core/src/linux/audio_manager_impl.cpp`, `.hpp` |
| Windows capture | `server-core/src/win32/audio_manager_impl.cpp`, `.hpp` |
| Networking / wire protocol | `server-core/src/network_manager.cpp`, `network_manager.hpp` |
| Protobuf contract | `protos/client.proto` |
| Build/version | `server-core/CMakeLists.txt`, `server-core/src/config.h.in`, `server-core/CMakePresets.json`, `server-core/pack.sh` |
| MFC GUI | `server-mfc/audio-share-server/*.cpp`, `*.h`, `AudioShareServer.rc`, `resource.h`, `i18n/strings.csv`, `i18n/generate_rc.py` |
| Android v0.3.4 client | `android-app/app/src/main/java/io/github/mkckr0/audio_share_app/service/NetClient.kt`, `NetworkIO.kt`, `AudioPlayer.kt`; `app/src/main/res/values/values.xml` |
| Release/CI | `.github/workflows/*.yml`, `scripts/get_version.sh`, `scripts/apply_version.sh`, `scripts/setup_sourceforge_release.sh`, `metadata/en-US/changelogs/*.txt`, `VERSION` |
| Protocol sketch | `docs/protocol.md` (sequence diagram only; byte details come from source) |

---

## 1. `as-cmd` CLI parity

### 1.1 Flags, values, defaults

From `server-core/src/main.cpp:24-35`:

| Short | Long | Value | Default | Implicit value | Notes |
|---|---|---|---|---|---|
| `-h` | `--help` | none | — | — | prints usage to **stdout**, exit 0 |
| `-l` | `--list-endpoint` | none | — | — | prints endpoints, exit 0 |
| `-b` | `--bind` | `[host][:<port>]` | — (no default; option absent = no server) | default LAN address, e.g. `<lan-ip>` | `cxxopts::value<string>()->implicit_value(default_address)` |
| `-e` | `--endpoint` | string | `"default"` | — | `"default"` or `""` selects default endpoint |
| — | `--encoding` | string | `"default"` | — | parsed by `operator>>` into `encoding_t` |
| — | `--list-encoding` | none | — | — | prints encoding list, exit 0 |
| — | `--channels` | int | `0` | — | `0` means device default |
| — | `--sample-rate` | int | `0` | — | `0` means device default; units Hz |
| `-V` | `--verbose` | none | — | — | `spdlog::set_level(trace)` |
| `-v` | `--version` | none | — | — | prints version to stdout, exit 0 |

Encoding value parser (`audio_manager.hpp:51-70`): only exact lowercase `default`, `f32`, `s8`,
`s16`, `s24`, `s32` are recognized. Anything else becomes `encoding_invalid` (1) and is **not**
rejected by the CLI parser. No case folding. An empty endpoint value (`--endpoint=`) is treated like
`"default"` by the capture code (`linux/audio_manager_impl.cpp:103`; `win32/...:122`).

Bind value parsing (`main.cpp:83-92`):

```
size_t pos = s.find(':');
string host = s.substr(0, pos);
if (pos == npos) port = 65530;
else port = (uint16_t)std::stoi(s.substr(pos + 1));
```

- Default port is hard-coded **65530** (`main.cpp:89`).
- If the machine has no default address, the help examples fall back to the literal `<lan-ip>`
  (`main.cpp:16-18`), while the actual implicit `-b` value is the empty string and therefore fails
  with `Invalid argument`.
- Only the **first** `:` is special; extra colons are passed to `std::stoi` (which parses the leading
  integer), so `--bind=127.0.0.1:65083:extra` binds `127.0.0.1:65083` **[run]**.
- `std::stoi` result is cast to `uint16_t`, so ports wrap modulo 65536: `99999 → 34463`,
  `65536 → 0`, `-5 → 65531` **[run]**.
- Port `0` binds an ephemeral port, but the log prints the requested `host:0` **[run]**.
- Empty host (`--bind=`, `--bind=:65082`) → `asio::ip::make_address("")` throws; stderr
  `Invalid argument`, exit 1 **[run]**. IPv6 `[::1]:port` is unsupported because the first `:`
  yields an empty host.
- Invalid host (`--bind=badhost:65080`) → `Invalid argument`, exit 1 **[run]**.
- Port < 1024 when not root → `bind: Permission denied`, exit 1 **[run]**.
- Huge numeric port → `stoi`, exit 1 **[run]**.

**Critical cxxopts quirk [run]:** because `--bind` has an implicit value, it does **not** consume a
following space-separated token.

| Invocation | Actual bind |
|---|---|
| `--bind` | default address:65530 |
| `-b` | default address:65530 |
| `--bind 127.0.0.1:65080` | default address:65530 (the token is ignored as a positional) |
| `--bind=127.0.0.1:65081` | 127.0.0.1:65081 |
| `-b=127.0.0.1:65041` | error: `Option ‘=’ does not exist`, exit 1 |
| `-b127.0.0.1:65044` | error: `Option ‘1’ does not exist` (stacked short options), exit 1 |
| `--bind=host:port --bind=host2:port2` | last one wins **[run]** |

By contrast, options with a `default_value` (not `implicit_value`) do consume a space-separated
value: `--encoding s16`, `--channels 1`, `--sample-rate 44100`, `-e nosuch` all worked **[run]**.

### 1.2 Dispatch precedence

Code order in `main.cpp:39-119`:

1. `--help` → print help, exit 0.
2. `--version` → print version, exit 0.
3. `--verbose` → set trace level (continues).
4. `--list-endpoint` → list, exit 0.
5. `--list-encoding` → list, exit 0.
6. `--bind` → start server and block in `wait_server()`.
7. otherwise → print help to **stderr**; falls off `main` and returns 0. This happens even when
   other options were supplied without `--bind` (e.g. `as-cmd --encoding=f32`); there is no
   "missing --bind" error.

Therefore `-l --list-encoding` lists endpoints; `-v -l` prints version; `--help` wins over
everything.

### 1.3 Exact transcripts [run]

`as-cmd --version` (stdout, exit 0; note the trailing blank line):

```
as-cmd
version: 0.3.4
url: https://github.com/mkckr0/audio-share

```

`as-cmd --help` (stdout, exit 0; `-h` behaves identically). The default address in the bind line is
interpolated from the machine at startup:

```
Example:
  as-cmd -b
  as-cmd --bind=<lan-ip>
  as-cmd --bind=<lan-ip> --encoding=f32 --channels=2 --sample-rate=48000
  as-cmd -l
  as-cmd --list-encoding

Usage:
  as-cmd [OPTION...]

  -h, --help                    Print usage
  -l, --list-endpoint           List available endpoints
  -b, --bind [=[host][:<port>](=<lan-ip>)]
                                The server bind address. If not set, will 
                                use default
  -e, --endpoint [endpoint]     Specify the endpoint id. If not set or set 
                                "default", will use default (default: 
                                default)
      --encoding [encoding]     Specify the capture encoding. If not set or 
                                set "default", will use default (default: 
                                default)
      --list-encoding           List available encoding
      --channels [channels]     Specify the capture channels. If not set or 
                                set "0", will use default (default: 0)
      --sample-rate [sample_rate]
                                Specify the capture sample rate(Hz). If not 
                                set or set "0", will use default. The 
                                common values are 44100, 48000, etc. 
                                (default: 0)
  -V, --verbose                 Set log level to "trace"
  -v, --version                 Show version
```

No arguments (help goes to **stderr**, exit **0**, stdout empty):

```
Example:
  as-cmd -b
  ...
  -v, --version                 Show version
```

`--list-encoding` (stdout, exit 0):

```
encoding list:
	default		Default encoding
	f32		32 bit floating-point PCM
	s8		8 bit integer PCM
	s16		16 bit integer PCM
	s24		24 bit integer PCM
	s32		32 bit integer PCM
```

`-l` with one sink (stdout, exit 0; the PipeWire init line is a spdlog info log):

```
[2026-11-09 15:08:46.055] [info] pipewire header_version: 1.0.5, library_version: 1.0.5
endpoint list:
	* id: 33   name: Test Sink
total: 1
```

`-l` with two sinks, default = highest `priority.session` (observed A=1009, B=2000):

```
endpoint list:
	  id: 38   name: Sink A
	* id: 44   name: Sink B
total: 2
```

`-l` with no sinks:

```
[2026-11-09 15:20:10.018] [info] pipewire header_version: 1.0.5, library_version: 1.0.5
endpoint list:
total: 0
```

Formatting source: `main.cpp:58-63`:
`fmt::format("\t{} id: {:4} name: {}\n", (id == default_endpoint ? '*' : ' '), id, name)`, then
`fmt::format("total: {}\n", endpoint_list.size())`. IDs are left-aligned in a 4-char field. The
`pipewire header_version` line is a normal spdlog info log emitted by the Linux `audio_manager`
constructor, not part of the list output; Windows has no equivalent line.

`-l` with **no PipeWire daemon** crashes (segfault, exit 139): the `audio_manager` constructor does
not check `pw_context_connect()` for null (`linux/audio_manager_impl.cpp:74-87`), then
`pw_core_get_registry(nullptr, ...)` dereferences null **[run]**. The new implementation must return
a readable error instead.

### 1.4 Error and exit codes

| Path | stderr | Exit | Evidence |
|---|---|---|---|
| `--help`, `-h` | — | 0 | [run] |
| `--version`, `-v` | — | 0 | [run] |
| no args | full help | 0 | [run] |
| `-l` success | — (info log on stdout) | 0 | [run] |
| `-l` no PipeWire daemon | — | 139 (SIGSEGV) | [run] |
| `--list-encoding` | — | 0 | [run] |
| unknown option `--bogus` | `Option ‘bogus’ does not exist` + help | 1 | [run] |
| missing value `-e` | `Option ‘e’ is missing an argument` + help | 1 | [run] |
| non-int `--sample-rate=abc` | `Argument ‘abc’ failed to parse` + help | 1 | [run] |
| non-int port `--bind=127.0.0.1:abc` | `stoi` | 1 | [run] |
| overflowing port | `stoi` | 1 | [run] |
| empty/invalid host | `Invalid argument` | 1 | [run] |
| privileged port bind | `bind: Permission denied` | 1 | [run] |
| TCP/UDP bind conflict | asio `bind: Address already in use` | 1 | [derived from `main.cpp:113-119`] |
| unknown encoding (`--encoding=bogus`) | no parse error; starts, then Linux capture exits 1 (`the capture format is not supported`); Windows logs `invalid encoding` and keeps serving with no audio | Linux 1 / Windows stays alive | [run] Linux; [code] Windows `win32/audio_manager_impl.cpp:152-154` |
| unknown endpoint (`-e nosuch`) | no parse error; server keeps running, logs `selected audio endpoint is not in list`, format stays empty, audio never starts | stays running | [run] Linux; [code] Windows exits via `exit_on_failed` (`win32/...:126-128`) |
| `--channels=-1` | segfault during PipeWire negotiation | 139 | [run] |
| `--encoding=s8` on Linux | negotiates U8 (SPA format 258), switch default logs `the capture format is not supported`, `exit(EXIT_FAILURE)` | 1 | [run]; bug in `linux/audio_manager_impl.cpp:229-250` |
| server normal lifetime | — | SIGTERM → 143 | [run] |

Note the platform divergence for `--endpoint` and `--encoding`: Linux logs and returns from the
capture thread (server keeps serving), Windows uses `exit_on_failed()` → `exit(-1)` or returns for
invalid encoding. `exit(-1)` is a Windows exit code of `0xFFFFFFFF`; how a shell reports it was not
verified here [unverified].

### 1.5 Default bind host algorithm

`network_manager::get_default_address()` (`network_manager.cpp:110-146`) =
`select_default_address(get_address_list())`.

Enumeration:

- **Linux** (`network_manager.cpp:81-105`): `getifaddrs()`; keep `AF_INET`; skip `IFF_LOOPBACK`;
  no interface-up filter; `inet_ntop`. Order is `getifaddrs` order. IPv6 excluded.
- **Windows** (`network_manager.cpp:53-79`): `GetAdaptersAddresses(AF_INET, GAA_FLAG_INCLUDE_ALL_INTERFACES)`;
  skip adapters whose `OperStatus != IfOperStatusUp` or `IfType == IF_TYPE_SOFTWARE_LOOPBACK`;
  enumerate unicast IPv4 addresses. Adapter order.

Selection (`network_manager.cpp:115-146`):

```
private_addr_list = { 0x0a000000, 0xac100000, 0xc0a80000 }
for each address: if ((ntohl(addr) & mask) == mask) return address;   // mask == the same value
return address_list.front();
```

- Matches any `10.x.x.x`, **only `172.16.x.x`** (the full RFC1918 `172.16.0.0/12` is not matched
  because the value is also used as the mask), and any `192.168.x.x`. This is a bug to decide on.
- First matching address in enumeration order wins; otherwise the first address overall.
- Empty list → empty string. Then bare `-b` yields host `""` and exits 1 with `Invalid argument`.
- The GUI uses the same first address as the default Host value.

### 1.6 `server.log`

- **as-cmd does not write `server.log`.** `main.cpp` never installs a file sink; spdlog's default
  logger writes to stdout. No `server.log` was created in any probe working directory **[run]**.
  The README statement at `README.md:108` ("open server.log") applies to the MFC GUI.
- The MFC GUI writes `<exe_dir>/server.log` via
  `spdlog::basic_logger_mt("server", exe_dir/"server.log")` (`AudioShareServer.cpp:54-55`), sets it
  as the default logger, and only in `_DEBUG` sets `flush_every(1s)` and level `trace`
  (`AudioShareServer.cpp:56-59`). Release level is spdlog's default `info`. `basic_logger_mt`
  truncates on each launch and does not rotate [derived from spdlog semantics; not run].
- This conflicts with spec/ticket 07 ("`server.log` 继续写入" for `as-cmd`). **Decision needed:**
  either add a file sink to the new CLI (intentional improvement) or correct the ticket. The new GUI
  should keep `<exe_dir>/server.log` to match MFC.

### 1.7 Runtime log lines (Linux, info level) [run]

```
[2026-...] [info] pipewire header_version: 1.0.5, library_version: 1.0.5
[2026-...] [info] tcp listen success on 127.0.0.1:65052
[2026-...] [info] udp listen success on 127.0.0.1:65052
[2026-...] [info] server started
[2026-...] [info] select audio endpoint: 38, Test Sink
[2026-...] [info] stream name: audio-share-server
[2026-...] [info] audio_info.info.raw.format: 283
[2026-...] [info] the capture format is supported
[2026-...] [info] block_align: 8
[2026-...] [info] AudioFormat:
encoding: ENCODING_PCM_FLOAT
channels: 2
sample_rate: 48000
[2026-...] [info] accept 127.0.0.1:34092
[2026-...] [info] fill_udp_peer fill udp peer id:1 tcp://127.0.0.1:34092 udp://127.0.0.1:.....
[2026-...] [info] 127.0.0.1:34092 timeout
[2026-...] [info] close 127.0.0.1:34092
```

`-V` raises the level to trace and adds command IDs, PipeWire properties, `AudioFormat::DebugString`,
send/receive trace, etc. (`main.cpp:50-52`; `network_manager.cpp:217` etc.).

---

## 2. `config.ini` parity

### 2.1 Location and API

- Path: `<directory of AudioShareServer.exe>/config.ini`
  (`AudioShareServer.cpp:61-62`: `m_pszProfileName = _wcsdup((exe_dir / "config.ini").c_str())`).
- Read/written through MFC `GetProfileIntW`/`GetProfileStringW`/`WriteProfileInt`/
  `WriteProfileStringW`, i.e. Win32 INI semantics: section and key lookup are case-insensitive;
  surrounding whitespace is trimmed; surrounding quotes are stripped; values are ANSI/Unicode text.
  Exact on-disk encoding of MFC-written files was not verified [unverified].
- New implementation must read these names case-insensitively (Windows compatibility) and preserve
  section/key spelling when writing.

### 2.2 Sections, keys, types, defaults

| Section | Key (exact spelling) | Type | Default | Meaning / values | Read at | Written at |
|---|---|---|---|---|---|---|
| `App` | `configVersion` | int | `0` (if missing) | Current = `1`. Any other value triggers destructive reset. | `AudioShareServer.cpp:110` | `AudioShareServer.cpp:115` |
| `App` | `language` | string | `"default"` | `"default"`, `"en-US"`, `"zh-CN"` | `AudioShareServer.cpp:118`; combo `CAppSettingsTabPanel.cpp:107` | combo change `CAppSettingsTabPanel.cpp:338-342` |
| `App` | `Running` | int (bool) | `false` | `0`/`1`; last server running state for "Keep last state" | `CServerTabPanel.cpp:72` | start `CServerTabPanel.cpp:219`, stop `CServerTabPanel.cpp:233` |
| `App Settings` | `AutoRun` | int (bool) | `false` | enables HKCU Run entry | `CAppSettingsTabPanel.cpp:85` | toggle `CAppSettingsTabPanel.cpp:125` |
| `App Settings` | `WhenAppStart` | int | `0` | `0` do nothing, `1` start server, `2` keep last state | `CAppSettingsTabPanel.cpp:88`; `CServerTabPanel.cpp:71` | radio `CAppSettingsTabPanel.cpp:245-249` |
| `App Settings` | `WhenClose` | int | `0` | `0` exit, non-zero (UI value `1`) minimize to tray | `CAppSettingsTabPanel.cpp:89`; `CMainDialog.cpp:226` | radio `CAppSettingsTabPanel.cpp:251-255` |
| `App Settings` | `AutoUpdate` | int (bool) | `false` | enables 3-hour update timer | `CAppSettingsTabPanel.cpp:91-94` | toggle `CAppSettingsTabPanel.cpp:233-243` |
| `Network` | `host` | string | default LAN address (or empty if none) | Host/address text; may be arbitrary hostname/IP | `CServerTabPanel.cpp:91` | successful start `CServerTabPanel.cpp:215` |
| `Network` | `port` | string | `"65530"` | Stored as text, parsed with `std::stoi` on start | `CServerTabPanel.cpp:97` | successful start `CServerTabPanel.cpp:216` |
| `Capture` | `endpoint` | string | `"default"` | `"default"` or endpoint ID | `CServerTabPanel.cpp:119` | successful start `:217`; reset fallback `:128`; **bug** `:159` |
| `Capture` | `encoding` | int | `0` (`encoding_default`) | enum: 0 default, 1 invalid/unused, 2 f32, 3 s8, 4 s16, 5 s24, 6 s32 | `CServerTabPanel.cpp:151` | successful start `:218` |

Enum definition: `server-core/src/audio_manager.hpp:41-49`. Note the on-disk encoding numbers are the
same enum used by the CLI.

### 2.3 Destructive `configVersion` migration [code]

```
int nConfigVersion = 1;
if (nConfigVersion != GetProfileIntW(L"App", L"configVersion", 0)) {
    CFileStatus fileStatus;
    if (CFile::GetStatus(m_pszProfileName, fileStatus)) {
        CFile::Remove(m_pszProfileName);          // delete entire config.ini
    }
    WriteProfileInt(L"App", L"configVersion", nConfigVersion);
}
```

`AudioShareServer.cpp:109-116`. Consequences:

- A config.ini with no `configVersion` (treated as 0) is deleted wholesale on first launch.
- Any future/unknown value is also deleted wholesale.
- After deletion, only `[App] configVersion=1` is written; all other keys are recreated lazily.
- No backup, no per-key migration, no user prompt.
- This is the old "migration" behavior the rewrite spec explicitly wants to improve
  ("configVersion 不匹配时给出明确迁移结果，而不是静默丢弃用户设置" — `.scratch/server-rewrite/spec.md:170`).
  The new implementation must still **read v0.3.4 files** and migrate their keys; it may report the
  outcome instead of silently deleting.

### 2.4 Language

- Read once in `InitInstance` before any window is created:
  `GetProfileStringW(L"App", L"language", L"default")`; if not `"default"`, call
  `SetProcessPreferredUILanguages(MUI_LANGUAGE_NAME, lang, &1)` (`AudioShareServer.cpp:118-124`).
- Combo values (`CAppSettingsTabPanel.cpp:96-106`): `default`, `en-US`, `zh-CN`, displayed as
  Default / English / Simplified Chinese (localized via `strings.csv`). Selecting one writes
  `[App] language` immediately (`CAppSettingsTabPanel.cpp:338-342`). The UI does **not** switch
  live; it takes effect on next launch.
- Resources: base `AudioShareServer.rc` is en-US; the build generates
  `i18n/AudioShareServer_zh-CN.rc` via `generate_rc.py` from `i18n/strings.csv`, compiled into the
  exe (`audio-share-server.vcxproj:141-146, 329-331`). Missing languages fall back to en-US.

### 2.5 Startup / close behavior enums

- `WhenAppStart` (`CServerTabPanel.cpp:71-74`):
  - `1` → start server.
  - `2` and `[App] Running` truthy → start server.
  - `0`, `2` with `Running=false`, or any other value → do nothing.
  - Evaluated in `CServerTabPanel::OnInitDialog`, i.e. after GUI construction but before the user
    sees the window; it posts `BM_CLICK` to the Start button.
- `WhenClose` (`CMainDialog.cpp:224-233`):
  - `0` → `DestroyWindow()` → application exits (server is not explicitly stopped).
  - non-zero → `theApp.HideApplication()` → window(s) hidden, server keeps running, tray stays.
- `WhenClose` applies only to the window close button (`WM_CLOSE`). The Exit command/button always
  destroys the window; Hide always hides.

### 2.6 AutoRun / AutoUpdate

- AutoRun (`CAppSettingsTabPanel.cpp:214-226`): default false. On settings-tab init the current
  value is re-applied to the registry (`:85-86`), so every launch with AutoRun=false deletes the Run
  value.
  - Key: `HKEY_CURRENT_USER\SOFTWARE\Microsoft\Windows\CurrentVersion\Run`
  - Value name: `AudioShareServer`
  - Value data (`REG_SZ`): `<full exe path>` + ` /hide` (space before `/hide`)
  - `RegOpenKeyExW` return value is not checked; failures are silent.
- AutoUpdate (`CAppSettingsTabPanel.cpp:91-94, 233-243`): default false. Enabling starts an MFC timer
  with ID 1 and interval `3*60*60*1000` ms; disabling kills it. The first automatic check fires after
  one full interval (3 h), not at startup.

### 2.7 Where server host/port/endpoint/encoding are read/written

- Reset (`CServerTabPanel::OnBnClickedButtonReset`, `CServerTabPanel.cpp:81-163`) is called at panel
  initialization (`:65`) and by the Reset button. It:
  1. Repopulates the editable Host combo from `get_address_list()` and sets the text to
     `[Network] host` (or default LAN address). It does **not** add the default address as a combo
     item unless it is in the enumeration.
  2. Sets Port from `[Network] port` (default `"65530"`).
  3. Rebuilds the Endpoint combo with localized "Default" (item data `"default"`) followed by
     enumerated endpoints. Selects `[Capture] endpoint`; if not found, writes `endpoint=default`
     and selects Default.
  4. Rebuilds the Encoding combo (Default, f32, s8, s16, s24, s32 with enum values as item data).
     Selects `[Capture] encoding`; if not found, selects item 0 and **bug**: calls
     `WriteProfileInt(L"Capture", L"endpoint", encoding_default)` — it writes the wrong key and
     clobbers `[Capture] endpoint` with an integer (`CServerTabPanel.cpp:158-161`).
  - "Reset" therefore means "reload controls from config/defaults", not "restore factory defaults".
- Start success writes host, port, endpoint, encoding, and `Running=1`
  (`CServerTabPanel.cpp:215-219`). Stop writes `Running=0` (`:233`). A failed start writes nothing.
- Editing Host/Port/Endpoint/Encoding alone does not persist; it persists when Start succeeds.
- Settings-tab controls persist immediately on change (AutoRun, WhenAppStart, WhenClose, AutoUpdate,
  language).

---

## 3. MFC GUI behavior [code]

All of this section is source-verified only; the MFC app is Windows-only and was not executed here.

### 3.1 Process / window / tray lifecycle

- `CAudioShareServerApp` constructor (`AudioShareServer.cpp:40-63`):
  - `EnsureSingleton()` before anything else.
  - App name `"Audio Share Server"`.
  - `m_exePath` = full module path.
  - Installs `<exe_dir>/server.log` as the default spdlog logger.
  - `m_pszProfileName` = `<exe_dir>/config.ini`.
- `InitInstance` (`AudioShareServer.cpp:77-181`):
  - Applies configVersion reset and language (see §2).
  - Parses the command line for the `hide` flag (see §3.8).
  - Creates a modeless `CMainDialog` (`IDD_MAIN`), sets it as `m_pMainWnd`, returns TRUE so MFC runs
    the message pump. There is no modal dialog.
- Main window `IDD_MAIN` (`AudioShareServer.rc:107-114`): 307×259 dialog units, caption
  "Audio Share Server", styles `WS_MINIMIZEBOX | WS_POPUP | WS_CAPTION | WS_SYSMENU`, centered,
  non-resizable, no maximize box. Icon `IDR_MAINFRAME`.
- Two tabs, in this order: "Server" (`IDD_SERVER`) and "App Settings" (`IDD_APP_SETTINGS`)
  (`CMainDialog.cpp:184-206`; strings `IDS_SERVER=113`, `IDS_APP_SETTINGS=114`).
- Tray icon (`CMainDialog.cpp:78-100`): `Shell_NotifyIconW(NIM_ADD)` with
  `NOTIFYICON_VERSION_4`, `uID=0`, tooltip "Audio Share Server", icon `IDR_MAINFRAME`, callback
  `WM_APP_NOTIFYICON`. Created in `OnInitDialog` (`:174`) and deleted in `OnDestroy` (`:276`).
- Window visibility: `ShowWindow(theApp.m_bHide ? SW_HIDE : SW_SHOW)` (`CMainDialog.cpp:181`).
  Hiding the window does not stop the server and does not remove the tray icon.
- Minimizing the window is ordinary taskbar minimization; there is no "minimize to tray" handler for
  `WM_SYSCOMMAND SC_MINIMIZE` (only About is intercepted, `CMainDialog.cpp:235-245`).

### 3.2 Tray icon interaction

`CMainDialog::OnNotifyIcon` (`CMainDialog.cpp:247-269`):

| Event | Action |
|---|---|
| `WM_LBUTTONDOWN` | Send `WM_COMMAND ID_APP_SHOW` → `OnAppShow`: `ShowWindow(SW_SHOW)`, `ShowWindow(SW_NORMAL)`, `SetForegroundWindow()` (`:302-307`) |
| `WM_RBUTTONDOWN` | `SetForegroundWindow()`, then MFC context-menu manager `ShowPopupMenu(IDR_MENU_SYSTEM_TRAY, cursor)` (`:257-263`) |
| `NIN_BALLOONUSERCLICK` | `ShellExecuteW(nullptr, nullptr, m_strUpdateLink, ...)` (`:264-266`) |
| other (including left double-click) | ignored |

Tooltip is always "Audio Share Server"; there is no balloon on start/stop or on errors.

### 3.3 Tray menu

`IDR_MENU_SYSTEM_TRAY` (`AudioShareServer.rc:312-322`):

```
Show
---
Start Server      (label changes to "Stop Server" when running)
---
Exit
```

- Menu items: `ID_APP_SHOW` (32771), `ID_START_SERVER` (32772), `ID_APP_EXIT` (`IDCANCEL`-class
  command, bound to `OnAppExit`).
- Enabled states: all items are always enabled; there is no `ON_UPDATE_COMMAND_UI` disabling.
  Only the Start/Stop **label** is dynamic (`OnUpdateStartServer`, `CMainDialog.cpp:315-320`, loads
  `IDS_STOP_SERVER`/`IDS_START_SERVER` based on `CServerTabPanel::IsRunning()`).
- `ID_START_SERVER` → `CMainDialog::OnStartServer` → `m_tabPanelServer->SwitchServer()` → posts
  `BM_CLICK` to the server button, so it runs the same Start/Stop handler. If start preconditions
  fail (no endpoint / no selection), the same message box appears.
- `ID_APP_EXIT` → `DestroyWindow()` (does not call `stop_server` explicitly).

### 3.4 Balloon notifications

`CMainDialog::ShowBalloonNotification` (`CMainDialog.cpp:102-118`):

- `NIM_MODIFY`, flags `NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_INFO`, `NIIF_INFO | NIIF_LARGE_ICON`,
  title and body from caller, tooltip "Audio Share Server".
- Used only for "new version available": title `IDS_NEW_VERSION` ("New Version"), body = raw GitHub
  `tag_name` (e.g. `v0.3.5`).
- Clicking opens `m_strUpdateLink`, set to the release `html_url`.

### 3.5 About dialog

`IDD_ABOUTBOX` (`AudioShareServer.rc:93-105`), `CAboutDialog.cpp:82-99`:

- Caption: "About Audio Share Server".
- Fields at runtime:
  - Name: `ProductName` from the exe version resource → "Audio Share Server".
  - Version: `ProductVersion` from the version resource → "0.3.4" (label "Version:").
  - Copyright: `LegalCopyright` → "Copyright © 2022-2024 mkckr0. All rights reserved."
- Home Page MFC link → `https://github.com/mkckr0/audio-share` (decoded from the `DLGINIT` block at
  `AudioShareServer.rc:289-304`).
- Bitmap `res/AudioShareServer.bmp`, OK button; name font Segoe UI 24 pt.
- Version source for both About and update check is the Win32 `VS_VERSION_INFO` resource
  (`AudioShareServer.rc:172-201`), which `scripts/apply_version.sh` rewrites from `VERSION`.
- `GetStringFileInfo` reads `\StringFileInfo\040904B0\<name>` (`CAboutDialog.cpp:27-64`).

### 3.6 Server tab (`IDD_SERVER`, `CServerTabPanel`)

Controls (`AudioShareServer.rc:116-135`):

- Group "Server Address": Host combo (`CBS_DROPDOWN`, editable), Port edit
  (`ES_NUMBER | ES_CENTER`, max 5 chars via `SetLimitText(5)`).
- Group "Capture": Audio Endpoint combo (`CBS_DROPDOWNLIST`, first item "Default"),
  Encoding combo (`CBS_DROPDOWNLIST`), Sound Panel button.
- Group "Action": Start Server button (default push button), Reset button.

Start/Stop handler (`CServerTabPanel.cpp:165-236`):

- If not running:
  1. If endpoint combo has only the Default item → message box `IDS_NO_ENDPOINT`
     ("No Audio Endpoint"), abort.
  2. If no selection (`CB_ERR`) → `IDS_NO_SELECTED_ENDPOINT`, abort.
  3. Disable Host, Port, Endpoint, Reset, Encoding, Sound Panel; disable Start button.
  4. Read host/port; `std::stoi(port)` happens **outside** the try block (`:188-195`) — an empty or
     non-numeric port throws uncaught inside the message handler (likely process termination).
  5. Build `capture_config{endpoint_id, encoding, channels=0, sample_rate=0}` (GUI never exposes
     channels/sample rate).
  6. `network_manager->start_server(host, port, config)`; on `std::exception`, show `e.what()` in a
     message box, re-enable controls, restore "Start Server", focus.
  7. On success, enable Start button, set its text to "Stop Server", write config (see §2.7), set
     `m_bStarted=true`.
- If running:
  1. Disable Start button.
  2. Call `network_manager->stop_server()` **synchronously on the UI thread** (blocks the UI).
  3. Re-enable controls, set text "Start Server", write `Running=false`.
- There is no async progress, no cancellation, no visual busy state beyond disabled controls.

Sound Panel (`CServerTabPanel.cpp:249-263`): `CreateProcessW("control.exe /name Microsoft.Sound")`,
then `WaitForSingleObject(pi.hProcess, INFINITE)` on the UI thread; the UI is blocked until the
Control Panel sound app exits. Return value of `CreateProcessW` is not checked; no error is shown.

Enable/disable while running (`CServerTabPanel.cpp:238-246`): Host, Port, Endpoint, Reset, Encoding,
Sound Panel disabled; Start/Stop button remains enabled. Settings-tab controls remain enabled.

Endpoints: Windows `EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)`;
ID = `IMMDevice::GetId()` (wide string converted using the ANSI code page);
name = `PKEY_Device_FriendlyName`, falling back to literal `"[Speakers]"` when the property is not
`VT_LPWSTR` (`win32/audio_manager_impl.cpp:292-330, 383-392`). Default endpoint for display is
`GetDefaultAudioEndpoint(eRender, eConsole)`.

Encoding combo labels: Default, 32 bit floating-point PCM, 8 bit integer PCM, 16 bit integer PCM,
24 bit integer PCM, 32 bit integer PCM; item data = `encoding_t` int.

### 3.7 App Settings tab (`IDD_APP_SETTINGS`, `CAppSettingsTabPanel`)

Controls (`AudioShareServer.rc:137-164`):

- Action: Exit, Hide, About.
- Behavior: "Auto run the app when system starts" checkbox.
  - "When app starts": Do nothing / Start server / Keep last state.
  - "When click close button": Exit / Minimize to system tray.
- Appearance: Language combo.
- Firewall: Repair Firewall button with shield icon (`SHGetStockIconInfo(SIID_SHIELD, ...)`,
  `CAppSettingsTabPanel.cpp:79-83`).
- Update: "Check for Update" button, "Auto check for update" checkbox.

Firewall repair (`CAppSettingsTabPanel.cpp:128-212`):

- Creates `INetFwPolicy2` through the COM elevation moniker
  `Elevation:Administrator!new:{CLSID_NetFwPolicy2}` with `CLSCTX_LOCAL_SERVER`, so Windows shows a
  UAC prompt. `hwnd = nullptr`.
- Enumerates all firewall rules. For each rule whose `ApplicationName` is
  `std::filesystem::equivalent` to the current exe, it sets the rule name to the application path and
  calls `INetFwRules::Remove(app_name)`. This is almost certainly ineffective (the stored rule name
  is "Audio Share Server(TCP-In)" / "(UDP-In)", not the path), but the subsequent `Add` overwrites
  rules with the same names. Label [derived/unverified].
- Computes profiles = `INetFwPolicy2::CurrentProfileTypes`; if the mask contains PUBLIC **and** is not
  exactly PUBLIC, PUBLIC is XORed out. Thus rules apply to the currently active profiles minus
  Public (Public is kept only if it is the only active profile).
- Creates one `INetFwRule3` object and adds it twice:
  - Name/Description `"Audio Share Server(TCP-In)"`, protocol `NET_FW_IP_PROTOCOL_TCP` (6),
    `ApplicationName` = full exe path (application-scoped, not port-scoped), Enabled, Action Allow,
    Profiles as above.
  - Name/Description `"Audio Share Server(UDP-In)"`, protocol `NET_FW_IP_PROTOCOL_UDP` (17),
    same other fields.
- Success → `AfxMessageBox(IDS_SUCCESS)` ("Success"); any exception → message box with a Win32 error
  string (`wstr_win_err(Win32ErrorFromCaughtException())`). Canceling UAC surfaces as an error.
- No firewall repair at startup; Windows may show its own firewall prompt on first listen.

Hide / Exit / About: Hide → `theApp.HideApplication()`; Exit → window destroy; About → modal
`CAboutDialog`.

### 3.8 Command line `/hide`

Custom `CCommandLineInfo::ParseParam` (`AudioShareServer.cpp:126-137`) sets `m_bHide=true` when a
flag's text is exactly `"hide"`. Documented form is `/hide` (`metadata/en-US/changelogs/15.txt:4-5`),
and `-hide` is also expected to match because MFC strips one leading flag character. `--hide` is
likely **not** matched (the remaining text would be `-hide`), but this was not run
[unverified]. `m_bHide` only hides the window at `OnInitDialog`; it does not affect startup behavior
or auto-start.

### 3.9 Single instance

`CAudioShareServerApp::EnsureSingleton` (`AudioShareServer.cpp:188-218`):

- Mailslot name: `\\.\mailslot\AudioShareServer\938609DF-0FF0-49C9-83BB-ABB6F391F544`.
- First instance: `CreateMailslotA(name, 0, MAILSLOT_WAIT_FOREVER, nullptr)` succeeds; a detached
  thread loops on blocking `ReadFile` and, for every 4-byte message, sends
  `WM_COMMAND ID_APP_SHOW` to the main dialog (shows the existing window).
- Second instance: `CreateMailslotA` fails because the mailslot exists; opens it with
  `CreateFileA(GENERIC_WRITE, FILE_SHARE_READ, OPEN_EXISTING)`, writes `int 1`, and
  `ExitProcess(0)`. There is no user-facing message.
- Races: `EnsureSingleton` runs in the app constructor before `m_pMainWnd` exists, and the reader
  thread dereferences `GetMainDialog()` without a null check; launching a second instance extremely
  early can crash the first. Also, two simultaneous first launches can race.
- A second launch always shows the existing window, even if its server is running or hidden.

### 3.10 Update check

`CAppSettingsTabPanel::CheckForUpdate` (`CAppSettingsTabPanel.cpp:26, 257-325`):

- URL: `https://api.github.com/repos/mkckr0/audio-share/releases/latest`.
- Transport: MFC `CInternetSession("Audio Share Server", ..., INTERNET_OPEN_TYPE_DIRECT, ...,
  INTERNET_FLAG_DONT_CACHE)` + `CHttpFile` with `INTERNET_FLAG_TRANSFER_BINARY |
  INTERNET_FLAG_RELOAD`; reads the whole body into a string, parses with nlohmann/json.
- Local version string: `"v" + ProductVersion` from the MFC version resource (e.g. `v0.3.4`).
- Remote tag: `tag_name` (raw string).
- Comparison `util::is_newer_version(lhs=tag_name, rhs=local)` (`util.cpp:7-38`):
  - Both strings must start with `v`; otherwise throw.
  - Split on `.`, ignoring empty substrings; exactly 3 components required; each parsed by
    `std::stoi` (leading numeric prefix accepted; throws on non-numeric).
  - Compare component-wise numerically; return true iff remote > local; equal → false.
- If newer: `SetUpdateLink(res["html_url"])` on the main dialog and show balloon
  title "New Version", body `tag_name`.
- If not newer and the check was manual: message box `IDS_NO_UPDATE` ("No Update").
  Automatic checks never show that box (fix #147, `metadata/en-US/changelogs/3004.txt`).
- Errors: logged (`CAppSettingsTabPanel::OnBnClickedButtonUpdate: ...`); message box only for manual
  checks.
- Automatic checks: timer `3*60*60*1000` ms, only while AutoUpdate is enabled; first fire after
  3 hours.
- No download, no silent install, no Tauri updater; a balloon click opens the release page with
  `ShellExecuteW`.

### 3.11 About/version/i18n data sources for the rewrite

- New GUI version must be `0.4.0` (spec) and About must show version, homepage, license.
- Keep en-US and zh-CN; strings should be centralized (spec `.scratch/server-rewrite/spec.md:163,
  169-172`).
- The old GUI has **no** channels/sample-rate controls and **no** log viewer; the new GUI must also
  keep channels/sample-rate CLI-only, and the log viewer is explicitly out of scope
  (`.scratch/server-rewrite/spec.md:199-210`).

---

## 4. Wire Protocol (frozen at v0.3.4)

### 4.1 Integers and framing

Old code writes native integers directly into asio buffers:
`cmd_t : uint32_t` (`network_manager.hpp:47-52`), `uint32_t size` (`network_manager.cpp:221`),
`int id` (`:234, 244`), UDP `int id` (`:331-333`). The released targets are x86_64 little-endian, so
the observed wire format is little-endian. The Rust rewrite must encode explicitly little-endian
(spec `.scratch/server-rewrite/spec.md:143`).

TCP has no length prefix around commands. Each command is exactly 4 bytes from the client. Only two
server responses carry extra data: `GET_FORMAT` (uint32 size + protobuf bytes) and `START_PLAY`
(int id). `HEARTBEAT` is 4 bytes only.

Accepted sockets get `TCP_NODELAY` (`network_manager.cpp:318-322`). The listener sets
`reuse_address(true)` and default backlog (`:154-157`).

### 4.2 Command values

| Name | Value (`uint32`) | Direction | Payload |
|---|---|---|---|
| `CMD_NONE` | 0 | — | never sent; receiving 0 closes the connection |
| `CMD_GET_FORMAT` | 1 | client → server, then server → client | request: none; response: `u32 size` + protobuf |
| `CMD_START_PLAY` | 2 | client → server, then server → client | request: none; response: `int id` (4 bytes, signed) |
| `CMD_HEARTBEAT` | 3 | both ways | none |

Source: `network_manager.hpp:47-52`; Android mirror `NetClient.kt:77-82`
(`CMD.entries` ordinal order), `NetworkIO.kt:39-56`.

Android sends commands as `Int` little-endian (`NetworkIO.kt:39-44`), so bytes are identical.

### 4.3 Byte-level example [run]

Observed on the wire for a default Linux capture (F32, 2 ch, 48 kHz):

```
C->S  01 00 00 00                                      GET_FORMAT
S->C  01 00 00 00  08 00 00 00  08 01 10 02 18 80 f7 02
       cmd=1        size=8       protobuf AudioFormat
C->S  02 00 00 00                                      START_PLAY
S->C  02 00 00 00  01 00 00 00                          cmd=2, id=1
C->S  (UDP) 01 00 00 00                                bind id=1 to server UDP port
S->C  (UDP) <raw PCM datagrams, max 1464 bytes>
S->C  03 00 00 00                                      heartbeat at t=3s, 6s, 9s, ...
C->S  03 00 00 00                                      heartbeat reply
```

Protobuf bytes `08 01 10 02 18 80 f7 02` decode as field 1 = 1, field 2 = 2, field 3 = 48000.

### 4.4 `AudioFormat` protobuf

`protos/client.proto:17-35`:

```proto
syntax = "proto3";
package io.github.mkckr0.audio_share_app.pb;
message AudioFormat {
  enum Encoding {
    ENCODING_INVALID = 0;
    ENCODING_PCM_FLOAT = 1;   // PCM encoding are all little endian
    ENCODING_PCM_8BIT = 2;
    ENCODING_PCM_16BIT = 3;
    ENCODING_PCM_24BIT = 4;
    ENCODING_PCM_32BIT = 5;
  }
  Encoding encoding = 1;
  int32 channels = 2;
  int32 sample_rate = 3;
}
```

- Field numbers, wire types, enum values must not change.
- proto3 omits zero-valued scalar fields, so an uninitialized format serializes to **0 bytes**
  (observed). The Android client's `AudioFormat.parseFrom(empty)` yields encoding=INVALID, channels=0,
  sample_rate=0.
- Negative `channels`/`sample_rate` would be encoded as 10-byte varints; not produced by the old
  server in normal paths.
- The Android app compiles this exact file from `../../protos` (`android-app/app/build.gradle.kts:80-81`).

### 4.5 `GET_FORMAT`

Server (`network_manager.cpp:219-232`):

- On receiving cmd 1, calls `audio_manager::get_format_binary()` = `_format->SerializeAsString()`
  (`audio_manager.cpp:24-27`).
- Responds with three buffers: cmd, `uint32 size`, protobuf bytes.
- No request payload is read.
- If the capture format is not known yet, the response is cmd=1, size=0 (observed during server
  startup before PipeWire negotiation, and permanently for an invalid endpoint). This is a race:
  the Android client immediately builds an `AudioTrack` from it and will use INVALID/0/0.

### 4.6 `START_PLAY`, session ids

Server (`network_manager.cpp:233-251, 352-366`):

- `add_playing_peer(peer)` inserts the TCP peer into `_playing_peer_list` and assigns
  `info->id = ++g_id`, where `g_id` is a function-static `int` shared by the whole process.
  IDs start at 1 and monotonically increase; they do not reset between clients.
- If the same TCP peer sends `START_PLAY` twice, `add_playing_peer` returns 0, the server logs
  `add_playing_peer repeat add ...` / `read_loop id error`, and closes the connection **without**
  sending a second response [run].
- Response: cmd 2 followed by the 4-byte id, no size field.
- `last_tick` is initialized to now.
- A `heartbeat_loop` coroutine is spawned for the peer only after a successful `START_PLAY`.
- The client rejects `id <= 0` (`NetClient.kt:152-155`).

### 4.7 Heartbeat and timeout

Server (`network_manager.cpp:266-304`; timeout constant `network_manager.hpp:90`):

- Timer resets to `3s` each iteration; after waking it:
  1. Exits if the peer socket is closed.
  2. Looks up the peer; if it is no longer in the playing list, closes the session.
  3. If `now - last_tick > 5s`, logs `<endpoint> timeout` and closes.
  4. Otherwise writes cmd 3 (4 bytes).
- `last_tick` is refreshed on **any** received cmd 3 for that peer (`network_manager.cpp:252-256`).
- Because the check runs just before each 3 s send, a client that never replies is closed at the
  **second** tick, about **6 s** after `START_PLAY`, not exactly 5 s. Observed: close at 6.005 s;
  heartbeats sent at 3.0, 6.0, 9.0, 12.0 s **[run]**.
- Only clients that have completed `START_PLAY` get heartbeats/timeouts. A TCP connection that only
  does `GET_FORMAT` can stay open indefinitely with no heartbeat **[run]**.
- Heartbeat cmd 3 received before `START_PLAY` is silently ignored (no reply, connection stays open)
  **[run]**.
- Unknown commands (including 0) close the connection with log `read_loop error cmd` and no response
  **[run]**.
- The server's protocol reader is a straightforward 4-byte `async_read` loop, so fragmented writes
  and multiple pipelined commands are handled **[run]**.
- Potential old bug: `read_loop` and `heartbeat_loop` can both write to the same socket concurrently
  if the client sends another command after `START_PLAY`; bytes could interleave. The shipped client
  does not do this.

Android client side (`NetClient.kt:166-186`):

- After `START_PLAY`, starts a loop that checks every 3 s: if more than 5 s have elapsed since the
  last server heartbeat, throws `heartbeat timeout` → error/retry.
- On receiving cmd 3, updates the tick and sends cmd 3 back.
- First check occurs at +3 s, so detection is also roughly +6 s.

### 4.8 UDP bind and broadcast

Server (`network_manager.cpp:328-341, 381-394, 396-425`):

- One UDP socket bound to the same host/port as TCP (`:166-174`).
- `accept_udp_loop` receives into a 4-byte `int id` from any source and calls `fill_udp_peer(id,
  source)`.
- `fill_udp_peer` searches `_playing_peer_list` by id; on match it sets/overwrites `udp_peer`.
  - Unknown id → log `fill_udp_peer no tcp peer id:<id> udp://<addr>`, no response, no crash [run].
  - A later bind for the same id **replaces** the destination (last bind wins). The old UDP socket
    stops receiving; the new one receives all subsequent datagrams [run].
  - A datagram of 0–3 bytes leaves the remaining bytes of `id` as initialized 0 (partial overwrite)
    [derived from `async_receive_from(buffer(&id,4))` semantics].
  - A datagram longer than 4 bytes is truncated to 4 bytes [derived].
- Before a client's UDP bind arrives, the peer is still in the playing list with a default-constructed
  `udp::endpoint` (`0.0.0.0:0`). Broadcast attempts to send to it; the completion handler ignores
  errors (`:421`). The client receives nothing until it binds [run: B got 0 packets before binding].
- Every capture callback calls `broadcast_audio_data(data, count, block_align)`.
  - `count <= 0` returns (note `count` is `size_t`, so only zero is caught).
  - Segment size:
    ```cpp
    constexpr int mtu = 1492;
    int max_seg_size = mtu - 20 - 8;          // 1464
    max_seg_size -= max_seg_size % block_align;
    ```
  - The buffer is split into consecutive chunks of `max_seg_size` (final chunk = remainder), each
    copied into its own vector.
  - It then posts one task to the network `io_context` which, for every segment, loops over **all**
    playing peers and calls `async_send_to(segment, peer->udp_peer)`. Errors are ignored.
  - There is no sequence number, no header, no reorder buffer, no retransmission, no pacing.
- Multiple clients: all playing peers receive the same segment sequence. Observed tails were
  byte-identical for two clients **[run]**.
- Datagrams are raw PCM in the capture/transfer format.

Edge cases:

- `block_align == 0` → `max_seg_size % 0` → integer division by zero → SIGFPE crash. Can occur if
  `process` runs before `param_changed` set `block_align`, or for an unsupported negotiated format
  **[derived]**.
- `block_align > 1464` → `max_seg_size` becomes 0 → the loop never advances `begin_pos` and appends
  zero-length segments forever (hang/OOM) **[derived]**.
- If `count` is not a multiple of `block_align`, the final datagram is not block-aligned; normal
  capture paths produce aligned counts.

### 4.9 Connection/session close

- `close_session` (`network_manager.cpp:343-350`): logs `close <remote>`, removes the peer from the
  playing list, `shutdown(both)`, `close()`.
- Closing a non-playing peer logs an extra `remove_playing_peer repeat remove tcp://...` error
  **[run]**.
- On server stop (`network_manager.cpp:183-194`): `_ioc->stop()`, join network thread, stop audio,
  clear playing peers, destroy UDP socket, reset io_context. There is **no protocol-level "bye"**;
  TCP connections are torn down when sockets/io_context are destroyed, so clients observe FIN/RST
  and reconnect.
- `as-cmd` has no graceful stop command or signal handler. SIGTERM terminates the process (exit 143
  observed); a connected Android client saw TCP EOF shortly after the signal, and UDP stopped
  **[run]**. MFC's Stop button invokes `stop_server()` and therefore the teardown above [code].
- If `stop_server()` is called while audio is still running on Linux, `audio_manager::stop()` joins a
  thread blocked in `pw_main_loop_run()` that never checks `_stopped` (see §5.2) — the old Linux core
  can hang on stop. New core must make stop cancelable (spec requirement).

### 4.10 Android v0.3.4 client behavior (acceptance contract)

From `NetClient.kt`, `NetworkIO.kt`, `AudioPlayer.kt`:

- TCP connect timeout: 3 s (`NetClient.kt:115-122`); error text `label_timeout` or
  `label_unresolved_address`.
- Flow per connection:
  1. TCP connect.
  2. Send `GET_FORMAT`; read 4-byte cmd (must be 1); read 4-byte size LE; read size bytes; parse
     protobuf.
  3. Create `AudioTrack` and wait for it before continuing (`:138-140`).
  4. Send `START_PLAY`; read cmd (must be 2); read 4-byte id LE; require id > 0.
  5. Bind UDP to the local IP of the TCP socket, ephemeral port; send 4-byte id LE to server
     host:port.
  6. Start heartbeat monitor/reply loops.
  7. Read raw UDP datagrams and `AudioTrack.write(..., WRITE_NON_BLOCKING)`.
- Any error cancels the client scope; `AudioPlayer` stops, waits 3 s (countdown), then reconnects
  (`AudioPlayer.kt:293-323`).
- The client never re-sends `GET_FORMAT` on the same connection, never sends arbitrary commands, and
  does not check UDP datagram alignment (it trusts the server).
- AudioTrack format mapping (`AudioPlayer.kt:191-223`):
  - FLOAT→`ENCODING_PCM_FLOAT`, 8BIT→`ENCODING_PCM_8BIT`, 16BIT→`ENCODING_PCM_16BIT`;
    24BIT→`ENCODING_PCM_24BIT_PACKED` and 32BIT→`ENCODING_PCM_32BIT` only on Android 12 (API 31)+,
    otherwise `ENCODING_INVALID`.
  - channels 1..8 map to mono/stereo/2.1/quad/5.0/5.1/6.1/7.1 masks; anything else INVALID.
  - Buffer size = `minBufferSize * bufferScale` (default scale 1); volume default 1; loudness
    enhancer default 0.
- Consequence: a new server must keep encoding enum values stable and must not send 24/32-bit PCM to
  pre-Android-12 clients expecting playback success; in practice the server sends whatever the
  endpoint/capture requests, and the client fails if it cannot create AudioTrack. Parity means
  preserving the same formats, not adding negotiation.

### 4.11 Protocol checklist for the rewrite

- [ ] `cmd`/`size`/`id` little-endian, fixed 4-byte.
- [ ] `GET_FORMAT` response exact structure; size = serialized length; empty format allowed only if
      old behavior is intentionally preserved.
- [ ] `START_PLAY` response cmd+id; ids > 0 and monotonically increasing per process.
- [ ] server heartbeat every 3 s; timeout after >5 s checked on the 3 s tick (close at ~6 s).
- [ ] heartbeat only after `START_PLAY`; pre-start connections not timed out.
- [ ] unknown/duplicate commands close the connection without a response.
- [ ] UDP bind = 4-byte LE id; last bind wins; unknown id ignored.
- [ ] MTU 1492 − 20 − 8 = 1464, aligned down by `block_align`.
- [ ] one capture callback → one or more datagrams; no header/seq; broadcast to all playing peers.
- [ ] no data to peers that have not sent `START_PLAY`.
- [ ] stop closes TCP/UDP, no protocol close message.
- [ ] protobuf field/enum values unchanged.

---

## 5. Audio capture semantics

### 5.1 Windows / WASAPI loopback [code]

Entry: `win32/audio_manager_impl.cpp:113-290`.

1. `CoInitializeEx(COINIT_MULTITHREADED)` in the audio manager constructor (`:101-104`).
2. Enumerate/choose endpoint:
   - `endpoint_id` empty or `"default"` → `GetDefaultAudioEndpoint(eRender, eConsole)`.
   - Otherwise `GetDevice(endpoint_id)`.
   - Any failure → `exit_on_failed` → error log + `exit(-1)`.
3. Open `IPropertyStore`, read `PKEY_Device_FriendlyName` for the log line; fallback name
   `"[Speakers]"`.
4. Activate `IAudioClient`; `GetMixFormat()`.
5. Start from the mix format (copy `WAVEFORMATEXTENSIBLE` when needed).
6. If encoding is invalid → log `invalid encoding`, return (capture thread ends; server keeps
   serving with whatever format was already published — on Windows nothing was published yet, so
   `GET_FORMAT` stays empty).
7. If encoding is explicitly requested:
   - Extensible: set `SubFormat` to IEEE_FLOAT for f32, PCM otherwise; non-extensible: set
     `wFormatTag` to IEEE_FLOAT/PCM.
   - Set `wBitsPerSample` to 32/8/16/24/32 for f32/s8/s16/s24/s32.
   - Extensible: set `Samples.wValidBitsPerSample` to the same value.
8. If `channels != 0`, override `nChannels`.
9. Recompute `nBlockAlign = wBitsPerSample * nChannels / 8`.
10. If `sample_rate != 0`, override `nSamplesPerSec`.
11. Recompute `nAvgBytesPerSec = nSamplesPerSec * nBlockAlign`.
12. `IsFormatSupported(AUDCLNT_SHAREMODE_SHARED, requested, &closest)`:
    - `AUDCLNT_E_UNSUPPORTED_FORMAT` → log `the capture format is not supported`, return (no audio,
      server lives on).
    - `S_FALSE` → log `the specified capture format is not supported, using a similar format` and
      use `closest`.
    - `S_OK` → use requested.
    - other failure → `exit(-1)`.
13. `set_format(_format, final_format)` (`:352-381`) maps:
    - PCM 8/16/24/32 bits → `ENCODING_PCM_8BIT/16BIT/24BIT/32BIT`.
    - IEEE float → `ENCODING_PCM_FLOAT`.
    - Anything else (unknown bits/subformat) → `ENCODING_INVALID`.
    - Sets channels and sample rate.
    - Note: it does not inspect `wValidBitsPerSample`; a 24-valid-in-32-container format is reported
      as 32-bit. Extensible PCM with other bit depths yields INVALID.
14. `Initialize(SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, hnsRequestedDuration = 1 s, 0, format,
    nullptr)` (`:222-223`). This is the only place with a **1 s buffer**.
15. `GetBufferSize` (frames) is logged; `GetService(IAudioCaptureClient)`; `Start()`.
16. Poll loop at `GetDevicePeriod` (default period, converted to ms):
    ```cpp
    timer.expires_at(timer.expiry() + duration); timer.wait(ec);
    GetNextPacketSize(&n);
    if (n == 0) continue;
    GetBuffer(&pData, &numFrames, &flags, ...);
    broadcast_audio_data(pData, numFrames * nBlockAlign, nBlockAlign);
    ReleaseBuffer(numFrames);
    ```
    `AUDCLNT_BUFFERFLAGS_SILENT` is ignored: silent packets are still broadcast (zeros).
17. `_stopped` is checked at the bottom of the loop (`while (!_stopped)`), so stop takes effect after
    the next period.
18. Errors in the loop call `exit_on_failed` → process exit, not a recoverable error.

Consequences for parity:

- Default Windows format is the endpoint mix format (usually F32 2ch 48 kHz, but can be 16/24-bit).
- Requested encoding/channels/rate may be replaced by an OS closest match; the reported
  `AudioFormat` must be the **actual** capture format, not the requested one.
- The 1 s buffer and device-period polling are Windows-only; Linux uses PipeWire defaults.
- `exit_on_failed` on Windows exits the process on many capture errors; the rewrite must return
  `Result` instead (spec requirement) but keep observable startup behavior meaningful.

### 5.2 Linux / PipeWire [code + run]

Entry: `linux/audio_manager_impl.cpp:100-348`.

- Construction (`:74-96`): `pw_init`, `pw_main_loop_new`, `pw_context_new`, `pw_context_connect`.
  Null checks are absent; no daemon → later null dereference/segfault (observed for `-l`, `-b`).
- Endpoint selection:
  - `endpoint_id` empty or `"default"` → `get_default_endpoint()`.
  - `get_endpoint_list()` enumerates registry globals of type `PipeWire:Interface:Node` whose
    `media.class` is exactly `Audio/Sink` (`:350-401`).
    - ID = `object.serial` string.
    - Name = `node.nick` if present, else `node.description`, else `node.name`.
    - Default index = first node with the highest `priority.session` (missing → -1, strict `>`), but
      this index is never used by the caller.
  - `get_default_endpoint()` (`:403-443`) repeats the registry scan and returns the `object.serial`
    of the first node with the highest `priority.session`; empty string if none. This is **not** the
    same as PipeWire's default sink metadata; it is purely priority.session.
  - If the endpoint list is empty → log `audio endpoint list is empty`, return (server keeps
    serving, no format).
  - If the requested endpoint is not in the list → log `selected audio endpoint is not in list`,
    return (same).
- Requested capture format (`:123-152`):

  | CLI encoding | SPA format |
  |---|---|
  | `default`, `f32` | `SPA_AUDIO_FORMAT_F32_LE` |
  | `s8` | `SPA_AUDIO_FORMAT_U8` |
  | `s16` | `SPA_AUDIO_FORMAT_S16_LE` |
  | `s24` | `SPA_AUDIO_FORMAT_S24_LE` |
  | `s32` | `SPA_AUDIO_FORMAT_S32_LE` |
  | invalid/unknown | `SPA_AUDIO_FORMAT_UNKNOWN` |

  `channels`/`sample_rate` of 0 mean "don't care"; otherwise passed through.
- Stream creation (`:316-343`):
  - Properties: `media.type=Audio`, `media.category=Capture`, `media.role=Music`,
    `application.name="Audio Share Server"`, `node.name="Audio Share Server"`,
    `target.object=<selected serial>`.
  - `pw_stream_connect(INPUT, PW_ID_ANY, AUTOCONNECT | MAP_BUFFERS | RT_PROCESS, [EnumFormat], 1)`.
  - This target-object capture of a sink is the monitor path; no explicit
    `stream.capture.sink` property is set. README documents monitor-based capture.
- Negotiated format handling (`param_changed`, `:193-291`):
  - Parses `SPA_PARAM_Format`, requires audio/raw.
  - Encoding mapping:
    - `F32_LE` → `ENCODING_PCM_FLOAT` (1)
    - `S8` → `ENCODING_PCM_8BIT` (2)
    - `S16_LE` → `ENCODING_PCM_16BIT` (3)
    - `S24_LE` → `ENCODING_PCM_24BIT` (4)
    - `S32_LE` → `ENCODING_PCM_32BIT` (5)
    - any other negotiated format → `ENCODING_INVALID` + log `the capture format is not supported`
      + `exit(EXIT_FAILURE)`.
  - **Old bug confirmed by run:** requesting `s8` asks for `U8`; PipeWire negotiates 258 (`U8`);
    the switch has no `U8` case → process exits 1. Therefore `as-cmd --encoding=s8` is unusable on
    Linux v0.3.4.
  - `U8` is nevertheless in the `bits_per_sample` switch (8 bits), as are S16/U16/S24/U24/S32/U32/
    F32_LE/F32_BE/F32P; this only matters if the encoding switch stops exiting.
  - `block_align = bits_per_sample/8 * channels`; logged. If bits is unknown → 0 (→ later crash in
    segmentation).
  - Sets channels and sample rate from the negotiated raw info.
- `process` callback (`:292-313`): dequeue buffer; if null, log `out of buffers` and return; if data
  null, return **without queueing the buffer** (old leak); otherwise
  `broadcast_audio_data(data + chunk->offset, chunk->size, block_align)` and queue the buffer back.
  No explicit buffer sizing: PipeWire decides quantum/buffers. Observed callback payload was 8192
  bytes (1024 frames × 8 bytes) every ~21 ms at 48 kHz stereo F32, producing 5×1464 + 872-byte
  datagrams per callback **[run]**.
- Modeled format observed on a null sink: F32_LE, 2 ch, 48 kHz; `block_align 8`; `AudioFormat` bytes
  `08 01 10 02 18 80 f7 02` **[run]**.
- `--channels=1` works (block_align 4); `--sample-rate=44100` works (44100 reported); both together
  work **[run]**.
- `--channels=-1` crashes (SIGSEGV, 139) during negotiation **[run]**; treat any non-positive or
  absurd channel/rate as an error in the new implementation.
- `--encoding=s16/s24/s32` work and report 16/24/32-bit PCM **[run]**.
- `--encoding=bogus` negotiates something (SPA format 518 in the probe), logs unsupported, exits 1
  **[run]**.
- `--endpoint=nosuch` keeps the server alive with an empty format forever **[run]**.

### 5.3 Shared `AudioFormat` and races in the old code

- `audio_manager::_format` is a bare `std::shared_ptr<AudioFormat>` (`audio_manager.hpp:95`),
  allocated in the constructor (`audio_manager.cpp:3-6`).
- Capture side mutates the same object:
  - Linux: `param_changed` calls `set_encoding`, `set_channels`, `set_sample_rate` from the
    PipeWire main-loop thread (`linux/audio_manager_impl.cpp:229-253, 287-289`).
  - Windows: `set_format` writes all fields from the capture thread (`win32/...:213, 352-381`).
- Network side reads it from the asio network thread: `get_format_binary()` calls
  `_format->SerializeAsString()` (`audio_manager.cpp:24-27`) from `read_loop`
  (`network_manager.cpp:220`).
- There is no mutex, atomic, or snapshot. This is a formal C++ data race; on Linux the format can
  also be renegotiated at runtime, so a client can observe a torn format (e.g., new encoding with old
  channels). The observed empty response before negotiation is a visible symptom.
- New requirement: publish a consistent `AudioFormat` snapshot (lock, `RwLock`, or atomic swap) and
  define `GET_FORMAT` behavior before the first capture format is known. Never return a torn struct.

### 5.4 Transfer format

Capture format and transfer format are identical in v0.3.4 (`README.md:100-135`); there is no
resampling/encoding conversion in the server beyond what WASAPI/PipeWire negotiation does. The
`AudioFormat` sent to the client is the actual capture format.

---

## 6. Parity checklist

Legend for verification: **U** unit, **I** scripted integration (wire protocol / process), **C** CLI
process assertions, **M** manual GUI, **A** audio A/B/soak. "Old evidence" cites the primary source.

### 6.1 CLI

| # | Observable behavior | Old evidence | New requirement | Verify |
|---|---|---|---|---|
| C1 | Binary/artifact is named `as-cmd` (`as-cmd.exe` on Windows) | `CMakeLists.txt:14,42-46` | keep | C |
| C2 | `-h/--help` prints examples + options, exit 0, stdout | `main.cpp:15-21,40-43` [run] | same flags; text may improve per spec | C |
| C3 | `-v/--version` prints bin name, version, homepage, exit 0 | `main.cpp:45-48` [run] | keep layout/values (version becomes 0.4.0) | C |
| C4 | No args prints help to stderr, exit 0 | `main.cpp:111-112` [run] | keep | C |
| C5 | Unknown option → `Option ‘x’ does not exist` + help, exit 1 | cxxopts catch `main.cpp:113-116` [run] | readable error, nonzero exit | C |
| C6 | Missing option value → `Option ‘e’ is missing an argument` + help, exit 1 | [run] | same | C |
| C7 | Non-integer value → `Argument ‘x’ failed to parse` + help, exit 1 | [run] | same intent | C |
| C8 | `-l/--list-endpoint` prints `endpoint list:`/`\t{* }id: {:<4} name:`/`total: N`, exit 0 | `main.cpp:54-65` [run] | keep format and default marker | C/I |
| C9 | `-l` default marker = default endpoint (Linux highest priority.session; Windows `GetDefaultAudioEndpoint`) | `main.cpp:56-61`; `linux/...:403-443`; `win32/...:333-350` [run] | same semantics | C/I |
| C10 | `-l` with no endpoints prints `total: 0`, exit 0 | [run] | keep | C |
| C11 | `-l` when backend unavailable returns readable error, not crash | old segfault [run] | **intentional improvement**: nonzero exit + message | C |
| C12 | `--list-encoding` prints six lines in fixed order (default/f32/s8/s16/s24/s32), exit 0 | `main.cpp:67-81` [run] | keep | C |
| C13 | Flag precedence help > version > verbose > list-endpoint > list-encoding > bind | `main.cpp:39-109` | keep | C |
| C14 | Bare `-b`/`--bind` uses default LAN address:65530 | `main.cpp:27,83-92` [run] | keep | C |
| C15 | `--bind=host:port` uses host/port; default port 65530 if no colon | `main.cpp:83-92` [run] | keep | C/I |
| C16 | Port cast wraps modulo 65536 | `(uint16_t)std::stoi` `main.cpp:91` [run] | decide: wrap (parity) vs validate; document | C |
| C17 | Empty/invalid host → `Invalid argument`, exit 1 | `ip::make_address` `network_manager.cpp:152` [run] | readable error, exit 1 | C |
| C18 | Unparseable port → `stoi`, exit 1 | `main.cpp:91` [run] | readable error, exit 1 | C |
| C19 | Privileged port/bind failure → message, exit 1 | asio exception `main.cpp:113-119` [run] | same | C |
| C20 | Default bind host = first private IPv4 (10/8, 172.16/16, 192.168/16), else first IPv4; loopback excluded | `network_manager.cpp:53-146` [run] | keep selection semantics; fix or document 172-range bug | U/I |
| C21 | `-e/--endpoint` default `"default"`; unknown ID accepted but yields no audio | `main.cpp:28`; `linux/...:102-120` [run] | keep CLI acceptance; return actionable runtime error/log | I/M |
| C22 | `--encoding` default `"default"`; unknown string accepted as invalid | `main.cpp:29`; `audio_manager.hpp:51-70` [run] | decide: reject unknown value or preserve; document | C |
| C23 | `--channels`/`--sample-rate` default 0 = device default; space or `=` value syntax works | `main.cpp:31-32` [run] | keep | C/I |
| C24 | Negative channels/rate produce an error, not a crash | old SIGSEGV [run] | **intentional improvement** | C/I |
| C25 | `-V/--verbose` raises logging to trace | `main.cpp:50-52` | keep equivalent behavior | C |
| C26 | `-b value` (space) ignores the value; only `--bind=value` works | cxxopts implicit value [run] | decide: preserve quirk or accept both; document (accepting both is script-safe) | C |
| C27 | as-cmd writes logs to stdout, not `server.log` | no file sink in `main.cpp` [run] | spec asks for `server.log`; **decision needed** | C |
| C28 | SIGTERM/SIGINT terminates and closes sockets | default signal handling [run] | graceful stop with same client-visible close | I |
| C29 | Exit codes are meaningful: 0 success/list/help/version; 1 parse/bind errors; 139 crashes are bugs | [run] | no crashes; documented codes | C |

### 6.2 Configuration

| # | Observable behavior | Old evidence | New requirement | Verify |
|---|---|---|---|---|
| F1 | File is `<exe_dir>/config.ini` | `AudioShareServer.cpp:61-62` | keep | U/M |
| F2 | Sections/keys/case exactly `App.configVersion/language/Running`, `App Settings.AutoRun/WhenAppStart/WhenClose/AutoUpdate`, `Network.host/port`, `Capture.endpoint/encoding` | §2.2 citations | read all; case-insensitive read; preserve spelling | U |
| F3 | configVersion current value 1; missing/other → old code deletes whole file | `AudioShareServer.cpp:109-116` | read v0.3.4 files and migrate keys; report outcome; do not silently lose settings | U/M |
| F4 | `language`: `default`/`en-US`/`zh-CN`; applied at startup only | `AudioShareServer.cpp:118-124`; `CAppSettingsTabPanel.cpp:338-342` | keep values and startup application | M |
| F5 | `WhenAppStart`: 0 nothing, 1 start, 2 start iff `App.Running` | `CServerTabPanel.cpp:71-74` | keep | U/M |
| F6 | `WhenClose`: 0 exit, nonzero hide-to-tray | `CMainDialog.cpp:224-233` | keep semantics | M |
| F7 | `AutoRun` default false; registry value `AudioShareServer` = `<exe> /hide` under HKCU Run; false deletes it | `CAppSettingsTabPanel.cpp:214-226` | keep Windows command exactly; Linux uses XDG autostart | M |
| F8 | `AutoUpdate` default false; 3-hour timer only when true | `CAppSettingsTabPanel.cpp:26,91-94,233-243` | keep interval/default | M |
| F9 | `Network.host` default = default LAN address; `port` default `"65530"` stored as string | `CServerTabPanel.cpp:91,97` | keep keys/defaults | U/M |
| F10 | `Capture.endpoint` default `"default"`; missing ID rewritten to `"default"` on Reset | `CServerTabPanel.cpp:119-130` | keep or improve with message | U/M |
| F11 | `Capture.encoding` int enum 0/2/3/4/5/6 | `CServerTabPanel.cpp:137-161`; `audio_manager.hpp:41-49` | keep mapping | U |
| F12 | Host/port/endpoint/encoding persist only after a successful Start; `App.Running` on start/stop | `CServerTabPanel.cpp:215-219,233` | keep observable persistence timing (or persist safely on change if spec allows) | U/M |
| F13 | Settings-tab changes persist immediately | `CAppSettingsTabPanel.cpp:121-255,338-342` | keep | M |
| F14 | Reset reloads controls from config/defaults, not factory reset | `CServerTabPanel.cpp:81-163` | keep meaning (spec says "Reset 恢复默认并写回"; clarify) | M |
| F15 | Corrupt/unknown encoding value falls back to Default but old code corrupts `Capture.endpoint` | `CServerTabPanel.cpp:158-161` | do not reproduce corruption; write correct key/default | U |
| F16 | Writes must be atomic to avoid half-written files | old MFC API has no atomicity guarantee | spec requires atomic write | U |

### 6.3 MFC GUI / new GUI behavior

| # | Observable behavior | Old evidence | New requirement | Verify |
|---|---|---|---|---|
| G1 | Tray-first, single window; status + Start/Stop primary | spec prototype A; old is two tabs | new layout per prototype A | M |
| G2 | Tray icon tooltip "Audio Share Server"; left click shows; right click menu | `CMainDialog.cpp:78-118,247-269` | keep tray/menu; Linux right-click only | M |
| G3 | Tray menu Show / Start Server (→Stop Server) / Exit; all enabled; dynamic label | `AudioShareServer.rc:312-322`; `CMainDialog.cpp:315-325` | keep items and dynamic label | M |
| G4 | Balloon on update; click opens release page | `CMainDialog.cpp:102-118,264-266` | keep | M |
| G5 | Close button: Exit vs Minimize to tray per `WhenClose` | `CMainDialog.cpp:224-233` | keep both options and default (Exit) | M |
| G6 | Startup: Do nothing / Start server / Keep last state | `CServerTabPanel.cpp:71-74` | keep three options and `App.Running` rule | M |
| G7 | Hide and Exit commands | `CMainDialog.cpp:228-231,327-330`; settings buttons | keep both | M |
| G8 | Single instance: second launch exits 0 and shows existing window | `AudioShareServer.cpp:188-218` | keep observable behavior; fix startup race | M |
| G9 | `/hide` starts without showing window; tray still present | `AudioShareServer.cpp:130-131`; `CMainDialog.cpp:181` | support `/hide`; decide `--hide` | M |
| G10 | About shows name, version, copyright, home page link | `CAboutDialog.cpp:82-99`; rc version resource | show version/homepage/license per spec | M |
| G11 | Language combo Default/English/Simplified Chinese; persisted; applied next launch | `CAppSettingsTabPanel.cpp:96-113,338-342` | keep values and startup application | M |
| G12 | Server tab Host/Port/Endpoint/Encoding/Start/Stop/Reset/Sound Panel | `AudioShareServer.rc:116-135` | keep equivalent controls (GUI only endpoint+encoding, per spec) | M |
| G13 | Running disables Host/Port/Endpoint/Reset/Encoding/Sound Panel | `CServerTabPanel.cpp:238-246` | keep or explicitly allow live edits only when stopped | M |
| G14 | Start/Stop synchronous (UI blocks) | `CServerTabPanel.cpp:165-236` | spec: async, non-blocking | M |
| G15 | Start failures shown as readable message box; controls restored | `CServerTabPanel.cpp:196-208` | keep readable errors, no process crash | M |
| G16 | No endpoint / no selection message boxes | `CServerTabPanel.cpp:174-182` | keep equivalent validation | M |
| G17 | Endpoint "Default" item first, then endpoint friendly names | `CServerTabPanel.cpp:110-117` | keep | M |
| G18 | Encoding labels/default order | `CServerTabPanel.cpp:137-144` | keep | M |
| G19 | Reset reloads addresses/endpoints/config | `CServerTabPanel.cpp:81-163` | keep | M |
| G20 | Repair Firewall (Windows): elevated, app-scoped TCP+UDP allow rules, current profiles minus Public, names `Audio Share Server(TCP-In/UDP-In)` | `CAppSettingsTabPanel.cpp:128-212` | keep behavior and rule names; Linux hides entry and shows explanation link | M |
| G21 | Sound Panel launches `control.exe /name Microsoft.Sound` | `CServerTabPanel.cpp:249-263` | Windows only; equivalent Linux explanation | M |
| G22 | Update check: URL, 3 h interval, tag/version comparison, balloon, click URL, manual "No Update" box, auto silent | `CAppSettingsTabPanel.cpp:257-342`; `util.cpp:7-58` | keep all observable behavior; no auto-download | U/M |
| G23 | About fields come from build version | `CAboutDialog.cpp:27-64`; rc `172-201` | use 0.4.0 build metadata | M |
| G24 | `server.log` in exe dir (GUI only) | `AudioShareServer.cpp:54-58` | keep for GUI; no log viewer | M |
| G25 | Config migration before UI shows | `AudioShareServer.cpp:109-116` | migrate safely with clear result | M |

### 6.4 Wire protocol

| # | Observable behavior | Old evidence | New requirement | Verify |
|---|---|---|---|---|
| W1 | All control integers 4 bytes little-endian | native writes, x86 targets; `network_manager.cpp:210,221-226,234,244,331-333` | explicit LE | U/I |
| W2 | Commands: 0 none, 1 get_format, 2 start_play, 3 heartbeat | `network_manager.hpp:47-52`; `NetClient.kt:77-82` | exact values | U/I |
| W3 | Client sends bare `u32 cmd` with no length | `read_loop` `network_manager.cpp:206-264`; `NetworkIO.kt:39-44` | keep | I |
| W4 | GET_FORMAT response = cmd + u32 size + protobuf | `network_manager.cpp:219-232` [run] | exact | I |
| W5 | START_PLAY response = cmd + int id | `network_manager.cpp:233-251` [run] | exact | I |
| W6 | Heartbeat = bare u32 3, both directions | `network_manager.cpp:295-301`; `NetClient.kt:177-186` [run] | exact | I |
| W7 | Heartbeat every 3 s after START_PLAY | `network_manager.cpp:271-273` [run] | exact | I |
| W8 | Timeout when >5 s without client heartbeat, checked on 3 s tick → close ~6 s | `network_manager.cpp:289-293`; `network_manager.hpp:90` [run] | keep observable timing | I |
| W9 | Unknown command closes connection, no response | `network_manager.cpp:257-260` [run] | keep | I |
| W10 | Duplicate START_PLAY closes connection, no second response | `network_manager.cpp:354-357,235-239` [run] | keep | I |
| W11 | Ids start at 1, process-wide monotonic | `network_manager.cpp:360-362` [run] | keep | I |
| W12 | Idle pre-START_PLAY TCP connection not timed out | heartbeat loop only spawned on START_PLAY [run] | keep | I |
| W13 | UDP bind = 4-byte LE int id | `network_manager.cpp:330-339`; `NetClient.kt:189-191` [run] | exact | I |
| W14 | Unknown UDP id ignored with log | `network_manager.cpp:387-390` [run] | keep | I |
| W15 | Re-binding same id redirects audio (last wins) | `network_manager.cpp:392` [run] | keep | I |
| W16 | Audio only after START_PLAY; unbound clients get nothing | `_playing_peer_list` + default endpoint [run] | keep | I |
| W17 | MTU 1492; payload max 1464; aligned down by block_align | `network_manager.cpp:404-406` [run] | exact | U/I |
| W18 | One capture callback split into consecutive datagrams; final remainder | `network_manager.cpp:408-416` [run] | keep | I |
| W19 | All playing peers receive identical datagrams | `network_manager.cpp:418-424` [run] | keep | I |
| W20 | No sequencing/header/reorder/retransmit | no such code | keep absent | I |
| W21 | block_align 0 must not crash | old `% 0` SIGFPE risk | new error handling | U |
| W22 | block_align > 1464 must not hang | old infinite loop risk | new reject/validate | U |
| W23 | Stop closes TCP/UDP without a protocol close message | `stop_server` teardown; SIGTERM [run] | keep client-visible reconnect behavior | I |
| W24 | AudioFormat field numbers 1/2/3 and enum 0..5 | `protos/client.proto:21-35` | exact | U/I |
| W25 | Empty format serializes to size 0; client sees INVALID/0/0 | old race [run] | define safe behavior; do not break client | U/I |
| W26 | Concurrent GET_FORMAT/START_PLAY on separate connections supported | accept loop + per-peer sessions [run] | keep | I |
| W27 | TCP_NODELAY on accepted sockets | `network_manager.cpp:318-322` | keep | I |
| W28 | GET_FORMAT can be repeated on one connection | read loop [run] | keep | I |
| W29 | Fragmented and pipelined commands handled | asio async_read [run] | keep | I |
| W30 | Client 3 s TCP connect timeout and 3 s reconnect delay preserved (client side) | `NetClient.kt:115-122`; `AudioPlayer.kt:293-323` | server must accept within that window | I |

### 6.5 Audio

| # | Observable behavior | Old evidence | New requirement | Verify |
|---|---|---|---|---|
| A1 | Windows endpoints = active render endpoints, IMMDevice ID + friendly name | `win32/...:292-330,383-392` | keep | M/A |
| A2 | Windows default endpoint = `GetDefaultAudioEndpoint(eRender, eConsole)` | `win32/...:122-124,333-350` | keep | M/A |
| A3 | Windows default capture = endpoint mix format | `win32/...:140-150` | keep | A |
| A4 | Requested f32/s8/s16/s24/s32, channels, rate; closest-match fallback on S_FALSE | `win32/...:152-211` | keep | A |
| A5 | Unsupported requested format (`AUDCLNT_E_UNSUPPORTED_FORMAT`) results in no audio, not a crash | `win32/...:198-201` | return error; keep app alive | A |
| A6 | 1 s shared-mode loopback buffer; poll at device period | `win32/...:218-240,253-289` | keep buffer/poll semantics | A |
| A7 | Silent packets broadcast as zeros | flags ignored `win32/...:272-278` | keep unless A/B proves otherwise | A |
| A8 | Reported format = actual negotiated capture format | `set_format` after `IsFormatSupported` | keep | A |
| A9 | Linux endpoints = `Audio/Sink` nodes; ID = object.serial; name = nick/description/name | `linux/...:350-401` [run] | keep | I/M |
| A10 | Linux default endpoint = highest `priority.session`; missing = -1; first wins ties | `linux/...:403-443` [run] | keep | I/U |
| A11 | Linux target-object capture of the sink (monitor) | `linux/...:316-343` [run] | keep | A |
| A12 | Linux default encoding F32_LE | `linux/...:124-128` [run] | keep | I/A |
| A13 | Linux mapping s8→U8, s16→S16_LE, s24→S24_LE, s32→S32_LE | `linux/...:129-140` [run] | keep request mapping; **fix U8 result mapping** so s8 works | I/A |
| A14 | Linux negotiated formats map to proto encodings 1/2/3/4/5 | `linux/...:229-253` [run] | keep for F32/S8/S16/S24/S32 | I |
| A15 | Unsupported negotiated format ends capture with an error | old `exit(1)` [run] | return error, keep server alive | I |
| A16 | block_align = bytes/sample × channels | `linux/...:254-289`; `win32/...:275-278` [run] | keep | U/I |
| A17 | Linux callback data offset/size used as-is | `linux/...:307-310` [run] | keep | A |
| A18 | No resampling/conversion in server | README; no code | keep | A |
| A19 | Format read/publish is race-free | old race (see §5.3) | required lock/snapshot | U/I |
| A20 | Stop is cancelable and does not hang | old Linux stop joins a never-ending loop | required | U/I |
| A21 | Endpoint hot-plug/sleep recovery | not implemented in old core | required by acceptance soak (ticket 15) | A |
| A22 | 8-hour soak without crash/leak/glitch regression | not tested in old repo | required | A |

### 6.6 Build, artifacts, release

| # | Observable behavior | Old evidence | New requirement | Verify |
|---|---|---|---|---|
| R1 | Windows GUI artifact `AudioShareServer.exe` | `vcxproj:79-97`; `release.yml:47-79` | keep name | release inspection |
| R2 | Linux CLI artifact `audio-share-server-cmd-linux.tar.gz` (+ `.sha256`) | `pack.sh:9-18`; `release.yml:81-139` | keep name | release inspection |
| R3 | Windows CLI artifact `audio-share-server-cmd-windows.zip` (+ `.sha256`) | `pack.sh:13-17` | keep name | release inspection |
| R4 | Archive contains directory `audio-share-server-cmd` + `LICENSE` | `pack.sh:5-7` | keep internal layout if practical | release inspection |
| R5 | Version tag/title `v<version>`; draft GitHub release; notes from changelog | `release.yml:150-162` | 0.4.0 big-bang; notes file for code 4000 | release inspection |
| R6 | Version scheme `MAJOR.MINOR.PATCH`; code = MAJOR×1e6 + MINOR×1e3 + PATCH | `scripts/get_version.sh`; `apply_version.sh` | keep scheme | U |
| R7 | Android APK `audio-share-app-<version>-release.apk` + `.sha256` | `settings.gradle.kts`; `build.gradle.kts:40`; `release.yml:37-45` | unchanged (no Android edits) | release inspection |
| R8 | VERSION/CMake/rc/Android versions kept in sync by `apply_version.sh` | `scripts/apply_version.sh` | update script for Rust workspace + Tauri | release inspection |
| R9 | SourceForge defaults updated by separate manual workflow | `.github/workflows/update_sourceforge.yml`; `scripts/setup_sourceforge_release.sh` | keep or document new equivalent | release inspection |
| R10 | Current workflows are manual-only; no push/PR CI; no automated tests | `.github/workflows/*.yml` | spec: CI on windows-latest + ubuntu-24.04, build/test/lint | CI inspection |
| R11 | No code signing, no MSI/NSIS, no Tauri updater | `release.yml`; spec out-of-scope | keep out of scope; Windows portable exe + WebView2 check; Linux deb/AppImage | release inspection |
| R12 | MFC build uses vcpkg/MSVC; core build uses vcpkg/CMake | `release.yml:47-139` | new build is cargo-only (spec) | CI inspection |

---

## 7. Artifacts, versioning, CI/release behavior

### 7.1 Current artifact names

| Artifact | Produced by | Name |
|---|---|---|
| Windows GUI | MSBuild project `TargetName` (`audio-share-server.vcxproj:79-97`) | `AudioShareServer.exe` |
| Windows CLI | `server-core/pack.sh:13-17` from CMake install tree | `audio-share-server-cmd-windows.zip` |
| Linux CLI | `server-core/pack.sh:9-12` | `audio-share-server-cmd-linux.tar.gz` |
| Checksums | `pack.sh:18` (`sha256sum file | tee file.sha256`) and `release.yml:37-38,70-72` | `<file>.sha256` |
| Android APK | Gradle, `rootProject.name="audio-share-app"`, `versionName=0.3.4`, release build type | `audio-share-app-0.3.4-release.apk` |
| Android checksum | `release.yml:37-38` | `audio-share-app-0.3.4-release.apk.sha256` |

The CLI archive contains a top-level directory `audio-share-server-cmd` (copy of the CMake install
tree) plus `LICENSE` (`pack.sh:5-7`). The installed binary is `as-cmd` (`CMakeLists.txt:14,42-46`).

### 7.2 Version scheme

- `VERSION` file: `0.3.4` (no newline).
- `scripts/get_version.sh -n` prints it raw; `-c` prints
  `1000000*major + 1000*minor + patch` (0.3.4 → 3004).
- `scripts/apply_version.sh` rewrites:
  - `android-app/app/build.gradle.kts` `versionName`/`versionCode`
  - `server-core/CMakeLists.txt` project `VERSION`
  - `server-mfc/.../AudioShareServer.rc` `FILEVERSION`, `PRODUCTVERSION`, `FileVersion`,
    `ProductVersion`
  - (The old `FileDescription` sed is commented out.)
- Release tag/name: `v$version` (e.g. `v0.3.4`).
- Release notes: `metadata/en-US/changelogs/<versionCode>.txt` (e.g. `3004.txt`); content is plain
  text, used verbatim via `gh release create ... -F notes`.
- The rewrite target is `0.4.0` with version code 4000 and changelog `metadata/en-US/changelogs/4000.txt`.

### 7.3 `.github/workflows/release.yml` (manual `workflow_dispatch`)

1. **`build_app`** (`ubuntu-latest`): checkout; JDK 17; decode `RELEASE_JKS`; create
   `keystore.properties`; `./gradlew :app:assembleRelease`; `signingReport`; sha256 APKs; upload
   artifact `app`.
2. **`build_server_mfc`** (`windows-latest`): checkout; setup MSBuild; run-vcpkg pinned to
   `10b7a178346f3f0abef60cecd5130e295afd8da4`; `vcpkg integrate install`;
   `vcpkg install asio protobuf spdlog[wchar] wil nlohmann-json`; `msbuild /m
   /p:Configuration=Release,Platform=x64`; sha256 exes; upload artifact `server-mfc`.
3. **`build_server_core`** (matrix `windows-latest`, `ubuntu-22.04`): install
   `libpipewire-0.3-dev` on Linux; vcpkg installs `asio protobuf cxxopts` + `spdlog` (+ `wil` on
   Windows); `lukka/run-cmake` with preset `windows-Release`/`linux-Release`; `bash pack.sh
   <platform>`; upload `server-core-windows`/`server-core-linux`.
4. **`create_release`** (`ubuntu-latest`, needs all three): download all artifacts into
   `release_bin`; notes = changelog file for `get_version.sh -c`; `gh release create "v$version" -d
   -F notes -t "v$version" release_bin/*/*` with `GITHUB_TOKEN`. The `-d` means **draft**.
5. There is no code-signing step for the server, no MSI/NSIS, no Tauri updater.

Other workflows:

- `.github/workflows/build-server-core-linux.yml`: manual-only Linux core build, same vcpkg/CMake/
  `pack.sh` flow on `ubuntu-22.04`.
- `.github/workflows/test.yml`: manual-only upload/download demo; **not** a test suite.
- `.github/workflows/update_sourceforge.yml`: manual-only; runs
  `scripts/setup_sourceforge_release.sh $SOURCEFORGE_RELEASE_API_KEY $(get_version.sh -n)`.
  It is **not** called by `release.yml`.

### 7.4 SourceForge step

`scripts/setup_sourceforge_release.sh <api_key> <version>` calls
`PUT https://sourceforge.net/projects/audio-share/files/<path>` with
`default=<folder>&api_key=<key>` for:

| Folder | Path |
|---|---|
| `windows` | `v<version>/AudioShareServer.exe` |
| `linux` | `v<version>/audio-share-server-cmd-linux.tar.gz` |
| `android` | `v<version>/audio-share-app-<version>-release.apk` |
| `others` | `v<version>/v<version>%20source%20code.tar.gz` |

### 7.5 CI/release requirements for the rewrite

- Workspace builds/tests with cargo only; no vcpkg/MSVC/MFC (spec `.scratch/server-rewrite/spec.md:132-136`).
- CI on `windows-latest` and `ubuntu-24.04` building, unit testing, clippy, rustfmt, and Tauri
  packaging (spec:178; ticket 01).
- Preserve `AudioShareServer.exe`, `audio-share-server-cmd-windows.zip`,
  `audio-share-server-cmd-linux.tar.gz` (spec:172); Linux additionally gets deb and AppImage
  (spec:176-178; ticket 14).
- Version 0.4.0; big-bang only; draft release + changelog `4000.txt` (spec; ticket 16).
- Android APK remains `audio-share-app-0.3.4-release.apk` in this round (no Android changes), and
  the `app` artifact/signing path stays.

---

## 8. Discrepancies, intentional deviations, and open decisions

1. **`server.log` in `as-cmd`**: ticket 07/spec say it should keep being written, but v0.3.4
   `as-cmd` never writes a file (stdout only). MFC writes `<exe_dir>/server.log`. Decide: add a file
   sink to the new CLI (likely desired) or correct the ticket. If added, define path (exe/CWD),
   truncation/rotation, and level.
2. **Linux `s8` is broken in v0.3.4** (`U8` negotiated but mapping switch exits 1). Spec/ticket 06
   explicitly require `s8 → U8` support. This is a behavior **fix**, not a parity regression; verify
   with the v0.3.4 Android client (which maps enum 2 to `ENCODING_PCM_8BIT`).
3. **Unknown `--encoding`** is accepted by the parser and fails later differently per platform.
   Decide: reject at CLI parse (cleaner, changes exit timing) or keep late failure. Document.
4. **`-b value` quirk**: old code silently ignores a space-separated bind value. Accepting both
   `-b value` and `--bind=value` in the new CLI changes behavior for scripts that accidentally used
   the space form. Recommend accepting both and documenting the difference; verify no script relies
   on the old silent default.
5. **Default host 172-range bug**: old selection only treats `172.16.x.x` as private. Preserve or fix
   (fix recommended; note user-visible default can change on networks using 172.17–172.31).
6. **`GET_FORMAT` before format known**: old returns an empty protobuf, which makes the Android client
   build an invalid AudioTrack. Decide: block until format is known (risk of client timeout), return
   INVALID consistently, or keep old behavior. Must be tested with the real APK.
7. **Config migration**: old behavior deletes the file on any `configVersion` mismatch. Spec requires
   reading/migrating v0.3.4 keys and reporting the result. Implement a non-destructive migration from
   version 1 (and treat missing version as 1 for existing files, with a clear result), while
   preserving all keys/defaults in §2.
8. **Process exit on capture errors**: Windows `exit_on_failed()` and Linux `exit(EXIT_FAILURE)`
   kill the process. New core must return `Result` so the GUI is not killed (spec requirement);
   observable CLI behavior (nonzero exit, message) should remain sensible.
9. **Linux stop hang**: old Linux capture never observes `_stopped` while inside
   `pw_main_loop_run`. New stop must be cancelable; this is a required fix.
10. **Torn AudioFormat**: old code races. New core must snapshot/publish atomically; define the
    pre-format response.
11. **`--hide` vs `/hide`**: old autostart writes `/hide`; parser matches flag text `hide`, so
    `/hide` and `-hide` work. Support `/hide` and `--hide` in the new GUI for compatibility, and keep
    the registry command as `<exe> /hide` to preserve upgrades.
12. **MFC Exit while running** does not call `stop_server`; the process exits with the server
    threads/sockets alive and relies on OS teardown. New GUI should stop the core gracefully on Exit
    while keeping client-visible close behavior.
13. **Firewall removal** code appears ineffective (removes by rule name = app path), but subsequent
    `Add` with the same rule names replaces rules. New implementation should remove by rule name
    and/or app name deliberately and report failures.
14. **Config file encoding** written by MFC (ANSI vs UTF-16) was not verified; read both, write a
    stable format, and keep a backup before migration.
15. **Sound Panel blocks the UI** until the Control Panel app closes; new GUI should launch it
    without waiting.
16. **Start/Stop are synchronous** in MFC and freeze the UI; new GUI must be async (spec).

---

## 9. Quick acceptance checklist for the new server (derived from the above)

- [ ] Run scripted wire tests against old and new `as-cmd` and diff control messages/PCM structure.
- [ ] `GET_FORMAT` returns `08 01 10 02 18 80 f7 02` (or the endpoint's real format) for default
      Linux capture.
- [ ] `START_PLAY` ids start at 1; UDP bind id works; multi-client receives identical datagrams.
- [ ] Heartbeats at 3 s; no-reply close at ~6 s; client close on stop.
- [ ] MTU datagrams ≤1464 and multiples of block_align; 8192-byte callback splits 5×1464 + 872.
- [ ] All five encodings, 1/2 channels, 44.1/48 kHz with the real v0.3.4 APK.
- [ ] `-l`, `--list-encoding`, `--version`, `--help`, error paths and exit codes.
- [ ] Config read/migration of a real v0.3.4 `config.ini` (including `configVersion=1`).
- [ ] GUI tray/menu/show-hide/close/start/stop/reset/autostart/single-instance/About/i18n/update.
- [ ] Windows firewall repair and sound panel; Linux equivalent explanation links.
- [ ] 8-hour soak, device hot-plug, sleep/resume, repeated start/stop.
