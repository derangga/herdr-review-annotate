# Call graphs: the agent CLI, the user CLI and open

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

## User CLI: `--as user`, `edit`, `delete`, `seen`, `archive`, `view`

These are what the panel runs ([gui](../gui.md)). The TUI writes through `actions.rs` and does not use them.
The agent graph above is unchanged, and an agent that does not pass `--as user` is still an agent.

```
-> parse args                              R: env     E: usage -> exit 2
                                                      `--as` takes only `user`, and no `--name` with it
                                                      with `--as user` the text may also come on stdin
                                                      `--file` and `--body-file` come with `--as user`, or with
                                                      `edit`, which is user-only. Not with an agent's reply
                                                      a `-` and a body file on one command
-> resolve root                            R: git     E: NotARepo -> exit 1
-> read the body file, if given            R: fs
     opened without following a symlink    E: missing, a symlink, not a regular file -> InvalidBody, exit 2
     read at most the limit and one byte   E: longer -> InvalidBody, exit 2. The limit is 16 KiB for a body
                                           file and 256 KiB for a batch file (cli.md)
  -> delete the file                       R: fs      runs on every path once the file was opened
                                                      E: the delete fails -> escape, ignored
-> author is User                          (pure)     no herdr or hc call, no agent name lookup
-> for `apply`: decode the batch, read the anchored lines (as above), with the spec from --spec and --base
                                                      E: InvalidBatch -> exit 2, nothing written
-> store::write                            R: dir, now
     apply, reply, resolve, reopen         as the agent commands, with `by: user`
     edit    -> actions::edit              E: NotAllowed (an agent's comment), UnknownId -> exit 2
     delete  -> actions::delete            E: NotAllowed, UnknownId -> exit 2
     seen    -> actions::seen              E: UnknownId -> exit 2. A thread that is not new writes nothing
                                           E: Busy, Io -> exit 1
-> no notification, never                              `--as user` does not call herdr or hc
-> print ids as text
```

```
archive
-> resolve root                            R: git     E: NotARepo -> exit 1
-> store::archive                          R: dir, now   E: Busy, Io -> exit 1, as in store.md
-> print { schema, archived, unsent } with --json   (unsent: taken threads with a never-sent user comment), else one line
                                                      nothing to archive -> exit 0, archived 0

view --path P --set | --unset
-> parse args                                         E: neither or both of --set and --unset, no --path -> exit 2
-> resolve root                            R: git
-> --unset: meta::save, remove the path    R: dir     E: Busy, Io -> exit 1. A path that is not marked is a no-op
-> --set: load the diff for the spec       R: git     E: Failed -> exit 1
     -> find the path in the diff          (pure)     E: not in the diff -> exit 2
     -> fingerprint the file               (pure)
     -> meta::save, insert path and fingerprint   R: dir   E: Busy, Io -> exit 1
```

Rules that fall out of the graphs:

- The body file is the only way the panel hands a body over. A body is never on argv, so it never shows in
  `ps`, and a shell never sees it. The CLI deletes the file once it has opened it, so a body lives on disk for
  the time of one call. A file it refused to open is left alone, because a symlink or a directory is not the
  panel's file.
- `edit`, `delete` and `seen` call the functions the TUI calls, so the rights rule has one copy
  ([ADR 0004](../../docs/adr/0004-either-side-resolves-nobody-edits-the-other.md)). The CLI cannot tell a person from an agent, as a write to
  `review.jsonl` by hand cannot. `--as user` is a label, and it protects against a mistake, not against a
  process that means harm. The agent's prompt names none of these commands.
- `view --set` touches one entry. A mark for a file that left the diff stays in `meta.json` until the TUI
  writes its whole map, and it is harmless, because a viewed flag needs the fingerprints to match.

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
