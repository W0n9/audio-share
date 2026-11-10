# Triage Labels

The skills speak in terms of five canonical triage roles. This file maps those roles to the actual label strings used in this repo's issue tracker.

> With the local-markdown tracker these strings are written as the `Status:` line near the top of each `.scratch/<feature-slug>/issues/<NN>-<slug>.md` file, not applied as GitHub labels.

| Label in mattpocock/skills | Label in our tracker | Meaning                                  |
| -------------------------- | -------------------- | ---------------------------------------- |
| `needs-triage`             | `needs-triage`       | Maintainer needs to evaluate this issue  |
| `needs-info`               | `needs-info`         | Waiting on reporter for more information |
| `ready-for-agent`          | `ready-for-agent`    | Fully specified, ready for an AFK agent  |
| `ready-for-human`          | `ready-for-human`    | Requires human implementation            |
| `wontfix`                  | `wontfix`            | Will not be actioned                     |

When a skill mentions a role (e.g. "apply the AFK-ready triage label"), use the corresponding label string from this table.

Edit the "Label in our tracker" column to match whatever vocabulary you actually use.

## Closing work

The five labels above are **pre-work triage roles**: they say who should pick up an issue and how. Once work has started, the local tracker records close state separately in the same `Status:` line:

- `Status: resolved` — the work is merged and verified. Use this for completed tickets; it is the local close state.
- `Status: ready-for-human` — the implementation is code-complete, but a human-only gate remains (real hardware, a phone, a signing step, and so on). It stays open until that gate is recorded.

A resolved ticket therefore does not need to be rewritten to one of the five triage labels; it is already closed. Keep `ready-for-human` off the resolved set until the human step is done.
