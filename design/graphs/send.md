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

## The same send under the hc backend

The graph above is the herdr backend. The hc backend runs the same graph with the three terminal calls
replaced ([send](../send.md), backends). The steps that differ:

```
-> choose the backend                      (pure)     --backend, else hc for --target or when hc is the
                                                      nearest host (HC_TERMINAL_ID set, HERDR_PANE_ID not), else herdr
-> collect threads                         (pure)     --thread ids name open threads, sent or not
                                                      E: a named id that is not an open thread -> UnknownId, exit 2
-> format prompt                           R: current_exe, root   `<bin>` is the running binary
-> resolve target
  -> --target: hc get <t> --json, the only candidate   R: hc
                                                      E: finished or exit 3 -> Refusal::AgentGone, nothing sent,
                                                         no other candidate asked
                                                      E: other failure -> TargetError::Backend
  -> else meta hc_target: hc get <t> --json   R: dir, hc
                                                      E: any failure, finished, cwd outside the root, or no
                                                         status -> next candidate
  -> hc ls --json                          R: hc      E: HcError -> propagate (TargetError::Backend)
                                                      E: NoAgent -> propagate
                                                      E: Ambiguous -> candidates in the output
-> readiness from `hc get`, of the terminal resolution chose
                                                      E: status null or reported false -> Refusal::NotReady
                                                      E: status.state blocked -> Refusal::Blocked
                                                      E: state finished -> Refusal::AgentGone
-> hc send <t> <text>                      R: hc      E: exit 9 -> Refusal::Blocked (it changed since the get)
                                                      E: exit 11 or 3 -> Refusal::AgentGone
                                                      E: any other exit -> HcError, nothing marked
                                                      exit 0 and "status":"working" -> Queued, else Sent
-> store::write(sent event), save hc_target           as above
-> print the outcome, or with --json one object        E: any error -> with --json, { error, message, candidates }
                                                          on stdout and exit 1
```

Decisions that fall out of the graph:

- No retry anywhere in send. A second `herdr agent prompt` after an unclear failure could deliver the
  batch twice, so the user decides by pressing the key again.
- Once the prompt is delivered the operation is a success. If the `sent` event cannot be written, the
  user sees "sent, but not recorded, the next send will repeat these comments".
- The TUI draws "sending" before the Herdr call, because the call blocks the single thread.
- `send` as a Herdr action runs the same function. Its edge turns every `Err` and `Warning` into
  `herdr notification show` and an exit code.
- Under the hc backend there is no `hc` notification, so the edge prints and exits. With `--json` the edge
  prints one JSON object on stdout in every case, so the panel reads the error from the object and not from
  stderr.
