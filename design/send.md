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

`<bin>` is `$HERDR_PLUGIN_ROOT/bin/herdr-review` under the herdr backend, and `std::env::current_exe()` under
the hc backend. Both paths are single-quoted with `'` escaped. Under herdr, `send` and the TUI both run as
Herdr plugin commands, so the variable is always set. herdr-app does not set it, and the `herdr-review` on
`PATH` may be an older release without the commands the panel needs. The agent therefore replies with the
binary that sent the prompt. The copy that `stage-app-mod.sh` puts in the mod folder has a stable path
([gui](gui.md)).

## Backends

Sending is three operations on a terminal CLI, and the code behind `send` sees nothing else:

| Operation | herdr backend | hc backend |
|---|---|---|
| `get` one terminal, parsed into a record and a readiness | `herdr agent get <pane>` | `hc get <t> --json` |
| `list` the agents, parsed into candidates | `herdr agent list` | `hc ls --json` |
| `prompt` one terminal with text | `herdr agent prompt <pane> <text>` | `hc send <t> <text>` |

The backend is chosen once, at the edge (`cli.md`, Backend). Each operation returns parsed shapes, so target
resolution and the refusal rules never read raw JSON, and the diff, store and TUI code do not change. A test
passes a recording fake with the same three operations.

### hc readiness and refusals

From `hc get <t> --json` ([herdr-app-facts](herdr-app-facts.md), section 4):

| `hc get` shows | Result |
|---|---|
| `state` is `finished`, or exit 3 (no such terminal) | `Refusal::AgentGone` |
| `status` is null, or `reported` is false | `Refusal::NotReady` |
| `status.state` is `blocked` | `Refusal::Blocked` |
| `status.state` is `idle`, `working`, `done`, or any other word | ready |

`hc send <t> <text>` is called without `--if`, so a working agent takes the text as a mid-turn message, which
is the "queued" case. The result is `Queued` when the reply says `"status":"working"` and `Sent` otherwise.
If `hc send` itself refuses, because the terminal changed after the `get`: exit 9 is `Blocked`, exit 11 and
exit 3 are `AgentGone`, any other exit is an `HcError` with its message, which the user reads as "hc send
failed: ...". The text ends with "Nothing was sent; your comments are still unsent." as under herdr. Nothing is typed in a refusal, and
nothing is marked sent. `ESC` is removed from the text before it is sent, as under herdr. Whether `hc send`
wraps the text in a bracketed paste is not known, and removing the byte is safe either way.

## Target resolution (ADR 0006)

Under the herdr backend, a candidate is valid when `herdr agent get <pane>` passes `agent_ready`, returns the
stored `terminal_id`, and has a `cwd` equal to the root, inside it, or an ancestor of it.

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

Under the hc backend:

1. `--target <t>`, an address the caller chose. When given, it is the only candidate. Its `cwd` and its
   state are not checked, because the caller picked it, from the panel's own pane list or from the picker. A
   finished address, or one `hc get` does not find (exit 3), is refused as `AgentGone` and nothing is sent:
   the prompt never goes to an agent the caller did not pick. Any other `hc get` failure is
   `TargetError::Backend`.
2. `hc_target` in `meta.json`. It is valid when `hc get` finds it live, it has a `status` (it is an agent that
   has reported, which keeps a shell that reused the address from being taken for the agent), and its `cwd` is
   the root, inside it, or an ancestor of it. A blocked agent is still valid, so the send can say why it
   refuses.
3. `hc ls --json`: the agents whose `cwd` is the root or inside it, else the agents whose `cwd` is an ancestor
   of the root. "Agent" is an entry whose `agent` is not a plain program, that is, it has a `status`. The
   stored address is not looked for here: it would be the entry step 2 has just rejected, and hc has no second
   id, such as herdr's terminal id, that could match under a new address. One match is taken and its
   `hc get` is read for the readiness check below.
4. Several matches: `send` returns them as `ambiguous` with the candidates, and the panel shows a picker and
   sends again with `--target`. With no `--json` it exits 1 and names them on stderr.
5. None: `no_agent`.

The readiness table above is applied to the `hc get` of whatever resolution chose, after resolution and not
as part of it.

The address is assumed not to survive a restart of herdr-core. `hc --skill` says programs keep running
across `hc server restart` and does not say the address stays ([herdr-app-facts](herdr-app-facts.md),
section 4), and the user decided to assume it does not. A stored address that no longer answers, or answers
for another folder, fails step 2 and resolution falls to `cwd` in step 3, as ADR 0006 does for herdr. If the
address does survive, step 2 simply keeps working.

The chosen target is written to `meta.json` as `hc_target`.

## Steps

1. Collect unsent threads. With `--all-open` or the resend key, collect the chosen open threads too. With
   `--thread <id>`, collect exactly the named open threads, sent or not, and no others.
2. None: say "nothing to send".
3. Resolve the target. Call `deliver_to_agent(Delivery::Send, pane, text, run_herdr_output)`, or the hc
   equivalent from the backend table.
4. On refusal, show the reason in the TUI and as a notification. Nothing is marked.
5. On success, append one `sent` event with the ids and a batch id, then show `sent N to <agent>`, or
   `agent is working, N comments queued` when the status was `working`.

## The reply of `send --json`

With `--json`, `send` prints one JSON object in every outcome ([shapes](shapes.md)). The refusals and the
target errors have a `kind` each (`blocked`, `not_ready`, `agent_gone`, `no_agent`, `ambiguous`, `busy`,
`backend`), so the panel decides from the object and does not parse a sentence. `ambiguous` carries the
candidates with address, agent, folder and status, which is what the picker shows. The message text is
for people, and the panel shows it as it is.

