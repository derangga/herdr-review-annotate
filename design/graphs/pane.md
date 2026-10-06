# Call graphs: the review pane

Part of the [herdr-review design](../../DESIGN.md).

## TUI start

```
-> load keymap                             R: env (config dir)   E: any -> escape, defaults + warning
-> load theme                              R: env (config dir)   E: unknown name, bad table -> escape, mocha + warning
-> load sidebar state                      R: env (config dir)   E: not a boolean, bad table -> escape, shown + warning
-> find root, load meta                    R: git, dir           E: NotARepo -> message screen
-> enter raw mode and alternate screen     R: term    scope: restored by a guard on exit, panic, SIGTERM, SIGHUP
-> store read, fold                        R: dir     E: Io -> message screen with the path
-> load diff (below)                       R: git     E: see below
-> save review_pane to meta                R: dir     E: Io -> escape, warning
-> resolve target for the status line      R: herdr   E: any -> escape, status line shows "no agent"
```

The pane never exits on a startup error. It shows the error and waits for `reload` or `quit`, so the
user can read what went wrong.

## Load the diff

```
-> git diff for the spec                   R: git     E: NotInstalled -> message screen
                                                      E: NoBase -> notice, fall back to WorkTree
                                                      E: Failed -> message screen with stderr
-> git ls-files --others                   R: git     E: Failed -> escape, notice "untracked files not shown"
-> parse patch into DiffFile               (pure)     E: unreadable file section -> Change::Unparsed
-> stat and read untracked files           R: fs      E: Io -> that file listed, not rendered
-> apply the 3 MiB cap                     (pure)
-> place every thread (diff.md)            (pure)     -> Placement per thread
```

Parsing and placement are pure functions of bytes and threads, so the whole diff engine is tested without
`git` installed.

Cardinality: one-shot, called again on each reload. A reload that fails keeps the previous diff on
screen and shows the error in the status line.

## TUI loop

This is the only stream in the program. It merges two sources.

```
-> loop
  -> termination flag set?                 R: signal flag   -> save draft (below), leave the loop
  -> poll terminal, 250 ms                 R: term    E: Io -> leave the loop, restore terminal
    -> key -> Keymap -> Action             (pure)     no binding -> ignored
    -> apply Action to the app state       R: per action, see the next graphs
  -> store length changed?                 R: dir     E: Io -> escape, warning, retry next tick
    -> read from offset, fold, reload diff
  -> anything changed?                     (pure)     no event, same size, log not read, no send -> skip both below
  -> highlight the files on screen         R: git, repo dir   E: unreadable, not UTF-8, over 1 MiB -> escape, hunk snippets
                                                              E: unknown language -> escape, plain rows
  -> draw                                  R: term
```

An idle tick builds no frame. The loop holds no lock between ticks. A panic hook restores the terminal
before the message prints.

If the pane is closed while the editor holds text, the draft is saved as a comment at the anchor that
was captured when `comment` was pressed. Losing typed text is the one data-loss path in the pane.

## User comment, reply, edit, delete, resolve

```
-> Action (comment, reply, edit, delete, resolve)
  -> editor returns text, or cancel        (pure state)
  -> store::write                          R: dir, now   E: Busy -> status line "review is busy, press again"
                                                         E: Io -> status line, the editor keeps the text
  -> fold the new events into the state
```

On a failed save the editor stays open with the text. The text is discarded only after the write
succeeds.
