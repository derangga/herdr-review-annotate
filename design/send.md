# Send

Part of the [herdr-review design](../DESIGN.md).

## Prompt

```
Address the review comments below. For each one: make the change, then resolve it with a one-line
reply. If you disagree or are unsure, reply and leave it open.

Resolve:
'<bin>' comment resolve --repo '<root>' <id> --reply - <<'EOF'
<one line>
EOF
Reply without resolving:
'<bin>' comment reply --repo '<root>' <id> - <<'EOF'
<text>
EOF

Comments on the diff (L = line in the original file, R = in the changed file):
- [u7] src/lib.rs:42 (R): body text
  more lines indented by two spaces
- [u8] src/lib.rs:50-57 (R): comment on a range
- [u9] src/store.rs (file): comment on the whole file
- [u3] src/cli.rs:10 (R), reopened: the original comment
  > agent: Added with_capacity
  > user: still allocates twice
- [a2] src/cli.rs:88 (R), your comment: the agent's comment, first line only
  > user: reply text
```

`<bin>` is `$HERDR_PLUGIN_ROOT/bin/herdr-review`. Both paths are single-quoted with `'` escaped.
`send` and the TUI both run as Herdr plugin commands, so the variable is always set.

## Target resolution (ADR 0006)

A candidate is valid when `herdr agent get <pane>` passes `agent_ready`, returns the stored
`terminal_id`, and has a `cwd` equal to the root, inside it, or an ancestor of it.

1. `REVIEW_DELIVER_TO` and `REVIEW_DELIVER_TERM` from the pane's environment. Only the TUI has these.
2. `target` in `meta.json`.
3. `herdr agent list`: the agent with the stored `terminal_id`, else agents whose `cwd` is the root or
   inside it, else a single agent in the workspace whose `cwd` is an ancestor of the root.
4. Several matches: the TUI shows a picker. The `send` action sends a notification that asks the user
   to pick in the review pane.
5. None: refuse.

After a Herdr restart every `terminal_id` is new (ADR 0006), so a stored target never matches and step 3
falls to `cwd`.

The chosen target is written to `meta.json`. The `send` action never has step 1, so it relies on what
the TUI saved.

## Steps

1. Collect unsent threads. With `--all-open` or the resend key, collect the chosen open threads too.
2. None: say "nothing to send".
3. Resolve the target. Call `deliver_to_agent(Delivery::Send, pane, text, run_herdr_output)`.
4. On refusal, show the reason in the TUI and as a notification. Nothing is marked.
5. On success, append one `sent` event with the ids and a batch id, then show `sent N to <agent>`, or
   `agent is working, N comments queued` when the status was `working`.
