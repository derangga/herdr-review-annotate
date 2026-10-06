# Call graphs: the store

Part of the [herdr-review design](../../DESIGN.md).

## Write to the store (every mutation goes through this)

```
-> store::write(dir, now, build)
  -> open lock file                        R: dir     E: Io -> propagate
  -> try_lock, up to 2 s                              E: held -> retry every 2 ms, then Busy
  -> read review.jsonl, fold               R: dir     E: Io -> propagate
                                                      E: bad line -> escape, Warning::SkippedLine
  -> build(&Review) -> Vec<Event>                     E: CommandError -> propagate, nothing written
       allocates ids from the folded state
  -> one write_all on O_APPEND                        E: Io -> propagate
  -> drop guard, lock released             scope: released on every path, also on panic or kill
```

`build` is where an operation validates against the current state. Validation and append happen under
one lock, so a `resolve` cannot race with a `delete` of the same thread.

Cardinality: one-shot.

## Archive (the one rewrite of the log)

```
-> store::archive(dir, now)
  -> open lock file                        R: dir     E: Io -> propagate
  -> try_lock, up to 2 s                              E: held -> retry every 2 ms, then Busy, neither file changes
  -> read review.jsonl line by line, fold  R: dir     E: Io -> propagate
                                                      E: bad line -> escape, the line stays in the log as it was
  -> take the threads resolved and not new (pure)     none -> Archived { 0, 0 }, nothing written
  -> split the lines into moved and kept   (pure)     a sent event of both kinds is split in two
  -> append moved to archive.jsonl, sync   R: dir     E: Io -> propagate, review.jsonl as it was
  -> write kept to a temp file, sync       R: dir     E: Io -> propagate, review.jsonl as it was, thread in both files
  -> rename it over review.jsonl                      E: Io -> remove the temp file, propagate, review.jsonl as it was
  -> drop guard, lock released             scope: released on every path
```

The pane's side:

```
-> Action::Archive
  -> count the threads to take             (pure)     none -> notice "nothing to archive", no prompt
  -> Prompt::Archive                                  esc -> nothing written
  -> store::archive                        R: dir, now   E: Busy -> failure "review is busy, press again"
                                                         E: Io -> failure with the file's name
  -> read the log again, lay the stream out
```

Decisions that fall out of the graph:

- The store counts under the lock, so it archives what is resolved then and not what the prompt counted.
  A thread an agent resolved in between is `new` and stays. A comment an agent adds in between is in the
  log the archive reads, so it is neither lost nor moved unless its thread is one the archive takes.
- Every failure leaves `review.jsonl` as it was. Pressing the key again is always safe.

Cardinality: one-shot.
