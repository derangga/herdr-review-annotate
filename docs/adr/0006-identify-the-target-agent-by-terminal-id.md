---
status: proposed
---

# Identify the target agent by terminal id, not by pane id alone

`send` must deliver to the agent the review belongs to. The obvious handle is the public pane id
(`w1:p2`), which is what `herdr-annotate` and plannotator-tui pass around. Public pane ids change: a
cross-workspace move assigns a new one (Herdr socket API docs), and during the review of this design a
live session held `HERDR_PANE_ID=w8D:p2` while the only pane was `w8D:p1`. A stale id either fails the
send or addresses a different pane, which delivers the user's comments to the wrong agent.

We store the pane id together with its `terminal_id`. Before every send, `herdr agent get <pane>` must
return the same `terminal_id` and a `cwd` that belongs to the review root. If not, we look the agent up
in `herdr agent list` by `terminal_id`, then by `cwd`, and ask the user when more than one matches.

## Consequences

- `herdr agent get` does not accept a `terminal_id` as its target (tested on 0.9.1), so the lookup
  needs `agent list`.
- An agent-initiated `open` cannot trust `$HERDR_PANE_ID` either. It resolves its own pane through
  `agent list` by `cwd` when the id does not exist.
- The cause of the stale id seen during review is unknown. Milestone 1 reproduces it before this
  decision is accepted.
