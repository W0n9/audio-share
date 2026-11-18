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

**Protocol Client**:
The player-role peer defined by Wire Protocol specifications, independent of
its implementation platform. The **Android Client** implements that role
today. Protocol documents write out this full term (abbreviated **Client**
inside a document that defines the abbreviation explicitly); they do not
rename the Android Client app itself.
_Avoid_: client app, generic backend endpoint

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

## Pairing

**Pairing**:
The out-of-band exchange through which a Server and an Android Client learn and persist each other's identities, so later connections authenticate without a new out-of-band step.
_Avoid_: linking, registration, login

**Pairing Invitation**:
A short-lived offer created by the Server for one pairing attempt, presented as a QR payload, an open SAS admission window, or both. It is **active** until the first well-formed pairing request consumes it (**consumed**) or its lifetime elapses (**expired**); a consumed or expired invitation is never reused.
_Avoid_: pairing session, pairing window (reserve "window" for the SAS admission window)

**Pairing Nonce**:
The single-use random value carried inside a QR pairing payload. The first well-formed pairing request that presents it consumes it, regardless of the pairing's outcome.
_Avoid_: token, secret (the payload contains no secret)

**Pairing Code (SAS)**:
The six-digit code derived from the pairing transcript and displayed identically on both ends; a human-comparison token, never key material.
_Avoid_: PIN, password

**Client Confirmation**:
The Android Client user's explicit agreement to complete a pairing. It is required in SAS mode; in QR mode the scan itself is the Client's action.
_Avoid_: accept (reserve "accept" for the Server operator)

**Device Approval**:
The Server operator's explicit authorization of a verified Client, expressed as "Accept device" in the Server GUI or the confirmation in `as-cmd`. Distinct from Client Confirmation.
_Avoid_: client confirmation, pairing confirmation

## Session

**Client Session**:
One Android Client's use of one Server, from Connect until the user stops it or it ends; it may span several connection attempts.
_Avoid_: session (unqualified; Wire Protocol documents use "session" for one connection), connection, transport session

**Connection attempt**:
One Wire Protocol session between an Android Client and a Server inside a Client Session — from handshake to close.
_Avoid_: retry, reconnect (those name the event, not the connection)

**Session outcome**:
The current answer to what a Client Session is doing: live, reconnecting, or ended.
_Avoid_: state, status, phase

**User stop**:
The user ending a Client Session (in-app Stop, notification Stop, leaving the app), as opposed to a connection that drops on its own.
_Avoid_: cancel, disconnect, close

**Surfaced close**:
A close the user must be told about, because automatic reconnection is not allowed.
_Avoid_: fatal error, terminal state

## Release stages

**Beta**:
A controlled-trial release of the v2 Android Client for external pilot customers, on a directly reachable Server (LAN, VPN, or user-configured port forwarding), with one phone per Server. It is not a daily-driver replacement for v1; known limitations are documented and v1 remains separately available.
_Avoid_: stable, general availability, v1 replacement

**Experimental**:
The Server-side status of Wire Protocol v2 before the stable release gate: enabled and runnable by default, but not stable.
_Avoid_: beta (beta is a Client release; experimental is a protocol status)

**Stable**:
The Wire Protocol v2 release status reached only after the full acceptance gate, migration guide, and rollback path exist. Not used for Beta.
_Avoid_: production-ready
