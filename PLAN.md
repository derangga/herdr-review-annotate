# herdr-review: technical implementation plan

Date: 2026-10-04. Sources: `docs/adr/` (the decisions that are hard to reverse), `CONTEXT.md` (terms).
`RESEARCH.md` and `RESEARCH.review.md` were removed; they stay in git history. The fixes they listed
are referenced below as B1 to B4, G1 to G20 and W1 to W24.

Section 9 lists every difference from `RESEARCH.md`.
Section 12 is the design pass: the types, the call graph of every operation, how each step fails, and
what each step needs. Read it before writing code for a milestone.

## 1. Scope of v1

The core loop, and nothing else:

1. The user opens a review pane beside the agent and reads the diff.
2. The user comments on a line, a range, or a file.
3. One key sends the unsent comments to the agent.
4. The agent fixes, then replies and resolves through the CLI.
5. The user sees the replies and resolved marks without restarting the pane.
6. The agent can also start a review by adding its own comments and opening the pane.

In v1: both diff specs, replies, resolve and reopen by either side, the `new` marker, the `outdated`
tag, resend, the quit prompt, edit and delete of your own comments. Archive was built after v1, with the
third round of feedback.

After v1, in this order: paste mode, side-by-side view, word-level diff, syntax highlighting,
Codex and pi, staged and single-commit specs, a reviewer agent.

Not planned: daemon, file watcher, outbox file, startup hook, generated skill, highlight marks, markup,
agent-driven navigation.

## 2. Repository layout

```
herdr-review/
  herdr-plugin.toml
  Cargo.toml
  src/
    main.rs            argument parsing and dispatch
    lib.rs             module list, so the copied files keep their `pub` items
    open.rs            the `open` action
    store.rs           event log: lock, append, read, fold
    meta.rs            meta.json: load, locked save, find the state directory
    diff.rs            git runner, unified diff parser, anchor matching
    agent.rs           the agent name for a `comment` command
    apply.rs           `comment apply`: decode, check, read the anchored lines, append
    cli.rs             arguments into a Command, a Command into output and an exit code
    comment.rs         what the `comment` subcommands read and print
    env.rs             Env: the process variables, read once in main.rs
    send.rs            prompt format, target resolution, mark sent
    tui.rs             review pane: layout, render, actions
    keymap.rs          default keys, config.toml overrides, key parsing
    theme.rs           colour roles, the Catppuccin flavors, the scope table for syntax colours
    syntax.rs          tokens per line of a file, and the cache of the files on screen
    view.rs            the stream of rows, the cursor, the sidebar, drawing
    cards.rs           a thread as the lines of its box
    editor.rs          multi-line comment editor   (adapted from herdr-annotate)
    edit_keys.rs       editor key map              (copied)
    width.rs           display width helpers       (copied)
    agent_delivery.rs  readiness check and prompt  (copied, see 2.1)
    herdr.rs           herdr CLI wrapper           (copied)
  skills/herdr-review/SKILL.md
  scripts/fetch-herdr-review.sh   prebuilt binary and SHA-256, for `plugin install`
  scripts/stage-local.sh          build and stage, for `plugin link`
  tests/fixtures/*.patch
  .github/workflows/ci.yml, release.yml
```

Dependencies: `ratatui` 0.30 (brings crossterm), `serde`, `serde_json`, `toml` (for the keymap file,
section 7.1), `chrono` with the `clock` feature. On Unix, `signal-hook` for clean terminal restore. Behind
the cargo feature `syntax`, which is on by default: `syntect` with `regex-fancy` and `default-syntaxes` and no
default features, and `two-face` with `syntect-fancy` for TypeScript, TSX, TOML and the other grammars `bat`
ships. Neither builds C code. `cargo build --no-default-features` leaves both out. No `uuid`, `notify`, `tokio`, `similar`,
`rustix` or git library in v1. Lints and the release profile are copied from
`herdr-annotate/rust/Cargo.toml`. `rust-version` is 1.89 or newer, for `std::fs::File::lock`.

### 2.1 What is copied from herdr-annotate, and what changes

| File | Change |
|---|---|
| `agent_delivery.rs` | Reword "annotations" and "focused pane" in the messages (`:73-75`, `:149`). Drop the five tests that import `archive_workflow` and `types::Annotation` (`:450-559`). Drop `Delivery::Paste` until paste mode is built. |
| `herdr.rs` | None. |
| `edit_keys.rs`, `width.rs` | None. |
| `termination.rs` | None. The TUI loop checks its flag on every tick (section 12.4). |
| `editor.rs` | Keep the text buffer, cursor and key handling. Remove the popup frame, the store calls and the invocation context (`:16-26`). It becomes a widget the TUI draws inline. |
| `layout.rs` | Not copied as a file. `layout_comment` moves into `editor.rs`, which is its only caller. |
| `format.rs` | Copy only `sanitize_terminal_text` (`:7-21`) into `tui.rs`. |
| `store.rs` | Not copied (ADR 0002). |

## 3. Data

### 3.1 Files

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

### 3.2 Events

Every event has `kind`, `at` (RFC 3339, display only) and `by` (`user`, `agent:<name>`, or plain
`agent` when the name is unknown, see section 5).

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

### 3.3 Writing

Every mutation is one call, `store::write(dir, now, build)`. It opens `lock` and tries
`File::try_lock` every 2 ms for up to 2 seconds, then gives up with `Busy`. A writer that releases the
lock waits 2 ms before returning, so a waiting process can take it. Without that pause, 8 processes in
a loop starved one that polled every 50 ms. Under the lock it reads
and folds `review.jsonl`, calls `build` with the folded review to validate the request and produce the
events, writes all lines with one `write_all` on an `O_APPEND` handle, and drops the lock. Ids (`u<n>`
for the user, `a<n>` for agents, `b<n>` for send batches) are the highest counter in the folded review
plus one. Section 12.4 has the graph.

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

### 3.4 Reading

Read the whole file and fold. Ignore a final line with no newline. Skip a line that does not parse and
count it, so the TUI can show "1 unreadable event". The TUI keeps the file length it last saw and
checks it every 250 ms on its input poll timeout. When the length differs, in either direction, it reads
and folds the whole file again. A review log is small, and this keeps a rewrite and an append the same case.
An archive by another pane is seen this way, because the rewritten log is always shorter: it loses at least
one thread's `add` and `resolve` lines and gains at most one `archived` line, which is shorter than the two.

## 4. Diff

### 4.1 Commands

Every `git` call runs with `GIT_OPTIONAL_LOCKS=0` in its environment, so the pane never takes
`index.lock` while the agent runs its own `git` commands.

Common flags, written as `GIT` below:
`git -C <root> -c core.quotePath=false diff --no-color --no-ext-diff --no-textconv --src-prefix=a/ --dst-prefix=b/ --submodule=short -M -U3`
and `--` after the revision. `--submodule=short` is there because `diff.submodule = log` in a user's config
would otherwise replace the two commit ids with a log.

| Spec | Command |
|---|---|
| Working tree | `GIT HEAD`, then `git ls-files --others --exclude-standard -z` |
| Branch | `GIT $(git merge-base <base> HEAD)`, then the same `ls-files` |

Default base: `origin/HEAD` if it resolves, else `main`, else `master` (`diff::default_base`, `NoBase` when
none does). A base that starts with `-` is treated as missing and never reaches `git`. `open --base <ref>` overrides
it and the choice is saved in `meta.json`.

### 4.2 Cases the pipeline must handle

| Case | Behaviour |
|---|---|
| Not a git repository | `open` shows a Herdr notification and exits non-zero |
| No commits (`HEAD` missing) | Diff against the empty tree, `git hash-object -t tree /dev/null` |
| Base missing | Message in the pane, working-tree spec offered |
| Empty diff | Message in the pane. Comments not in the diff are still listed |
| Untracked file | Stat first. Over 1 MiB or containing a NUL byte: listed, not rendered. Otherwise rendered as all-added lines. A symlink shows its target as one line. Anything that is not a file or a link (a nested repository) and any read error: listed, not rendered. The untracked files count against the 3 MiB cap too |
| Binary file | Listed with a "binary" row. File comments allowed |
| Total patch over 3 MiB | Files past the cap are listed and collapsed |
| Rename with edits | `old_path` from `rename from`. An old-side comment cites `old_path` (zeron's `cite_path`) |
| Submodule | One row with the two commit ids |
| CRLF | `\r` stripped for display and for anchor comparison |
| Deleted then recreated | Whatever `git` reports. No special handling |

Paths come from the `---`, `+++` and `rename` lines. A section with none of them (a binary file, a mode
change alone) has no other source, so the parser reads its `diff --git a/P b/P` line, and only when
both halves are the same path. A section it cannot read at all is `Unparsed`, listed under that path or
under a placeholder name.

### 4.3 Anchor matching (ADR 0008)

For each root comment whose `spec` equals the current spec:

1. Same path, side and line, and the line text is equal: matched.
2. Else the nearest row in that file's hunks, on that side, with equal text: matched at the new line.
3. Else, if the file is in the diff: outdated, drawn at the stored line or the closest hunk.
4. Else: listed in "comments not in this diff".

Step 1 and step 2 are one search: the row on that side with equal text nearest the stored line, the earlier
one on a tie. The stored line itself is distance 0. A file is found by `path`, or by `old_path` when an agent
cited the old name. A file comment is matched whenever its file is in the diff. `Outdated.near` is the row on
that side nearest the stored line, in any hunk.

Comments written against the other spec go to the same list. Ranges match on their first line.

### 4.4 Reload

The diff reloads when the store changes (the agent replying is the moment its fix landed), when the
user presses `R`, and when the pane regains focus. It never reloads while the editor is open.

## 5. Command line

```
herdr-review tui    [--repo <root>]
herdr-review open   [--repo <root>] [--base <ref>]
herdr-review send   [--repo <root>] [--all-open]
herdr-review comment apply   [--repo <root>] [--name <agent>] --stdin
herdr-review comment list    [--repo <root>] [--status open|resolved] [--author user|agent] [--json]
herdr-review comment reply   [--repo <root>] [--name <agent>] <id> -
herdr-review comment resolve [--repo <root>] [--name <agent>] <id> --reply -
herdr-review comment reopen  [--repo <root>] <id>
```

- `--repo` defaults to `git rev-parse --show-toplevel` of the current directory. Sent prompts always
  include it, because an agent's `cwd` can be a parent of the repository.
- `-` reads the text from stdin. There is no form that takes the text as an argument, so shell
  substitution in agent text cannot run (G8).
- `comment apply` reads `{"comments":[{"path","side","line","end_line","body"} | {"reply_to","body"}]}`
  and validates the whole batch before writing. Limits are in section 12.2.
- The CLI reads the anchored line's text itself: from the worktree file for the new side, from
  `git show <base>:<path>` for the old side. A line that does not exist rejects the batch.
- `resolve` on a resolved thread and `reopen` on an open thread exit 0 and write nothing.
- Exit codes: 0 done, 1 failed and worth retrying later (busy, disk, git), 2 the request is wrong
  (usage, unknown id, not allowed, invalid batch).
- Everything the `comment` subcommands write is authored by an agent. The user writes through the TUI.
- `side` defaults to `new` when a `line` is given. `end_line` equal to `line` is a one-line comment. A
  file comment (`path` only) is not checked against the worktree, so a deleted file can be commented.
  An old-side line is read at `HEAD`, or at the merge base for the branch spec, under the path the
  agent gives. A renamed file therefore needs its old path.
- Agent name: `--name` wins. Otherwise the CLI runs `herdr agent list` and takes the `agent` field of
  the entry whose `pane_id` is `$HERDR_PANE_ID`, or else of the single entry whose `cwd` matches the
  current directory, and writes `agent:<name>`. Outside Herdr, or with no single match, it writes
  `agent`. A failed Herdr call never fails the command.
- A wrong id prints the open ids (G7). Every failure prints one line on stderr.
- `comment apply` ends with `herdr notification show "N review comments from <name>"`.

`open`:

1. Find the root. From an action, use `focused_pane_cwd` in `HERDR_PLUGIN_CONTEXT_JSON`. From an
   agent's shell, use the current directory.
2. Find the agent pane: the focused pane when it hosts an agent, else `$HERDR_PANE_ID` when
   `herdr agent get` accepts it, else resolution as in 6.2.
3. If `meta.json` has a `review_pane_id` that still exists, run `herdr plugin pane focus <id>` and stop
   (G13).
4. Otherwise run `herdr plugin pane open --plugin review --entrypoint tui --placement split
   --direction right --target-pane <agent pane> --cwd <root> --env REVIEW_DELIVER_TO=<pane id>
   --env REVIEW_DELIVER_TERM=<terminal id> --focus`.

The TUI writes its own `HERDR_PANE_ID` to `meta.json` as `review_pane_id` when it starts.

## 6. Send

### 6.1 Prompt

```
Address the review comments below. For each one: make the change, then resolve it with a one-line
reply. If you disagree or are unsure, reply and leave it open.

Resolve:
'<bin>' comment resolve --repo '<root>' <id> --reply - <<'EOF'
<one line>
EOF
Reply without resolving:
'<bin>' comment reply --repo '<root>' <id> - <<'EOF'
<text>
EOF

Comments on the diff (L = line in the original file, R = in the changed file):
- [u7] src/lib.rs:42 (R): body text
  more lines indented by two spaces
- [u8] src/lib.rs:50-57 (R): comment on a range
- [u9] src/store.rs (file): comment on the whole file
- [u3] src/cli.rs:10 (R), reopened: the original comment
  > agent: Added with_capacity
  > user: still allocates twice
- [a2] src/cli.rs:88 (R), your comment: the agent's comment, first line only
  > user: reply text
```

`<bin>` is `$HERDR_PLUGIN_ROOT/bin/herdr-review`. Both paths are single-quoted with `'` escaped.
`send` and the TUI both run as Herdr plugin commands, so the variable is always set.

### 6.2 Target resolution (ADR 0006)

A candidate is valid when `herdr agent get <pane>` passes `agent_ready`, returns the stored
`terminal_id`, and has a `cwd` equal to the root, inside it, or an ancestor of it.

1. `REVIEW_DELIVER_TO` and `REVIEW_DELIVER_TERM` from the pane's environment. Only the TUI has these.
2. `target` in `meta.json`.
3. `herdr agent list`: the agent with the stored `terminal_id`, else agents whose `cwd` is the root or
   inside it, else a single agent in the workspace whose `cwd` is an ancestor of the root.
4. Several matches: the TUI shows a picker. The `send` action sends a notification that asks the user
   to pick in the review pane.
5. None: refuse.

After a Herdr restart every `terminal_id` is new (ADR 0006), so a stored target never matches and step 3
falls to `cwd`.

The chosen target is written to `meta.json`. The `send` action never has step 1, so it relies on what
the TUI saved.

### 6.3 Steps

1. Collect unsent threads. With `--all-open` or the resend key, collect the chosen open threads too.
2. None: say "nothing to send".
3. Resolve the target. Call `deliver_to_agent(Delivery::Send, pane, text, run_herdr_output)`.
4. On refusal, show the reason in the TUI and as a notification. Nothing is marked.
5. On success, append one `sent` event with the ids and a batch id, then show `sent N to <agent>`, or
   `agent is working, N comments queued` when the status was `working`.

## 7. TUI

One thread, one loop: `event::poll(250 ms)`, then the store length check. Layout: file sidebar, one stream of all files, comment cards under their lines, unified view
only in v1.

| Action name | Default keys | What it does |
|---|---|---|
| `up`, `down` | `k` `up`, `j` `down` | Move the cursor one row |
| `page_up`, `page_down` | `pageup`, `pagedown` | Scroll one page |
| `prev_hunk`, `next_hunk` | `[`, `]` | Previous or next hunk |
| `prev_thread`, `next_thread` | `shift+n`, `n` | Previous or next thread |
| `scroll_left`, `scroll_right` | `h` `left`, `l` `right` | Scroll the code of the diff 8 cells sideways |
| `scroll_reset` | `0` | Scroll back to the start of the lines |
| `switch_panel` | `tab` | Switch between sidebar and stream |
| `toggle_sidebar` | `f` | Show or hide the sidebar |
| `comment` | `c` | Comment on the line, the selected range, or the file when the cursor is on a file header |
| `select_range` | `v` | Start a range |
| `reply` | `r` | Reply to the focused thread |
| `edit`, `delete` | `e`, `d` | Edit or delete your own focused comment |
| `resolve` | `x` | Resolve or reopen the focused thread |
| `archive` | `shift+a` | Move the resolved threads out of the review, after asking |
| `send` | `shift+s` | Send unsent |
| `resend` | `s` | Resend the focused thread |
| `reload` | `shift+r` | Reload the diff |
| `switch_spec` | `b` | Switch diff spec |
| `toggle_layout` | `t` | Side by side or unified |
| `help` | `?` | Show every action with its current keys |
| `quit` | `q` | Quit. With unsent comments, ask send, keep, or stay |

The mouse wheel scrolls (the sideways wheel scrolls the code) and a click moves the cursor. These are not remappable.

How the body behaves (built in M4):

- The sidebar groups the files under a heading for their directory (`./` for the repository root), in the
  diff's order, so a directory that comes again later gets a second heading. A file row is a `•` when a thread
  hung in it is unsent, a letter for how it changed (M modified, A added, D deleted, R renamed, ? untracked,
  B binary, S submodule, L too large, ! unparsed) in the colour of the change (M warning, A added,
  D and ! removed, R accent, ? agent, B S L subtle), the file's name, and its added and removed line counts
  against the right edge. The name is cut with `…` before the counts. The sidebar is left out below 50
  columns and follows the file under the cursor. With the sidebar focused, `up` and `down` move to the
  previous and next file, and the page keys move by a page of files. A click on a heading does nothing.
- `toggle_sidebar` hides the sidebar and shows it again. While it is hidden the stream has the whole
  width and is laid out again, as after a resize, so cards and the editor box are as wide as the pane.
  Hiding it moves the focus to the stream. While the sidebar is not drawn, because it is hidden or the
  pane is under 50 columns, `switch_panel` does nothing and the focus stays on the stream. In a pane
  under 50 columns the key still flips the state, which shows once the pane is wider, and the status
  line says the pane is too narrow to show the sidebar. The state is not saved: `areas` in `view.rs` is
  the one place that decides whether the sidebar is drawn, from the state and the width.
- Next and previous hunk and thread move to the next row after, or the previous row before, the cursor. From
  inside a hunk, previous hunk goes to that hunk's own header. A thread is at the first row of its card.
- A file with no hunks has one row that says why (binary, too large, unreadable, mode changed).
- The sideways keys move the code of every diff row 8 cells at a time, in the unified and the split layout
  alike, and stop where the widest line of the diff ends. The line numbers, the sign, hunk headers and cards
  stay where they are. A `‹` takes the first cell of a row cut off on the left, and a `›` the last cell of a
  row cut off on the right. The offset starts over when the cursor moves to another file, and the sidebar
  ignores the keys.
- The wheel moves three rows. A click in the stream or the sidebar moves the cursor there and focuses it.
- The status line is a bar in the theme's header colour across the whole line. Its left part is the state:
  the diff on screen as a chip (` WORKING TREE ` or ` VS MAIN `, upper case, base colour on the accent
  colour), then `→ claude w8G:p1` in the text colour or `→ no agent` in the removed colour, then a chip
  ` 3 unsent ` on the warning colour when a thread is unsent. A chip is coloured cells with one space of
  padding on each side and no border glyphs.
- Against the right edge the line names the keys of `toggle_sidebar`, `send`, `resolve`, `reload`,
  `help` and `quit` as the keymap has them: `f sidebar  S send  x resolve  R reload  ? help  q quit`. The six
  need 54 cells. `sidebar` comes first, so it is the first to drop: an 80 column pane with an agent
  named on the left shows the keys from `send` on. A key is bold in the accent colour and
  its label is in the subtle colour. The keys keep two cells clear of the left part. When they do not fit,
  they drop off from the left, so `help` and `quit` go last.
- A message takes the place of the state and the keys stay. It is a notice in the warning colour for
  something that happened or that the cursor's place does not allow (`sending`, `deleted u2`, `no thread
  here`), and a failure in the removed colour for something that went wrong (a reload error, a refused
  send, a busy review, a write that failed). The next action clears it.
- `help` opens an overlay of every action, and any key closes it without doing anything else.
- `switch_spec` flips between the working tree and the branch spec and saves the choice, with the base
  (`meta.base`, else `origin/HEAD`, `main`, `master`), in `meta.json`. With no base it says so and stays.
- A reload puts the cursor back on its row in the same file, or on its line of the same card.

How the layout behaves (built after M6):

- A pane 120 columns wide or more draws the diff side by side, old on the left and new on the right, and a
  narrower one draws it unified. `toggle_layout` forces the other layout until it is pressed again or the pane
  restarts, and the width stops deciding. The cursor stays on its line across the change.
- Side by side pairs a run of removed lines with the run of added lines after it, line by line. The longer
  run's extra lines sit opposite an empty half. A context line is on both halves with its own numbers.
- Side by side marks the lines the diff leaves out, above the first hunk and between hunks, with a row that
  says `▾ N unchanged lines`. Unified does not, so its rows are the diff's rows.
- A file header has its name on the left and the added and removed counts on the right. Removed and added
  rows are tinted across the row.
- A row of a split diff has two halves. `comment` and `select_range` use the new half, or the old half of a
  row with no new line. A click on a half chooses it for that row until the cursor moves. A range takes the
  side of its first row and ends at the last line on that side.
- A card and the editor box hang under their row where `note_box` in `view.rs` puts a note, as hunk does.
  Side by side, in a stream of 84 cells or more, the box is the half its line is on: the old half for a
  comment on the old side and the new half for one on the new side. In the unified layout, in a narrower
  split stream, for a file comment and in the block of threads not in the diff, the box starts four cells
  into the stream and runs to its right edge. It is never narrower than 28 cells unless the stream is.
- A code row under the mouse shows `[+]` in its gutter, in both layouts. In a split row it is on the half under
  the mouse, where the sign is, so the line number stays visible. A click on it opens the comment editor on that
  line and half, as `comment` does. Until the mouse has moved once, which tells the pane that Herdr delivers
  motion, the `[+]` is on the cursor's row instead, and a click elsewhere only moves the cursor. There is no
  drag to select a range.

How syntax colours behave (built after M6, behind the `syntax` feature):

- A code row is drawn token by token. `syntect` only splits a line into scopes. The colour of a token comes
  from the theme: `SCOPES` in `theme.rs` maps a TextMate scope and everything under it to one of seven tokens
  (comment, string, number, keyword, operator, function, type), and the theme has one colour for each, so the
  colours follow the flavor. The innermost scope with an entry decides. Text between tokens is the theme's
  text colour.
- A token's colour replaces the green or red of an added or removed row. The sign keeps that colour, and the
  tint stays behind the whole row.
- The language comes from the file's name, then its extension. A file in a language `syntect` and `two-face`
  do not know is drawn as before, each row in the colour of its kind.
- A file is highlighted against its whole text, so a row inside a block comment or a multi-line string is
  coloured as one. The new side is read from the work tree. The old side is `git show <rev>:<path>`, with the
  old path of a renamed file, where `rev` is what the diff was taken against and `Diff` keeps. A side with no
  row of its own is not read: context rows use the new side.
- A row takes the tokens of its line only when that line of the file still reads as the row does. A file
  edited after the diff was loaded draws the rows that moved plain until the next reload.
- A side that cannot be read, is not UTF-8, or is over 1 MiB is highlighted hunk by hunk instead, each hunk
  as a snippet of its rows on that side. A snippet cannot know it starts inside a comment or a string.
- A file is highlighted when one of its rows first comes into the window, before the frame is drawn, and its
  tokens are kept until the diff is loaded again. Only the tokens of the lines the diff shows are kept.
- A build without the feature knows no language, reads nothing, and draws every row as before.

How the cards behave (built in M5):

- A card is drawn under the line its thread is placed at, or under the file header for a file comment. An
  outdated thread is drawn under the nearest row, or under the file header when the file has no row on that
  side. Several threads on one line stack in the review's order. Where the box goes and how wide it is
  are under "How the layout behaves".
- An open card is a rounded box, in the theme's warning colour for the user's thread and its agent colour for
  an agent's. The top border reads `● <author> · <age> · <path> R<line>`: the author is `Your note` for the
  user and the agent's name otherwise (`agent` when it has none), the side letter is `L` or `R`, a range is
  `R101-110`, and a file comment has the bare path. Then come the badges `[outdated]` (on an open thread
  only), `[new]`, `(edited since sent)` and `[unsent]`. A path that does not fit is cut from the left with `…`,
  and is left out when there is no room for it. The id is not on an open card.
- Inside the box are an empty row, then the line text as it was when the comment was written, for a thread
  that is outdated or not in the diff, then the body wrapped to the box, then each reply indented under it.
  The text keeps a cell clear of each side.
- The bottom border holds the keys on the right, read from the effective keymap: `reply`, `edit` and `delete`
  for the user's thread, and only `reply` for an agent's. Each is the action's first key as the keymap prints
  it, so the defaults read `r reply  e edit  d delete`, and an action with no key is left out.
- The age is `now` under a minute, then `2m`, `3h`, `2d`, counted from the `at` of the comment's `add` event
  to the time the stream was laid out. No timer runs, so the age on an idle pane is as old as its last layout.
  A comment whose time is not RFC 3339 has no age.
- A resolved thread is one line: a green `✓` in place of the bullet, the id, `[new]`, who resolved it and the
  first line of its last comment, with no `outdated` tag.
- Threads that are not in the diff, because their file is not in it or they were written against the other
  spec, are listed in a block at the top of the stream under "Comments not in this diff". A resolved one
  also says the path and line it pointed at, which an open card has on its border. With an empty diff the block is shown above the message. A
  thread in the block is reached with next and previous thread, like any other.
- The thread a key acts on is the one whose card holds the cursor, or the first one hung under the line the
  cursor is on. A card's height is its number of lines, so the stream is laid out again when the pane's
  width changes. Both borders of a box belong to its root comment, so `edit` and `delete` on the bottom
  border act on the root.

How comments are written (built in M5):

- `comment` reads what the comment points at when the key is pressed and keeps it until the text is saved. On
  a diff line it is that line, on the new side, or on the old side for a removed row. On a file header, or the
  note of a file with no rows, it is the file. A hunk header is refused. On a card it is the row the card
  hangs under, and a card in the not-in-diff block is refused.
- `select_range` starts a range at the cursor and a second press drops it. The range runs to the cursor, stays
  inside one file, takes the side of its first line, ends at the last line on that side, and keeps the text of
  its first line. A reload drops it. `comment` uses it and ends it.
- The editor is a rounded box in the theme's warning colour, placed and sized as the card of the saved
  comment will be. Its text has an empty row above it and a cell clear of each side. It is drawn under the
  cursor row, or under the last row of the range a new comment points at when that is lower,
  and above that row when there is no room below. It grows with its text up to two thirds of the stream's
  height. `Ctrl+S` saves and `Esc` cancels.
- The top border reads `Draft note - <path> R<line>` for a new comment: `L` for the old side, `R101-110` for a
  range, and the bare path for a file comment. A path that does not fit is cut from the left with `…`. A reply
  reads `Reply to <id>` and an edit reads `Edit <id>`. The bottom border holds `^S save  Esc cancel` on the
  right. An empty editor shows a dim `Write a note…`.
- While the editor of a new comment is open, the row or range it points at has the selection tint and a bar in
  its first cell, and no `[+]` is drawn. The bar gives way to a digit of a line number.
- A failed save keeps the editor open with its text, and the reason replaces the keys on the bottom border and
  is on the status line. The diff is not reloaded while the editor is open: a store change, `reload` and
  regaining focus wait until it closes.
- A saved comment, reply, edit, delete, resolve or reopen is written with `actions.rs`, then the pane reads the
  log again and lays the stream out. It does not reload the diff. A new comment or reply moves the cursor to
  its thread's card.
- `reply` replies to the thread the cursor is on. `edit` and `delete` act on the comment whose line of the card
  the cursor is on, or on the root when it is on the line above the card. They refuse an agent's comment.
  Deleting a root deletes its thread and nothing asks first. `resolve` flips any thread, whoever wrote it.
- A store change is read within one tick, 250 ms, whoever wrote it, and the diff is reloaded with it. A log
  that is shorter than the one read, or gone, is read again from the start.
- A resolved card says `[new]` while an agent's resolve has not been looked at. After a key or a click that
  leaves the cursor on a new thread, on its card or on the line it hangs under, one `seen` event is written.
  The marker stays cleared after a restart, and moving around the thread writes nothing more. A write that
  fails is a warning, and the next key tries again.
- When the pane is told to end, or its terminal fails, with text in the editor, the text is written as the
  comment, reply or edit it was for.

How archive behaves (built after M6):

- Resolved threads leave the pane only through `archive`. Nothing is archived or hidden on its own.
- `archive` takes every resolved thread except the ones still marked `[new]`, so an agent's resolve the
  user has not looked at stays. A resolved thread that was never sent goes too.
- It asks first, in a box like the quit prompt: `archive 3 resolved threads (1 never sent)?`, then
  `[y] yes` and `[n] no`. The part in brackets is there only when some were never sent. These two
  keys are not remappable, and any other key leaves the question open. The numbers are counted from the
  review when the box is drawn.
- With nothing to archive the status line says `nothing to archive` and no box opens.
- After `y` the status line says `archived 3`, with the number the store moved, and the pane reads the log
  again. The threads are gone from the pane, from the unsent count and from the agent's `comment list`.
  A busy review or a failed write is a failure on the status line, and the threads stay.
- There is no viewer, no unarchive and no agent command for it. Only the user archives.

How send behaves (built in M6):

- `send` sends the unsent threads and `resend` sends the thread under the cursor again, with the text it has
  now. A resolved thread is not resent. Both draw "sending" before the Herdr call, since the call blocks the
  pane.
- Several matching agents open a picker (`up`, `down` or `k`, `j`, then `enter`, `esc` cancels). The choice is
  saved in `meta.json`, so the next send does not ask, and the send that asked runs at once.
- A refusal is on the status line and in a notification, and nothing is marked sent.
- `quit` with unsent comments asks: `s` sends and then quits (a refusal keeps the pane open), `k` quits and
  keeps them unsent, `esc` stays. These keys are not remappable.
- The status line names the target as `→ claude w1:p2`, or `→ no agent`. The target is resolved when the
  pane starts and after each send, and what is found is saved for the `send` action.

### 7.1 Keymap file

The user can change any key in the table. The file is `config.toml` in the plugin config directory
(`HERDR_PLUGIN_CONFIG_DIR`, printed by `herdr plugin config-dir review`). It holds `[keys]`, `[theme]` and
`[sidebar]`.

```toml
[keys]
send = "ctrl+s"                 # one key
next_hunk = ["]", "ctrl+n"]     # several keys
switch_spec = ""                # unbound
```

- Key spelling follows Herdr's config: `ctrl+`, `shift+`, `alt+` and named keys such as `enter`, `tab`,
  `pageup`. `shift+s` and `S` mean the same key.
- An action that is not listed keeps its default keys.
- When a configured key is also another action's default, the configured binding wins and the other
  action loses that key. The pane shows one warning line that names each action left with no key.
- Two configured actions on one key: the earlier in the table above keeps it, and an action left with no key is named in the same warning line. An action whose listed keys all fail to parse keeps its defaults. `[]` unbinds like `""`.
- An unknown action name or a key that does not parse is skipped with a warning. A file that is not
  valid TOML is ignored as a whole with a warning. The pane always starts.
- The file is read once when the pane starts. The footer and the `help` overlay are drawn from the
  effective keymap, never from hard-coded text.
- Keys inside the comment editor (cursor movement, save, cancel) are not in `[keys]`. They stay as
  `edit_keys.rs` defines them.
- The agent CLI, `open` and `send` do not read the file.

The same file chooses the pane's colours:

```toml
[theme]
name = "catppuccin-latte"
```

- The names are `catppuccin-mocha` (the default), `catppuccin-macchiato`, `catppuccin-frappe` and
  `catppuccin-latte`. A `config.toml` with no `[theme]`, or a `[theme]` with no `name`, is mocha.
- A name that is not one of the four, a `name` that is not a string, and a `theme` that is not a table are
  each one `Warning::Config`, and the pane starts in mocha.
- The table is read once when the pane starts, with `[keys]`.
- `theme.rs` is the only module that names a colour. Every other module draws with a role of `Theme`: base,
  text, subtle text, border, accent, agent, cursor, selection, added, removed, their two tints, filler,
  header, popup, warning and success. The tint behind an added or a removed row is the flavor's green or red
  mixed 15 parts in a hundred into its base, so it follows the flavor.
- The pane paints the theme's base behind everything and its text colour on unstyled text, so it does not
  show the terminal's own background. The colours are 24-bit. A terminal without truecolor is not handled.

The same file says whether the sidebar starts open:

```toml
[sidebar]
open = false
```

- `open` is `true` or `false`. A `config.toml` with no `[sidebar]`, or a `[sidebar]` with no `open`, starts
  with the sidebar shown.
- An `open` that is not a boolean and a `sidebar` that is not a table are each one `Warning::Config`, and
  the pane starts with the sidebar shown.
- The table is read once when the pane starts, with `[keys]` and `[theme]`. `toggle_sidebar` flips the
  state for the session and writes nothing, so the next start takes the file's value again.

### 7.2 Rules

- Every string from the store or from `git` passes through `sanitize_terminal_text` before it is drawn.
- `c` captures the anchor when it is pressed. The diff is frozen while the editor is open (G16).
- A thread is marked seen, with one `seen` event, when the cursor lands on it while it is `new`.
- The `outdated` tag is drawn on open threads only.
- The status line is the state on the left and the keys on the right, drawn in roles of `Theme` and
  never reversed. Every message that replaces the state is a notice or a failure, and the place that
  sets it says which.
- The target is resolved at start and after each send, not on every frame.
- Rendering builds rows for the visible window only. Each file keeps its row count so scrolling does
  not lay out files that are off screen.

## 8. Plugin manifest and skill

```toml
id = "review"
name = "Review"
version = "0.1.0"
min_herdr_version = "0.9.1"
platforms = ["macos", "linux"]

[[build]]
command = ["bash", "scripts/fetch-herdr-review.sh"]

[[actions]]
id = "open"
title = "Review: open diff"
contexts = ["workspace", "pane"]
command = ["./bin/herdr-review", "open"]

[[actions]]
id = "send"
title = "Review: send comments to agent"
contexts = ["pane"]
command = ["./bin/herdr-review", "send"]

[[panes]]
id = "tui"
title = "Review"
placement = "split"
command = ["sh", "-c", "exec \"$HERDR_PLUGIN_ROOT/bin/herdr-review\" tui"]
```

Herdr keys. `prefix+r` and `prefix+shift+r` are Herdr's defaults for resize mode and reload config
(W3), so they replace decision 12's keys. The README suggests `prefix+i` to open and `prefix+shift+i`
to send (decided 2026-10-04). Neither is in `herdr --default-config`, the user's `config.toml`, or the
herdr-annotate README. These bindings live in the user's own Herdr `config.toml` as `plugin_action`
entries, so the user can pick any other key there.

`skills/herdr-review/SKILL.md` is a static file. It says:

- Use it only when `HERDR_ENV=1`.
- Find the binary: `herdr plugin list --plugin review --json`, field `plugin_root`, plus
  `/bin/herdr-review`. Read it each time, do not remember it (ADR 0005).
- When a prompt lists review comments, follow the commands in that prompt.
- To review your own changes: pipe a JSON batch to `comment apply --stdin`, run `open`, then end the
  turn. The user's replies arrive as the next message.

Install: `plugin install` runs the fetch script. `plugin link` does not, so `stage-local.sh` builds and
copies the binary to `bin/`. On this machine plugins are linked from the Nix store, so the Nix package
must build the binary itself.

## 9. Differences from RESEARCH.md

| RESEARCH.md | This plan | Reason |
|---|---|---|
| Copy `store.rs`, `flock` | New store with `File::lock` | B2, ADR 0002 |
| `notify` on the review file and the worktree | Length check on the poll timeout, reload on store change | G15 |
| `startup` hook, `binary-path` file, `skill` subcommand | `herdr plugin list --json`, static skill | B1, ADR 0005 |
| Pane id as target | Pane id plus `terminal_id`, checked before send | B3, ADR 0006 |
| `base...HEAD` | Working tree against the merge base | B4, ADR 0007 |
| Anchor hash | Line text | G2, ADR 0008 |
| Cards only under diff lines | Plus a "comments not in this diff" block | B4 |
| Short random ids `u-7f3a` | Counters `u7` | G6 |
| `comment reply <id> "<text>"`, then `resolve` | Text from stdin, `resolve --reply -` | G8, G20 |
| Reopen is the only way to send again | Resend key and `--all-open` | G1 |
| Outbox file over 8 KB | Removed | Untested guess. Herdr rejects oversized pastes itself |
| `context`, `files`, `comment add`, `rm`, `clear`, `archive` | Removed or deferred | Not needed for the core loop. Archive is a key of the pane and not a command |
| `r` for reload and reply | `R` reload, `r` reply | Key conflict |
| `prefix+r`, `prefix+shift+r` | `prefix+i`, `prefix+shift+i` | W3, decided |
| Fixed keys in the pane | `[keys]` in `config.toml` | Decided: the user can remap review actions |
| Config setting for placement | Not in v1, the config holds keys only | Decided |
| Edit after sent: open question | Stays sent, marked "edited since sent", resend by hand | Decided |
| `agent:<name>` from an unspecified source | Auto-detect through `herdr agent list`, `--name` overrides | Decided |
| Paste mode and archive in v1 | After v1 | Decisions 2 and 7 stand. Archive is built (section 3.3), paste mode is not |
| macOS state in `~/Library/Application Support` | `~/.local/state` on both systems | One rule |

## 10. Milestones

Each milestone ends with a check that a person can run.

### M1. Skeleton and spikes

Build: Cargo project, manifest, `stage-local.sh`, an `open` action that opens a pane printing its
environment and `HERDR_PLUGIN_CONTEXT_JSON`, and a hidden `spike-send` action that sends a fixed
two-line prompt to the agent pane through `agent_delivery.rs`.
The `spike-send` action was removed before the first release, once `send` covered it.

Checks:

- `herdr plugin link .` succeeds, and the action opens a split to the right of the agent.
- The pane prints `REVIEW_DELIVER_TO` and `REVIEW_DELIVER_TERM` (W15).
- `spike-send` to an idle Claude Code starts a turn. To a working Claude Code, the prompt is handled
  after the current turn. Record what happens when the user presses Esc while it is queued (W13).
- Move the agent pane to another workspace, and close a sibling pane. Record whether the pane id
  changes and whether `terminal_id` stays the same (B3). This accepts or rejects ADR 0006.
- Bind the two proposed keys and confirm Herdr reports no duplicate.

### M2. Store and agent CLI

Build: `store.rs`, and `comment apply`, `list`, `reply`, `resolve`, `reopen`.

Checks:

- Unit tests for the fold: rights, unsent, `new`, unknown ids, delete of a root.
- A test starts 8 processes that each append 200 events. The file has 1600 valid lines and no
  duplicate ids.
- A test truncates the last line mid-record. The reader returns every earlier event.
- `comment resolve zz9 --reply -` exits 2 and prints the open ids.
- The store-write and agent-CLI rows of section 12.7 pass, with no real `git` or Herdr in the tests.

### M3. Diff engine

Build: `diff.rs` with the git runner, the parser, both specs, and anchor matching.

Checks:

- Fixture tests: rename with edits, binary, no newline at end of file, CRLF, a path with spaces and
  non-ASCII characters, submodule, new and deleted files, a patch over the cap.
- A test in a temporary repository: no commits, untracked file, base missing, base equal to `HEAD`.
- Anchor tests for the four outcomes in 4.3.

### M4. TUI, read-only

Build: sidebar, stream, scrolling, hunk keys, reload, spec switch, messages for the empty and error
cases.

Checks:

- Opened on the `hunk/` checkout with a 50-file diff, the first frame appears with no visible delay.
  Record cold start and peak memory with `/usr/bin/time -l`.

  Measured on 2026-10-05, release build, macOS, `/usr/bin/time -l`, on a local clone of `hunk/` with 50
  `.ts` files each given two added lines (`git diff HEAD` is 20 KB), a `q` sent as the pane starts: wall time
  0.07 s warm (0.35 s on the first, cold run), maximum resident set size 8.6 MB, peak memory footprint
  2.4 MB. That covers `git` for the root, `HEAD`, the diff and `ls-files`, the first frame, and exit.
- Scrolling to the end and back shows no stale rows.
- A file whose name contains an ESC byte does not corrupt the screen.
- Unit tests for the keymap: an override, a list of keys, an unbind, a conflict where the user's
  binding wins, an unknown action, invalid TOML.
- With `reload = "r"` in `config.toml`, `r` reloads, the footer shows it, and a warning says `reply`
  has no key.

### M5. User comments

Build: editor widget, cards, threads, `c`, `r`, `e`, `d`, `x`, `v`, `n` `N`, the not-in-diff block, the
`outdated` tag, live pickup of agent writes, the `new` marker.

Checks:

- A comment on a line, a range and a file survives closing and reopening the pane.
- With the pane open, `comment reply` from another shell appears within one second.
- Editing the commented line in an editor and pressing `R` tags the thread outdated. Committing
  everything moves the thread to the not-in-diff block.
- An agent resolve shows `new` until the cursor reaches the thread.
- `comment reply` run from the Claude Code pane is labelled `agent:claude`. Run from a plain terminal
  outside Herdr it is labelled `agent`.

### M6. Send

Build: `send.rs`, `S`, `s`, the `send` action, the quit prompt, the status line, the real `open`.

Checks, the v1 acceptance test:

- With Claude Code idle: write two comments, press `S`. Without the user typing anything, the agent
  edits the code, and both threads show a reply and a resolved mark in the pane.
- Press `S` again: "nothing to send". Reopen one thread, press `S`: only that thread goes out, marked
  reopened.
- Edit a sent comment: `S` says "nothing to send" and the card shows "edited since sent". `s` on that
  thread sends the new text.
- With the agent at a permission dialog: `S` is refused, the reason is shown, the comments stay
  unsent.
- With focus in the review pane, the `prefix` key for send delivers to the same agent as `S`.
- Two agents in one worktree: the picker appears once and the choice is remembered.
- A comment body containing `` `id` `` and `$(id)` reaches the agent as text.

### M7. Agent-initiated review and release

Build: the skill, `open` from the agent's shell, the notification, the fetch script, CI (fmt, clippy,
tests on macOS and Linux), the release workflow with SHA-256 files, the README.

Checks:

- Ask Claude Code to "review your changes with herdr-review". Comments appear in a pane it opened, and
  a second `open` focuses that pane instead of adding a split.
- The user replies to an agent comment and presses `S`. The agent receives the reply.
- `herdr plugin install <repo>` on a clean machine yields a working `prefix` key.
- Measure hunk and herdr-review on the same diff with `/usr/bin/time -l` and record both.

### After v1

Paste mode (`p`), side-by-side view, word diff, syntax
highlighting behind a cargo feature, Codex and pi, staged and single-commit specs, reviewer agent.

## 11. Open questions and spike findings

Settled on 2026-10-04 and written into the sections above: the Herdr keys, the keymap file, edit after
sent, and the agent name.

Spike findings, all on Herdr 0.9.1 with the plugin linked from this checkout (2026-10-04):

- **`--env` reaches the pane process (W15).** `herdr plugin action invoke open --plugin review`, then
  `herdr pane read w8D:p4` showed `REVIEW_DELIVER_TO=w8D:p1` and
  `REVIEW_DELIVER_TERM=term_65cfeec9f35116` among the pane's variables. Both match `herdr pane list`
  for the agent pane. The pane's own `HERDR_PANE_ID` is its own id (`w8D:p4`), and its
  `HERDR_PLUGIN_CONTEXT_JSON` holds the agent pane as `focused_pane_id`, with
  `invocation_source: "api"`. The plan holds.
- **The `send` action runs from the review pane, and its context names the review pane (W24).** With the
  review pane focused, `herdr plugin action invoke send --plugin review` was accepted and started the
  command. Its context had `focused_pane_id` = the review pane (`w8D:p6`), `focused_pane_status:
  "unknown"` and no `focused_pane_agent`. So from the review pane the action never sees the agent
  pane, and section 6.2 step 2 (`meta.json`) is what finds it, as the plan says. Not tested: whether
  Herdr's command palette or key handler offers a `contexts = ["pane"]` action in the review pane.
  `action invoke` is the only path the CLI exposes. Check it by hand once `prefix+shift+i` is bound.
- **`prefix+i` and `prefix+shift+i` bind without a warning.** `herdr config check` on a copy of the
  user's `config.toml` with these two entries appended (`XDG_CONFIG_HOME=/tmp/hc herdr config check`)
  printed `config: ok`. A control entry on `prefix+a`, which the config already binds, printed
  `prefix+a: kept keys.command[4].key, disabled keys.command[9].key`, and an invalid key printed
  `invalid keybinding`. So `check` does report duplicates between custom commands. It printed `ok` for a
  `plugin_action` on `prefix+r`, which W3 says is a default, so it does not report a clash with a built-in.
  It also printed `ok` for `command = "review.nosuch"`, so it does not check that the action exists.
  The user's `config.toml` is a read-only link into the Nix store, so the keys were not added to it.

- Target platform: `min_herdr_version = "0.9.1"` matches what is installed. Raise it only if a later
  spike finds a bug fixed in a later release. None has.

## 12. Design pass

This section applies the design-thinking method (shapes, happy path, failures, dependencies) to every
operation. It uses the Effect model `Effect<A, E, R>` and maps it onto Rust:

| Effect | Rust in this project |
|---|---|
| `A`, the success value | The `Ok` type of a function |
| `E`, the error channel | The `Err` type, one enum per module. Errors are values until the edge |
| `R`, the requirements | Function parameters: a `git` closure, a `herdr` closure, a state directory, a clock value, an `Env` struct. No globals, no `std::env` or `Command` calls below `main.rs` |
| `gen` body is A, `pipe` is E | Inner functions use `?` only. Each error is matched once, at the edge that owns the reaction: `main.rs` for the CLI, the action dispatcher for the TUI |
| Retry, escape, die | Retry is a bounded loop at the node. Escape returns a fallback value and records a warning. Die is `panic!`, reserved for a broken invariant in our own code |
| Layer swap for tests | Pass a closure that returns canned output and a temp directory |
| Scope | A guard value whose `Drop` releases the resource |

Graph notation: steps start with `->`, nested steps are indented, `R:` names what the step needs and
`E:` names how it fails and what happens then.

### 12.1 Shapes

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
| `Event` | `kind`, `at`, `by: Author`, and the fields in section 3.2 |
| `Comment` | `id`, `parent`, `author`, `at` (the time of its `add` event), `body`, `sent_batch`, `edited_since_sent` |
| `Thread` | `root: Comment`, `anchor: Anchor`, `replies`, `status`, `is_new`, `unsent` |
| `Anchor` | `path`, `old_path`, `target: AnchorTarget`, `spec: Spec` |
| `Review` | `threads` in file order, `skipped_lines`, `ids` (the counters the next id comes from). The result of the fold |
| `Archived` | `threads`, `unsent`: how many threads one archive moved, and how many of them were never sent |
| `Meta` | `root`, `spec`, `target: Option<Target>`, `review_pane: Option<PaneId>` |
| `Target` | `pane: PaneId`, `terminal: TerminalId`, `agent: String` |
| `DiffFile` | `path`, `old_path`, `change`, `hunks`, `flags` |
| `Diff` | `files`, `rev` (what the working tree was compared against), `spec`, `notices` (cap reached, base missing, and so on) |
| `Keymap` | Key to `Action`, plus `warnings` |
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
| `Action` | The action names in section 7 |
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

### 12.2 Boundaries

Untrusted data enters at eight places. Each is parsed once, into the shapes above, and code past the
boundary never sees a raw string or `serde_json::Value`.

| Boundary | Parsed into | On bad input |
|---|---|---|
| `review.jsonl` line | `Event` | `Warning::SkippedLine`, the read continues |
| `meta.json` | `Meta` | `Warning::MetaUnreadable`, an empty `Meta` is used and rewritten on the next save |
| `config.toml` | `Keymap`, `Theme`, the sidebar's starting state | `Warning::Config`, defaults are used |
| `git` stdout | `Diff` | `Change::Unparsed` for that file |
| `herdr` stdout and stderr | `Target`, `AgentStatus`, `HerdrError` | `Refusal::AgentGone` or the raw message |
| Environment | `Env` | A missing value is `None`. A pane id is trusted only after `agent get` confirms it |
| CLI arguments | `Command` enum | Usage text, exit 2 |
| Agent stdin (batch, reply text) | `Vec<NewComment>`, `String` | `CommandError::InvalidBatch`, nothing is written |

Limits checked at the agent stdin boundary: a body is at most 16 KiB after trimming and must not be
empty, a batch holds at most 200 comments, a path must parse as `RelPath`, a line must be 1 or more
and exist in the file. Every body is stored as written. Control characters are removed when it is
drawn and ESC is removed when it is sent.

### 12.3 Requirements

Six dependencies exist. Every function below `main.rs` receives the ones it needs as parameters.

| Name | Production | Tests |
|---|---|---|
| `git` | `Command::new("git")` with `GIT_OPTIONAL_LOCKS=0` | A closure that returns fixture bytes |
| `herdr` | `herdr.rs`, through `HERDR_BIN_PATH` | A closure that records calls and returns canned JSON, as `agent_delivery.rs` tests do today |
| `dir` | The state directory from section 3.1 | A temp directory |
| `now` | `chrono::Utc::now()` formatted once per operation | A fixed string |
| `env` | `Env::from_process()` | An `Env` literal |
| `term` | crossterm on stdout | ratatui `TestBackend` and a list of key events |

`GIT_OPTIONAL_LOCKS=0` stops `git diff` from taking `index.lock` to refresh the index. Without it the
review pane can make the agent's own `git` commands fail while both run in one worktree.

### 12.4 Call graphs

#### Write to the store (every mutation goes through this)

```
-> store::write(dir, now, build)
  -> open lock file                        R: dir     E: Io -> propagate
  -> try_lock, up to 2 s                              E: held -> retry every 2 ms, then Busy
  -> read review.jsonl, fold               R: dir     E: Io -> propagate
                                                      E: bad line -> escape, Warning::SkippedLine
  -> build(&Review) -> Vec<Event>                     E: CommandError -> propagate, nothing written
       allocates ids from the folded state
  -> one write_all on O_APPEND                        E: Io -> propagate
  -> drop guard, lock released             scope: released on every path, also on panic or kill
```

`build` is where an operation validates against the current state. Validation and append happen under
one lock, so a `resolve` cannot race with a `delete` of the same thread.

Cardinality: one-shot.

#### Archive (the one rewrite of the log)

```
-> store::archive(dir, now)
  -> open lock file                        R: dir     E: Io -> propagate
  -> try_lock, up to 2 s                              E: held -> retry every 2 ms, then Busy, neither file changes
  -> read review.jsonl line by line, fold  R: dir     E: Io -> propagate
                                                      E: bad line -> escape, the line stays in the log as it was
  -> take the threads resolved and not new (pure)     none -> Archived { 0, 0 }, nothing written
  -> split the lines into moved and kept   (pure)     a sent event of both kinds is split in two
  -> append moved to archive.jsonl, sync   R: dir     E: Io -> propagate, review.jsonl as it was
  -> write kept to a temp file, sync       R: dir     E: Io -> propagate, review.jsonl as it was, thread in both files
  -> rename it over review.jsonl                      E: Io -> remove the temp file, propagate, review.jsonl as it was
  -> drop guard, lock released             scope: released on every path
```

The pane's side:

```
-> Action::Archive
  -> count the threads to take             (pure)     none -> notice "nothing to archive", no prompt
  -> Prompt::Archive                                  esc -> nothing written
  -> store::archive                        R: dir, now   E: Busy -> failure "review is busy, press again"
                                                         E: Io -> failure with the file's name
  -> read the log again, lay the stream out
```

Decisions that fall out of the graph:

- The store counts under the lock, so it archives what is resolved then and not what the prompt counted.
  A thread an agent resolved in between is `new` and stays. A comment an agent adds in between is in the
  log the archive reads, so it is neither lost nor moved unless its thread is one the archive takes.
- Every failure leaves `review.jsonl` as it was. Pressing the key again is always safe.

Cardinality: one-shot.

#### Agent CLI: `comment apply`, `reply`, `resolve`, `reopen`

```
-> parse args                              R: env     E: usage -> exit 2
-> resolve root                            R: git     E: NotARepo -> exit 1
-> read stdin, decode batch or text                   E: InvalidBatch -> exit 2, nothing written
-> for each new root comment: read the anchored line
  -> new side: read the file in the worktree  R: dir of repo  E: missing line -> InvalidBatch
  -> old side: git show <base>:<path>         R: git          E: missing line -> InvalidBatch
-> detect agent name                       R: herdr, env   E: any -> escape, Author::Agent(None)
-> store::write                            R: dir, now     E: UnknownId -> exit 2
                                                           E: Busy, Io -> exit 1
-> notification show (apply only)          R: herdr        E: any -> escape, ignored
-> print ids as text or JSON
```

Rules that fall out of the graph:

- `resolve` on a resolved thread and `reopen` on an open thread succeed and write nothing. An agent
  that repeats a command does not see an error.
- The anchored line text is read by the CLI, not supplied by the agent. An agent cannot anchor a
  comment to text that is not there.
- Nothing after `store::write` can turn a success into a failure.

#### `open`

```
-> find root
  -> from HERDR_PLUGIN_CONTEXT_JSON        R: env     E: absent or unparseable -> escape, use cwd
  -> git rev-parse --show-toplevel         R: git     E: NotARepo -> notify, exit 1
-> load meta                               R: dir     E: unreadable -> escape, empty Meta
-> review pane already open?
  -> herdr pane get <review_pane>          R: herdr   E: pane_not_found -> continue to open
  -> herdr plugin pane focus               R: herdr   E: any -> continue to open
-> resolve agent pane (section 6.2)        R: herdr, env   E: NoAgent -> open without a target
                                                           E: Ambiguous -> open without a target
-> herdr plugin pane open                  R: herdr   E: HerdrError -> notify, exit 1
```

A review with no target still opens. The user can read and comment, and the picker runs at the first
send. An action has no terminal, so every failure of `open` is shown with `herdr notification show`.

#### TUI start

```
-> load keymap                             R: env (config dir)   E: any -> escape, defaults + warning
-> load theme                              R: env (config dir)   E: unknown name, bad table -> escape, mocha + warning
-> load sidebar state                      R: env (config dir)   E: not a boolean, bad table -> escape, shown + warning
-> find root, load meta                    R: git, dir           E: NotARepo -> message screen
-> enter raw mode and alternate screen     R: term    scope: restored by a guard on exit, panic, SIGTERM, SIGHUP
-> store read, fold                        R: dir     E: Io -> message screen with the path
-> load diff (below)                       R: git     E: see below
-> save review_pane to meta                R: dir     E: Io -> escape, warning
-> resolve target for the status line      R: herdr   E: any -> escape, status line shows "no agent"
```

The pane never exits on a startup error. It shows the error and waits for `reload` or `quit`, so the
user can read what went wrong.

#### Load the diff

```
-> git diff for the spec                   R: git     E: NotInstalled -> message screen
                                                      E: NoBase -> notice, fall back to WorkTree
                                                      E: Failed -> message screen with stderr
-> git ls-files --others                   R: git     E: Failed -> escape, notice "untracked files not shown"
-> parse patch into DiffFile               (pure)     E: unreadable file section -> Change::Unparsed
-> stat and read untracked files           R: fs      E: Io -> that file listed, not rendered
-> apply the 3 MiB cap                     (pure)
-> place every thread (section 4.3)        (pure)     -> Placement per thread
```

Parsing and placement are pure functions of bytes and threads, so all of milestone 3 is tested without
`git` installed.

Cardinality: one-shot, called again on each reload. A reload that fails keeps the previous diff on
screen and shows the error in the status line.

#### TUI loop

This is the only stream in the program. It merges two sources.

```
-> loop
  -> termination flag set?                 R: signal flag   -> save draft (below), leave the loop
  -> poll terminal, 250 ms                 R: term    E: Io -> leave the loop, restore terminal
    -> key -> Keymap -> Action             (pure)     no binding -> ignored
    -> apply Action to the app state       R: per action, see the next graphs
  -> store length changed?                 R: dir     E: Io -> escape, warning, retry next tick
    -> read from offset, fold, reload diff
  -> highlight the files on screen         R: git, repo dir   E: unreadable, not UTF-8, over 1 MiB -> escape, hunk snippets
                                                              E: unknown language -> escape, plain rows
  -> draw                                  R: term
```

The loop holds no lock between ticks. A panic hook restores the terminal before the message prints.

If the pane is closed while the editor holds text, the draft is saved as a comment at the anchor that
was captured when `comment` was pressed. Losing typed text is the one data-loss path in the pane.

#### User comment, reply, edit, delete, resolve

```
-> Action (comment, reply, edit, delete, resolve)
  -> editor returns text, or cancel        (pure state)
  -> store::write                          R: dir, now   E: Busy -> status line "review is busy, press again"
                                                         E: Io -> status line, the editor keeps the text
  -> fold the new events into the state
```

On a failed save the editor stays open with the text. The text is discarded only after the write
succeeds.

#### Send

```
-> collect unsent threads from Review      (pure)     none -> SendOutcome::Nothing
-> format prompt                           R: env (plugin root), root   (pure)
-> resolve target (section 6.2)
  -> candidates from env, meta             R: env, dir
  -> herdr agent get                       R: herdr   E: mismatch or not found -> next candidate
  -> herdr agent list                      R: herdr   E: HerdrError -> propagate
                                                      E: NoAgent -> propagate
                                                      E: Ambiguous -> picker (TUI) or notification (action)
-> deliver_to_agent(Send, pane, text)      R: herdr   E: Refusal -> propagate, nothing marked
                                                      E: HerdrError -> propagate, nothing marked
-> store::write(sent event)                R: dir, now   E: any -> escape, Warning::SentNotRecorded
-> save target to meta                     R: dir     E: Io -> escape, warning
-> SendOutcome::Sent or Queued
```

Decisions that fall out of the graph:

- No retry anywhere in send. A second `herdr agent prompt` after an unclear failure could deliver the
  batch twice, so the user decides by pressing the key again.
- Once the prompt is delivered the operation is a success. If the `sent` event cannot be written, the
  user sees "sent, but not recorded, the next send will repeat these comments".
- The TUI draws "sending" before the Herdr call, because the call blocks the single thread.
- `send` as a Herdr action runs the same function. Its edge turns every `Err` and `Warning` into
  `herdr notification show` and an exit code.

### 12.5 Meta file

`meta.json` is rewritten on every save, and three processes write it (`open`, the TUI, the
`send` action). A save takes the same lock as the store, reads the current file, changes its own
fields, writes a temp file and renames it. A save that fails is a warning and never blocks a review
action.

### 12.6 Where each error is handled

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

### 12.7 Tests that the graphs give

Each `E:` line above is one test, and each test swaps only the parameters listed under `R:`.

| Graph | Swapped | Tests |
|---|---|---|
| Store write | temp `dir`, fixed `now` | Lock held by another process gives `Busy` after 2 s. A killed holder frees the lock. `build` returning an error writes nothing |
| Archive | temp `dir`, fixed `now` | A held lock gives `Busy` and changes neither file. A bad line stays. Nothing to archive writes nothing. An archive that cannot be appended to and a rename that fails each leave the log as it was. Other processes appending beside it lose nothing and get no id twice |
| Agent CLI | fixture `git`, recording `herdr`, temp `dir` | Unknown id, a second `resolve` is a no-op, a line past the end of the file, an oversized body, a `herdr` failure does not fail the command |
| `open` | recording `herdr`, `Env` literal | No context JSON, review pane already open, stale review pane, no agent |
| Load diff | fixture `git` | Every row of section 4.2, and a failed reload keeps the old diff |
| TUI loop | `TestBackend`, key list, temp `dir` | A write from another process appears, a failed save keeps the editor text, termination with a draft saves it |
| Highlight | recording `git`, temp repo dir | A side that cannot be read falls back to its hunks, so does one over 1 MiB, an unknown extension reads nothing |
| Send | recording `herdr`, temp `dir` | Each `Refusal`, a stale pane id falls through to the list, `Ambiguous`, `sent` not recorded still reports success |

If a test needs a real `git`, a real Herdr or a real terminal to exercise one of these graphs, the
function has a hidden dependency and the design is wrong. The temporary-repository tests in M3 and
the manual checks in M6 are the only places that use the real tools, and they test the tools'
behaviour, not our graphs.

### 12.8 What the pass changed in this plan

- A store write is now read, validate, append under one lock, with a 2 second bounded wait
  (section 3.3).
- `meta.json` saves take the store lock and use temp file plus rename (12.5).
- `git` runs with `GIT_OPTIONAL_LOCKS=0` (section 4.1).
- The agent CLI reads the anchored line itself and needs `git` for old-side comments (section 5).
- Agent input has size limits (12.2).
- Repeating `resolve` or `reopen` is a no-op with exit 0.
- A diff file that cannot be parsed is shown as unparsed and does not stop the review.
- A failed reload keeps the previous diff.
- A review opens without a target agent, and the target is chosen at the first send.
- A draft is saved when the pane is closed, and kept in the editor when a save fails.
- Exit code 1 means "try again later" and 2 means "the request is wrong".
- `termination.rs` joins the files copied from herdr-annotate.
