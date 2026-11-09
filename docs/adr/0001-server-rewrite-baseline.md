# Server rewrite targets upstream v0.3.4 with a big-bang replacement

The fork's `main` branch carries experimental UDP-header/reorder work, but it never became a released contract. The Server rewrite (core + GUI + CLI, in Rust) will therefore target the upstream v0.3.4 Wire Protocol and use the released v0.3.4 Android Client as its acceptance client. The replacement ships as a big-bang release: no public intermediate versions, though internal spikes and a parity harness gate the cutover. The `main` branch's protocol work is out of scope and may return as a separate effort after parity.

## Considered Options

- **Baseline `main`**: rejected because its UDP header/reorder contract is experimental and would require changing the Android Client in lockstep, making regressions unattributable.
- **Incremental sidecar first as the release strategy**: rejected in favour of a big-bang cutover; the final architecture is an in-process Rust core, not process separation.
- **Go instead of Rust**: rejected because Linux capture would fall back to the PulseAudio compatibility layer, losing native PipeWire endpoint semantics.

## Consequences

- Audio-glitch work from `main` is not fixed by the rewrite; it is deferred until after parity.
- The Wire Protocol implementation stays behind a seam so a future protocol revision remains possible.
- The v0.3.4 Android Client is a hard acceptance gate; no protocol changes ship in v0.4.0.
