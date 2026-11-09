# Windows maintenance: Firewall Repair + Sound Panel (ticket 11)

Status: **implemented and compile/logic validated on the Linux development
host.** A real UAC prompt, real `netsh` rule changes, and Android Client
connectivity through the repaired rules were **NOT EXECUTED** here: this host
has no Windows runtime and no MSVC linker. See "Residual manual steps".

## What landed

| Piece | Location |
|---|---|
| Pure profile/script/error logic (all hosts, unit-tested on Linux) | `gui/src-tauri/src/maintenance.rs` |
| Windows COM + ShellExecuteExW runtime (`#[cfg(windows)]`) | `gui/src-tauri/src/windows_maintenance.rs` |
| Commands `repair_firewall`, `open_sound_panel` | `gui/src-tauri/src/main.rs` |
| Frontend API / store / settings rows | `gui/src/lib/ipc.ts`, `gui/src/lib/store.svelte.ts`, `gui/src/views/SettingsView.svelte` |
| en-US / zh-CN messages | `gui/src/locales/{en-US,zh-CN}.json` |
| Target-gated dependency | `gui/src-tauri/Cargo.toml` (`windows 0.62.2` features) |

The commands are registered on every platform. On non-Windows they return the
localized `error.maintenance.unsupported` error; the settings rows are rendered
only when the bootstrap `platform` field is `windows`, so Linux never shows
them (ticket 12 owns the Linux documentation links).

## Firewall rules

Repair rebuilds two application-scoped inbound allow rules for the running
executable, matching the v0.3.4 MFC GUI:

| Property | TCP rule | UDP rule |
|---|---|---|
| Name | `Audio Share Server(TCP-In)` | `Audio Share Server(UDP-In)` |
| Description | same as the name | same as the name |
| Direction | inbound (`dir=in`) | inbound |
| Protocol | TCP (6) | UDP (17) |
| Action | allow | allow |
| Enabled | yes | yes |
| Program | full path of the running `AudioShareServer.exe` | same |
| Profiles | active profiles minus Public, unless Public is the only active one | same |

There is no port constraint: the server's configured port (default 65530) is
not embedded in the rule, exactly like the MFC implementation.

### Profile selection

`INetFwPolicy2::CurrentProfileTypes` returns a bitmask
(`DOMAIN=1`, `PRIVATE=2`, `PUBLIC=4`). It is read through COM, never by parsing
localized `netsh` text. `maintenance::netsh_profiles` then applies the MFC rule:
Public is removed unless it is the only active profile. An empty or
unknown-only mask falls back to all profiles so the rule is never inactive.

| Active mask | `profile=` |
|---|---|
| Domain | `domain` |
| Private | `private` |
| Public | `public` |
| Domain + Private | `domain,private` |
| Domain + Public | `domain` |
| Domain + Private + Public | `domain,private` |
| 0 / unknown bits | `domain,private,public` |

## Elevation and UAC behaviour

1. The profile mask is read unelevated in the command's blocking task.
2. A uniquely named `audio-share-firewall-<pid>-<nanos>.ps1` is created with
   `create_new` in the user's temp directory. It deletes both rules (ignoring
   a missing rule), then adds the TCP rule and the UDP rule. The executable
   path and rule names are PowerShell single-quoted literals (`'` doubled), so
   spaces, `$`, `%`, `&`, and embedded double quotes are inert.
3. `ShellExecuteExW` starts
   `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe` with the
   `runas` verb and `-NoProfile -NonInteractive -ExecutionPolicy Bypass -File
   "<script>"`. Windows shows the standard UAC consent prompt.
4. The command waits with `WaitForSingleObject(INFINITE)` and reads the exit
   code. It runs inside `tauri::async_runtime::spawn_blocking`, so neither the
   UAC prompt nor `netsh` blocks the UI thread.
5. The temporary script is deleted on every path after creation (RAII guard),
   including UAC cancellation and netsh failure.

Exit codes: `0` success; `10` the TCP add failed; `11` the UDP add failed.
Delete failures are ignored because the rules may simply not exist yet.

### Error mapping

| Situation | Typed error | Localized key |
|---|---|---|
| UAC dismissed (`ShellExecuteExW` HRESULT `0x800704C7`, Win32 1223) | `ElevationCancelled` | `error.firewall.cancelled` |
| Elevated process could not start | `ElevationFailed` | `error.firewall.elevation` |
| `netsh` exited non-zero | `NetshFailed { exit_code }` | `error.firewall.netsh` (includes the code and manual-repair hint) |
| `CurrentProfileTypes` failed | `ProfileQuery` | `error.firewall.profile` |
| Temp script/write failure | `TempScript` | `error.firewall.script` |
| Unusable/empty executable path | `AppPathUnavailable`/`UnsafePath` | `error.firewall.path` |
| Wait/exit-code query failed | `WaitFailed` | `error.firewall.wait` |
| Non-Windows command call | `UnsupportedPlatform` | `error.maintenance.unsupported` |

`netsh` output itself is localized, so it is never parsed: only exit codes are
used. The UI shows the success message `maintenance.firewallSuccess` or the
localized error in the settings view (`role="alert"` for errors).

## Sound Panel

`open_sound_panel` spawns `control.exe /name Microsoft.Sound` and does **not**
wait. The v0.3.4 MFC GUI called `WaitForSingleObject(..., INFINITE)` on the UI
thread and blocked until the sound app closed; the new command returns
immediately. Failures map to `error.soundPanel` with the OS message. On
success the UI shows `maintenance.soundPanelOpened`.

## Validation performed on this host (Linux, 2026-11-09)

- `cargo test -p audio-share-server-gui` — 72 passed, including 9 new
  maintenance tests: profile masks (domain/private/public combinations and
  fallbacks), script ordering/contents, PowerShell quoting (spaces, `'`, `"`),
  rejection of empty/multiline paths and invalid profile lists, argument
  quoting, UAC-cancel HRESULT mapping, exit-code mapping, and en-US/zh-CN
  messages.
- `cargo test --workspace`, `cargo clippy --workspace --all-targets --
  -D warnings` — pass.
- `cargo check`/`cargo clippy --target x86_64-pc-windows-msvc -p
  audio-share-server-gui -- -D warnings` — pass (only the known tauri-build
  GNU-compiler warnings from the Linux host).
- `npm run check` — 0 errors / 0 warnings; `npm run build` — pass.
- Linux gating: `bootstrap.platform` is `linux` and the settings rows are
  inside `{#if server.platform === "windows"}`; the Linux build renders no
  Repair Firewall or Sound Panel row.

## Manual test steps (require a Windows 10+ host)

1. Build the portable exe (`npm run build && npx tauri build --no-bundle`) and
   launch `AudioShareServer.exe` as a normal user from a path that contains a
   space (for example `C:\Program Files\Audio Share\`).
2. Open Settings and click **Repair**. Approve the UAC prompt. Verify success
   text appears and the UI stayed responsive during the prompt.
3. Verify the rules as an administrator:
   `netsh advfirewall firewall show rule name="Audio Share Server(TCP-In)"` and
   the UDP equivalent. Confirm `Direction: In`, `Action: Allow`, `Enabled: Yes`,
   `Program:` is the running exe, and the profile list matches the active
   profiles (Public excluded unless the machine is on a Public-only network).
4. Start the server, connect the released v0.3.4 Android Client to the shown
   address, and confirm audio plays (this is the acceptance check).
5. Put the active network on Public only and repair again: the rule must then
   include Public. Return to Domain/Private and repair: Public must be removed.
6. Click **Repair** again and dismiss the UAC prompt: a readable cancellation
   error must appear, not a crash or a silent no-op. No duplicate rules.
7. Force a netsh failure (for example, temporarily revoke write access to the
   firewall policy with a restrictive local policy, or block PowerShell) and
   confirm the readable `netsh exited with code …` message.
8. Click **Open sound panel**: the Windows Sound dialog opens and the Server
   GUI remains interactive (it must not wait for the dialog).
9. Check `%TEMP%` after each repair: no `audio-share-firewall-*.ps1` remains.
10. On Linux, open Settings: neither row is shown.

## Residual manual steps (NOT EXECUTED here)

- Real UAC consent and cancellation (`ERROR_CANCELLED` 1223) on Windows.
- Real `netsh` create/delete and the resulting rule properties/profiles.
- Android Client connectivity through the repaired rules (TCP control + UDP
  PCM), including the Public-only profile case.
- Sound dialog launch and the non-blocking behaviour on Windows.
- Windows visual/localization check of both new rows and all error messages.
