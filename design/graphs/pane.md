# Call graphs: the review pane

Part of the [herdr-review design](../../DESIGN.md).

## TUI start

```
-> load keymap                             R: env (config dir)   E: any -> escape, defaults + warning
-> load theme                              R: env (config dir)   E: unknown name, bad table -> escape, mocha + warning
-> load sidebar config                     R: env (config dir)   E: open not a boolean -> escape, shown + warning
                                                                 E: icons not a boolean -> escape, icons shown + warning
                                                                 E: [sidebar] not a table -> escape, both defaults + one warning
                                                                 E: file missing, unreadable, not TOML -> escape, defaults, no warning here
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
  -> anything changed?                     (pure)     no event, same size, log not read, no send, no slice -> no frame
    -> draw                                R: term    with the tokens there are, a file not highlighted yet draws plain
  -> poll terminal                         R: term    E: Io -> leave the loop, restore terminal
                                                      0 ms while a file on screen is not highlighted to its end, else 250 ms
    -> no event, and a file on screen is not highlighted to its end?
      -> highlight one slice (below)       R: git, repo dir, budget
    -> key -> Keymap -> Action             (pure)     no binding -> ignored
    -> apply Action to the app state       R: per action, see the next graphs
  -> store length changed?                 R: dir     E: Io -> escape, warning, retry next tick
    -> read from offset, fold, reload diff
```

An idle tick builds no frame. The loop holds no lock between ticks. A panic hook restores the terminal
before the message prints.

## Highlight one slice

```
-> the files on screen not highlighted to their end, the cursor's file first   (pure)
-> per file, while the budget says more
  -> first step of this file? read it      R: git, repo dir   E: unknown language -> escape, no tokens, done
                                                              E: side unreadable, not UTF-8, over 1 MiB -> escape, hunk snippets
  -> parse one line, then ask the budget   R: budget          E: the engine cannot parse the line -> escape, no tokens for it
    -> the line reads as its row does? keep its tokens   (pure)
  -> no line left -> the file is done, its text is dropped
```

Cardinality: many steps per file. A step always parses one line, so the file gets further whatever the
budget says. The budget is a closure: the loop passes 8 ms of the clock, a test passes a counter. A key
waits for at most one slice and the read of one file.

If the pane is closed while the editor holds text, the draft is saved as a comment at the anchor that
was captured when `comment` was pressed. Losing typed text is the one data-loss path in the pane.

## Filter the files

```
filter action                              R: view, diff
-> no diff on screen?                      (pure)   E: start-up message screen -> ignored
-> pane under 50 columns?                  (pure)   E: too narrow -> notice, no filter
-> sidebar hidden? show it, rebuild        (pure)
-> focus the sidebar                       (pure)
-> filter = the applied query or empty, typing

key while typing                           R: view, diff
-> character with no ctrl or alt           (pure)   -> push, refilter
-> backspace                               (pure)   -> pop, refilter. On an empty query nothing happens
-> ctrl+u                                  (pure)   -> empty the query, refilter
-> enter                                   (pure)   E: empty query -> clear the filter, refilter
                                                    E: no match -> ignored, still typing
  -> typing = false
  -> cursor's file matches? else move to the first match
-> esc                                     (pure)   -> clear the filter, refilter, cursor untouched
-> any other key                           (pure)   -> ignored

esc with a filter applied                  R: view, diff
-> a visual selection is on?               (pure)   -> cancel it, keep the filter
-> clear the filter, refilter              (pure)

refilter                                   R: view, diff
-> sidebar_rows(diff, query)               (pure)
  -> per file in the diff's order: matches(query, path)?   (pure)
  -> heading when the directory differs from the last matching file's   (pure)

rebuild the view (reload, resize, store change)
-> Stream::build, as today
-> refilter, when a filter is set          (pure)   E: nothing matches any more -> `no match` line, filter stays

sidebar up, down, page keys                R: view
-> the file rows of Stream::side           (pure)
-> next after, or last before, the cursor's file   (pure)   none -> stay
```

Nothing here fails outside the state. Every `E:` line is a state the user can reach, and each one is an
escape: ignore the key or show a line. The typed characters pass through `sanitize_terminal_text` when
drawn, like every string that reaches the screen.

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
