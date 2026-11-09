# Where each error is handled

Part of the [herdr-review design](../DESIGN.md).

| Error | CLI edge (`main.rs`) | TUI edge (action dispatcher) |
|---|---|---|
| Usage, `UnknownId`, `InvalidBody`, `InvalidBatch` | One line on stderr, exit 2. `UnknownId` also lists the open ids | Cannot occur for ids, the TUI only acts on threads it shows |
| `StoreError::Busy` | "review is busy, try again", exit 1 | Status line, the action can be repeated |
| `StoreError::Io` | Message with the path, exit 1 | Status line, the editor keeps its text |
| `GitError` | Message, exit 1 | Message screen or status line, as in the graphs |
| `TargetError`, `Refusal`, `HerdrError`, `HcError` | Notification and exit 1 (actions). Under hc, no notification. With `send --json`, one JSON object with an `error` kind on stdout and exit 1 | Status line and notification. Nothing marked sent |
| `CommandError::NotAllowed` (edit or delete of an agent's comment) | One line on stderr, exit 2 | Cannot occur, the TUI offers the action on the user's comments only |
| Body file unreadable, a symlink, too long (`--body-file` 16 KiB, `--file` 256 KiB) | `InvalidBody`, one line, exit 2. The file is deleted when it was opened | Not used |
| `view --set` for a path not in the diff, `file --path` for the same | One line, exit 2 | Not used |
| `Warning` | Printed to stderr, exit 0 | One line under the status line until the next action |
| Panic | Default hook, exit 101 | Terminal restored first, then the message |

Exit codes for the agent: 0 done, 1 failed and can be retried later, 2 the request itself is wrong.

Exit codes for the panel: the same three, and one more reading of them. Exit 2 with `unknown command`,
`unknown option` or `unknown comment command` on stderr means the binary is too old. Other exit 2 means the
panel's request was wrong, which is a stale screen in practice. `send --json` puts its error kind on stdout.

## Where the GUI handles each error

The panel's edge is the reducer's `failed` function, which the recovery wrapper feeds ([gui](gui.md), Where it
breaks). One row per error.

| Error | Panel edge |
|---|---|
| `BinaryMissing` | Next candidate binary, then banner "herdr-review not found". No retry |
| `TooOld` | Banner "update herdr-review". No retry |
| `ContractMismatch` | Banner with the field path and both schema numbers. No retry |
| `Busy` | Status line, the user repeats the action |
| `BackendFailed` | Status line and the last good screen kept. Banner when there is none |
| `BadRequest` | Status line with stderr, then one reload. A `seen` that fails shows nothing |
| `Refused` (`Blocked`, `NotReady`, `AgentGone`) | Status line and notify. Nothing marked sent |
| `NoTarget` | Status "no agent found for this folder" |
| `AmbiguousTarget` | Picker over the candidates, then `send --target` |
| `WatchUnavailable` | The 10 s timer only |
| Body file not written | Status line, the editor keeps its text |
| An unhandled `Msg`, a row index out of range | Throws. herdr-app shows the mod error and `herdr-app status` lists it |

An error that stops the panel from reading (`BinaryMissing`, `TooOld`, `ContractMismatch`) is a banner and
no loads from the timer or the watch, because a retry would fail the same way. The Refresh button tries
again, and so does a new copy of the panel. Every other error keeps the last good screen.

