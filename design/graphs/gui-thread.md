# Call graphs: the GUI acts on a thread

Part of the [herdr-review design](../../DESIGN.md). The mod is described in [gui](../gui.md). The CLI side is in
[commands](commands.md), user writes.

Reply, resolve, reopen, edit, delete and seen share one shape: the model turns a click into a `Cmd`, the
runner runs one CLI call, `Wrote` follows, and a load shows the result. Their failures differ only where
written below.

```
-> ThreadAction { action, id }                         user action
     id is a comment id. A card is a head item and one item per comment, so a click on the head is `toggle` with
     the root's id and a click on a comment is `focus`. The buttons are on the card, at the bottom of each comment
     (`rows.cardButtons`), and name that comment's id; a press on one stops there and is not also a `focus`. Reply is
     on every comment, resolve or reopen and resend on the thread's last, edit and delete on the user's own.
     reply, resolve and reopen act on the thread of that comment, edit and delete on the comment itself.
  -> update(model, ThreadAction)                       (pure)
       any action but focus while the editor is open   status "finish or cancel the comment first"
       reply   -> open the editor for that thread, under its card (gui-comment.md), and reveal it
       edit    -> open the editor with the comment's text, only on a comment whose author is "user", under its card, and reveal it
       resolve -> Cmd.Write { resolve, id }            only on an open thread
       reopen  -> Cmd.Write { reopen, id }             only on a resolved thread
       delete  -> sets Model.confirm = id and nothing else   only on a comment whose author is "user"
                  the bar under the diff becomes the question: "Delete this comment? Its replies go too." when
                  the comment is a root with replies, else "Delete this comment?", then a red Delete and a Cancel
       toggle  -> a resolved thread opens or closes. Opening one that is_new -> Cmd.Write { seen, id }, and seenSent
                  gains the id. A card that stays on screen sends the same write by itself (gui-stream.md, seen by looking)
  -> EditorSaved for reply and edit                    text empty after trim -> ignored, as in gui-comment.md
-> DeleteConfirmed { id }                              the Delete button of the question
  -> update(model, DeleteConfirmed)                    (pure)
       confirm is not id, the comment is gone, or not the user's -> nothing
       else confirm = null, Cmd.Write { delete, id }
-> ConfirmCancelled                                    the Cancel button, or Esc on the bar
  -> update(model, ConfirmCancelled)                   confirm and confirmArchive are null, nothing else changes
-> run Cmd.Write                                      the Cmd names the op, and Wrote carries its kind: only
                                                       comment, reply and edit close the editor
  -> reply   comment reply   --as user <id> --body-file <f> --repo <root>      body file as in gui-comment.md
  -> edit    comment edit    <id> --body-file <f> --repo <root>
  -> resolve comment resolve --as user <id> --repo <root>
  -> reopen  comment reopen  --as user <id> --repo <root>
  -> delete  comment delete  <id> --repo <root>
  -> seen    comment seen    <id> --repo <root>
                                                       R: Host.exec, 30 s, and Host.writeTextFile for a body
                                                       E: Busy -> Msg.Failed, status "review is busy, try again"
                                                       E: BackendFailed -> Msg.Failed, the editor keeps its text
                                                       E: BadRequest unknown id (the thread went away, an
                                                          archive by another pane) -> Msg.Failed, one reload
                                                       E: BadRequest not allowed (an agent's comment) ->
                                                          Msg.Failed, one reload. The buttons are hidden
                                                          for them, so this is a race or a bug
                                                       E: seen failed of any kind -> escape, nothing shown,
                                                          the marker stays until the next load
  -> Msg.Wrote
-> update(model, Wrote)                                (pure)
  -> editor closed, Triggered
```

-> ThreadJumped { forward }                            the Previous and Next comment buttons, and n and shift+n
     the keys are mapped by rows.panelKey, which says nothing while the editor, a question or the picker is up
  -> update(model, ThreadJumped)                       (pure)
       editor open                                     status "finish or cancel the comment first", nothing else
       picker up                                       nothing
       rows.jumpStop(model, forward) is null           status "no comments in this review"
         the order: threads not in the diff, then the files in the sidebar's order, in a file the cards as
         drawn (no row first, then by row). A file whose threads are not drawn (not fetched yet, or folded) with
         FileSummary.threads > 0 is one stop
         from the focused thread: the next or previous stop, wrapping at the ends. Nothing focused: next is
         the selected file's first stop, previous the last stop before it
       a thread                                         focus = id, reveal = that card's key (gui-stream.md),
                                                        confirm null, no Cmd
       a file, fetched but folded                       the file is opened (folds[path] = false) and the jump lands at
                                                        once, as on a thread
       a file, not fetched                              opened, jump = { path, first | last }, reveal = its header,
                                                        and the fetch starts with it; focus stays until it is there
  -> FilesLoaded with the key of that fetch             (pure)
       jump set and its file is in the reply            focus = the file's first (or last) card, reveal = that card,
                                                        jump null. No card -> status "no comments in <path>"
  -> FileChosen, SpecChosen, Failed { load | files }    jump null
-> a key n or shift+n on the panel                     only while no editor, question or picker is up
  -> rows.panelKey -> ThreadJumped                     the root element tracks the nav focus handle
-> the "focus" command from herdr-app                  runner.listen -> main.ts gives nav the keyboard
-> "next-comment" or "previous-comment" from herdr-app runner.listen -> ThreadJumped (herdr-app do herdr-review.next-comment)
                                                        E: a command that is not "focus" -> nothing
                                                        E: nextCommand rejects -> the loop ends, nothing shown

A resolve of a resolved thread, and a reopen of an open one, exit 0 and write nothing, so a double click
does no harm. The mod hides the wrong button anyway.

An expanded resolved thread stays expanded in `Model.expanded` across loads, keyed by the root id. Collapsed
is the default for a resolved thread, so a long review stays short.

Cardinality: once per user action.

## Tests

Each `E:` line is one test. The jumps are tested in `test/jump.test.ts`: the order across files and the cards
drawn, wrap-around both ways, nothing focused, no thread, a thread already drawn, a folded file (landed on at once,
or opened and waited for), a jump into a file not fetched yet that waits for the fetch (forward lands on the first
card, back on the last), a stale reply leaving the jump waiting, a failed fetch, a file choice or a spec choice dropping it,
a file that arrives empty, the editor open, the keys (`n`, `shift-n`, `N`, other chords, nothing while the editor, a
question or the picker is up), and the commands. `test/stream.test.ts` checks that the card index a jump scrolls to
is the card's item. Also: the argv of each of the six calls, `--as user` on reply, resolve and
reopen and on nothing else, each comment's card buttons (Reply on every one, Resolve or Reopen and Resend on the last only, Edit and Delete absent on an agent's comment, each button's message and Delete red) and none of them in the dock, a click on Delete only
asks and the row has the right sentence and buttons, only DeleteConfirmed writes, Cancel and Esc leave the
model as it was, focusing another comment drops the question and a reload keeps it while the comment is there, a `seen` for a thread that is not new is never sent, and the editor text survives a failed reply.
