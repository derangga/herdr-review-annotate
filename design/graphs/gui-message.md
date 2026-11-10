# Call graphs: the GUI reviews the agent's last message

Part of the [herdr-review design](../../DESIGN.md). The mod is described in [gui](../gui.md), the feature
in [message-review](../message-review.md), the commands in [cli](../cli.md). The mod holds no message
logic: it reads the message and sends the comments through `herdr-review`, as the TUI pane does through the
same Rust.

## Open the view, read the message

```
-> ViewChosen { view: "message" }                      user action: the header button "Agent message"
  -> update(model, ViewChosen)                         (pure)
       editor open                                          E: status "finish or cancel the comment first", no change
       no target                                            E: view "message", status "no agent to read ...", no call
       view = "message", loading, Cmd.ReadMessage { target }
-> run Cmd.ReadMessage
  -> ReviewCli.message(target)                         R: Host.exec, 60 s
       message --json --target <t>
                                                       E: BinaryMissing, TooOld -> Msg.Failed op "message"
                                                          (a binary without `message --json` says "unknown option")
                                                       E: exit 1 (no session, no transcript, not claude, no agent,
                                                          hc failed) -> Msg.Failed { BackendFailed } with the reason
    -> decodeMessage(stdout)                           boundary
                                                       E: ContractMismatch -> Msg.Failed
  -> Msg.MessageLoaded { reply }
-> update(model, MessageLoaded)                        (pure)
  -> same id as the message on screen                  the comments stay
  -> another id (or none on screen)                    E: the comments, the selection and an open editor are dropped, and the
                                                          status says "the agent wrote a new message, N comments dropped"
-> update(model, Failed { op "message" | "message-send" })   (pure)
  -> status = the first line of stderr, or the text of the error. The message and the comments stay. Nothing
     else of the model changes: the diff's status, banner and `stopped` are its own
-> ViewChosen { view: "diff" }
  -> view = "diff". The diff is as it was, and the comments stay in the model until the copy closes
-> MessageRefreshed                                    user action: Refresh in the message header
  -> a read is running                                      E: nothing, so one click is one exec
  -> Cmd.ReadMessage, as above
```

The view reads the message when it opens and on Refresh. The 10 s timer and the watches reload the diff and
leave the message alone, since a message that changes under the user's selection would move the lines.

## Select, comment, edit, delete

```
-> MessageHovered { line, on }                         the pointer over a line: the [+] is drawn on that line only
       a line the message does not have                     E: ignored
       a leave for a line that is not the hovered one       E: ignored
-> MessagePressed { line }                             a left press on a line: starts a drag and changes nothing else
       editor open                                          E: ignored
-> MessageHovered while a press is held                the range from the line pressed to this one, either way round
-> MessageReleased                                     the button went up anywhere in the panel
  -> the press never moved: a click, so the selection is cleared
-> MessagePlusClicked { line }                         the [+] on a line
       editor open                                          E: status "finish or cancel the comment first"
  -> the line is inside the selection: the range. Else the line alone. It is the selection, and the editor opens
     with the title "Comment on line 3" or "Comment on lines 3-5"
-> EditorSaved { text }                                the existing editor; in the message view it adds a comment
       text blank                                           E: nothing, the editor stays
  -> comment { id, start, end, body } appended to the model, editor closed, selection cleared. No Cmd: the comment
     is not written anywhere, as the TUI keeps its own in the pane's memory
-> EditorCancelled                                     closes the editor and leaves no comment
-> MessageCommentAction { id, action: "edit" }
       editor open                                          E: status "finish or cancel the comment first"
  -> editor on the comment's text, titled "Edit comment on lines 3-5". Save replaces the body, the card stays
-> MessageCommentAction { id, action: "delete" }
  -> the comment is gone at once, as the TUI's delete. An id that is not there changes nothing
```

A comment is a small card under the last line it covers: its range, its text, `Edit` and `Delete`.

## Send

```
-> MessageSendClicked                                  user action: "Send N" in the header
  -> update(model, MessageSendClicked)                 (pure)
       a send or a read is running, editor open             E: nothing, so one click is one exec
       no comments                                          E: status "nothing to send", no call
       a send is running: a [+], Edit or Delete              E: refused with "wait for the send to finish" (its reply clears the comments)
       Refresh with the editor open                         E: refused with the busy status, so a new message cannot drop the text
       no target                                            E: status "no agent to read ...", no call
       sending, status "sending...", Cmd.SendMessage { target, id, comments }
-> run Cmd.SendMessage
  -> write { id, comments: [{ start, end, body }] } to a file in the private folder     R: Host.writeTextFile
       The comments are the user's text, so they go in a file the CLI reads and deletes, as a body does, and
       never on argv. When the exec fails before the CLI opened it (BinaryMissing, TooOld) the file is
       overwritten with nothing
  -> ReviewCli.messageSend(target, file)               R: Host.exec, 30 s
       message send --target <t> --file <f>
                                                       E: BinaryMissing, TooOld -> Msg.Failed op "message-send"
                                                       E: exit 2 (a line the message does not have, a message that is
                                                          no longer the one the comments were written on, an empty body)
                                                          -> Msg.Failed { BadRequest } with the reason
                                                       E: exit 1 (the agent is blocked, not ready or gone, hc failed,
                                                          no message) -> Msg.Failed { BackendFailed } with the reason
    -> decodeMessageSent(stdout)                       boundary
  -> Msg.MessageSent { sent }
-> update(model, MessageSent)                          (pure)
  -> comments = [], selection = null, status "sent N comments to <agent>". The message stays on screen
-> update(model, Failed { op "message-send" })         (pure)
  -> the comments stay and the status is the reason. The next click sends the same comments again
```

`id` is the id of the message the lines were counted in. The CLI refuses a send when the agent's newest message
has another id, so a message the agent wrote since the last read never receives comments meant for the old
one. The prompt is built by the CLI (`format_prompt`), the same one the TUI pane uses, so the two agree.

No retry anywhere. A second send after an unclear failure could deliver the comments twice, so the user decides
by clicking again.

## Tests

Each `E:` line is one test (`test/message.test.ts`). Also: the header button's two states, the items of a
message (a card under its last line, comments in the order written), a drag from either end, a click clears,
the `[+]` inside and outside a selection, edit then save, delete of one comment of two, a send of two comments
asks for one command and a second click for none, a success clears the comments, a refusal keeps them and the
diff's status, the argv of both commands, the comments never on argv, the draft overwritten when the binary is
too old, `decodeMessage` and `decodeMessageSent`, and a diff that is as it was after a visit to the message.
