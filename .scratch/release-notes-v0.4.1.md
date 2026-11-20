0.4.1 is the first follow-up to the 0.4.0 Rust rewrite, and it is entirely about the Server GUI. The main window is rebuilt around the selected **Variant C ("focused minimal")** direction, Settings becomes one grouped scrolling page, and the app gains **System / Light / Dark themes** that apply immediately and persist across restarts. The Wire Protocol, capture pipeline, `as-cmd`, and the released v0.3.4 Android Client are untouched — **no new APK ships with this release; keep using the v0.3.4 release or F-Droid.**

**If you run the Server, we recommend upgrading.** Nothing below the UI changed, so 0.4.1 is a drop-in replacement for 0.4.0: replace the executable or package and keep your existing `config.ini`. The modernized GUI was accepted on Windows 11 and re-checked with the real v0.3.4 Android Client on an Android 12 phone; the Host-address regression found during that pass was fixed before release. (#2)

## Highlights

- **A focused main window.** One server status is the center of the screen: a static status dot, the status title, and the listening `host:port`. Start/Stop is a single circular primary action, Starting and Stopping disable it while a command is in flight, and Stop keeps a restrained danger style rather than a saturated red block. Endpoint and Encoding stay visible as quick settings; Host and Port live in a collapsed-by-default **Network configuration** disclosure; Reset moved here and resets all four Server fields while keeping application settings. (#2)
- **System, Light, and Dark themes.** The Appearance section offers a segmented System / Light / Dark control with `aria-pressed` state. Theme changes apply without a restart, System follows live OS color-scheme changes, and the chosen mode is written to `[App] theme` in `config.ini`. The persisted theme is applied before the shell mounts, so there is no wrong-theme flash on launch, and a build check keeps the two dark token blocks identical.
- **One scrolling, grouped Settings page.** Settings no longer duplicates Host, Port, Endpoint, or Encoding. It is a single scroll area with grouped disclosures: Appearance (always expanded, with theme and language), Startup & window, Updates, System, and About. Application settings stay editable while the Server runs, and the existing language-restart hint, platform maintenance entries, and About content are preserved.
- **Host suggestions return.** The Host field is editable free text again with a `datalist` of the machine's non-loopback IPv4 addresses, deduplicated and with the v0.3.4 default address first. This restores the behavior of the v0.3.4 MFC combo that the first modernized build had dropped, without changing how the default Host is chosen or how the server binds.
- **Accessible by design.** Character icons (`♪`, `⚙`, `←`) are replaced by a shared SVG icon component, focus rings are token-driven and visible in both themes, the status live region covers only the status content so unrelated updates are not re-announced, and non-essential transitions are disabled under `prefers-reduced-motion: reduce`. Every control stays reachable at the `380×560` minimum window size in both themes.
- **Nothing below the UI changed.** Only `config.rs` (for `[App] theme` and the Host address list), the GUI application-settings plumbing, and the frontend changed. The Wire Protocol files, capture pipeline, lifecycle code, and `as-cmd` have no diff; the v0.3.4 Android Client reconnected and played unchanged. Automated checks are green: `svelte-check` 0 errors / 0 warnings, the production Vite build emits a single JS/CSS pair with no prototype chunks, and the full `cargo test --workspace` suite passes.

## :warning: Breaking changes

None. The Wire Protocol, Capture Format, Server Core, `as-cmd` CLI, and the Android Client are unchanged. `[App] theme` is an optional new key: a missing or unknown value falls back to `system` without touching other fields, and existing `config.ini` files (including v0.3.4 migrations) are written with `theme=system` on the next save.

## Known limitations

- **No automatic capture recovery.** If the selected endpoint disappears (unplug, sink change) or the machine sleeps and resumes, capture stops while the process keeps running with its last negotiated format. Recovery is an operator Stop then Start in the GUI, or a restart of `as-cmd`. This is unchanged from 0.4.0.
- **Windows 10 is untested.** The modernization acceptance pass ran on Windows 11; Windows 10 is explicitly accepted as untested because no host was available. The Linux real-panel tray and a live Linux system-theme switch remain covered by the earlier baseline/Xvfb evidence rather than this pass.

## What's Changed

* config: persist `[App] theme` with `system`, `light`, and `dark` values ([68a0083](https://github.com/W0n9/audio-share/commit/68a0083))
* gui: carry theme through application settings and reset ([19d5714](https://github.com/W0n9/audio-share/commit/19d5714))
* gui: apply the persisted theme before the shell mounts ([31acff4](https://github.com/W0n9/audio-share/commit/31acff4))
* gui: add the Appearance theme picker to Settings ([d9231e8](https://github.com/W0n9/audio-share/commit/d9231e8))
* gui: rework `styles.css` into light and dark token sets ([6f77951](https://github.com/W0n9/audio-share/commit/6f77951))
* gui: add the shared SVG icon component ([36ee0db](https://github.com/W0n9/audio-share/commit/36ee0db))
* gui: rebuild the main view as the focused Variant C layout ([4615260](https://github.com/W0n9/audio-share/commit/4615260))
* gui: move the shell header to brand and settings entry ([d8f22c4](https://github.com/W0n9/audio-share/commit/d8f22c4))
* gui: scope the status live region to the status content ([8744fb1](https://github.com/W0n9/audio-share/commit/8744fb1))
* gui: rebuild Settings as grouped disclosure sections ([04f5036](https://github.com/W0n9/audio-share/commit/04f5036))
* gui: drop theme changes made while a save is in flight ([908227d](https://github.com/W0n9/audio-share/commit/908227d))
* build(gui): assert the dual dark token blocks stay identical ([fe5724c](https://github.com/W0n9/audio-share/commit/fe5724c))
* refactor(gui): dedupe theme buttons, error lines, and view switching ([9643828](https://github.com/W0n9/audio-share/commit/9643828))
* fix(gui): suggest local host addresses in the network configuration ([bb6e24b](https://github.com/W0n9/audio-share/commit/bb6e24b))
* docs: refresh README for the modernized GUI ([d5a024b](https://github.com/W0n9/audio-share/commit/d5a024b))

**Full Changelog**: https://github.com/W0n9/audio-share/compare/v0.4.0...v0.4.1

## Contributors

[W0n9](https://github.com/W0n9) — design, implementation, and acceptance.
