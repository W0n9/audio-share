# Android Client 现代化 — spec

Feature: android-modernization
Status: ready-for-agent

## Problem Statement

`android-client-v2` 是 Wire Protocol v2 的 Beta 级 Android Client，功能已经能用，但仍是「最小可验收」形态：界面是硬编码英文的单列表单，没有深色主题与中文；会话结束与重连的屏幕归属含糊，重连会把用户拉回 HOME，配对成功依赖 UI 轮询；内核的 backoff、rebind、停止顺序无法在 JVM 构造测试；构建链停在 AGP 8.7 / targetSdk 35，16 KB ELF 对齐与 targetSdk 36 两个发布阻塞尚未解决；测试大量依赖源码字符串与旧布局，重构时无法区分真实回归与断言失效。

Pilot 用户因此得到的是一个能用但不够可信、不可本地化、状态反馈不一致的 Beta 客户端；维护者则缺少一份可交给实现者的、决策完整的现代化方案。

## Solution

在**不改变 Wire Protocol 行为、不改变 Beta 验收范围、不抬高 minSdk 26** 的前提下，交付一份决策完整的 `android-client-v2` 现代化方案：

- **UI**：方向 A「状态优先」的五屏结构（HOME / PAIRING / SESSION / SETTINGS / ABOUT），每屏一个 hero、一个主操作；失败、重连、结束原位呈现；静态 Material 3 跟随系统 Light/Dark；en + zh-CN；a11y 基线。
- **应用架构**：`SessionOutcome` 单一归属与可重放 store；Service 只依赖 `SessionEngineFactory`；配对终局由 `PairingStateRelay` 独占，配对成功交接由 Service 推送；手写组合根，不引入 DI 框架；单 `:app` module + 包边界。
- **内核/音频**：抽取 JVM 可测的 `SessionLifecycleDriver` 与端口，行为不变；清理 Transfer Format 的 Kotlin 死分支，Rust 闸门不动。
- **构建/工具链**：四阶段升级到 AGP 9.5.1 / Gradle 9.7.1 / Kotlin 2.4.21 / compileSdk 37 / targetSdk 36 / Compose BOM 2026.09.00；16 KB 对齐先行；构建 JDK 17、测试 JDK 21。
- **并行与证据**：独立 worktree/分支；25 GO + 冻结后采集 post-refactor 证据；JVM / `EMULATOR` / 真机分层；证据绑定精确 SHA 与产物哈希。

不引入任何新功能。

## User Stories

1. As a pilot customer, I want a status-first Home screen with one clear primary action, so that I know whether to connect, pair, or fix an address without scanning a wall of controls.
2. As a pilot customer, I want my paired Server shown as a named card with its address, so that I can confirm I am connecting to the right computer.
3. As a pilot customer, I want an invalid Server address explained inline, so that I can correct it without losing what I typed.
4. As a pilot customer, I want QR pairing to complete with the scan itself, so that I do not need a second confirmation tap that adds no security.
5. As a pilot customer, I want the Pairing Code (SAS) shown prominently and read digit by digit by TalkBack, so that I can compare it with the Server reliably.
6. As a pilot customer using SAS mode, I want to type the code shown on the Server and confirm explicitly, so that the human comparison remains the security gate.
7. As a pilot customer, I want a failed pairing to show the error code, guidance, and a Try again action in place, so that I do not have to guess what went wrong.
8. As a pilot customer, I want an expired or consumed QR code to block with an explicit Scan again action and never auto-reopen the camera, so that I stay in control of the camera.
9. As a pilot customer, I want to see that the Server still needs to accept the device after my Client Confirmation, so that I do not think the pairing is already complete.
10. As a pilot customer, I want pairing success to move me to the Client Session automatically, so that I do not have to press Connect again.
11. As a pilot customer, I want Live phases (connecting, authenticating, ready, playing) shown on the SESSION screen, so that I can tell the difference between handshake and playback.
12. As a pilot customer, I want a reconnecting Client Session to stay on the SESSION screen with the attempt count, so that I do not lose context while the phone recovers.
13. As a pilot customer, I want a User stop to show “you stopped this session” with Connect again, and a Surfaced close to show an error card with Retry/Pair again, so that the two endings are visibly different.
14. As a pilot customer, I want to stop from the app or the notification, so that I control playback from wherever I am.
15. As a pilot customer, I want playback to continue across screen lock, so that the phone stays a speaker.
16. As a pilot customer, I want a temporary network drop to reconnect without me pressing Connect again, so that a Wi-Fi blip does not end the Client Session.
17. As a pilot customer, I want the app to follow the system Light/Dark setting, so that it is readable in my environment without another preference.
18. As a Chinese-speaking pilot customer, I want the entire UI, notification, and error guidance in zh-CN when my system is Chinese, so that I do not have to operate in English.
19. As a visually impaired pilot customer, I want outcome and error changes announced by TalkBack, so that I know what changed without re-reading the screen.
20. As a visually impaired pilot customer, I want 48dp touch targets and 200% font support, so that I can operate the app without clipping or mis-taps.
21. As a pilot customer, I want a read-only Settings screen showing my paired Server and version, so that I can check pairing state without exposing controls that do not exist.
22. As a pilot customer, I want About to state the beta version, known limitations, v1 warning, update instructions, and privacy, so that I understand what I installed.
23. As a pilot customer, I want the app to never silently fall back to v1 or plaintext, so that a v2 failure cannot downgrade my security.
24. As a pilot customer, I want rotation to keep me on the same screen and keep playback running, so that turning the phone does not interrupt listening.
25. As a pilot customer, I want process death to land on Home without auto-connecting, so that a killed app does not surprise me with audio.
26. As a pilot customer, I want Back to navigate within the app and only leave the app when I press Back on Home, so that navigation is predictable.
27. As a pilot customer, I want a signed, installable APK that coexists with v1, so that I can compare and roll back.
28. As a pilot customer, I want the app to run on API 26 through current Android versions, so that I can use the phone I have.
29. As a maintainer, I want one module to own the Session outcome, so that the UI never has to interpret native state strings or close codes.
30. As a maintainer, I want a replayable outcome store in the Service, so that a recreated Activity sees the current outcome instead of a stale error.
31. As a maintainer, I want Stop attribution to come from the call site, so that a late CLOSED cannot be mistaken for a User stop.
32. As a maintainer, I want the Service to depend on a Session engine factory, so that pairing handoff and Connect/Stop orchestration can be tested with a fake engine.
33. As a maintainer, I want a Session lifecycle driver with injected ports, so that backoff, rebind, STOP/create interleavings, and media-stop ordering have JVM tests.
34. As a maintainer, I want pairing to have one owner with source ownership and replay, so that a stale snapshot from a stopped session cannot overwrite a new exchange.
35. As a maintainer, I want pairing success handoff to be Service-owned and push-based, so that the UI does not poll credentials for five seconds.
36. As a maintainer, I want a single `:app` module with package boundaries, so that the refactor does not fight cargo/NDK wiring or beta merge conflicts.
37. As a maintainer, I want no DI framework, so that the toolchain upgrade does not gain an annotation-processing compatibility risk.
38. As a maintainer, I want the Transfer Format contract stated once and dead Kotlin branches removed, so that the client does not pretend to support formats the Rust gate rejects.
39. As a maintainer, I want a four-phase build upgrade with per-phase gates, so that a failure can be reverted to the bridge phase instead of the original toolchain.
40. As a maintainer, I want 16 KB ELF alignment and targetSdk 36 handled as independent release blockers, so that they are not assumed to follow from an AGP upgrade.
41. As a maintainer, I want an `android-ci` workflow that runs JVM tests, build/ELF/signing assertions, and never claims device coverage, so that CI strengthens but does not fake the evidence.
42. As a tester, I want JVM, `EMULATOR`, and real-device tiers with explicit statuses, so that I know exactly what was verified where.
43. As a tester, I want post-refactor evidence bound to a commit SHA and artifact hash, so that pre-refactor `PASS` results cannot be silently inherited.
44. As a tester, I want the four beta-critical real-device paths re-run, so that pairing, playback, lock, and reconnect are known good after the refactor.
45. As a tester, I want existing test assets classified as keep/rewrite/drop, so that I know which failures are real regressions.
46. As a release maintainer, I want versionName/versionCode untouched on the refactor branch, so that the beta release validation continues to pass.
47. As a release maintainer, I want the branch point recomputed from the frozen tip, so that the refactor includes the latest Android fixes and evidence is correctly reset.
48. As a developer, I want no new features in this effort, so that the modernization stays a refactor with a stable acceptance surface.

## Implementation Decisions

### UI direction and screens

- Adopt **Direction A (status-first)**: one hero and one primary action per screen; errors and reconnection render in place; Settings/About are one tap off the header; no tab bar.
- Five destinations in the existing `AppScreen` enum: HOME, PAIRING, SESSION, ABOUT, plus a new SETTINGS. No Navigation Compose, no new navigation dependency.
- Must-have states per screen: Home (no credential / saved Server / address error / camera permission denied); Pairing (not started / scanning / STARTING / AWAITING_CONFIRMATION / FAILED / COMPLETE); Session (four Live phases / Reconnecting / Ended User stop / Ended Surfaced close); Settings (no pairing / read-only paired Server, version, About link); About (version, limitations, v1 warning, updates, privacy).
- SETTINGS is read-only: no switches, no delete, no rename. Anything else would be a new feature.
- QR mode auto-confirms exactly once when the Client reaches AWAITING_CONFIRMATION; SAS mode keeps code entry plus explicit Client Confirmation. This preserves ticket 32 behaviour.

### Navigation and restoration

- The reducer is the single owner of screen choice. The outcome→screen table: Connect/PairingCompleted → SESSION; Live/Reconnecting/Ended stay SESSION; pairing failure stays PAIRING; a Surfaced close stays SESSION with an error card; only explicit Home/About/Settings actions leave.
- A restored screen is a hint only. On first bind, if the outcome is absent or Ended and there is no pairing snapshot, SESSION/PAIRING degrade to HOME.
- If a restored PAIRING has no pairing snapshot but the outcome is Live/Reconnecting (handoff happened, the completion push was missed), land SESSION.
- Config change saves only `screen`, `address`, and `sasInput`; session, pairing, credentials, and errors replay from the Service. Camera scanning and transient errors are never restored.
- Process death lands HOME and does not auto-connect (the existing saved-state policy suppresses it); a cold start keeps the existing auto-connect policy. Pairing is not resumed; no outcome is persisted.
- Back: PAIRING cancels the exchange then goes HOME; SETTINGS/ABOUT go HOME; SESSION goes HOME and keeps playing; HOME finishes the Activity. Activity finish still sends ACTION_STOP, so two Backs from SESSION stop the Client Session. This is intended.

### Session outcome module

- One owner for a Client Session's outcome, with the type shape from ticket 01:

```text
SessionOutcome = Live(LivePhase) | Reconnecting(attempt, lastCloseCode) | Ended(UserStop | Surfaced(code))
LivePhase      = CONNECTING | AUTHENTICATING | READY | PLAYING
```

- `ClientSession` classifies (it holds lifetime, handle generation, and reconnect policy); the Service holds the single replayable store and injects a generation-scoped sink per Client Session.
- Stop attribution comes from the call site: `User`, `Superseded`, `Shutdown`, `PairingCancel`; only `User` produces `Ended(UserStop)`.
- The native state string does not cross the Service seam. `sessionPhase`, `connectRequested`, `stopping`, `expectedStop`, and the Service's last error code are deleted.
- Reconnection is mid-session: the UI stays on SESSION across Reconnecting; only Ended terminates.
- No Wire Protocol or `stats_json` change. The behaviour-neutral native cleanup (write close info before state; record 701 on user stop) belongs to the session-outcome work, not to a Rust refactor.

### Pairing terminal

- `PairingStateRelay` remains the only owner of pairing state; no `PairingOutcome` type and no shared generic store with Session outcome.
- Invariants: one source owns publishing; `setListener` replays synchronously; a new exchange invalidates the old source and clears the visible snapshot; late terminal events are dropped; pairing terminal is not part of SessionOutcome.
- A pairing exchange does **not** publish into the session outcome store. Only the post-handoff authenticated Client Session feeds the outcome store.
- Service owns the handoff: after the credential is persisted it starts the new authenticated Client Session and pushes a one-shot `PairingCompleted` to the UI. The UI only navigates and updates the Server list. The 100 ms / 5 s credential poll is removed. Handoff is dispatched asynchronously (Service action or main-thread handler), never synchronously inside the pairing watcher callback.
- Persistence failure keeps the COMPLETE snapshot, logs, and does not auto-connect.

### Session engine and driver

- The Service depends on a `SessionEngineFactory`; `ClientSession` is the default implementation. The engine surface is only what the Service calls: start, stop, confirm pairing, plus callbacks/outcome sink. The factory accepts the outcome sink and pairing/log callbacks.
- A JVM-testable `SessionLifecycleDriver` owns session timing and ownership; `ClientSession` becomes the Android adapter. Ports: NativeSession, NetworkMonitor, Scheduler, MediaController, OutcomeSink.
- Driver responsibilities: create/connect failure → bounded backoff (reusing the existing policy); network available during handshake → no rebind; rebind success continues; rebind failure → teardown + backoff; STOP/create interleaving drops old-generation work without leaking native handles; media stops before the native session is destroyed.
- The driver consumes the single outcome policy and reconnect policy; it does not reclassify close codes.

### DI and module boundaries

- Hand-written composition roots: the Service (session graph) and the Activity (UI graph). Constructor injection everywhere else. No Hilt, Koin, or annotation processing.
- Keep one Gradle `:app` module. Organize incrementally into `ui`, `session`, `pairing`, and `jni` packages; the JNI binding class stays in the root package because its package/class name is load-bearing for native symbol exports. Do not use `native` as a package name (Java keyword).
- A single module cannot enforce package boundaries at compile time; the constraint is small package facades, non-public implementation classes, and review. If drift becomes severe, revisit a source-scanning architecture test or a module split.
- Inject Random and Clock where timing decisions depend on them; Context stays in platform adapters; Handler/Looper stay in the platform layer. A Scheduler abstraction is owned by the driver decision.

### Theme, components, localization, accessibility

- Static Material 3 light/dark using the system setting; no dynamic color, no user theme preference.
- Default M3 components: Scaffold/small TopAppBar, Card/OutlinedCard, Button family, OutlinedTextField, LinearProgressIndicator, HorizontalDivider. Static Canvas hero; no custom motion. Icons from material-icons-core or vector drawables.
- Introduce `strings.xml` (English default) and `values-zh-rCN`; localize UI, notification, and error guidance. Follow the system locale; no in-app language picker.
- `ErrorGuidance` keeps code and retry/repair/auto-retry flags; the UI maps code to `error_<code>_title` / `error_<code>_guidance` resources instead of rendering Kotlin English.
- Accessibility: content descriptions on icon-only actions; SAS announced digit by digit; outcome/error text with assertive live region; ≥48dp targets; 200% font support; focus order title → status/error → primary action; theme color roles only; camera permission/close affordances; start/end padding for RTL.
- Deferred: dynamic color, theme setting, Settings switches, pause/resume, multi-Server management, tablet/landscape layouts, custom animation, gallery QR, extended notification controls, in-app language picker.

### Build and toolchain

- Four phases: (0) 16 KB ELF alignment + CI SDK bump; (1) bridge to AGP 8.13.2 / Gradle 8.13 / protobuf plugin 0.10.1 / Kotlin 2.0.21; (2) AGP 9.5.1 / Gradle 9.7.1 / built-in Kotlin / KGP 2.4.21; (3) compileSdk 37 / targetSdk 36 / Compose BOM 2026.09.00 / updated AndroidX set.
- Build/runtime JDK 17; Robolectric test job JDK 21; app bytecode 17. JDK 25 is not adopted.
- Never change versionName/versionCode/VERSION on the refactor branch; production signing and the `-PtestSupport`/`-PdeviceTests` semantics remain intact.
- A new `android-ci` workflow runs JVM tests, a test-signed release build, dual-ABI/ELF/signing assertions, and proto-diff checks. It does not run emulators or devices. The release workflow only updates SDK versions.
- Each phase is one `--no-ff` merge; failure reverts to the previous phase, with the bridge phase as the minimum fallback.

### Verification and evidence

- Tiers: JVM (outcome/store/UI/driver/policies), `EMULATOR` (signed install, pairing, playback/lock, rotation, process death, rebind/recreate, API 26, 30-minute trend), real device (QR+SAS pairing, 5–10 minutes audible playback, lock 2–5 minutes, network drop + Stop).
- Pre-refactor beta evidence (Windows, Server protocol, v1 frozen artifacts, 30-minute real-device audio) is frozen and never inherited as post-refactor PASS.
- Post-refactor evidence lives under `evidence/ticket-08-refactor/` with a manifest binding each item to tier, status, commit SHA, APK/Server hashes, device/AVD, build flags, command, and evidence path. Final evidence is collected after ticket 25 is GO and the beta is frozen.
- Merge gate: JVM green; EMULATOR matrix PASS or maintainer WAIVED; four real-device items PASS or explicitly WAIVED; manifest complete; no open FAIL; evidence bound to the exact refactor tip that will merge.

### Test asset disposition

- Keep: pairing payload, QR decoder, protocol/media fixture replay, media playback, credential store, DER signature, reconnect policy, session startup policy.
- Rewrite: Compose screen tests for Direction A + resources/semantics; reducer tests for SessionOutcome/navigation/handoff; error guidance tests for flags and resource keys.
- Adapt: QR auto-confirm gate tests.
- Delete/replace: command-executor and session-state-relay tests whose implementations disappear.
- Source-scanning contract tests keep only structure/seam invariants (manifest, permissions, no boot receiver, applicationId/version, UI-only-SessionController); implementation-shape and string assertions become behaviour tests.

## Testing Decisions

A good test asserts external behaviour: a user-visible state, an emitted event, a screen destination, a replay result, a lifecycle ordering, or an artifact property. It does not assert that a particular class contains a particular string, nor that a private method was called.

Four existing seams (confirmed with the maintainer):

1. **UI seam** — `AudioShareApp(state, onEvent)` and the pure reducer. Tests cover every screen's must-have/empty state, the outcome→screen table, restoration rules, resource keys, and accessibility semantics. Prior art: the reducer tests and Robolectric Compose screen tests.
2. **Service orchestration seam** — `SessionEngineFactory` + `SessionEngine` with a fake engine. Tests cover Connect/Stop commands, pairing handoff (persist → start → one-shot completion), persistence failure, and generation guarding. Prior art: the fake SessionController used by the command-executor tests.
3. **Session timing seam** — `SessionLifecycleDriver` with fake NativeSession/NetworkMonitor/Scheduler/MediaController/OutcomeSink. Tests cover create/connect failure backoff, rebind success/failure, network events during handshake, STOP/create interleaving, media-stop ordering, and stale-generation rejection.
4. **Device seam** — the existing instrumentation runner and device regression script. `EMULATOR` covers signed install, pairing, playback, lock, rotation, process death, rebind/recreate, API 26, and screenshots; real device covers the four beta-critical paths.

Test matrix:

- **JVM**: five screens × must-have/empty states; outcome/store invariants; navigation/restoration; handoff; driver timing; pairing relay ownership; resource en/zh key parity; a11y semantics.
- **`EMULATOR`**: signed APK install; five screens × key states in Light/Dark; zh-CN; 200% font; rotation/process death; API 26 smoke; 30-minute trend.
- **Real device**: QR+SAS pairing, 5–10 minutes audible playback, lock 2–5 minutes, network drop recovery + Stop.
- No screenshot-golden infrastructure and no third-party accessibility scanner are introduced. `EMULATOR` screenshots are evidence, not pixel assertions.

Test ownership follows the implementation tickets: outcome policy/store with the session-outcome work; reducer navigation with navigation/restoration; driver timing with the kernel work; screen/a11y/resource tests with UI; handoff with pairing; final asset disposition in this spec.

## Out of Scope

- v1 modernization, v1 feature migration, and merging v1 and v2 into one app.
- Any Wire Protocol behaviour change (version negotiation, transport, framing, error codes).
- M5 impairment work (jitter buffer, drift correction, PLC, FEC), multiple Clients on one Server, diagnostics export.
- F-Droid distribution, in-app auto-update, boot start, mDNS/STUN/TURN/relay.
- AAudio/Oboe or another low-latency audio engine, and large-scale multi-module repartitioning.
- Full Rust `session.rs`/`jni.rs` refactor and JNI error-model changes.
- Baseline Profile / Macrobenchmark (left to a post-stable-gate effort).
- The v1 independent disable switch and other stable-gate work.
- UI deferrals: dynamic color, user theme setting, Settings switches, pause/resume, multi-Server management, tablet/landscape layouts, custom animation/transitions, gallery QR selection, extended notification controls, in-app language picker.

## Further Notes

- Implementation tickets are numbered **13–27** in `.scratch/android-modernization/issues/` and use the `**Status:**` + triage-label form; the planning tickets 01–12 use `Status:` + `wayfinder:` labels. Do not treat 13–27 as wayfinder tickets when scanning the completed map.
- Ticket index (dependency order): 13 handoff (human-gated) → 14 build phase 0, 15 outcome; 15 → 16 navigation, 17 pairing handoff, 21 engine factory; 16 → 18 UI shell; 17+18 → 19 pairing screen; 15+18 → 20 session screen; 21 → 22 driver; 14+19+20+22 → 23 bridge → 24 AGP 9 → 25 SDK/Compose → 26 test closeout; 26 + beta freeze → 27 evidence/merge (human-gated).
- Reference decisions live in the 12 tickets under `.scratch/android-modernization/issues/`; the map is `.scratch/android-modernization/map.md`. The Direction A prototype and its full screenshot matrix live under `.scratch/android-modernization/prototypes/ui-direction/`.
- Implementation order: beta freeze/worktree preparation → session outcome → build phase 0 in parallel → engine factory → driver → navigation + pairing handoff → UI → build phases 1–3 → evidence and merge.
- The beta branch has moved past the original branch point: it is now at `cb2faf1` and includes Android fixes. The refactor branch must start from the frozen tip (or merge forward after starting from the old point); Android evidence is reset per the freeze rules.
- The refactor branch must not change `VERSION`, `versionName`, or `versionCode`; the release workflow validates them.
- `GLOSSARY.md` and the session-outcome ADR are committed with the refactor branch's first commit, not to the beta branch. **Conflict to fix there:** ADR-0005's clause 3 still says only `Ended` changes screens (i.e. Ended returns to Home). Ticket 06 supersedes it: both `Ended(UserStop)` and `Ended(Surfaced)` stay on SESSION. Revise ADR-0005 when it is first committed on the refactor branch; do not edit the untracked copy in the beta worktree.
- No new features are introduced by this spec. Dark mode and zh-CN are presentation requirements, not new product capabilities.
