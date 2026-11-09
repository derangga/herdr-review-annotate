# Call graphs: the GUI sends

Part of the [herdr-review design](../../DESIGN.md). The mod is described in [gui](../gui.md). The Rust side,
including target resolution for hc, is in [send](../send.md) and [the send graph](send.md).

## Send the unsent comments, or resend one thread

```
-> SendClicked { threads? }                            user action
  -> update(model, SendClicked)                        (pure)
       no threads given and no unsent thread in the state   E: status "nothing to send", no call
       editor open                                          E: status "finish or cancel the comment first"
                                                            (the text is in the view, so the model cannot tell
                                                            an empty editor from one with text)
       a send already running                               E: nothing, so one click is one exec
       status "sending...", Cmd.Send { target, threads }
-> run Cmd.Send
  -> ReviewCli.send(root, target, threads)             R: Host.exec, 30 s
       send --repo <root> --json [--target <t>] [--thread <id>]...
                                                       E: BinaryMissing, TooOld -> Msg.Failed
                                                          (send --json is new, so v0.5.0 gives TooOld)
    -> decodeSend(stdout)                              boundary
                                                       E: ContractMismatch -> Msg.Failed
  -> exit 0: Msg.Sent { sent, agent, queued }
  -> exit 1 with a JSON error:
       E: blocked, not_ready, agent_gone  -> Msg.Failed { Refused }
       E: no_agent                        -> Msg.Failed { NoTarget }
       E: ambiguous                       -> Msg.Failed { AmbiguousTarget { candidates } }
       E: busy                            -> Msg.Failed { Busy }
       E: backend                         -> Msg.Failed { BackendFailed }
-> update(model, Sent)                                 (pure)
  -> status "sent N to <agent>", or "agent is working, N comments queued" when queued
  -> Triggered                                         the reload shows the threads as sent
-> update(model, Failed { Refused })                   (pure)
  -> status with the reason, Cmd.Notify                R: Host.notify
       nothing is marked sent, the next click sends the same comments again
       E: AgentGone -> the model's target is dropped and the status says so. The next click, which the
          user makes knowing the agent is gone, resolves by folder or shows the picker
-> update(model, Failed { AmbiguousTarget })           (pure)
  -> picker = candidates                               the picker lists address, agent, folder and status.
                                                       It is a block of buttons in the bar under the diff, not
                                                       an overlay, so there is nothing to release in `close`
-> PickerChosen { address }                            user action
  -> picker = null, target = address, Cmd.Send with --target and the threads of the click that asked
       an address the CLI did not offer sends nothing
-> PickerCancelled
  -> picker = null, nothing sent
```

The model keeps the chosen `target` for the rest of the copy's life. The CLI also saves it in `meta.json`
([send](../send.md)), so the next copy of the panel starts from it.

A resend names the thread's id with `--thread`. It sends that thread even if it was already sent, and only if
it is open. Resend is offered on an open thread whose comments are all sent.

No retry anywhere in send. A second send after an unclear failure could deliver the batch twice, so the
user decides by clicking again.

Cardinality: once per user action.

## Tests

Each `E:` line is one test. Also: the argv with and without a target and with two `--thread` flags, `Sent`
with `queued` true shows the queued line, a `Refused` calls `notify` once, a picked address resends with
`--target` and nothing else changed, a picker cancel calls nothing, and `exec` is never called twice for one
click.
