# Server rewrite uses Rust, Tauri v2, and an in-process core

The Server rewrite is implemented in Rust: a `core` crate owns capture, the Wire Protocol, and the network layers, and both `as-cmd` and the Tauri v2 GUI link it in-process. The GUI uses Tauri v2 with a Svelte/Vite frontend, targeting Windows 10+ and Ubuntu 24.04+ (deb + AppImage). Rust has the cleanest audio path (`wasapi` loopback and the official PipeWire bindings), Tauri has the most mature packaging and tray story while keeping the app small, and in-process linking avoids a second IPC contract.

## Considered Options

- **Slint instead of Tauri**: rejected because it has no official installer bundler and its GPLv3 / commercial / royalty-free licensing needs legal review.
- **Go with Wails or Fyne**: rejected because Linux capture would fall back to the PulseAudio compatibility layer, losing native PipeWire endpoint semantics, and Wails on Windows requires cgo + MinGW.
- **Separate core process instead of an in-process crate**: rejected because crash isolation and remote control are not requirements, and a second IPC protocol is not worth the cost.

## Consequences

- Windows requires the system WebView2 runtime (Windows 10 1803+).
- The Linux GUI depends on WebKitGTK 4.1; deb and AppImage target Ubuntu 24.04+.
- The visual and information architecture is decided by a throwaway prototype; functional parity with the MFC GUI is fixed.
