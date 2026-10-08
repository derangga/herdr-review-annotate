# Shapes

Part of the [herdr-review design](../DESIGN.md).

Ids, each a newtype that can only be built by its parser:

| Type | Form | Built from |
|---|---|---|
| `CommentId` | `u<n>` or `a<n>` | Store allocation, CLI argument, JSONL field |
| `BatchId` | `b<n>` | Store allocation |
| `PaneId` | `w1:p2` | Herdr JSON, environment |
| `TerminalId` | `term_...` | Herdr JSON, environment |
| `RepoRoot` | Canonical absolute path | `git rev-parse --show-toplevel`, then `canonicalize` |
| `RelPath` | Repo-relative path with no `..` and no leading `/` | Diff parser, agent batch |

Records:

| Type | Fields |
|---|---|
| `Event` | `kind`, `at`, `by: Author`, and the fields in `data.md`, events |
| `Comment` | `id`, `parent`, `author`, `at` (the time of its `add` event), `body`, `sent_batch`, `edited_since_sent` |
| `Thread` | `root: Comment`, `anchor: Anchor`, `replies`, `status`, `is_new`, `unsent` |
| `Anchor` | `path`, `old_path`, `target: AnchorTarget`, `spec: Spec` |
| `Review` | `threads` in file order, `skipped_lines`, `ids` (the counters the next id comes from). The result of the fold |
| `Archived` | `threads`, `unsent`: how many threads one archive moved, and how many of them were never sent |
| `Meta` | `root`, `spec`, `target: Option<Target>`, `review_pane: Option<PaneId>` |
| `Target` | `pane: PaneId`, `terminal: TerminalId`, `agent: String` |
| `DiffFile` | `path`, `old_path`, `change`, `hunks`, `flags` |
| `Diff` | `files`, `rev` (what the working tree was compared against), `spec`, `notices` (cap reached, base missing, and so on) |
| `Select` | `row`, `half: Option<Side>`: where a range being selected started, and the half of a split row it started on. `View::select` holds one while visual mode is on, and `View::drag` is whether a left press in the stream is held. Neither is stored |
| `Filter` | `query`, `typing`: the sidebar's file filter. `View::filter` holds one while a filter is on, and `None` is no filter. `typing` says whether keys go to the query. It is a struct and not a two-variant enum because both states carry the same query and only the key router reads which one it is. Not stored |
| `View::collapsed` | A set of `RelPath`: the files folded to their header. `Stream::build` reads it into `folded` and `hidden`, one entry per file of the diff: whether the file is collapsed, and the threads it hides in the order of their lines. Not stored |
| `Keymap` | Key to `Action`, plus `warnings` |
| `SidebarConfig` | `open` (default true) and `icons` (default false), read once from `[sidebar]` into `View::sidebar` and `View::icons` |
| `Theme` | One colour per role, filled from the Catppuccin flavor `[theme] name` chose |
| `Env` | Every `HERDR_*` and `REVIEW_*` value, read once in `main.rs` |

Variants:

| Type | Cases |
|---|---|
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
| `CommandError` | `UnknownId { id, open }`, `InvalidBody(why)`, `InvalidBatch { index, why }`, `NotAllowed { id }` | The caller asked for something the review cannot do. Only the TUI's edit and delete build `NotAllowed`, for an agent's comment: the `comment` subcommands never edit or delete, so no request of theirs breaks the rights rule |
| `GitError` | `NotInstalled`, `NotARepo`, `NoBase { tried }`, `Failed { args, stderr }` | `git` could not answer |
| `HerdrError` | `code`, `message` | As parsed by `agent_delivery.rs` |
| `TargetError` | `NoAgent`, `Ambiguous(Vec<Target>)`, `Herdr(String)` | Resolution found zero or several agents, or `herdr agent list` failed |
| `Refusal` | `Blocked`, `NotReady`, `AgentGone` | The agent cannot take a prompt now |
| `Warning` | `SkippedLine(n)`, `Config(String)`, `MetaUnreadable`, `SentNotRecorded`, `TargetNotSaved` | Not an error. Collected and shown, the operation still succeeds |

There is no `ParseError` for diffs. A file the parser cannot read becomes `Change::Unparsed` and the
review continues.
