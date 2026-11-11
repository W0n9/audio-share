# Audio Share

Audio Share shares a computer's audio to an Android phone over the network, so the phone becomes the speaker.

## Roles

**Audio Share**:
A system that captures a computer's audio and plays it on an Android phone over the network.
_Avoid_: audio cast, audio streamer

**Server**:
The computer-side half of Audio Share; captures an audio endpoint and broadcasts it over the network.
_Avoid_: backend, daemon

**Android Client**:
The phone-side app that connects to a Server and plays the received PCM stream.
_Avoid_: client, phone app

**Server Core**:
The capture and network component of the Server, usable headless through `as-cmd`.
_Avoid_: core, engine

**Server GUI**:
The desktop GUI that controls a Server Core instance on the same computer.
_Avoid_: MFC app, desktop client

**as-cmd**:
The command-line entry point of Server Core.
_Avoid_: CLI

## Appearance

**Theme**:
The Server GUI appearance preference; one of System, Light, or Dark, where System follows the operating system color scheme.
_Avoid_: color scheme, dark mode, skin

## Audio

**Endpoint**:
An audio device on the Server from which audio is captured; on Windows a render endpoint, on Linux an `Audio/Sink` node.
_Avoid_: device, source

**Capture Format**:
The PCM format the Server captures from an Endpoint.
_Avoid_: input format

**Transfer Format**:
The PCM format carried by the Wire Protocol; today it is identical to the Capture Format.
_Avoid_: stream format, output format

## Protocol

**Wire Protocol**:
The contract by which a Server and an Android Client exchange control messages and PCM audio.
_Avoid_: API, socket protocol
