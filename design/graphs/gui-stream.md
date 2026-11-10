# Call graphs: the GUI's stream of every file

Part of the [herdr-review design](../../DESIGN.md). The mod is described in [gui](../gui.md), and what herdr-app does
with the handlers and the scrolling the mod draws is in [herdr-app-facts](../herdr-app-facts.md), sections 10 to 12. The TUI it
copies is "How the body behaves" in [tui](../tui.md).

## The stream

```
-> render                                              pure, thin
  -> layout(model, now)                                rows.layoutCache(): the stream below, kept while nothing it is built from changes
  -> rows.streamLayout(model, now)                     (pure) { items, sizes, starts, total, wide }, one height for each item
       the threads not in the diff: a label and their cards            key `|l:orphans`, `|h:<id>`, `|c:<id>`
       for each file of state.files, in the diff's order:
         a header                                      key `<path>|file`, 30 px
         folded?  nothing more                         isFolded: folds[path], else the file's `viewed`
         fetched? its note, its file comments and its rows with the cards under them
                                                       key `<path>|r<n>`, `<path>|h:<id>`, `<path>|c:<id>`, `<path>|note`
           side by side (rows.isSplit: model.split, else width >= SPLIT_W 900)? one item per code row, key `<path>|p<n>`, 20 px:
             a run of del rows pairs line by line with the add run after it, the longer run's rest faces a null half;
             a ctx row is on both halves; a hunk header stays a plain row; the cards of a pair come after it, its left row's first,
             each in the half its thread's anchor side names (`half`: old left, new right; a file comment and the orphans full width)
         an open editor: an `editor` item (key `<path>|editor`, `EDITOR_H` 200 px) under its row's cards, its thread's card, or at the top of the file
         else a placeholder `<path>|wait`              rows.estimateRows(summary) lines of 20 px
       starts = the prefix sum of sizes (one longer than items), total = the last of it, wide = the width the longest line needs (0 side by side,
       which fits the pane: a half's code shifts sideways only as far as the longest line on screen needs)
       E: the item at a pixel is the last one that begins at or above it (rows.itemAt, the hit test of a pixel)
       per file, built once and kept (rows.fileStream) until an opened thread, the focus, the minute, the width or the layout changes
       E: the stream is built once and kept while a wheel tick, a hover, a drag or a selection changes nothing it is built from
  -> view.diffPane: the items that overlap the screen, as plain children of a clipped box
       rows.visibleRange(stream, offset, height) -> { first, end, shift }
       E: the items drawn are the ones that overlap the screen, the first shifted up by the part above the top
       review-stream-content (clipped, overflow hidden) > review-stream-items (absolute, top = -shift) > one box per item, `sizes[i]` tall
       review-stream-x (overflow_x_scrollbar) around it scrolls the code sideways; the vertical bar is the mod's own (below)
       on_size_change -> Msg.Resized { width, height }   the width the card heights are estimated from, and the screen's height
```

A key is the file's path, `|`, and what the item is. Only what follows the last `|` is read, so a path may hold one.

Heights are the mod's to give: nothing is measured. `rows.heightOf` is exact for a code row (20 px) and
for a header, and estimated for a card: its text wrapped at 7.5 px a character in the card's width
(`cardWidth`: the stream's width less 60, at most 560; in a half of a side-by-side stream `halfCardWidth`: half the width less the rule, 28 and 12, at most 560), 18 px a line, padding and the closing gap added. It
errs on the side of too tall, so text never reaches the next item. An item is drawn in a box of exactly its height
with its overflow hidden, and a card's background fills its box. The card is drawn at that fixed height, and not at its natural
one, because the prefix sums, the scrollbar, the hit test and a scroll to an item all use the same numbers: a card that drew
taller or shorter than it was counted would move every item under it away from where the offset says it is.

The layout is kept while nothing it is built from changes (`rows.layoutCache`): the state, the files, the folds, the open threads,
the focus, the width, the layout (`isSplit`), the editor and the view, the files that failed and the minute. A wheel tick, a hover, a drag and a selection change none of
them, so none of them cuts items or prefix sums again, and no render rebuilds the whole stream because the pointer moved. The code
rows' text cut into runs is kept per file reply on top of that (`rowsOf`), so the minute or the width changing does not cut every
row again inside one frame.

## Scrolling

The mod keeps the scroll position itself, in pixels: `offset` in the model, with `height` (the screen) and `total` (the stream),
which `Scrolled` sets from the stream.

```
-> on_scroll_wheel on review-stream-content and on the track     user action
  -> rows.wheelDelta(event)                            (pure) pixels to scroll down by, or null
       mostly sideways (|dx| > |dy|)                   null: left to the sideways scroll of review-stream-x
       delta_lines present                             lines * ROW (20); else delta.y; the sign flips: content up is offset down
  -> a vertical one: cx.stop_propagation() (else the sideways box would take it too), Msg.Wheeled { dy }
-> update(model, Wheeled)                              (pure) offset = clampOffset(offset + dy, total, height), 0 .. total - height
     E: the offset stays between the top and the end; a stream shorter than the screen does not scroll
     E: a wheel tick scrolls by its pixels, a device that reports lines by a row a line, a sideways one is left to the code box
     E: already at the top or the end                 the model as it was, so no frame
-> E: the thumb is sized and placed from the offset, the screen and the stream, and a stream that fits has none
   the thumb: rows.thumbOf(total, height, offset)      (pure) { top, size }, size = max(24, height^2 / total); null when the stream fits (no track)
  -> ThumbGrabbed { y }                                 the press on the thumb: grip = { y, offset }
  E: dragging the thumb moves the stream by the pointer's distance in the track, and the release lets go
  -> ThumbMoved { y }                                   on_mouse_move of review-stream-box while grip is set
       offset = grip.offset + (y - grip.y) * (total - height) / (height - size)   measured from the press, so it does not drift
  E: a click on the track centres the thumb there
  -> TrackClicked { y }                                 a press on the track: rows.offsetOfThumb centres the thumb at y
  -> Released                                           the button went up anywhere in the panel: grip = null
-> main.dispatch, after every update that changed the model
  -> rows.scrolled(model, layout(model, now))          (pure) Msg.Scrolled or null, see below
```

A wheel tick during a fling is one message and one render of the visible items only: a few dozen elements, whatever the review's size.

## The files on screen

```
-> E: after any change the model learns the offset, the first item on screen and the files on screen from the stream
   rows.scrolled(model, stream)                        (pure) what the model must learn from the stream
  -> a request to scroll (model.reveal) whose key is in the stream?    offset = that item's start, revealed = true
  -> offset = clampOffset(offset, stream.total, height)
  -> first, end = rows.visibleRange(stream, offset, height)
  -> top = the key of the first item, visible = rows.pathsOf(items, first, end), topmost first
  -> fresh = rows.freshOn(items, first, end): the ids of the cards in the range whose thread is resolved by an agent and new
  -> all of offset, total, top, visible and fresh as the model has them, and no request?   null: nothing to say
  -> Msg.Scrolled { offset, total, top, visible, fresh, revealed }
-> update(model, Scrolled)                             (pure)
  -> offset, total, top, visible, fresh = the message
  -> selected = the file the request scrolled to when it is on screen (revealed), else the topmost file on screen
       no file on screen (only the threads not in the diff): selected stays
  -> reveal = null when the request was served; a request whose key is not in the stream yet is kept
  -> the files the stream wants, if nothing runs:      Cmd.LoadFiles, below
```

There is no timer: the stream is the mod's, so the offset and the layout are known in the same step that changes them, and the
sidebar and the fetches follow a wheel tick, the thumb, a file arriving and a request to scroll alike.

## The files on screen

```
-> the renderer is called with the visible range      from inside layout: it may not dispatch or notify
  -> ui.seen(start, end) keeps it in a field of the View   (a frame calls the renderer twice: first with the one item it measures,
                                                       then with the items on screen, so the wider of the last two is the screen's)
-> a 150 ms timer, main.readRange
  -> the range changed?                                the paths of the files in it, and the key of its first item
  -> Msg.Scrolled { top, visible }
-> update(model, Scrolled)                             (pure)
  -> top, visible = the reply
  -> selected = the file the last request scrolled to when it is on screen, else the topmost file on screen
       no file on screen (only the threads not in the diff): selected stays
  -> reveal = null
  -> the files the stream wants, if nothing runs:      Cmd.LoadFiles, below
-> main.dispatch: no frame for a Scrolled that left `selected` as it was
```

## Seen by looking

A resolved card an agent closed counts as seen once it has been on screen, as the TUI counts a cursor landing on it
([tui](../tui.md)). Only a card of a resolved thread with `is_new` is fresh: an open thread, a thread the user resolved and a
thread already seen never are.

```
-> a 500 ms timer in main.ts                           cx.timer.every, started with the root watch
  -> Msg.Glanced
-> update(model, Glanced)                              (pure)
  -> no root, or stopped?                              nothing
  -> the first id in `fresh` that is also in `glance` and not in `seenSent`
       none                                            glance = fresh, no Cmd
       found                                           glance = fresh, seenSent gains the id, Cmd.Write { seen, id }
-> run Cmd.Write { seen }                              the command opening a card sends (gui-thread.md), by runner.ts
       E: seen failed of any kind -> nothing shown (gui-thread.md); seenSent keeps the id, so looking does not send it
          again, and opening the card does
  -> Msg.Wrote { seen } -> Triggered                   the reload brings is_new false, and the card leaves `fresh`
```

A card has to be on screen at two ticks in a row, so a card that a fast scroll carries past between ticks sends nothing, and
one id goes a tick, so a screen of new cards is a queue of a few writes and not a burst. Rendering writes nothing: the model
learns `fresh` from the stream in `Scrolled`, and the write is a `Cmd` from `update` that `runner.ts` runs. Opening a new
resolved card ([gui-thread](gui-thread.md)) adds its id to `seenSent` too.

## Fetching the files

```
-> flush                                               after every Loaded, FilesLoaded, Failed, Scrolled, FoldToggled, FileChosen and an editor closing
  -> inFlight, an editor open or stopped?              nothing
  -> pending?                                          Cmd.Load (gui-load.md): a state load goes before more files
  -> rows.wanted(model)                                (pure) the paths to fetch, in order
       on screen = the files in `visible`, or the file a request is scrolling to, or the selected file, or the first
       first the files from the first on screen to the last on screen plus AHEAD (3)
       then every file below those, then the files above, in the diff's order
       a file is wanted when it has no diff, or its diff is in `dirty`
       and it is not folded and not in `skipped`
  -> the first BATCH (3) of them: batch = those, Cmd.LoadFiles { key, paths }, inFlight = true
-> run Cmd.LoadFiles                                   one in flight, shared with Load
  -> ReviewCli.files(root, spec, paths): one `files --json --path P1 --path P2 ...` process for the batch   R: Host.exec, 60 s
       the CLI reads the diff once for the batch; decodeFiles checks that the reply holds the paths asked for, in that order
       E: BinaryMissing, TooOld, BackendFailed, BadRequest, ContractMismatch -> Msg.Failed { op: files }
       a path the CLI does not find in the diff (exit 2, `not in diff: P`) fails the whole batch: the files in it are asked for again, later
  -> Msg.FilesLoaded { key, files }
-> update(model, FilesLoaded)                          (pure)
  -> key differs from the model's?                     E: stale reply -> dropped, then flush
  -> each file replaces its diff and leaves `dirty`
       a selection in a file whose rows changed is dropped; a focus or a question whose comment is gone is dropped
  -> E: a file above the first one on screen arrived, and no scroll is under way?
       reveal = the key of the item at the top of the screen, so it stays at the top of the screen
       (an item above the screen that is another height than its estimate moves everything under it; the offset goes to that item's start)
  -> a jump waits for one of them? it lands (gui-thread.md)
  -> flush: the next three, until nothing is wanted
-> update(model, Failed { files })                     (pure)
  -> inFlight = false, `skipped` takes the batch, so the stream cannot ask again and again
  -> the status or banner by the E table in gui.md, then flush
       `skipped` is emptied by the next load of the state (the timer, a watch wake, Refresh, a write)
```

A reload of the state marks as `dirty` the files whose summary changed, and nothing else: with 100 files, a 10 s timer
must not fetch 100 diffs, nor the files on screen again when nothing in them changed. The summary is compared whole (counts,
`viewed` and `hash`). The `hash` changes with any hunk header, any row with its kind and line numbers, and the cards of the
file's threads, and the file view is drawn from those rows, threads, `viewed` and the change (`note`): so a file whose lines
or cards changed at the same counts is fetched again, wherever it is, and one whose summary is the same keeps its view,
its cached runs and its layout. The colours of the rows are the one thing read beyond the hash (the file's whole text at
the revision, so a line outside the hunks that opens a comment or a string); a change only there that leaves the diff
as it was shows at the next change of the file or at Refresh, which fetches the files on screen again (the timer and the watch do not).
A draft written (a comment, a reply, an edit) adds its file to `dirty`, so the fetch that follows the write, and not a state load
already in flight, lands the draft with its card.

## Selecting a file and scrolling to an item

```
-> FileChosen { path }                                 a click in the sidebar, Enter in the filter
  -> selected = path, reveal = { key: "<path>|file" }, flush      the fetch starts at that file
-> ThreadJumped, ViewedClicked (marking)               reveal = the card's key, or the header's
-> E: a request to scroll to an item moves the offset to it, and the end of the stream stops it short
   main.dispatch: rows.scrolled sees the request
  -> the key is in the stream?                         offset = rows.offsetOfKey(stream, key, height), the item's start, clamped; Scrolled { revealed }
     not in the stream yet (its file has not arrived): the request is kept and tried at the next dispatch
```

The end of the stream stops a scroll short: the last file's header may not reach the top. The file the request named
stays selected as long as it is on screen, so the click does not move the sidebar's mark up a row.

## The layout

```
-> LayoutToggled                                       the header button (rows.layoutButton, "Side by side" or "Unified")
  -> split = !isSplit(model), hover = null             the item under the pointer is another one in the other layout
  -> E: the item that was on top is revealed           reveal = the key of the item holding the same row in the new layout
       (`r<n>` <-> `p<n>`: the item that holds row n)  a file header or card key is found as it is
-> Resized across SPLIT_W with split = null            the same: reveal, hover = null
-> E: a forced layout (split true or false) ignores the width
```

The layout is not saved. A drag across halves ends at the hovered half's partner row on the drag's side ([gui-comment](gui-comment.md)).

## Fold

```
-> FoldToggled { path }                                a click on a file header (its key `<path>|file`)
  -> folds[path] = the opposite of isFolded; a selection in a file that folds is dropped; flush (an opened file is wanted)
-> ViewedClicked { path, viewed }                      folds[path] = viewed (gui-archive.md)
-> Loaded                                              a fold that agrees with the file's `viewed` is dropped
```

A fold is not saved, and `folds` is not cleared by a reload, so a file the user opened stays open while it is viewed.
A folded file is not fetched, so its colours are not read, as in the TUI.

## Tests

Each `E:` line is one test, in `test/scroll.test.ts` and `test/stream.test.ts`; seen by looking is in `test/glance.test.ts`: the
fresh cards (open, user-resolved and seen threads are not), a seen after a whole tick and once, nothing for a card scrolled past
between ticks, one write a tick, nothing again after a stale or a fresh reload, nothing after opening the card, nothing when
stopped or without a root, and a failed seen not retried by looking. Also: the prefix sums and total of the stream, the
items and heights across a file boundary, a folded
file (viewed, or folded by a click) and what its header says it hides, the placeholder heights, the card height
estimate for the text, the width and the line count, the layout kept while a wheel tick, a hover, a drag and a selection leave
its inputs alone, and rebuilt when the minute, the width, the focus or a fold changes, the rows' runs kept across a new minute,
the order of fetches (the screen and the next three, the rest below, the rest above, one
fetch at a time, a folded file left out, a failed file not asked for again until the next load), a reload fetching the
files whose summary changed and not a file on screen whose summary is the same, a stale reply dropped, the selection (topmost file wins, the
file a click scrolled to wins where the stream ends short, only the threads not in the diff leave it alone), the offset of a click in the
sidebar and of next and previous comment (and a key not in the stream kept waiting), a stream shorter than the screen sitting at the top, and the item that was at the top staying there when
a file above it arrives. Side by side is in `test/split.test.ts`: the pairing, the card order, the threshold and the toggle, and the item on top kept across a flip.
