# Call graphs: the agent CLI and open

Part of the [herdr-review design](../../DESIGN.md).

## Agent CLI: `comment apply`, `reply`, `resolve`, `reopen`

```
-> parse args                              R: env     E: usage -> exit 2
-> resolve root                            R: git     E: NotARepo -> exit 1
-> read stdin, decode batch or text                   E: InvalidBatch -> exit 2, nothing written
-> for each new root comment: read the anchored line
  -> new side: read the file in the worktree  R: dir of repo  E: missing line -> InvalidBatch
  -> old side: git show <base>:<path>         R: git          E: missing line -> InvalidBatch
-> detect agent name                       R: herdr, env   E: any -> escape, Author::Agent(None)
-> store::write                            R: dir, now     E: UnknownId -> exit 2
                                                           E: Busy, Io -> exit 1
-> notification show (apply only)          R: herdr        E: any -> escape, ignored
-> print ids as text or JSON
```

Rules that fall out of the graph:

- `resolve` on a resolved thread and `reopen` on an open thread succeed and write nothing. An agent
  that repeats a command does not see an error.
- The anchored line text is read by the CLI, not supplied by the agent. An agent cannot anchor a
  comment to text that is not there.
- Nothing after `store::write` can turn a success into a failure.

## `open`

```
-> find root
  -> from HERDR_PLUGIN_CONTEXT_JSON        R: env     E: absent or unparseable -> escape, use cwd
  -> git rev-parse --show-toplevel         R: git     E: NotARepo -> notify, exit 1
-> load meta                               R: dir     E: unreadable -> escape, empty Meta
-> review pane already open?
  -> herdr pane get <review_pane>          R: herdr   E: pane_not_found -> continue to open
  -> herdr plugin pane focus               R: herdr   E: any -> continue to open
-> resolve agent pane (send.md)            R: herdr, env   E: NoAgent -> open without a target
                                                           E: Ambiguous -> open without a target
-> herdr plugin pane open                  R: herdr   E: HerdrError -> notify, exit 1
```

A review with no target still opens. The user can read and comment, and the picker runs at the first
send. An action has no terminal, so every failure of `open` is shown with `herdr notification show`.
