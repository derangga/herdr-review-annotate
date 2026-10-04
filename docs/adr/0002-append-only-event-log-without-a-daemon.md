---
status: accepted
---

# Store each review as an append-only event log, with no daemon

The TUI and the agent CLI are separate processes that write the same review at the same time. Each
review is one `review.jsonl` file of events (`add`, `edit`, `resolve`, `reopen`, `delete`, `sent`,
`seen`). Every reader folds the events into the current state. The position of an event in the file is
the only order. The `at` timestamp is display data and is never used to sort.

A writer takes an exclusive `std::fs::File` lock on a `lock` file, waiting at most 2 seconds. Under
the lock it reads and folds the log, validates its request against that state, writes its events with
a single `write_all` on an `O_APPEND` handle, and unlocks. Readers do not lock. A reader ignores a last
line that has no newline and skips a line that does not parse.

## Considered options

- **A daemon that owns the state**, as hunk does. It gives live updates for free, and costs a process
  to supervise, a transport, and authentication.
- **Mutable rows rewritten in place**, as `herdr-annotate/rust/src/store.rs` does with a temp file and
  `rename`. Two writers then overwrite each other unless every update holds the lock for a full read
  and rewrite.
- **Copy `store.rs`.** Rejected. Its lock is a `mkdir` directory that fails at once with "busy" and is
  only considered stale after 30 seconds, and it rejects the whole file when one line is malformed
  (`store.rs:253-256`, `:305-340`). A kernel lock is released when the writer dies.

## Consequences

- The file only grows until the user archives. Archiving is the one operation that rewrites the file.
  It runs under the same lock, and every writer opens the file after taking the lock, so an append
  cannot land in a replaced file.
- The TUI learns about agent writes by checking the file length on its input poll timeout and reading
  from its last offset. There is no file watcher.
- Comment ids are counters (`u1`, `a1`) allocated under the lock, so they cannot collide.
