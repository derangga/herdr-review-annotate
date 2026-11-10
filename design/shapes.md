# Shapes

Part of the [herdr-review design](../DESIGN.md).

Ids, each a newtype that can only be built by its parser:

| Type | Form | Built from |
|---|---|---|
| `CommentId` | `u<n>` or `a<n>` | Store allocation, CLI argument, JSONL field |
| `BatchId` | `b<n>` | Store allocation |
| `PaneId` | `w1:p2` | Herdr JSON, environment |
| `TerminalId` | `term_...` | Herdr JSON, environment |
| `TerminalAddress` | `t` then one or more ASCII letters or digits (`t7`, `t1d`) | hc JSON, `--target`, `HC_TERMINAL_ID` |
| `RepoRoot` | Canonical absolute path | `git rev-parse --show-toplevel`, then `canonicalize` |
| `RelPath` | Repo-relative path with no `..` and no leading `/` | Diff parser, agent batch |

Records:

| Type | Fields |
|---|---|
| `Event` | `kind`, `at`, `by: Author`, and the fields in `data.md`, events |
| `Comment` | `id`, `parent`, `author`, `at` (the time of its `add` event), `body`, `sent_batch`, `edited_since_sent` |
| `Thread` | `root: Comment`, `anchor: Anchor`, `replies`, `status`, `is_new`, `unsent` (open only; see `data.md`) |
| `Anchor` | `path`, `old_path`, `target: AnchorTarget`, `spec: Spec` |
| `Review` | `threads` in file order, `skipped_lines`, `ids` (the counters the next id comes from). The result of the fold |
| `Archived` | `threads`, `unsent`: how many threads one archive moved, and how many of them were never sent |
| `Meta` | `root`, `spec`, `target: Option<Target>` (the herdr one), `hc_target: Option<HcTarget>` (the hc one), `review_pane: Option<PaneId>`, `viewed`: path to fingerprint |
| `DiffFile` | `path`, `old_path`, `change`, `hunks`, `flags` |
| `Diff` | `files`, `rev` (what the working tree was compared against), `spec`, `notices` (cap reached, base missing, and so on) |
| `Select` | `row`, `half: Option<Side>`: where a range being selected started, and the half of a split row it started on. `View::select` holds one while visual mode is on, and `View::drag` is whether a left press in the stream is held. Neither is stored |
| `Filter` | `query`, `typing`: the sidebar's file filter. `View::filter` holds one while a filter is on, and `None` is no filter. `typing` says whether keys go to the query. It is a struct and not a two-variant enum because both states carry the same query and only the key router reads which one it is. Not stored |
| `View::collapsed` | A set of `RelPath`: the files folded to their header. `Stream::build` reads it into `folded` and `hidden`, one entry per file of the diff: whether the file is collapsed, and the threads it hides in the order of their lines. Not stored |
| `View::viewed` | A set of `RelPath`: the files whose fingerprint equals the one `meta.viewed` saved. Worked out again on each load of the diff. `Stream::build` reads it into one flag per file for the sidebar and the header |
| `View::whole` | A set of `RelPath`: the files shown whole. `App::read_diff` fills them after each load. Not stored |
| `Keymap` | Key to `Action`, plus `warnings` |
| `SidebarConfig` | `open` (default true) and `icons` (default false), read once from `[sidebar]` into `View::sidebar` and `View::icons` |
| `Theme` | One colour per role, filled from the Catppuccin flavor `[theme] name` chose |
| `Env` | Every `HERDR_*`, `REVIEW_*` and `HC_TERMINAL_ID` value, and `exe` (the running binary's path), read once in `main.rs` |

Variants:

| Type | Cases |
|---|---|
| `Reached` | `Herdr(Target)`, `Hc(HcTarget)`. The agent a prompt reached. `Target` is the herdr record (`pane`, `terminal`, `agent`) and `HcTarget` the hc one (`address`, `agent`). See Target below |
| `Backend` | `Herdr`, `Hc`. Which CLI `send` and target resolution talk to. Chosen once, at the edge |
| `Scope` | `Unsent`, `AllOpen`, `Threads(ids)`. Which threads a send carries. `Threads` is exactly the named open threads |
| `Author` | `User`, `Agent(Option<String>)` |
| `Side` | `Old`, `New` |
| `AnchorTarget` | `Line { side, line, text }`, `Range { side, start, end, text }`, `File` |
| `Status` | `Open`, `Resolved { by: Author }` |
| `Spec` | `WorkTree`, `Branch { base: String }` |
| `Placement` | `Matched { line: Option<u32> }` (no line for a file comment), `Outdated { near: Option<u32> }` (no line when the file has no rows on that side), `NotInDiff`. Computed, never stored |
| `Change` | `Modified`, `Added`, `Deleted`, `Renamed`, `Untracked`, `Binary`, `Submodule`, `TooLarge`, `Unparsed` |
| `AgentStatus` | `Idle`, `Working`, `Blocked`, `Done`, `Unknown` |
| `Action` | The action names in `tui.md` |
| `SendOutcome` | `Nothing`, `Sent { n, agent }`, `Queued { n, agent }` |

Errors, one enum per module:

| Type | Cases | Meaning |
|---|---|---|
| `StoreError` | `Io(kind)`, `Busy` | The disk failed, or the lock was not free within 2 seconds |
| `CommandError` | `UnknownId { id, open }`, `InvalidBody(why)`, `InvalidBatch { index, why }`, `NotAllowed { id }` | The caller asked for something the review cannot do. Edit and delete build `NotAllowed` for an agent's comment. They are the TUI's actions and the user CLI's `comment edit` and `comment delete`. The agent's subcommands never edit or delete, so no request of theirs breaks the rights rule |
| `GitError` | `NotInstalled`, `NotARepo`, `NoBase { tried }`, `Failed { args, stderr }` | `git` could not answer |
| `HerdrError` | `code`, `message` | As parsed by `agent_delivery.rs` |
| `HcError` | `exit`, `kind`, `message` | As parsed from the JSON `hc` prints on stderr, `{"error":{"kind","message","hint"}}`, and its exit code |
| `TargetError` | `NoAgent`, `Ambiguous(Vec<Target>)` (herdr), `AmbiguousHc(Vec<Candidate>)` (hc, with `address`, `agent`, `cwd`, `status`), `Backend(String)` | Resolution found zero or several agents, or `herdr agent list` or `hc ls` failed |
| `Refusal` | `Blocked`, `NotReady`, `AgentGone` | The agent cannot take a prompt now |
| `SendError` | `Store`, `Target`, `Command(UnknownId)`, `Refused(Refusal, text)`, `Failed(text)` | Everything that ends a send with nothing delivered. `Failed` is a `HerdrError`, an `HcError` or a repository that could not be found. `Command` is a `--thread` id that is not an open thread, which is exit 2 and never JSON |
| `Warning` | `SkippedLine(n)`, `Config(String)`, `MetaUnreadable`, `SentNotRecorded`, `TargetNotSaved` | Not an error. Collected and shown, the operation still succeeds |

There is no `ParseError` for diffs. A file the parser cannot read becomes `Change::Unparsed` and the
review continues.

## Target

There is one target record per backend, because the two CLIs name a terminal differently. Herdr has a pane
and a terminal id, and hc has one address (`t7`). Neither is a stand-in for the other, so the address is not
squeezed into `terminal_id` with an empty pane. `Target` stays the herdr record, as the TUI and `open` use it,
`HcTarget` is the hc one, and `Reached` is the sum a send returns.

In `meta.json` each variant has its own key:

```json
"target":    { "pane_id": "w1:p2", "terminal_id": "term_abc", "agent": "claude" },
"hc_target": { "address": "t7", "agent": "claude-code" }
```

- `target` is exactly what v0.5.0 writes and reads. The new binary writes it only under the herdr backend,
  so a v0.5.0 binary reads every `meta.json` the new one saves. It has no `deny_unknown_fields` on `Meta`,
  so it ignores `hc_target`.
- A v0.5.0 binary that saves `meta.json` rewrites it from its own `Meta`, which has no `hc_target`, so the hc
  target is dropped. The next send under hc resolves the agent by folder again (`send.md`). That costs one
  `hc ls` and the stored address was already assumed not to survive a core restart, so nothing is lost
  that the design relied on.
- Only `send_with` (in `send.rs`) looks at which key holds the target: a `Reached::Herdr` saves `target` and a
  `Reached::Hc` saves `hc_target`. Resolution under each backend reads its own key.

The JSON that commands print has one more form, in which both variants carry a `backend` tag:

```json
{ "backend": "hc",    "address": "t7", "agent": "claude-code" }
{ "backend": "herdr", "pane_id": "w1:p2", "terminal_id": "term_abc", "agent": "claude" }
```

## JSON the commands print

These are the contract between the Rust binary and the mod. They live in `src/json.rs`, which has the types
and nothing else, and `decode.ts` is its twin. Every one carries `schema`, an integer, `1` today. One number
covers all of them. Any change a decoder would notice (a field removed, renamed or retyped, a meaning
changed) raises it. A field added that a decoder may ignore does not. A reader that sees another number
stops with `ContractMismatch`, and that is also how the v0.5.0 binary shows up if it answers at all.

Arrays hold their items in the order the TUI shows them. `null` is written, never left out, so a decoder
checks a field and not its presence. Strings are UTF-8, and nothing is escaped beyond JSON. One field is
optional and is left out when it has nothing to say: a row's `tokens`, below.

### `state`

```
{ "schema": 1,
  "root": "/abs/path",             the canonical work tree root
  "store_dir": "/abs/path",        the state directory, which may not exist yet
  "spec": "worktree" | "branch:<base>",   the spec this reply used. The same text as meta.json
  "base": "<ref>" | null,          the base ref asked for, else saved in meta.json, else the default base
                                   (`diff.md`), for offering the branch view. null when none resolves
  "rev": "<id>",                   what the work tree was compared against
  "notices": ["..."],              diff notices: a base that is missing, untracked files not shown
  "warnings": ["..."],             unreadable events, an unreadable meta.json
  "archivable": 3,                 the threads `archive` would move now, orphans included
  "unread_resolved": 1,            resolved threads `archive` leaves only because an agent's resolve is unseen
  "files": [ FileSummary ],
  "orphans": [ Thread ] }          threads placed `not_in_diff`

FileSummary
{ "path": "src/a.rs", "old_path": null | "src/b.rs",
  "change": "modified" | "added" | "deleted" | "renamed" | "untracked" | "binary" | "submodule"
          | "too_large" | "unparsed",
  "add": 12, "del": 3,             0 for a file with no rows
  "hash": "9e3779b97f4a7c15",      16 hex chars, FNV-1a of everything drawn for the file's diff: change, old
                                   path, mode flag, each hunk header and row (kind, line numbers, text),
                                   then the file's threads as `Thread` JSON. Differs from the fingerprint:
                                   it takes context, line numbers and the cards too
  "viewed": false,                 its fingerprint equals the one in meta.json
  "threads": 2, "open": 1, "new": 0, "unsent": 1 }   counts of the threads placed in this file
```

`archivable` counts the threads that are resolved and not new: a thread the user resolved, or one an agent
resolved that the user has seen. A thread an agent resolved and the user has not looked at is `new` and stays,
as it does in `archive`. Both read the same `Thread::archivable`, so the number is what the next `archive` moves
unless the review changes in between. `unread_resolved` is the rest of the resolved threads: the ones
`Thread::archivable` refuses, read through `Thread::unread_resolved`, which is "resolved and not archivable", so the
rule has one definition and the two numbers never count the same thread. A client uses it to say why Archive has
less to take than the resolved threads it can see. Both are required and integers. It is a field a decoder could ignore, so
schema stays 1, but the mod's decoder is strict like the per-file counts, and a binary without it fails the
decode and not the Archive button.

`threads` counts every thread placed in the file, resolved ones too, so a client knows which files hold a
thread before it loads them. `open` counts open threads, `new` counts threads an agent resolved that the user has not looked at, and
`unsent` counts open threads with a user comment that no send carried, or that the user reopened after a send. A resolved thread is not counted. A thread belongs to the file where it is
`matched` or `outdated`. A thread that is `not_in_diff` is in `orphans` and in no file.

### `file`

```
{ "schema": 1,
  "path": "src/a.rs", "old_path": null | "src/b.rs",
  "change": "...", "add": 12, "del": 3, "viewed": false,
  "note": null | "binary file" | "too large to show" | "submodule" | "could not be parsed",
  "rows": [ Row ],
  "threads": [ Thread ] }          matched and outdated threads of this path

Row
{ "kind": "hunk" | "ctx" | "add" | "del",
  "old": 41 | null, "new": 41 | null,
  "text": "...",                   no leading + or -. A hunk row holds the whole `@@ ... @@` line
  "tokens": [ [0, 2, "keyword"], [3, 7, "function"] ],     optional, see below
  "marks": [ { "from": 4, "to": 7 } ] }                    always written, `[]` when none, see below

TokenKind: "comment" | "string" | "number" | "keyword" | "operator" | "function" | "type"
```

`tokens` says which parts of `text` the highlighter the TUI uses colours. Each part is `[start, end, kind]`:
`start` and `end` count UTF-16 code units from the start of `text`, end exclusive, so `text.slice(start, end)` in
JavaScript is the part, with no conversion. A byte or code point count would be wrong for a row with a
non-ASCII character, and an emoji is two units. The parts of a row come in order, do not overlap and are not
empty. A stretch of `text` that is in no part has no colour of its own. The kinds are those of `theme::Token`,
which is what the TUI's theme has a colour for. The colours are the reader's: the TUI maps kinds to its Catppuccin
palette, the mod to `theme()` ([gui](gui.md)).

`tokens` is left out, and is never `null` or `[]`, when a row has none: a hunk header, a language the
highlighter does not know, a row the highlighter did not reach, a binary built without the `syntax` feature.
Left out and not written as `null` because it is the one field that is absent on most rows of a big diff, and
`"tokens":null` on every row would add a seventh to the reply. It is an addition under schema 1 and not a
bump, because a field a decoder may ignore does not raise `schema`: a mod from before this field ignores it and
draws plain, and the mod with a binary from before it finds no `tokens` and draws plain. The mod's decoder is
strict about every other field and loose about this one, because a wrong colour is not worth a banner that stops
the panel. A row's `tokens` come from the whole new side of the file, or the whole old side for a removed row,
as the pane's do, so a block comment or a string that opens above the hunk is coloured. A side over 1 MiB, or
one `git show` cannot give, is parsed from the rows of each hunk alone. `file` stops after 4,000 lines of the file
are parsed (the new side first), and the rows past that have no `tokens`: the pane parses a bit at a time in
the background and this command in one go, so it is bounded where the pane is not. In a release build that is under
half a second at most.

`marks` says which words of `text` changed, on a removed row and the added row that replaced it, as the TUI marks
them ([tui-layout](tui-layout.md)). Each is `{ "from", "to" }` in UTF-16 code units like `tokens`, end exclusive,
in order, not overlapping and not empty. Pairing is the TUI's: a run of removed rows pairs with the run of added
rows right after it, the first with the first, and `words::changed` compares the pair. A row with no partner, a
pair `changed` gives no marks (a rewritten line, a line of over 200 tokens), a context row and a hunk row have
`[]`. Unlike `tokens` it is written on every row, as `[]` when empty: Rust does the comparison and the panel only
draws it. It is an addition under schema 1, and a binary from before it sends none, which the decoder reads as no
marks. The decoder is as loose about `marks` as about `tokens`, since a missing highlight is not worth a banner.

A file that is not rendered (`binary`, `submodule`, `too_large`, `unparsed`) has `rows: []` and a `note`.
`old` is null on an `add` row, `new` is null on a `del` row, and both are null on a `hunk` row. A thread
whose `row` is not null has it as an index into `rows`.

### `Thread`

```
{ "id": "u7",                       the root comment's id
  "status": "open" | "resolved", "resolved_by": null | "user" | "agent:claude" | "agent",
  "is_new": false, "unsent": true,
  "placement": { "kind": "matched" | "outdated" | "not_in_diff", "row": 8 | null, "start_row": 6 | null },
  "anchor": { "path": "src/a.rs", "old_path": null | "...",
              "kind": "line" | "range" | "file",
              "side": "old" | "new" | null, "line": 42 | null, "end_line": 57 | null,
              "text": "..." | null, "spec": "worktree" | "branch:<base>" },
  "comments": [ Comment ] }         the root first, then the replies in order

Comment
{ "id": "u7", "parent": null | "u7", "author": "user" | "agent:claude" | "agent",
  "at": "2026-11-09T10:00:00Z", "body": "...", "sent": true, "edited_since_sent": false }
```

- `placement.row` is the row a card is drawn under: for `matched`, the last row of the anchor (the last row
  on the anchor's side whose line is one of the range's lines, and the row of the line for a single line), and
  for `outdated`, the row nearest where the comment was written. `start_row` is the first row of a `matched`
  range, and is null otherwise. A file comment is `matched` with both null. A `not_in_diff` thread has both
  null.
- `anchor.line` is the start of a range and `end_line` its end, and `end_line` is null for a single line.
  `side`, `line`, `end_line` and `text` are null for a file comment.
- `author` and `resolved_by` are the strings the log stores (`data.md`).
- `sent` is whether a send carried the comment. `unsent` on the thread is the fold's value: false for a resolved thread.

This is a different shape from `comment list --json`, which prints the store's own `Thread`. That output is
what agents read and has no `schema`, so it does not change.

### `send --json` and `archive --json`

```
send, exit 0:  { "schema": 1, "sent": 3, "queued": false, "agent": "claude-code" | null,
                 "target": Target JSON | null, "warnings": ["..."] }      sent 0 when there was nothing to send
send, exit 1:  { "schema": 1, "error": "ambiguous" | "no_agent" | "blocked" | "not_ready"
                                     | "agent_gone" | "busy" | "backend",
                 "message": "...",
                 "candidates": [ { "address": "t7", "agent": "claude-code", "cwd": "/abs", "status": "idle" } ] }
archive:       { "schema": 1, "archived": 2, "unsent": 1 }
```

`candidates` is empty except for `ambiguous` under the hc backend. Under herdr the picker is the TUI's, so the
list stays empty and `message` says how many agents matched. `status` is hc's plain word from `hc ls`. A usage error exits
2 and prints a line on stderr, not JSON.

### `message --json` and `message send`

```
message --json, exit 0:  { "schema": 1, "agent": "claude" | "claude-code", "status": "idle" | "working" | "blocked" | "done" | "unknown",
                           "id": "<message id>", "lines": ["...", ""] }
message send --file:     { "id"?: "<message id>", "comments": [ { "start": 2, "end": 3, "body": "..." } ] }
message send, exit 0:    { "schema": 1, "sent": 2 }
```

`lines` is the newest assistant message split into source lines, as the pane shows it, and `start` and `end` count
into it from 1, both included. `agent` is the name Herdr or hc reports. `status` is the word of `herdr agent get`, or hc's
`status.state`, and `unknown` when the agent has not reported. A failure prints no JSON: exit 1 with the reason on
stderr (the message cannot be read, or the delivery was refused), exit 2 when the file is wrong. `send --json`'s `error`
kinds are not used. Rust types: `MessageJson` and `MessageSentJson` in `json.rs`, `Target` in `message_json.rs`.
