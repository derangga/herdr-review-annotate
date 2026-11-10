# The GUI: a herdr-app mod over the CLI

Part of the [herdr-review design](../DESIGN.md). The decision is [ADR 0011](../docs/adr/0011-the-gui-is-a-herdr-app-mod-over-the-cli.md). What
herdr-app does, as found by running it, is in [herdr-app-facts](herdr-app-facts.md). The graphs are
`graphs/gui-*.md`. The Rust commands the mod calls are in [cli](cli.md), their JSON is in [shapes](shapes.md).

## What it is

A TypeScript mod for herdr-app. It draws a panel beside the agent: the changed files, the unified diff of
every changed file in one scrolling stream, and the comment threads under their lines. The user points at a line to get its `[+]`,
drags over lines to select a range, clicks the `[+]` to write a comment, and sends the unsent comments to the agent. The agent replies and resolves from its own
shell, and the panel updates by itself.

The mod holds no review logic. It runs the `herdr-review` binary for every read and every write and draws
what comes back. The diff, the anchors, placement, the event log, the fold, the prompt and the choice of
target agent stay in Rust, once. The mod and the TUI show the same review because both read it through the
same code.

In v1:

- A sidebar of the changed files grouped by folder, with an icon for each file type and change counts, beside one scrolling stream of every file's unified diff. The selected file in the sidebar is the topmost file on
  screen, and a click on a file scrolls the stream to it.
  A header button hides and shows it and another moves it to the other side. The panel remembers both.
- A filter box at the top of the sidebar, as the TUI's `filter`: typing narrows the file list to the paths that
  match, and `/` goes to the box.
- Comments on a line, a range or a whole file, as in the TUI. The `[+]` shows only on the code line under the mouse; a plain click on a
  line highlights nothing and clears a range; holding the mouse on a line and dragging over others selects the lines between, tinted;
  the `[+]` opens the editor on the line, or on the whole range when the line is inside it; Esc clears the selection. There is no
  cursor and no shift-click. Threads with reply, resolve, reopen, edit and delete.
- Next and previous comment, by two buttons in the bar under the stream and by the keys `n` and `shift+n`, across
  files, as the TUI's `next_thread` and `prev_thread`.
- Unsent and new markers, send, resend of one thread, archive of resolved threads, a file marked viewed (which folds it
  to its header, as in the TUI; a click on the header opens it).
- The working tree or the merge base, and a live refresh.

- Syntax colours in the code rows, from the same highlighter as the TUI: Rust sends token kinds in `file --json`
  and the mod colours them from `theme()`. No highlighter runs in the mod. A build
  of the binary without the `syntax` feature, or one older than the mod, sends none and the rows draw plain.

Left out on purpose, each a follow-up: side-by-side layout, changed words inside a line, the
agent-message review, vim motions, the whole-file view, a key for the sidebar and a key to fold.

## Method: how Effect maps onto plain TypeScript

[method](method.md) maps `Effect<A, E, R>` onto Rust. The mod is plain TypeScript, because `import { Schema }
from "effect"` fails in this runtime with `TextEncoder is not defined` (herdr-app-facts, section 2). The same
three channels map like this:

| Effect | TypeScript in the mod |
|---|---|
| `A`, the success value | The value an `async` function returns. A handler is a straight line of `await` calls. It has no `try` and no `catch` |
| `E`, the error channel | A tagged object `{ _tag: "Busy" }` that an inner function throws, the way a Rust function uses `?`. `withRecovery(handler)` wraps each handler, catches a tagged object and turns it into `Msg.Failed`. The reducer reads the table below and decides. A thrown value that is not a tagged object is a bug, and the wrapper rethrows it |
| `R`, the requirements | The `host: Host` parameter. A function takes `host` and calls nothing else outside the process. There is no module-level `exec` call below `host.ts` |
| Retry, escape, die | There is no retry (below). Escape is the reducer keeping the last good model and setting a banner or a status line. Die is a `throw` of a plain `Error`: an unhandled `Msg`, a row index out of range |
| Layer swap | `FakeHost` in place of the real `Host` |
| Scope | `close(cx)` releases what the copy holds (Scope, below) |

Strip-only type stripping (`node --test`) rejects `enum` and constructor parameter properties. The tagged
objects are `type` unions, and `import type` is used for types.

## Shapes

```
ids        CommentId ("u7", "a2")   TerminalAddress ("t7")   RepoRoot (absolute)   RelPath
records    State, FileSummary, FileView, Row, Thread, Comment, Anchor, Placement, Candidate
           Model, Editor, Selection, Hover, Drag, Grip, Panel
variants   Spec          WorkTree | Branch { base }
           Change        modified | added | deleted | renamed | untracked | binary | submodule
                         | too_large | unparsed
           Placement     matched | outdated | not_in_diff, with a row index or null
           Msg           what happened: Opened, Triggered, Loaded, Hovered, RowPressed, Released, PlusClicked, SelectionCleared,
                         FileCommentClicked, EditorSaved, EditorCancelled, ThreadAction, SendClicked, PickerChosen, PickerCancelled, ArchiveClicked, ArchiveConfirmed, DeleteConfirmed, ConfirmCancelled, Archived, ViewedClicked,
                         SpecChosen, FileChosen, ThreadJumped, FilesLoaded, Scrolled, Glanced, Wheeled, ThumbGrabbed, ThumbMoved, TrackClicked, Resized, FoldToggled, SidebarToggled, SidebarFlipped, FilterChanged, FilterFocused, FilterOpened, FilterEntered, FilterCleared, PanelLoaded, Failed, Wrote, Sent ...
           Cmd           what to do next: Load, LoadFiles, WatchStore, Write, Send, Archive, SetViewed, Notify, ReadPanel, SavePanel
errors     BinaryMissing | TooOld | ContractMismatch { path } | Busy | BackendFailed { code, stderr }
           BadRequest { stderr } | Refused { Blocked | NotReady | AgentGone }
           NoTarget | AmbiguousTarget { candidates } | WatchUnavailable
```

`State`, `FileSummary`, `FileView`, `Row`, `Thread`, `Comment`, `Anchor`, `Placement` and `Candidate` are the
JSON that [shapes](shapes.md) defines, decoded once. `Placement` is computed by Rust and never stored. A row
index in it counts into the `rows` of the same `file` reply.

- `Model`: `root`, `target` (a terminal address or null), `spec` (null until the first reply names the spec
  `meta.json` holds, so the first load passes no flag), `key` (root and spec as one value, used to drop a stale
  reply), `state` (the last good one, or null), `files` (the diffs fetched so far, by path), `selected` (the sidebar's
  file, a path or null: the topmost file on screen, or the one a click chose), `top` (the key of the first item on
  screen) and `visible` (the paths of the files on screen, topmost first), `reveal` (a scroll not made yet:
  the key of an item to put at the top), `width` and `height` (the stream's box, for the height of a card and the part of the stream
  on screen), `offset` (how far the mod has scrolled the stream, in pixels from its top) and `total` (the stream's height as the last
  `Scrolled` saw it), `hover` (the code row under the pointer), `drag` (a press on a row that has not been released) and `grip` (a press
  on the scrollbar's thumb that has not been released), `folds` (the files the
  user folded or opened, by path; a file not in it is folded when it is viewed), `dirty` (fetched files to fetch again),
  `fresh`, `glance` and `seenSent` (the new resolved cards on screen, as of the last scroll and the last tick, and the threads a seen was sent for), `skipped` (files whose last fetch failed) and `batch` (the files the running fetch is for), `expanded` (a set of root ids: the
  resolved threads the user opened), `selection`,
  `focus` (the comment the thread buttons act on), `confirm` (the comment a click on delete asked about, until Delete or Cancel answers), `jump` (a next or previous comment that waits for its file to be fetched: the path and `first` or `last`),
  `editor`, `saving` (a draft is being written, so a second save does nothing), `picker` (candidates or null), `sending` (a send runs, so a second click does nothing), `sendThreads` (the threads of the send the picker interrupted), `confirmArchive` (a click on archive asked, until Archive or Cancel answers), `sticky` (the status is a result a reload must not clear), `banner`, `status`, `inFlight`, `pending`, `stopped` (after
  `BinaryMissing`, `TooOld` or `ContractMismatch`: only Refresh loads again), `storeWatch` (the store
  directory the watch loop runs on, or null), `retried` (a `BadRequest` already led to a reload that has
  not succeeded yet), and `panel` (`side`, `left` or `right`, and `open`, whether the sidebar shows; the defaults are
  left and shown, and "The panel's own settings" below says where they are kept), `filter` (the sidebar's query,
  empty for no filter; it is not saved) and `filtering` (the query box holds the keyboard).
- A question is a bar, not a second click. While `confirm` or `confirmArchive` is set, the bar under the stream shows
  a sentence, a button named for the action (`Delete`, red; `Archive`, the accent) and `Cancel` in place of the other buttons,
  and only that button writes. `rows.ts` builds it (`confirmRow`) and `view.ts` draws it. Esc is a key down on that
  bar, whose focus handle is given the keyboard when the question opens, and it is the same message as `Cancel`
  (`ConfirmCancelled`). The first click sets one field and nothing else, so `Cancel` puts the model back exactly.
  A reload keeps the question while the comment is still there, because the timer reloads every 10 seconds.
- The Archive button knows how many threads it would move: `State.archivable` from `state --json`. It reads
  `Archive 3`, plain `Archive` for none, and is off for none as well as during a load (`rows.ts` `dock`). Its question
  names the number, `Archive 3 resolved threads?` and `Archive 1 resolved thread?`. A reload that brings the count
  to 0 drops the question, and one that changes it to another number keeps it with the new number.
  Archive only takes a resolved thread whose agent resolve the user has seen (`Thread::archivable`), so
  `State.unread_resolved` counts the resolved threads it leaves for that reason. When it is above 0 the question adds
  a clause, `Archive 1 resolved thread? 1 more resolved by the agent is unread.` (`are unread` for more). With none
  the sentence is as it was. When nothing is archivable and some are unread, the button is off and `rows.dock` returns a
  note that `view.ts` draws in `dim` next to the buttons: `Nothing to archive yet: 2 resolved by the agent are unread
  (marked new)`. A note beside the button and not a changed label, because the bar's left label is for the focused
  comment or the selection and the button must read as the same button whose count comes back when the cards are seen.
  With something archivable the question carries the unread ones and the bar has no note.
- A resolved card an agent closed counts as seen once it has been on screen, as the TUI counts a cursor landing on it.
  `Scrolled` carries `fresh`, the ids of the cards on screen whose thread is resolved and `new` (`rows.freshOn`, read from
  the head item's `new` badge), and the model keeps it with `glance` (`fresh` as the last tick saw it) and `seenSent`
  (the ids a seen was already sent for). `main.ts` starts a `cx.timer.every(500)` that dispatches `Glanced`; a tick
  sends `Cmd.Write { seen }` for one id that is in both `fresh` and `glance` and not in `seenSent`, then sets `glance`
  to `fresh`. So a card must stay on screen for a whole tick, a fast scroll marks nothing, and at most one seen goes per
  tick, so a screen of new cards is a short queue and not a burst. The write is the one opening the card sends, run
  by `runner.ts` like any other, never from a draw. Opening a card also adds its id to `seenSent`. A failed seen keeps its
  handling (nothing shown) and is not retried by looking; opening the card sends it again.
- Next and previous comment. `ThreadJumped { forward }` comes from two buttons, `Previous comment` and `Next
  comment`, and from the keys. The buttons are the first two of the second row of the bar under the stream, with
  Send, Archive and Mark viewed: the header already wraps on a narrow panel, and the bar is where the other
  thread buttons are. They are off when the review has no thread, and the bar is replaced by the editor or
  the question while those are up, so they are not there then. The row wraps when it is narrow.
  - The order is the TUI's. The threads not in the diff come first (they are drawn above every file), then
    the files in the sidebar's order, and in a file the cards as they are drawn: the ones with no row, then
    by row (`cardOrder`; the reply lists threads in the order the review wrote them, which is not that). A
    resolved thread is a stop too, and it stays folded. A file whose threads are not drawn, because its diff has
    not been fetched yet or because it is folded, and whose `FileSummary.threads` is above zero, is one stop,
    `{ path }`, and the jump lands on its first card going forward and its last going back. `rows.stops`
    builds the list and `rows.jumpStop` picks.
  - From the focused thread, next is the stop after it and previous the one before, and the ends wrap round:
    next from the last thread goes to the first, previous from the first to the last. A focus on a reply counts
    as a focus on its thread. With nothing focused, next is the first thread of the selected file, and previous
    the last stop before that file (the threads not in the diff, or the file above). The row selection is
    not used, because the panel has no cursor, and the top of the stream is what is on screen when
    nothing is chosen. With no thread in the review the buttons are off and a key sets the status
    `no comments in this review`.
  - A stop that is a thread already drawn sets `focus` and `reveal` (the key of its card) at once and starts no
    load. A stop that is a file opens it (a folded file is unfolded, as the TUI opens a collapsed file on a jump). When its
    diff is fetched the jump lands at once. When it is not, the stream scrolls to its header, `jump` holds the
    target, and the `FilesLoaded` that brings the file finishes it: it sets `focus` and `reveal` to the first or last card,
    and clears `jump`. `focus` stays where it was until then, so a failed or slow fetch leaves the screen as it was. A
    `FileChosen`, a `SpecChosen` or a failed fetch drops `jump`. A file that arrives with no thread says `no comments in <path>`.
  - Showing the thread. The mod scrolls the stream itself. `reveal` is the card's key, and `rows.scrolled` turns it into the item's
    start in pixels, the new `offset`; "The stream" below says how.
  - Keys. A key reaches `on_key_down` only along the focus path, so the root element tracks a focus handle
    of its own (`nav`) and `rows.panelKey` says what a key means: `n` is next, `shift-n` or `N` previous, and
    nothing else, and nothing at all while the editor, the question bar or the picker is up, so typing `n`
    in a note types `n`. The root does not track the handle while the editor is open, so a click in the text
    box cannot take the keyboard. `main.ts` gives `nav` the keyboard after a jump, when the editor or a
    question closes, and when herdr-app sends the `focus` command to this copy (`runner.listen`, reading
    `nextCommand`). Clicking the pane also gives it the keyboard, and herdr-app-facts says what is known.
  - Commands. `runner.listen` also takes the mod commands `next-comment` and `previous-comment`, which
    `herdr-app do herdr-review.next-comment` and a `keys.json` binding to that name deliver to every shown
    copy, whatever has the keyboard. A user who wants a key that needs no click binds it there, for example
    `"mod:herdr-review": { "alt-n": "herdr-review.next-comment" }`; the mod never edits `keys.json`.
- The stream. The graph is [gui-stream](graphs/gui-stream.md). One stream holds every file: the threads not in the diff, then for each
  file a header row, and under it the rows and cards of its diff. The mod scrolls it itself and draws only the items on screen.
  - `rows.streamLayout(model, now)` returns `{ items, sizes, starts, total, wide }`: a height for each item, the prefix sum of them
    (where each item begins), the stream's height and the width its longest line needs. Nothing is measured. Code rows and the header are exact.
    A card is estimated from its text and the stream's width (`rows.heightOf`), a little tall so that text never clips. An item is drawn in a
    box of its counted height, overflow hidden, and a card's background fills its box, so the extra height is padding inside the card and not
    a gap between its parts. A card is drawn at that fixed height and not at its natural one, because the prefix sums, the scrollbar, the
    hit test and a scroll to an item all use the same numbers. `rows.layoutCache` keeps the stream while nothing it is built from changes (the
    state, the files, the folds, the open threads, the focus, the width, the failed files and the minute), so a wheel tick, a hover and a drag
    do not rebuild it. A fetched file's items are kept per reply (`rows.fileStream`), and the code rows' runs per reply (`rowsOf`).
  - A key is the file's path, `|`, and the item: `src/a.rs|r12`, `src/a.rs|h:u3`, `src/a.rs|file`. Each drawn item is an element with
    an id made from its key. A header, a card head and a card body take a click, which `rows.itemClick` turns into a message that names
    the file or the thread; a code row has its own handlers (below).
  - The scroll position is `offset` in the model, in pixels. `on_scroll_wheel` becomes `Wheeled { dy }` (`rows.wheelDelta`), clamped to
    `0 .. total - height`. The scrollbar is the mod's own, a track and a thumb on the right of the stream (`rows.thumbOf`), dragged by a
    grip (`ThumbGrabbed`, `ThumbMoved`, `Released`) or jumped by a click on the track (`TrackClicked`). Sideways scroll is the
    `overflow_x_scrollbar` box around the code, as before.
  - Files load as the user scrolls. The state loads alone (`Cmd.Load`, `Msg.Loaded`). The diffs come after it in
    `Cmd.LoadFiles`, three files at a time and one command at a time, shared with the state's load by `inFlight` and
    `pending`: a state load goes before more files. `rows.wanted` orders them: the files on screen and the three after
    them, then every file below, then the files above. A file not yet fetched is its header and a placeholder as tall
    as its diff is guessed to be (`rows.estimateRows`: the changed lines and a hunk's header and context). Each fetch runs
    its files one after the other, so a file that scrolls into view waits for the rest of three at most. A reload fetches
    again the files whose summary changed and the files on screen. A fetch that fails puts its files in `skipped`, so the stream
    does not ask again and again, and the next load of the state clears it.
  - The sidebar follows the stream. After every update that changed the model, `main.dispatch` asks `rows.scrolled(model, stream)`
    what the model has yet to learn (the offset clamped, the stream's height, the key of the first item on screen, the files on screen)
    and, if anything, feeds the answer back as `Scrolled`, in the same step. There is no timer. `selected` is the topmost file on
    screen. A scroll the panel asked for (a click in the sidebar, a jump) keeps its file selected while it is on
    screen, because the end of the stream can stop a scroll short of putting a header at the top.
  - A click on a file in the sidebar is `FileChosen`: `selected` is the path, `reveal` is its header, and the files around it are
    fetched. `rows.scrolled` moves the offset to the header's start. The same goes for a card (a jump)
    and for a file marked viewed. A target that is not in the stream yet is kept until it is.
  - A file above the top of the screen that arrives is another height than its placeholder and would move everything
    under it, so the item that was at the top is scrolled to again (`reveal`), to within that item.
  - What is seen and what is not: [herdr-app-facts](herdr-app-facts.md) sections 10 to 12 list each finding, with
    "not yet seen in the window" where it is read from the declarations only. Those are what the user confirms in their own panel.
  - A fold. A header is a click (`FoldToggled`). A file marked viewed is folded and a click opens it, as the TUI's `z`
    and `m`: `rows.isFolded` is `folds[path]`, else the file's `viewed`. A folded file is its header, which says `viewed`
    and how many threads it hides, and is not fetched. Marking a file viewed folds it and scrolls to its header. The fold
    is the panel's and is not saved; a reload drops a fold that agrees with the file's mark.
- `Selection`: `path`, `side` (`old` or `new`), `from` and `to` as row indexes, and `anchor`, the row a drag began on (`from` or
  `to`). A drag makes it (`rows.span`), and `rows.pick` makes the one-row selection a `[+]` comments on when its row is outside any
  range. It is in memory only. `rows.plusTarget` says what a `[+]` comments on.
- `Hover`, `Drag`, `Grip`: where the pointer is (`path` and row), a press on a row (the row it began on, `head` the row the pointer is
  on now, `moved` whether it ever left the first row, so a release without a move is a click), and a press on the thumb (the pointer's
  height and the stream's offset when it began). They are in the model, so the update that handles them is a pure function with tests;
  none of them is something the stream is built from.
- `Editor`: `kind` (`comment`, `reply`, `edit`), the anchor, the thread id (reply) or comment id (edit) it belongs
  to, and `initial`, the text its textarea starts with (the comment's own for an edit). The text lives in
  the textarea state, which the view owns, so the model never holds a draft that could differ from the screen.
- Errors are tagged objects, never strings.

## Happy path (A), with cardinality

```
-> open                                              once
  -> resolve root and target from props(), else focused()
-> load                                              many: one in flight
  -> ReviewCli.state(root, spec)                     R: Host.exec
    -> decodeState(stdout)                           boundary
  -> update(model, Loaded)                           pure
-> load files                                        many: one in flight, three files at a time
  -> ReviewCli.file(root, spec, path)                R: Host.exec, one per file
    -> decodeFile(stdout)                            boundary
  -> update(model, FilesLoaded)                      pure
-> render                                            pure, thin
  -> rows.layoutCache(model, now)                    pure: every file's header, rows and thread cards in order, their heights and prefix sums
  -> view.draw(model)                                GPUI elements only: the items that overlap the screen
-> comment                                           once per user action
  -> update(model, Hovered, RowPressed, Released)    a drag selects a range, a click clears it, no Cmd
  -> update(model, PlusClicked)                      the editor opens on the line or the range, no Cmd
  -> update(model, EditorSaved)  => Cmd.Write
    -> body written to a file under appHome()/data/herdr-review   R: Host.writeTextFile
    -> ReviewCli.apply(root, file, spec)             R: Host.exec, the CLI deletes the file
  -> load                                            the reload shows the new thread
-> send                                              once per user action
  -> ReviewCli.send(root, target)                    R: Host.exec
  -> load
```

The call graph of each operation is in `graphs/gui-*.md`.

### Triggers for `load`

Five things ask for a load of the state: a change under the repo root, a change in the store directory, a 10 s
timer, the Refresh button, the end of any write, and a change of root or spec. They form a stream, and the
reducer coalesces it with two flags. At most one load runs (`inFlight`). A trigger that comes while one
runs, or while the editor is open, sets `pending`. When the load ends, or the editor closes, `pending` starts
exactly one more load and clears. A burst of 50 file events is two loads at most.

A load is the state alone. The diffs of the files are fetched after it, three at a time, as the stream wants them
([gui-stream](graphs/gui-stream.md)), by a command that shares `inFlight` and `pending` with the load: at most one
command of either kind runs, and a state load goes first when `pending` is set. A choice of file is not a load.

The editor defers a reload, and a fetch, so the lines do not move under the text being typed.

`state` and `file` write nothing: no `seen` event, no `meta.json`, nothing in the work tree, and they leave
`.git/index` as it was ([json](graphs/json.md)). If they wrote, the store watch and the root watch would
wake the mod, and the mod would call them again without end. Marking a thread seen is its own command,
`comment seen`, sent when the user opens a resolved thread that is `new`, or when its card has stayed on screen for a tick.

### The diff spec

The spec lives on the model. It goes on every `state`, `file`, `view` and `comment apply` call as `--spec`
and `--base`. With no flag, the CLI reads the spec from `meta.json`. Switching it in the panel does not save
it, because `state` writes nothing, so the choice lasts as long as the panel copy. A comment written under
the branch spec carries that spec in its anchor, so it is placed when the panel shows the branch view and is
`not_in_diff` under the working tree view, as the TUI does.

## Where it breaks (E)

A handler throws a tagged object. The wrapper turns it into `Msg.Failed { op, error }`. The reducer's
`failed` function is the one place that says what each error does, and it is a pure function, so each row is
one test.

```
-> ReviewCli.* (any)
  E: BinaryMissing      exec rejected                -> next candidate binary (below). None left:
                                                        escape, banner "herdr-review not found". No retry
  E: TooOld             exit 2 and stderr starts      -> escape: banner "update herdr-review". No retry.
                        "unknown command|option|         The mod builds every argv itself, so this means the
                        comment command"                 binary lacks the subcommand or the flag
  E: ContractMismatch   decode failed, or schema       -> escape: banner with the field path and both schema
                        number differs                   numbers (the error carries `path` and a `detail`
                                                         text, which names both numbers for a schema
                                                         mismatch). A v0.5.0 binary that answered `state`
                                                         at all would land here
  E: Busy               exit 1, stderr "busy"          -> escape: status line, the user repeats the action.
                                                         The CLI waits 2 s for the lock already, so a mod
                                                         retry adds delay and nothing else
  E: BackendFailed      any other exit, a signal,      -> escape: keep the last good model, status line with
                        or an exec timeout               stderr. No good model yet: banner instead
  E: BadRequest         exit 2 otherwise               -> escape: status line with stderr, then one reload.
                                                         "not in diff" and "unknown id" mean the screen is
                                                         stale, and the reload fixes it. A second BadRequest
                                                         in a row only sets the status, so a request that is
                                                         always wrong cannot loop
-> ReviewCli.file (a fetch of files, Cmd.LoadFiles)
  E: every error above                                -> the same escape, as op `files`, and the files of the fetch go into
                                                        `skipped` so the stream does not ask for them again until the next
                                                        load of the state. The fetch of a file that is gone (BadRequest) also
                                                        reloads the state once, which drops the file
-> ReviewCli.send
  E: Refused Blocked    json error "blocked"           -> escape: status line and notify. Nothing is sent
  E: Refused NotReady   json error "not_ready"            or marked sent
  E: Refused AgentGone  json error "agent_gone"        -> same, and the target is dropped from the model,
                                                         so the next Send resolves by folder
  E: NoTarget           json error "no_agent"          -> escape: status "no agent found for this folder"
  E: AmbiguousTarget    json error "ambiguous"         -> escape: picker over the candidates the CLI
                                                         returned, then send again with --target. Target
                                                         resolution stays in Rust, one copy
-> watch loop
  E: WatchUnavailable   watchFolder rejects            -> escape: the timer only. Try again after the next load
-> temp file
  E: writeTextFile rejects                             -> escape: status line, nothing written to the store
-> panel settings
  E: readTextFile rejects, or the file is bad          -> escape: left, shown. No message, nothing to show
  E: the write is refused three times                  -> escape: status "could not remember the sidebar setting"
die: a Msg the reducer does not handle (the switch ends in `never`); a row index out of range.
     Let both throw. herdr-app shows the mod error and `herdr-app status` lists it.
```

The `Busy` stderr text, and `unknown command|option|comment command`, are the CLI's own messages
([errors](errors.md)). A test on the Rust side holds them fixed.

A stale `exec` reply is dropped. Each load carries the `key` it was asked for, and `Loaded` with another key
than the model's is ignored, as `mods/changes/main.js` does with its `key()`.

## What each node needs (R)

```
Host        the only door to the outside. One interface, one object per mod copy.
            exec, readTextFile, writeTextFile, watchFolder, notify, run, focused, panes, terminals, props,
            appHome, nextCommand, theme
ReviewCli   R: Host. Builds argv, maps exit codes and JSON errors to the error list above, decodes stdout.
update      R: nothing. (Model, Msg) -> (Model, Cmd[]). Holds the single-flight flags and `failed`
rows        R: nothing. Rows, thread cards, selection to anchor, placement lookup
runner      R: Host, ReviewCli. Runs a Cmd, returns the Msg it produces. Each run is under withRecovery
view        R: Model, a dispatch function. Draws. Holds no logic and calls no Host method
main        R: the real Host. Wires init, render, close, the watch loops, the key loop and the timer
```

The mod never calls `hc`. Target resolution, the readiness check and the delivery are one copy, in Rust
([send](send.md)). The mod only passes `--target` when it knows the terminal.

`Host.theme` returns `Record<string, string>`, and the GPUI setters want `` `#${string}` ``, so `host.ts`
casts it once.

## Boundary

Untrusted: the stdout of `state`, `file`, `send --json` and `archive --json` (version skew, a half-written
reply), `props()`, `focused()`, `panes()` and `terminals()`. Each is decoded once, in `decode.ts`, by a
hand-written check of the fields the mod reads:

```
decodeState(raw: unknown): State          throws ContractMismatch { path }
decodeFile(raw: unknown): FileView        throws ContractMismatch { path }
decodeSend(raw: unknown): SendReply       throws ContractMismatch { path }
decodeProps(raw: unknown): Props          never throws: a bad field is dropped
decodePanel(text: string | null): Panel   never throws: a bad field is its default
```

Every JSON reply carries an integer `schema` ([shapes](shapes.md)). A different number is
`ContractMismatch`, and it is checked before any other field. Inside the mod the types are trusted. There is
no schema library: the Effect one does not load here, and the check is about 80 lines.

Row text and comment bodies are drawn after the view removes control characters, as the TUI does. A tab is
drawn as spaces by the view.

Syntax colours. A row's `tokens` ([shapes](shapes.md)) are decoded loosely, as `props` are, because colour is only
decoration: a missing or odd `tokens` is none, and a part that is malformed, out of order, overlapping, empty,
past the end of the text or of a kind the mod does not know is dropped, with no error. `rows.ts` cuts the row's
text at the tokens into runs, `{ text, color }`, where `color` is the name `theme()` gives the colour. The text
between tokens takes `foreground`. Each piece is cleaned (tabs, control characters) on its own, which gives what
cleaning the whole row gives, so the cut points are never shifted by a tab. The offsets are UTF-16 code units, the
unit `String.prototype.slice` counts, so the mod slices with them as they are. `view.ts` draws a code row as a
row of one div per run, so the added and removed backgrounds, the selection and the click on the row are those
of the row and not of a run. The list builds only the rows on screen, so the number of runs is the visible
rows' and not the file's.

| Kind | Theme colour | TUI colour it stands for |
|---|---|---|
| `comment` | `dim` | subtle text |
| `string` | `ansi2` | green |
| `number` (also any other constant) | `ansi3` | peach |
| `keyword` | `ansi5` | mauve |
| `operator` | `ansi7` | sky |
| `function` (also a tag) | `ansi4` | blue |
| `type` | `ansi6` | yellow |

The theme has no peach, mauve or sky, so each takes the nearest of the terminal palette. `ansi1` is left to the
removed lines, so no kind uses a red. `rows.test.ts` checks that every kind names a colour `theme()` has and that
no two kinds share one.

Card and warning colours. They are named in `rows.ts` the way the syntax colours are, and `view.ts` looks the name up
in `theme()`. Nothing uses `blocked`: it is the agent-status colour and is red in some themes, so a user's own card
drawn in it read as an error. Red (`ansi1`) is left to removed lines and to Delete. `rows.test.ts` checks the choices.

| Thing | Theme colour |
|---|---|
| The user's card (head, left bar, tint), and the title of the editor where the user writes | `accent` |
| The agent's card | `ansi5` |
| A resolved card | `ansi2` |
| A warning: the `unsent` and `new` tags, the picker sentence, the status line, the `M`, `R` and other letters of a changed file in the sidebar, an unsent count | `ansi3` |
| Other tags (`outdated`, `edited since sent`, `resolved by`) | `dim` |
| The Delete question's sentence, and every Delete button | `ansi1` |
| The Archive question's sentence | `foreground`, plain |
| The note beside a disabled Archive (resolved threads the agent closed are unread) | `dim` |
| The Archive button in the question | `accent` (border and fill), not red: archive moves threads to `archive.jsonl` and deletes none |

## Behavior wrappers

- `withRecovery(handler)`: the E table above. Every handler in `runner` goes through it.
- Timeouts, passed to `exec`: 60 s for `state` and `file`, 30 s for everything else. A timeout is a
  `BackendFailed`, because `exec` reports it as a non-zero code (the exact form is unknown, see below).
- Single flight lives in `update`, not in a wrapper, because it is state and not a side effect. It is a
  few lines of the reducer and a test needs no `Host`.

Neither wrapper appears inside a handler.

## Scope

Per copy of the mod, released in `close(cx)`, which is bounded: start the cleanup and return.

- The two `watchFolder` loops (repo root, store directory). Each loop checks `this.closed` after a wake and
  ends. `watchFolder` cannot be cancelled, so one extra wake after close is accepted.
- The `cx.timer.every` timers (the 10 s reload and the 500 ms `Glanced`).
- Every textarea state: `release()`.
- The body file of a write that has not reached the CLI. The CLI deletes a file once it has opened it. When
  the `exec` fails before that (`BinaryMissing`, `TooOld`), the runner overwrites the file with an empty
  text, since a mod cannot delete.
- Nothing for the picker: it is drawn in the bar under the diff, and holds no overlay.

State kept in module scope would be shared by every copy, so nothing is. It lives on the instance. The one cache is
`rows.ts`'s memo of a fetched file's stream items, a `WeakMap` keyed by the reply's own object: it is pure (the same reply and
the same inputs give the same items), no copy can reach another's reply, and an entry goes with its reply.

## Swap R

The same graphs, with a `FakeHost` behind R. `exec` answers from a table keyed by argv and records each
call. `writeTextFile` records its calls. `watchFolder` resolves when the test says so. `focused`, `panes`
and `props` return what the test sets. The tests run `runner`, `update`, `rows` and `decode` against it. The
GPUI `view` is not unit tested, which is why it holds no logic. It is checked with `herdr-app check` and a
screenshot of the running app.

A golden file for each JSON reply lives in `test/fixtures/`. The Rust tests compare their output with the same
files ([json](graphs/json.md)), so a change to the contract fails a test on one side or the other before it
reaches the panel.

## Code shape

```
herdr-app-mod/
  package.json  tsconfig.json  mod.json  package-lock.json
  src/  types.ts  decode.ts  host.ts  cli.ts  update.ts  icons.ts  rows.ts  runner.ts  view.ts  main.ts
  icons/ *.svg                the file-type drawings, copied to the mod folder by the stage script
  test/ *.test.ts   test/fixtures/*.json
scripts/stage-app-mod.sh     build, then copy (the bundle, mod.json, icons/) to ${HERDR_APP_HOME:-~/.config/herdr-app}/mods/herdr-review
```

| File | Does | Imports |
|---|---|---|
| `types.ts` | Every type above. No code | nothing |
| `decode.ts` | The boundary checks, for the replies, for `props()`, `focused()` and `panes()`, and for `panel.json` | `types.ts` |
| `host.ts` | The `Host` interface and the real one | `herdr` |
| `cli.ts` | `ReviewCli`: argv, exit code and JSON error mapping | `types.ts`, `decode.ts` |
| `update.ts` | `update`, `failed`, single flight for the state and the files, the fetch order, the wheel, the thumb, the hover and the drag | `types.ts`, `rows.ts` |
| `icons.ts` | `fileIcon(name)`: the drawing for a file name, whole names then lower-cased extensions then `file` | nothing |
| `rows.ts` | The stream (every file's items, heights and prefix sums, the layout cache, the scroll math, the thumb, the card height estimate, the fetch order, the keys), the thread cards, the sidebar's folder and file rows (with `icons.fileIcon`) and the filter's matcher, count and Enter pick, its header buttons and pane order, selection math, anchor and batch from a selection, which rows are tinted or hovered, the bar under the diff, what a click or a key means | `types.ts`, `icons.ts` |
| `runner.ts` | `run(env, cmd)`, `withRecovery`, `watch` (one watch loop), `open(host)` (root and target at `init`), the read and write of `panel.json` | `cli.ts`, `decode.ts`, `types.ts` |
| `view.ts` | GPUI drawing, the stream's visible items, its scrollbar and the rows' events | `gpui-kit`, `gpui-base`, `rows.ts` |
| `main.ts` | The exported `View` class: `init`, `render`, `close`, the loops, the layout cache, and the step after each update that feeds `Scrolled` back | `host.ts`, `view.ts`, the rest |

Layout of the panel: a header (and the banner, status and notices when there are any) across the top, then one row
under it with two children, the sidebar and the stream pane, so a later change can swap them. The sidebar is
`SIDEBAR_W` (280 px) wide and `flex_none`; the diff pane is `flex_1` with `min_w(0)` and takes the rest of the width and the full height
under the header. Which of the two come, and in which order, is `rows.paneOrder`: the sidebar then the diff, the diff
then the sidebar, or the diff alone when the sidebar is hidden, from `model.panel`. The sidebar's border is on the side
that faces the diff. The bar under the diff (selection, thread buttons, editor, picker, confirm question) belongs to
the diff pane, so it spans the diff and not the sidebar. A fixed width was chosen over a share of the panel because
a name needs the same room in a wide panel as in a narrow one.

The header holds two buttons for the sidebar, from `rows.sidebarButtons`, left of the spec buttons: `Hide files` or
`Show files` (`SidebarToggled`), and `Move right` or `Move left` (`SidebarFlipped`). The second is drawn only while the
sidebar shows, since moving a hidden column changes nothing on screen. Both change `panel` at once and send `Cmd.SavePanel`.
The header wraps its buttons onto a second line when the panel is too narrow for them.
A button that is off has no click handler and draws at 40% opacity, so it reads as off at a glance.

### The gutter and the [+]

The graph is [gui-comment](graphs/gui-comment.md). A code row is, left to right: a gutter `GUTTER_W` (24) px wide, the old line number,
the new one, the sign, and the code. The gutter holds `icons/plus.svg`, a 16 px filled rounded square with the `+` cut out of it, an
`svg` tinted `theme().accent`, and only on the hovered line (`rows.rowState`). `diffWidth` counts the gutter and its gap.

- A row is an ordinary element, so it carries `on_hover` and `on_mouse_move` (the row the pointer is on), `on_mouse_down("left")` (the
  press that starts a drag) and the `[+]`, which has an `on_mouse_down` that stops the press (so it starts no drag) and an `on_click`
  of its own (`PlusClicked`). A hunk header takes the hover only, so a drag passes over it, and has no `[+]`. The selected rows are tinted.
- The release is `on_mouse_up("left")` on the panel's root, so it is read wherever the button goes up in the panel. A release outside the
  window never arrives and a move carries no button state; the next press replaces a drag that was never released.
- `hover` is in the model and a row is redrawn when it changes. After a wheel tick with a still pointer the line under it changes
  without a move; the `[+]` follows at the next move.

### The filter box

The box is the first thing in the sidebar, always drawn, so the user sees the files can be filtered. It is the
`Input` element over an `InputState` (`gpui-base`), the single-line input the shipped Changes and picker mods use,
in a rounded frame whose border is the accent colour while the box holds the keyboard. The state is made in `init`
(never in `render`), lives on the `View` like the editor's textarea, and is released in `close`. Its own
placeholder is the hint `filter (/)`, which shows while the box is empty. On the right of the frame is the count
`n/m`, the files that match over all of them (`rows.filterInfo`), always shown. Under the box, `no match` when
the query leaves no file out of a review that has some (the empty-list text, `No changes`, is for a review with none).

The matcher is the TUI's, ported from `matches` in `view.rs` (`rows.fileMatches`): a path matches when the query's
characters appear in it in order; a query with no upper case letter ignores case and one upper case letter makes it
exact; a space is a character; there is no score and no highlight; an empty query matches every path. Both sides
walk code points. `rows.sidebarRows(files, query)` leaves out the files that do not match and the heading of a folder with
no match, and keeps the diff's order. Only the sidebar is filtered: the diff, the jumps (`rows.stops`) and send
read `state.files` as before, and a file shown that the query hides stays shown.

- `FilterChanged { text }` comes from the input's `change` event and sets `filter`. The model holds the text so the
  list, the count and the reload all read one value, which a reload keeps (the timer reloads every 10 seconds).
- `FilterFocused { on }` comes from the input's `focus` and `blur` events and sets `filtering`. It is what stops
  `n` and `shift+n` from jumping while the user types: `rows.panelKey` returns nothing in the box except Esc, and the
  keys reach the root's handler only because the input is inside it. Hiding the sidebar sets `filtering` to false,
  since a box that is not drawn cannot hold the keyboard.
- `FilterOpened` is the key `/` on the panel root (`panelKey`; not while the editor, a question or the picker is
  up). With the sidebar hidden it shows it and saves that, as the TUI's `filter` does. `main.ts` then gives the box the
  keyboard (`focusFilter`, tried again after 300 ms for a box that was not drawn yet).
- `FilterEntered` is the input's `submit` event (Enter). With text and a match, it keeps `filter`, sets `filtering` to
  false and, when the file shown does not match, does what `FileChosen` does for the first match (`rows.filterPick`),
  so the shown file follows the filter as the TUI's cursor does. `main.ts` gives the keyboard back to the root
  (`focusNav`) when the model drops `filtering` for any reason but a blur. With no match it does nothing, so the
  user keeps typing or presses Esc, as in the TUI. On an empty query it only gives the keyboard back.
- `FilterCleared` is Esc, in the box and on the panel root while a filter is on (`panelKey`). It empties `filter`,
  sets `filtering` to false, and `main.ts` empties the input (`set_value("")`) and gives the keyboard back. It is
  the only message that writes the input's text from the model, so a late `change` event cannot be overwritten.
- Left out of the TUI's rules: `ctrl+u` and the rest of the editing keys are the input's own, and the box has no
  fixed key set.

### The panel's own settings

The panel keeps its own settings and ignores the TUI's `config.toml` `[sidebar]`.

They are one small file, `appHome()/data/herdr-review/panel.json`, written as one line:
`{"side":"right","open":false}`. `side` is `left` or `right`, `open` is `true` or `false`. A reopened panel starts the
way it was left. The file is not in the review store and the CLI never reads it.

- Read: `init` starts `Cmd.ReadPanel`. The runner calls `Host.readTextFile` (it gives the text, or null for a missing
  file) and `decodePanel` turns the text into a `Panel`. The reply is `Msg.PanelLoaded`. The panel is drawn with the
  defaults until it arrives, so a right-hand sidebar can show on the left for a moment.
- Decode: the file is untrusted, since the user may edit it. `decodePanel` never throws. Text that is not JSON, a missing
  file, or a read that rejects gives the defaults (left, shown), and a field that is not an allowed value gives that
  field's default, so `{"open":false}` hides the sidebar on the left.
- Write: `Cmd.SavePanel` reads the file, then calls `Host.writeTextFile(path, text, expected)` with the text it read as
  `expected` (null when there was no file), because that call replaces a file only when it still matches. A refused write
  is read and tried again, three times at most, so two quick clicks both end up saved. Then it fails as
  `BackendFailed` on op `panel`, and the reducer shows the status "could not remember the sidebar setting" and leaves the
  view as it is. `writeTextFile` makes a missing `data/herdr-review/` itself, so there is no `mkdir` or `chmod`: the file
  holds no text that needs to stay private.

The sidebar groups the files as the TUI does, in the diff's order. `rows.sidebarRows` returns the rows: a folder
heading (`src/`, or `./` for the top folder) where the folder changes, then a file row under it. A folder that
comes again later, as untracked files do, gets a second heading. A file row has the status letter, an icon for the
file's type, the name without the folder, `+n` and `-n` (left out when zero), and a second line, only when there is something to say, with
`n unsent`, `n open`, `n new` and `viewed`. Heading text is cleaned of control characters like every other text.
The icon is an SVG drawing, `svg("mods/herdr-review/icons/<icon>.svg")` at 14 px, `flex_none`, tinted by the dim
theme colour, so it stays when the name is cut, as the TUI keeps its glyph. The TUI draws Nerd Font glyphs; the panel
cannot count on a Nerd Font being installed (the default UI font draws a missing-glyph box, see
[the facts](herdr-app-facts.md) section 9), so it ships drawings. `icons.fileIcon(name)` picks one by the TUI's
rule in `icons.rs`: a whole name (`Cargo.toml`, `Dockerfile`, `Makefile`, `LICENSE`, `.gitignore`) first, then the
lower-cased extension (rust, toml, lock, markdown, json, yaml, shell, js, ts, image, text, diff, and one `code`
drawing shared by the other languages the TUI knows), else the generic `file` page. The table in `icons.ts` has the
TUI's keys and is kept by hand beside `icons.rs`, not generated from it: the two map to different things (a glyph
and a drawing) and the keys are few. The drawings are in `herdr-app-mod/icons/`, drawn for this repo as simple
strokes with `currentColor` and a 24 unit box, so the repo's licence covers them. The second line of badges
is indented to start under the name. A name too long for the width is cut by the drawing with an ellipsis at the end (`truncate`), and a heading with
an ellipsis at the start (`text_ellipsis_start`), so the deepest folder shows, as the TUI cuts a heading from the left.
No cut is computed in `rows.ts`, since the text width is the renderer's. A file row selects the file on click. A
heading is a plain `div` with no id and no handler, so a click on it does nothing (folders do not fold).
The column is a plain `v_flex` and not a `list`, so a row may carry its own `on_click`.

The column is a `v_flex` with `overflow_y_scroll` laid over a relative box with `absolute().inset_0()`, which
gives it a scroll handle of its own, and `Scrollbar.vertical("review-files")` draws its bar. Wrapping it in
`overflow_y_scrollbar()` instead loses the flex sizing, which is why the shipped Changes panel does the same.
The stream is a clipped box (`overflow_hidden`) holding an absolutely placed column of the items that overlap the screen, shifted up by
the part of the first one above the top, inside a box with `overflow_x_scrollbar` as wide as the longest line. The scrollbar is
a track and a thumb of the mod's own on the right, outside the sideways box so it does not move with it. The model holds the scroll
position (`offset`, `total`, `height`) and `top` and `visible`; the View keeps nothing for the stream.

Only `host.ts`, `view.ts` and `main.ts` import `herdr`, `gpui-kit` or `gpui-base`. Node cannot resolve those
modules, so a test that imports one of the three fails to load. Tests import the `.ts` files directly with
the extension, which esbuild accepts.

- `tsconfig.json` starts from the shipped `mods/pane/jsconfig.json`: ES2022, bundler resolution,
  `lib: ["ES2022"]`, `noEmit`. `strictNullChecks` is on (herdr-app-facts, section 1: it works with `rows!:`
  field declarations). Two declaration files are needed: `gpui-kit.d.ts`, which declares `gpui-kit`,
  `gpui-base` and a few Node-like modules, and `herdr.d.ts`, which is the `herdr` module itself, reached
  with `paths: { "herdr": ["./herdr.d.ts"] }`.
- Neither `.d.ts` is in the repo. `npm run setup-types` copies both from the app folder to `herdr-app-mod/`,
  where they are git-ignored, and `tsconfig.json` lists `gpui-kit.d.ts` in `files`. The typecheck runs locally
  before a bead closes, and CI does not run it. CI has no copy of the files, and the user decided to keep
  them out of the repo (herdr-app-facts, section 1).
- Tests use `node:test`, so the typecheck also needs `@types/node` with `"types": ["node"]`. `gpui-kit.d.ts`
  declares `process`, `console`, `URL`, `Buffer` and others itself, and they collide with `@types/node`, so
  `skipLibCheck` is on. The collision is inside the `.d.ts` files, which are not checked, and `src` and
  `test` are still checked in full.
- `theme()` is cast once, in `host.ts`.
- `package.json` scripts: `build` (esbuild to one `out/main.js`, `gpui-kit`, `gpui-base` and `herdr`
  external, `--format=esm --target=es2022`), `test` (`node --experimental-strip-types --test
  'test/*.test.ts'`, the glob, because a directory argument fails), `typecheck` (`tsc -p .`), `setup-types`.
- `mod.json` is `{ "place": "none", "focusable": true }`. `focusable` lets `pane.go` reach the panel so the
  user can type a comment.
- `stage-app-mod.sh` runs `build`, copies `main.js` and `mod.json` to the mod folder, copies the release
  binary to `bin/herdr-review` there, and writes `main.js` last, because saving any file in the folder
  reloads the mod. `herdr-app check` writes `gpui-kit.d.ts` and `jsconfig.json` into that folder, and they
  are not copied back.
- The mod's name is `herdr-review`.

### Which binary the mod runs

The first of these that `exec` can start:

1. `props().bin`, for a test or a custom build.
2. `${appHome()}/mods/herdr-review/bin/herdr-review`, the copy `stage-app-mod.sh` put there.
3. `herdr-review` on `PATH`.

`BinaryMissing` (the `exec` rejected) moves to the next one. `TooOld` stops with the banner. The choice is
made on the first load and kept on the instance. The nix setup pins the v0.5.0 release binary on `PATH`,
which lacks `state`, so without a staged copy the panel shows `TooOld`. The staged copy is also why the
prompt can name an absolute `<bin>` ([send](send.md)).

### Opening the panel

One key in `keys.json`, an object action that calls `pane.split` with the mod name and no launcher mod
(herdr-app-facts, section 3):

```json
"cmd-shift-r": {"command": "pane.split", "args": {"direction": "right", "mod": "herdr-review"}}
```

The object form cannot put the focused folder in `props`, so the mod finds its own root and target at `init`:

1. `props().root` and `props().target` when given (a test, or a later key that carries them).
2. Else `focused()`: `cwd` is the root, and `terminal` is the target when `agent` is set.
3. Else the pane of the same tab with a `cwd` from `panes()`, preferring one with an agent.
4. Else a banner "open the panel from a pane that has a folder".

The root may be any folder inside the work tree, because `--repo` resolves it with
`git rev-parse --show-toplevel`. A missing target is not an error: `send` without `--target` resolves the
agent by folder in Rust.

### Window checks: what the running app did

Settled on 2026-11-09 in a real window ([herdr-app-facts](herdr-app-facts.md), section 7).

| Unknown | Seen | What the design does |
|---|---|---|
| What `focused()` returns inside `init` of the new pane | It names the pane that was split, with its `cwd` (`agent` null for a plain shell). One second later it names the mod pane. Same as the default | Nothing changes. `init` reads it synchronously. The `mod.open` launcher is not needed |
| The mode `writeTextFile` gives, and whether it makes a missing parent | Mode 644. It creates missing parents (755). **`fs/promises.mkdir` rejects `mode`** (``unknown option `mode` ... expected recursive``), so the default call fails | The body file goes to `appHome()/data/herdr-review/<n>.txt`. The app already makes `data/herdr-review/` (755), and `exec("mkdir", ["-p", dir])` covers the case that it is gone (the mod imports no `fs` module for this). Then `exec("chmod", ["700", dir])` makes it private (checked: `drwx------`; `exec("mkdir", ["-p","-m","700",dir])` also works). The file's own mode (644) does not matter. A rejected `chmod` stops the write with a status line, nothing reaches the store |
| The `PATH` `exec` sees after a Dock launch | A login-shell PATH: `~/.local/bin`, nix profile folders, `/usr/local/bin`, system folders, `/opt/homebrew/bin`. `exec("herdr-review")` resolves to the older nix build, which answers `state` with exit 2 | The order stands, and it matters: staged copy before `PATH`, because the `PATH` copy here is `TooOld` |
| That `exec` runs `mods/herdr-review/bin/herdr-review` | It runs, exit 0, schema 1 JSON | Same. The restart question is still open: documented to survive, not run |
| `Effect.runPromise` settling | Not run, Effect is not used | |

`herdr-app check` showed one more fact: `node:fs` does not resolve in the mod runtime ("an application may
otherwise import only its own files"), while `import { mkdir } from "fs/promises"` does load and runs in the
window. `check` also rejects an `init` that writes with `fs/promises` outside the mod's own folders ("outside
every granted write root"), where the window allowed it.

One more is not in the facts file: what `exec` reports for a timeout (a non-zero `code`, a rejection, or a
signal). Every form lands in `BackendFailed`, so the design does not depend on it.

## Repo changes

What the repository needs for a second language. The first bead that writes TypeScript does these.

- `.gitignore`: `/herdr-app-mod/node_modules/`, `/herdr-app-mod/out/`, `/herdr-app-mod/gpui-kit.d.ts`,
  `/herdr-app-mod/herdr.d.ts`, `/herdr-app-mod/jsconfig.json`. The staged copy under `~/.config/herdr-app` is the only built one.
- `AGENTS.md` (this repo's, not the one in `~/.config/herdr-app`): the work loop's "done" line names
  `cargo clippy` and `cargo test`. Add the TypeScript tests and `herdr-app check` for a bead that touches
  `herdr-app-mod/`, and a row in "Where things are" pointing at `gui.md`.
- `scripts/ci.sh` and `.github/workflows/ci.yml`: after the cargo steps, `npm ci`, `npm test` and
  `npm run build` in `herdr-app-mod/`. `ci.yml` sets up Node 22. The typecheck is not in CI.
- `herdr-plugin.toml` does not change. The Herdr plugin installs from a checkout and fetches the binary in
  `scripts/fetch-herdr-review.sh`, so `herdr-app-mod/` adds source files to the plugin folder and nothing
  else.
- `skills/herdr-review/SKILL.md` covers both hosts. Under Herdr it gates on `HERDR_ENV=1` and finds the binary
  through `herdr plugin list`. Under herdr-app it gates on `HC_TERMINAL_ID` and uses the absolute path in the
  prompt it received. The README section "The herdr-app panel" has the install and the key.
