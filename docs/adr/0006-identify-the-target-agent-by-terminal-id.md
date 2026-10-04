---
status: accepted
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
- Observed on 2026-10-04 on Herdr 0.9.1, with a shell already running in the pane and
  `herdr pane list` compared before and after each step:
  - Closing a sibling pane, including one with a lower number: no pane id changes, and the terminal ids
    are the same. Ids are counters, not positions. A closed number is not reused.
  - Moving the pane to another tab in the same workspace (`herdr pane move --new-tab`): same pane id,
    same terminal id.
  - Moving the pane to another workspace (`pane move --tab <t> --split right --target-pane <p>`): a new
    id (`w8D:p9` became `w8E:p2`) and the same terminal id. The shell that was already running still
    holds the old `HERDR_PANE_ID`. `herdr pane get w8D:p9` still returned the pane, under its new id, so
    an old id resolves while the server runs.
  - Restarting the server with session restore (an isolated server, `herdr server stop` then
    `herdr server`): every pane id came back the same, and every terminal id was new. The restored panes
    run new shells, so any agent that was in them is gone.
- `terminal_id` therefore identifies a pane across moves but not across a Herdr restart. After a restart
  the stored terminal id never matches, the check fails safe, and resolution falls to `cwd`, which is
  the right result because the user starts the agent again.
- The stale id seen during the design review (`HERDR_PANE_ID=w8D:p2` while the only pane was `w8D:p1`)
  was not reproduced. The one mechanism found is the cross-workspace move above, which leaves a running
  process with a stale id. The same stale value was still present in a tool shell on the day of the
  spike, and nothing in this work had moved that pane. The decision stands either way, because it never
  trusts the id alone.
