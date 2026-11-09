# Call graphs: the GUI writes a comment

Part of the [herdr-review design](../../DESIGN.md). The mod is described in [gui](../gui.md). The CLI side of
a user write is in [commands](commands.md), user writes.

## Hover a line, drag a range, comment from a [+]

The stream's rows are ordinary elements the mod draws itself ([gui-stream](gui-stream.md), "Scrolling"), so a row can carry the
pointer's handlers ([herdr-app-facts](../herdr-app-facts.md) section 12): `on_hover` and `on_mouse_move` (the row it is on), `on_mouse_down`
(a press), and in its gutter the `[+]`, with an `on_click` of its own. This is what the TUI does ([tui-layout](../tui-layout.md)): the
`[+]` shows only on the line under the mouse, a plain click on a line highlights nothing, holding the mouse on a line and dragging
selects the lines between, and the `[+]` on a line inside the range comments on the range, the `[+]` elsewhere on that line.

The model has `hover` (the row under the pointer, `{ path, row }`), `drag` (a press that has not been released: the row it began on,
the row the pointer is on now, and whether it ever left the first row) and `selection` (the range, as before; a range is in one file).
There is no cursor and no shift-click.

```
-> on_hover(on) and on_mouse_move of a row               Msg.Hovered { path, row, on }; the move always says on = true, so a
                                                       row the stream scrolled under a still pointer is found at the next move
-> update(model, Hovered)                              (pure)
  -> on = false?  hover = null only when it is this row (the next row's enter can come before this row's leave)
  -> a row that is not in the file's rows?             ignored (a message from a frame drawn before the diff was replaced)
  -> hover = this row; the same row again returns the model as it was, so a move within a row draws nothing
  -> a drag in this file, and the row is not its head? the pointer moved: drag.head = row, moved = true
       selection = rows.span(file, drag.row, row)      (pure) the range between the row the press began on and this row
         side = the side of the first row, new for ctx and add, old for del
         a range that holds rows of both sides keeps the first row's side. Its ends are the first and last
         row between the two that have a line on that side. The first row stays where it was (anchor)
         a hunk header as the first row: no selection   E: a hunk header row is ignored
         back on the first row: that row alone
       focus and confirm are cleared                   (a card and a line are never both picked)
       a row of another file: ignored                  a drag is in one file
-> on_mouse_down("left") of a code row                  Msg.RowPressed { path, row }; main.ts gives the keyboard to the panel (focusNav) so Esc reaches it
-> update(model, RowPressed)
  -> editor open, or a row not in the file? ignored    the rows are not pressable while the editor is open
  -> drag = { path, row, head: row, moved: false }, hover = the row
       the selection is not touched yet: a click clears it on release, a move replaces it
       a drag still held from a release that never came is replaced by this press
-> the release: on_mouse_up("left") on the panel root     Msg.Released, wherever in the panel the button went up
-> update(model, Released)
  -> no drag and no grip on the thumb? the model as it was
  -> E: a plain click on a line highlights nothing, opens nothing, and clears a selection      (drag.moved = false: selection = null)
  -> a drag that moved keeps its range
  -> drag = null, grip = null
-> E: holding the press on a line and moving onto others selects the lines between, from the first row's side
```

A release outside the window never arrives, and a move carries no button state, so a drag cannot be told to have ended. The
next press replaces it and the next release ends it. The release is read on the panel's root, so it also ends a drag let go over the
sidebar, the header or the bar below the stream.

```
-> the [+]: drawn in a code row's gutter only when rows.rowState(model, row).hovered (a hunk header has none)
-> on_mouse_down("left") of the [+]                      cx.stop_propagation(): the press is the [+]'s, it must not start a drag
-> on_click of the [+]                                   Msg.PlusClicked { path, row }
-> PlusClicked { path, row }
  -> editor open? status "finish or cancel the comment first", nothing else
  -> rows.plusTarget(file, selection, row)             (pure)
       E: the row is in the selection and has a line on its side:  the selection, the whole range
       E: any other row:                                rows.pick(file, row), the line alone
       a hunk header row                                E: ignored
  -> selection = that target, so the commented lines stay lit while the editor is open
  -> anchor from rows.anchorOf, the editor opens
-> SelectionCleared                                    Esc on the panel (rows.panelKey), when a selection exists
  -> selection = null
-> FileCommentClicked                                  the Comment on file button of the bar under the diff
  -> the editor opens on { path }, no selection
```

The Esc order on the panel is: the selection first, then the filter. Esc in the editor closes it (`EditorCancelled`) and keeps the
selection, and the next Esc on the panel clears it. The bar under the diff has no Comment and no Cancel button: with no selection it says
`Drag over lines to select them, [+] to comment`, with one it names it (`src/a.rs R12-18 (7 lines). [+] to comment, Esc to clear`), and
`Comment on file` stays. The selected rows are tinted (`rows.rowState`), also while the editor is open.

The editor is not inline under the row: it is a box docked under the diff, with its title (`Draft note - src/a.rs R12-18`, `Reply to
u3`, `Edit u3`), the textarea, a Save and a Cancel button and the line `Ctrl+S save, Esc cancel`.

A selection is in memory. A reload keeps it when the file and the rows it covers are unchanged, and clears
it otherwise.

## Write the comment

```
-> EditorSaved { text }                                user action
  -> text empty after trim, or a save already running? E: ignored, the editor stays
  -> the editor holds the anchor it opened with, from rows.anchorOf(file, selection)   (pure)
       { path, old_path, side, line, end_line }, or { path } for a file comment
       an old-side range on a renamed file carries old_path
  -> Cmd.Write { op: { kind: comment, anchor, text } }, saving = true
-> run Cmd.Write
  -> the batch as JSON, one entry: {"comments":[{"path","side","line","end_line","body"}]}
  -> exec mkdir -p <dir>, exec chmod 700 <dir>         the folder is private whatever its mode was
  -> Host.writeTextFile(appHome()/data/herdr-review/<n>.txt, batch, null)    R: Host
                                                       E: rejected -> Msg.Failed, the editor keeps its text
  -> ReviewCli.apply(root, file, spec)                 R: Host.exec, 30 s
       comment apply --as user --file <path> --repo <root> --spec .. --base ..
                                                       E: BinaryMissing, TooOld -> the file is emptied, Msg.Failed
                                                       E: BadRequest "invalid batch" (a line past the end of
                                                          the file, a body over 16 KiB) -> Msg.Failed, the
                                                          editor keeps its text, one reload
                                                       E: Busy, BackendFailed -> Msg.Failed, the editor keeps
                                                          its text
  -> Msg.Wrote
-> update(model, Wrote)                                (pure)
  -> editor closed, selection cleared, saving = false, Triggered    the reload shows the new thread
-> update(model, Failed { write })                     saving = false, the editor stays
```

On any failure the editor stays open with the text. The text is discarded only after the CLI exits 0, as in
the TUI ([pane](pane.md), user comment).

Why a batch file and not `--body-file`: a new thread needs an anchor, and `comment apply` already takes one,
with the CLI reading the anchored line itself. One way in for a new thread, and no second one to keep in step.

Cardinality: once per user action.

## Tests

Each `E:` line is one test. Also: a hover is the row under the pointer and a leave only counts for the row that has it, a drag is
in one file and a row that is not in the file is ignored, a new drag starts over and a press whose release was lost is replaced by
the next press, moves with no press select nothing, the selected and the hovered rows are marked for drawing and a hunk header has no
`[+]`, rows and the `[+]` do nothing while the editor is open, Esc clears the selection before the filter, the bar has no Comment or Cancel and
names the drag hint, a drag that starts on a `del` row anchors on the old side, a drag from a `ctx` row onto an `add` row anchors on the new
side, a range over both a `del` and an `add` row keeps the first side, a header and a card click name their message and a code row none, a file
comment writes `{"path", "body"}`, the batch file is under `appHome()/data/herdr-review/` (made private with mkdir and chmod 700) and its path is the only thing on
argv, the body never appears in an `exec` call, and the root and spec flags are the model's.
