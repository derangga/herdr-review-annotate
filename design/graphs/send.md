# Call graphs: send

Part of the [herdr-review design](../../DESIGN.md).

```
-> collect unsent threads from Review      (pure)     none -> SendOutcome::Nothing
-> format prompt                           R: env (plugin root), root   (pure)
-> resolve target (send.md)
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
