# Call graphs: the GUI writes a comment

Part of the [herdr-review design](../../DESIGN.md). The mod is described in [gui](../gui.md). The CLI side of
a user write is in [commands](commands.md), user writes.

## Select a line, a range or a file

The diff is a `list`, and herdr-app forbids a handler inside a list's item renderer. The only click a row can
have is the list's `on_item_click`, which reports the item's key and nothing else: no modifier, no mouse
move, no row under the pointer. So a drag cannot be built, and a range is two clicks.

```
-> RowClicked { row }                                  user action, on_item_click key "r<row>"
  -> editor open? ignored                              the rows are not clickable while the editor is open
  -> rows.pick(file, row)                              (pure)
       a hunk header row                               E: ignored
       a ctx, add or del row: one-row selection, side new for ctx and add, old for del
  -> a one-row selection exists?                       (pure)
       the same row: the selection is dropped
       another row: rows.span(file, first, second)     the range between the two clicks
         side = the side of the first row clicked, new for ctx
         a range that holds rows of both sides keeps the first row's side. Its ends are the first and last
         row between the two that have a line on that side
       a range exists: a click starts a new one-row selection
-> CommentClicked                                      the Comment button of the bar under the diff
  -> selection -> anchor, the editor opens
-> FileCommentClicked                                  the Comment on file button of the same bar
  -> the editor opens on { path }, no selection
```

`span` cannot find no row on the side: the first row clicked has a line on its own side by construction, so the
"select lines of one side" status of the first design never happens and is not built.

The editor is not inline under the row. A card item cannot hold a textarea, whose state a list renderer may not
create, so the editor is a box docked under the diff, with its title (`Draft note - src/a.rs R12-18`, `Reply to
u3`, `Edit u3`), the textarea, a Save and a Cancel button and the line `Ctrl+S save, Esc cancel`. The
selected rows have a tint while it is open.

A selection is in memory. A reload keeps it when the file and the rows it covers are unchanged, and clears
it otherwise. `EditorCancelled` closes the editor and keeps the selection, and a second one drops the selection.

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

Each `E:` line is one test. Also: a click on a `del` row anchors on the old side, a second click from a `ctx` row
on an `add` row anchors on the new side, a range over both a `del` and an `add` row keeps the first side, a file
comment writes `{"path", "body"}`, the batch file is under `appHome()/data/herdr-review/` (made private with mkdir and chmod 700) and its path is the only thing on
argv, the body never appears in an `exec` call, and the root and spec flags are the model's.
