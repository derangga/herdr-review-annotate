# Call graphs: the GUI's keys

Part of the [herdr-review design](../../DESIGN.md). The mod is described in [gui](../gui.md), "The keyboard". The TUI's keys
are in `keymap.rs` and [tui](../tui.md). Next and previous comment (`n`, `shift+n`) are in [gui-thread](gui-thread.md).

A key reaches the panel only along the focus path ([herdr-app-facts](../herdr-app-facts.md)): the root element tracks `nav`,
and `main.ts` gives it the keyboard as it does for `n`. Every key below is `rows.panelKey`, which returns a `Msg`; the model's
change is `update`, pure. Nothing here calls the host, so there is no `R:`.

```
-> a key on the panel                                   rows.panelKey(model, keystroke), pure
     E: editor, question (confirm or confirmArchive), or picker up -> nothing, so a note can say "j"
     E: the filter box has the keyboard -> nothing but Esc, which clears the filter
     E: a chord that is not one of these (alt-j, ctrl-j)         -> nothing

-> the cursor                                           Model.cursor { path, row, side } | null
     A code row of a loaded, unfolded file; never a hunk header. `side` is the half in side by side, null in unified.
     It is null until a motion key or a press on a row, and drawn as a neutral tint on its row (its half), apart from the accent of a
     selection (rows.cursorOn). rows.spots lists where it can stand, in the order the stream draws: the rows of each file in
     the diff's order; in side by side the pairs (a context row is both halves, a removed row faces an added one), left then right.
  -> j, down / k, up            Msg.CursorMoved { by: 1 | -1 }
  -> ctrl+d, pagedown / ctrl+u, pageup   Msg.CursorMoved { by: +half | -half }   half = a half screen of rows (height / ROW / 2, at least 1)
  -> [ / ]                      Msg.CursorHunk { forward }
  -> h, left / l, right         Msg.CursorSide { side: old | new }   side by side only; in unified the key is nothing
  -> update(model, CursorMoved)                          (pure)
       the walk is rows.lane: the spots on the cursor's half (all of them in unified)
       no cursor, or one that stands on no drawn row (its file folded, replaced, gone) -> it is put on rows.topSpot, the first code
         row at or below the item on top of the screen (the file's first when a card or header is on top), a context row on its new
         half, and goes no further
       else it moves `by` spots along the lane, in stream order and across files, and stops at the ends
       files not loaded, and files folded, have no spots, so they are passed over
       a move sets reveal = { key: <path>|r<row>, near: true } and focus = null (a line and a card are never both picked)
  -> update(model, CursorHunk)                           (pure)
       the first spot after the cursor's (or at or before it, going back) that is the first of its side after a hunk header,
       across files; none -> nothing. No cursor -> as a motion
  -> update(model, CursorSide)                           (pure)
       the cursor goes to rows.partner of its row, on the side asked; no partner, the side already asked, or a range open -> nothing
  -> rows.scrolled                                       the one place a reveal becomes an offset
       near: the offset stays when the item's box is on screen, else it is the item's top (above) or its bottom edge at the
       bottom of the screen (below), clamped. A reveal without near puts the item on top, as before
     E: j on no cursor -> the cursor on the top row, not one past it
     E: j and k across files, in order, stopping at the first and last row of the stream
     E: a file not loaded, a file folded by the user, a file marked viewed: passed over
     E: a fetch that makes the cursor's row another line (rows.sameRows, as for the selection) drops the cursor
     E: [ and ]: first row of the hunk, across files, the cursor's half in side by side
     E: ctrl+d and ctrl+u: a half screen, clamped
     E: h and l in side by side: the other half of a changed pair, of a context row, nothing when it has no row, nothing in unified
     E: the reveal is `near`, as little as it takes: below, above, already on screen, an item not in the stream
     E: a press on a code row moves the cursor there (its half in side by side); a press on a hunk header does not
     E: a flip of the layout, by the button or a resize, keeps the cursor on its row: no half in unified, the row's own half in side by side
     E: a spec change drops the cursor

-> v                            Msg.VisualToggled
  -> update(model, VisualToggled)                        (pure)
       a selection is open (a range v began, or a drag's)  selection = null, visual = false
       a cursor on a drawn row                             selection = rows.pick(file, cursor.row, cursor.side), visual = true, focus = null
       no cursor                                           the cursor is put on the top row, nothing else
  -> j, k, ctrl+d, ctrl+u, [, ] while visual                the motion above, on the cursor's half and in its file only
       the cursor stops at the first and last row of its file, so a range never crosses files
       selection = rows.span(file, selection.anchor, cursor.row, selection.side): the side is the first row's, even over rows
         that have no line on it, and the ends are the first and last row between that do
  -> h, l while visual                                    nothing: the range keeps its half
  -> Esc                        Msg.SelectionCleared      selection = null, visual = false. The cursor stays
  -> a press on a row, or a [+] click                     visual = false: what the mouse makes is the mouse's
     E: v on a row: a one-row range, side = the cursor's half or the row's own side
     E: j and k grow and shrink it; going above the anchor keeps the rows that have a line on its side
     E: the range stops at the first and last row of its file, also for ctrl+d, ctrl+u, [ and ]
     E: v again, and Esc, end it and leave the cursor; after that a motion only moves
     E: v on no cursor only shows it
     E: in side by side the range keeps the half it began on, and h and l leave it

-> c                            Msg.PlusClicked { path, row, side } of the cursor's spot  exactly what a [+] click sends
  -> update(model, PlusClicked)                          (pure) gui-comment.md
       the cursor's row is in the selection (on its side) -> the editor opens on the selection
       else -> on the row alone, on the cursor's half (or the row's own side in unified)
     E: c with no selection: the row; with a range and the cursor in it: the range, the same model as the click
     E: c with the cursor outside the range: the row alone
     E: a removed row is on the old side, a side by side half is the cursor's
     E: c with no cursor only shows it and opens no editor
     E: c on a cursor whose file is folded opens no editor

-> r, e, d, x with a thread focused (Model.focus)       the Msg of the same button on the focused comment's card
                                                       (rows.cardButtons, taken as the thread's last comment)
     r -> ThreadAction reply, e -> edit, d -> delete (which only asks: the question's Delete writes), x -> resolve on an open
     thread and reopen on a resolved one, from any of its comments. A card offers no edit or delete on an agent's comment, so
     the key is nothing too; with nothing focused, or a selection open, they are nothing
     E: each of the four, on an open thread, a resolved thread and an agent's comment; the model they give is the button's
     E: nothing focused -> nothing
```

Cardinality: once per key.

## Tests

Each `E:` line is one test, in `test/keys.test.ts`, which drives `panelKey` and then `update`.
