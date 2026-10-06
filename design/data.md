# Data

Part of the [herdr-review design](../DESIGN.md).

## Files

State directory: `${XDG_STATE_HOME:-~/.local/state}/herdr-review/<hash>/`, where `<hash>` is
`std::hash::DefaultHasher` over the canonical root path, printed as hex. The standard library hasher
avoids a SHA dependency. The directory is created with mode `0700` and files with `0600`.

| File | Content |
|---|---|
| `review.jsonl` | Events, one JSON object per line |
| `archive.jsonl` | The events of archived threads, in the same form. Only `archive` writes it, and nothing reads it |
| `meta.json` | `root`, `spec`, `base`, `target { pane_id, terminal_id }`, `review_pane_id` |
| `lock` | Empty. Target of the exclusive lock |

`DefaultHasher` output is not guaranteed stable across Rust releases. `meta.json` stores `root`, so on
a miss the binary scans the sibling directories for a matching `root` before creating a new one.

## Events

Every event has `kind`, `at` (RFC 3339, display only) and `by` (`user`, `agent:<name>`, or plain
`agent` when the name is unknown, see `cli.md`).

| `kind` | Other fields |
|---|---|
| `add` | `id`, `parent` (replies only), `path`, `old_path`, `side`, `line`, `end_line`, `line_text`, `spec`, `body`. A file comment has no `side`, `line` or `line_text`. A reply has only `id`, `parent`, `body`. |
| `edit` | `id`, `body` |
| `delete` | `id` |
| `resolve` | `id` (a root) |
| `reopen` | `id` (a root) |
| `sent` | `ids`, `batch` |
| `seen` | `id` (a root) |
| `archived` | `user`, `agent`, `batch`: the highest counter each kind of id had reached when threads were archived |

Fold rules:

- Events apply in file order. An event that names an unknown id, or breaks the rights rule (ADR 0004),
  is skipped.
- A thread is unsent when its root (if the user wrote it) or any reply by the user has no `sent` event,
  or when a `reopen` by the user comes after its last `sent`. A `sent` event's `ids` are comment ids.
  Naming a root id also clears that thread's reopen.
- An `edit` never makes a comment unsent. A user comment with an `edit` after its last `sent` is
  "edited since sent". The card shows that mark, and the agent sees the new text only on a resend.
- A thread is `new` when its last `resolve` is by an agent and no `seen` follows it.
- Deleting a root deletes its thread.
- An `archived` event raises each id counter to at least its value and changes no thread. It is how a log
  that no longer holds `u1` to `u5` still gives the next comment `u6`.

## Writing

Every mutation is one call, `store::write(dir, now, build)`. It opens `lock` and tries
`File::try_lock` every 2 ms for up to 2 seconds, then gives up with `Busy`. A writer that releases the
lock waits 2 ms before returning, so a waiting process can take it. Without that pause, 8 processes in
a loop starved one that polled every 50 ms. Under the lock it reads
and folds `review.jsonl`, calls `build` with the folded review to validate the request and produce the
events, writes all lines with one `write_all` on an `O_APPEND` handle, and drops the lock. Ids (`u<n>`
for the user, `a<n>` for agents, `b<n>` for send batches) are the highest counter in the folded review
plus one. The graph is in `graphs/store.md`.

`store::archive(dir, now)` is the one operation that rewrites `review.jsonl`. It takes the same lock, so it
cannot run beside an append. Under the lock it reads the log line by line, folds it, and takes every thread
that is resolved and not `new`. An unsent one goes too. Then:

- The events of those threads are appended to `archive.jsonl` with one `write_all`, and the file is synced.
  These are every event that names the root or one of its replies, a deleted reply included. A `sent` event
  that carried comments of both kinds is split in two, and each part keeps the batch id.
- The rest is written to `review.jsonl.<pid>.tmp`, synced, and renamed over `review.jsonl`. Its first line
  is one `archived` event with the counters of the folded log, and an earlier `archived` line is dropped,
  so the log holds one. A line that did not parse stays as it was, and so does an event that names an id
  no thread has.
- With nothing to archive it writes neither file.

Ids are never reused, because the counters are in the log itself and reach the disk in the same rename
that removes the archived lines. No second file has to agree with the log.

The archive is written first. A crash, or a failed write, between the two steps leaves a thread in both
files and never in neither. `review.jsonl` decides: a thread whose root is still there is live, and the copy
of it in `archive.jsonl` is ignored. The next archive appends that thread's events again, so `archive.jsonl`
can hold a thread's events twice. A reader folds it with the same fold as the log: an `add` of an id it has
seen is skipped, and the other events of the thread replay in their order and end in the same state, so the
fold has the thread once.

Every writer opens `review.jsonl` after it has the lock, so an append cannot land in the replaced file.

## Reading

Read the whole file and fold. Ignore a final line with no newline. Skip a line that does not parse and
count it, so the TUI can show "1 unreadable event". The TUI keeps the file length it last saw and
checks it every 250 ms on its input poll timeout. When the length differs, in either direction, it reads
and folds the whole file again. A review log is small, and this keeps a rewrite and an append the same case.
An archive by another pane is seen this way, because the rewritten log is always shorter: it loses at least
one thread's `add` and `resolve` lines and gains at most one `archived` line, which is shorter than the two.

## Meta file

`meta.json` is rewritten on every save, and three processes write it (`open`, the TUI, the
`send` action). A save takes the same lock as the store, reads the current file, changes its own
fields, writes a temp file and renames it. A save that fails is a warning and never blocks a review
action.
