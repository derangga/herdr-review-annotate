# Where each error is handled

Part of the [herdr-review design](../DESIGN.md).

| Error | CLI edge (`main.rs`) | TUI edge (action dispatcher) |
|---|---|---|
| Usage, `UnknownId`, `InvalidBody`, `InvalidBatch` | One line on stderr, exit 2. `UnknownId` also lists the open ids | Cannot occur for ids, the TUI only acts on threads it shows |
| `StoreError::Busy` | "review is busy, try again", exit 1 | Status line, the action can be repeated |
| `StoreError::Io` | Message with the path, exit 1 | Status line, the editor keeps its text |
| `GitError` | Message, exit 1 | Message screen or status line, as in the graphs |
| `TargetError`, `Refusal`, `HerdrError` | Notification and exit 1 (actions) | Status line and notification. Nothing marked sent |
| `Warning` | Printed to stderr, exit 0 | One line under the status line until the next action |
| Panic | Default hook, exit 101 | Terminal restored first, then the message |

Exit codes for the agent: 0 done, 1 failed and can be retried later, 2 the request itself is wrong.
