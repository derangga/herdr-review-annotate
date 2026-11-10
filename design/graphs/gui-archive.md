# Call graphs: the GUI archives threads and marks a file viewed

Part of the [herdr-review design](../../DESIGN.md). The mod is described in [gui](../gui.md). The CLI side is
in [commands](commands.md), user writes, and the store side in [store](store.md).

## Archive the resolved threads

```
-> ArchiveClicked                                      user action
  -> update(model, ArchiveClicked)                     (pure)
       a load is in flight or the editor is open                E: nothing (the button is off), no call
       State.archivable is 0, or no state yet                   E: nothing (the button is off), no call
       else confirmArchive = true and nothing else. The bar under the diff becomes the question
       "Archive 3 resolved threads?" ("Archive 1 resolved thread?" for one), an accent Archive and a Cancel.
       State.unread_resolved above 0 adds " N more resolved by the agent is unread." ("are unread" for
       more than one), the threads archive leaves because the user has not seen the agent's resolve
       E: unread_resolved 0 -> the sentence is the plain question
-> ConfirmCancelled                                    the Cancel button, or Esc on the bar
  -> confirmArchive = false, nothing else changes
-> Loaded while the question is up
  -> update(model, Loaded)                             (pure)
       the new state has archivable 0                           E: confirmArchive = false, the question goes
       else the question stays and says the new number
-> ArchiveConfirmed                                    the Archive button of the question
  -> update(model, ArchiveConfirmed)                   (pure)
       confirmArchive is false or a load is in flight           E: nothing, no call
       else confirmArchive = false, status "archiving...", Cmd.Archive
-> run Cmd.Archive
  -> ReviewCli.archive(root)                           R: Host.exec, 30 s
       archive --repo <root> --json
                                                       E: Busy -> Msg.Failed, status "review is busy, try again"
                                                       E: BackendFailed -> Msg.Failed, the files are as they were
    -> decodeArchive(stdout)                           boundary: { archived, unsent }
                                                       E: ContractMismatch -> Msg.Failed
  -> Msg.Archived { archived, unsent }
-> update(model, Archived)                             (pure)
  -> status "archived N threads", with ", M of them were never sent" when unsent is above 0
     status "nothing to archive" when archived is 0                 E: the CLI found nothing, not the button
  -> Triggered
```

The Archive button with nothing archivable and `unread_resolved` above 0 is off, and `rows.dock` returns a dim note,
"Nothing to archive yet: N resolved by the agent is unread (marked new)", which `view.ts` draws next to the buttons.
The unread cards become seen by being opened or by staying on screen for a tick ([gui-stream](gui-stream.md), seen by
looking), and the reload that follows brings `archivable` up.

The panel counts before asking: `state` carries `archivable`, the threads `archive` would move now, and the
Archive button shows it and turns off at 0 ([shapes](../shapes.md)); `unread_resolved` is the rest of the resolved threads. The number is from the last load, and the
reply of the archive says what moved.

The store counts under its lock, so it archives what is resolved then and not what the button counted. A
thread an agent resolved in between is `new` and stays ([store](store.md)). The count in the status line is
the reply's, not the button's.

Cardinality: once per user action.

## Mark a file viewed

```
-> ViewedClicked { path, viewed }                      user action, the button in the bar under the stream: it acts on
                                                       the sidebar's selected file
  -> folds[path] = viewed, so the file folds at once to its header (or opens, when the mark is taken off); marking
     it also sets reveal to its header, as the TUI puts the cursor there
  -> Cmd.SetViewed { path, set }
-> run Cmd.SetViewed
  -> ReviewCli.view(root, spec, path, set)             R: Host.exec, 30 s
       view --repo <root> --path <p> --set|--unset [--spec ..] [--base ..]
                                                       E: BadRequest "not in diff" (the file left the diff)
                                                          -> Msg.Failed, one reload
                                                       E: Busy, BackendFailed -> Msg.Failed, the mark is as it was
  -> Msg.Wrote
-> update(model, Wrote)
  -> Triggered                                         the reload carries the new `viewed` flag, and a fold that
                                                       agrees with it is dropped, so the file follows the mark again
```

A viewed file is the one whose fingerprint equals the saved one, so the mark goes by itself when the file
changes ([data](../data.md), meta file). A viewed file is folded to its header in the stream, as in the TUI, and a
click on the header opens it. The fold is the panel's own and is not saved: a new panel folds every file that is
viewed, and an open one stays open until the mark goes. A file that changed, and so lost its mark, is open.

Cardinality: once per user action.

## Tests

Each `E:` line is one test. Also: the archive button is disabled while a load is in flight and when `archivable` is 0 (a click then
asks nothing), and otherwise reads `Archive N`, a click on Archive
only asks and the row has the sentence with the number (3 plural, 1 singular) and the two buttons, the sentence with one unread, with several, and with none, the off button and its note when
nothing is archivable but some are unread (and no note with something archivable or nothing unread), a reload to 0
drops the question and a reload to another number changes it, ArchiveConfirmed without the question sends nothing,
Cancel and Esc leave the model as it was, a reload keeps the question and a send drops it, the status text for 0, 1 and several unsent, and the `view` argv carries the
model's spec. A status that reports a result (sent, archived, sending) survives the reload it starts and goes at
the next refresh, file or spec choice, or when another status replaces it.
