# ADR-0005: Session outcome is owned by one Kotlin module; the UI never sees native session state

Status: accepted (2026-11-16)

## Context

The v2 Android Client's reconnect loop lives in `ClientSession`, but its consequence — "did this session end, and may it reconnect?" — had no owner. `ClientSession.scheduleReconnect` closed its state source before retrying, publishing `CLOSED`; `AppReducer` treated `CLOSED` as terminal (`screen = HOME`, `connectRequested = false`), so a successful reconnect could never return to the session screen. Close attribution was split across `SessionCommandExecutor.expectedStop` (Activity-scoped), `AppState.stopping` (UI-only), and `ControlSessionService.lastErrorCode` (Service-scoped), so a late `CLOSED` after Activity recreation could surface a previous session's error code as a new one. The two relay layers each documented a different meaning for `CLOSED`; neither was authoritative.

Ticket [01](../../.scratch/android-modernization/issues/01-session-outcome-module.md#answer) resolved the ownership question with the user.

## Decision

1. **One outcome, one owner.** A Kotlin deep module models a Client Session's outcome as `Live(LivePhase) | Reconnecting(attempt, lastCloseCode) | Ended(UserStop | Surfaced(code))`. `ClientSession` classifies (it holds the lifetime, handle generation, and `ReconnectPolicy`); `ControlSessionService` holds the single replayable store and hands each Client Session a generation-scoped sink. The native state string does not cross the Service seam.
2. **Stop attribution comes from the call site, not an error code.** `ClientSession.stop(reason)` distinguishes `User`, `Superseded`, `Shutdown`, and `PairingCancel`; only `User` produces `Ended(UserStop)`. `expectedStop`, `stopping`, and `lastErrorCode` are deleted.
3. **Reconnection and ending are mid-screen, not navigation.** The UI stays on the session screen across `Live`, `Reconnecting`, and both `Ended` reasons; a User stop and a surfaced close differ only in presentation (a "you stopped" state with Connect again, versus an error card with Retry / Pair again), and only an explicit Home/About/Settings action leaves the screen. (Ticket 06 supersedes the earlier "only `Ended` changes screens" reading.) A connection attempt is not a Client Session.
4. **Cross-language representation is unchanged in phase one.** No Wire Protocol or `stats_json` change: `close_code == null` means "no outcome recorded" and is retryable. A second, behaviour-neutral native cleanup (write close info before state; record 701 on user stop) removes the remaining ambiguity.

## Considered Options

- **Keep the state string and add a side-channel for close reasons**: rejected — the UI would still decide terminality, which is exactly the split ownership that produced the bug.
- **Derive the outcome in the Service from the state string plus `lastErrorCode`**: rejected — the Service does not own the reconnect loop's lifetime and would have to re-implement the policy.
- **Let `ClientSession` own the replay store**: rejected — the store must outlive a torn-down session so an `Ended` can be replayed to a recreated Activity.
- **Introduce `StateFlow` now**: deferred — the app has no explicit coroutines dependency; the callback seam matches the existing binder style, and the DI ticket owns that change.

## Consequences

- `AppState` shrinks to `outcome: SessionOutcome?` plus navigation; `sessionPhase`, `connectRequested`, and `SessionCommandExecutor` disappear.
- Every late or stale event is dropped by generation, so Activity recreation can no longer resurrect a previous session's error.
- The pairing terminal flow remains outside this module; ticket [11](../../.scratch/android-modernization/issues/11-pairing-outcome.md) decides whether it reuses the generation/replay machinery.
