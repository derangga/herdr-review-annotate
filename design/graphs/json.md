# Call graphs: `state` and `file`

Part of the [herdr-review design](../../DESIGN.md). The JSON they print is in [shapes](../shapes.md), the
commands in [cli](../cli.md). The mod is the caller ([gui](../gui.md)), and the TUI does not use these.

Both commands read the diff and the review and print one JSON object. They write nothing, and that is the
property their tests hold hardest, because the mod watches the repo root and the store directory. A write
by `state` would wake the mod, which would call `state` again.

## `state`

```
-> parse args                              R: env     E: usage -> exit 2, one line on stderr
                                                      a v0.5.0 binary gives this for the whole subcommand
-> resolve root                            R: git     E: NotARepo, NotInstalled -> exit 1
                                                      --repo may be any folder inside the work tree
-> locate the state directory              R: dir     the directory is not created
                                                      E: no HOME and no XDG_STATE_HOME -> exit 1
-> load meta, never save                   R: dir     E: unreadable -> escape, empty Meta, warning in the output
-> choose the spec                         (pure)     --spec and --base win, else meta, else WorkTree
                                                      --base alone means --spec branch. --spec worktree
                                                      reads the working tree and still reports a base
                                                      E: --spec branch and no base resolves (NoBase)
                                                         -> escape, WorkTree, notice in the output
                                                            ("no base branch found, tried ..., showing the
                                                            working tree against HEAD") and `base` null
-> load the diff (pane.md)                 R: git     E: Failed -> exit 1 with stderr
                                                      E: ls-files fails -> escape, notice, as in the TUI
-> read review.jsonl, fold                 R: dir     E: file or directory missing -> escape, empty review
                                                      E: a bad line -> escape, warning "N unreadable events"
                                                      E: a last line with no newline -> ignored (a writer is
                                                         mid-append)
-> place every thread (diff.md)            (pure)
-> per file: counts and viewed flag        (pure)
     add, del from the hunks; viewed = the fingerprint equals meta.viewed;
     threads, open, new, unsent = the threads placed Matched or Outdated in this file (threads counts
     them all, resolved ones too; unsent counts open ones only, so a resolved
     thread the user never sent is not counted)
     hash = FNV-1a of the rows (DiffFile::content_hash), then the JSON of this file's threads in review order
                                                      E: an edit that keeps the counts changes the file's
                                                         hash, and another file's stays
-> threads placed NotInDiff -> orphans     (pure)
-> archivable = the threads of the review with `Thread::archivable` (pure)
                                                      every thread, orphans too, the filter `store::archive`
                                                      uses: resolved and not new
-> unread_resolved = the threads of the review with `Thread::unread_resolved` (pure)
                                                      resolved and not archivable: an agent resolved it and no
                                                      `seen` follows. Defined as the negation of the predicate
                                                      above, so archive and this count cannot disagree
-> print the JSON with schema              R: stdout
```

No lock is taken. The reads of `review.jsonl` and `meta.json` see either the old or the new file, because
`archive` and `meta::save` replace them by rename. An append that is half written is a last line with no
newline, which the fold ignores.

Every `git` call runs with `GIT_OPTIONAL_LOCKS=0`, `-c core.fsmonitor=false` and `-c
diff.autoRefreshIndex=false` ([diff](../diff.md)), so `git diff` leaves `.git/index` untouched.

Cardinality: one-shot, called once per load of the panel.

## `file`

```
-> parse args                              R: env     E: usage, a missing --path -> exit 2
-> resolve root, load meta, choose the spec, load the diff, read the review, place the threads
                                           as in `state`, with the same failures
-> find the path in the diff               (pure)     E: not in the diff -> exit 2, "not in diff: <path>"
                                                         (a path that is not a repo-relative path is not
                                                         in the diff either)
                                                      the path is matched against `path`, and against
                                                      `old_path` for a renamed file only when no `path` fits
-> rows of the file                        (pure)     hunk header, ctx, add and del rows, in order, with
                                                      old and new line numbers, text without the marker
                                                      E: change binary, submodule, too_large, unparsed
                                                         -> rows empty, `note` says why
                                                      an untracked file is read, and its rows are all add
-> highlight the rows                      R: dir, git   the pane's `syntax::Cache`: the new side from the work tree,
                                                      the old side from `git show <rev>:<path>`, each side over
                                                      1 MiB by hunks. Stops after 4,000 parsed lines
                                                      E: a file with no `syntax` feature or no known language,
                                                         an unreadable side, a file not UTF-8
                                                         -> escape, rows with no tokens, no error
                                                      E: a line that no longer reads as its row (the file
                                                         changed since the diff) -> that row has no tokens
                                                      a file with a note is not read
                                                      `tokens` are kinds with UTF-16 offsets, in `rows`
-> mark the changed words                 (pure)      `words::changed` on each removed row and the added row
                                                      paired with it, as the TUI pairs them, in `rows`
                                                      E: a pair `changed` gives no marks (rewritten, over
                                                         200 tokens), a row with no partner, a context or
                                                         hunk row -> `marks: []`, no error
                                                      `marks` are `{from, to}` with UTF-16 offsets
-> threads of this path, with row indexes  (pure)     Matched and Outdated only. `row` and `start_row`
                                                      index into `rows`
-> print the JSON with schema              R: stdout
```

`file` repeats the work of `state` up to the diff, so a load costs two diffs. A review is small and `git
diff` is fast, and a single command that returned every file's rows would grow with the diff and not with the
screen. If a load becomes slow the answer is a cache keyed by `rev`, in a later change.

Cardinality: one-shot, called once per load for the selected file.

## Tests

Each `E:` line is one test. The rest are in [tests](../tests.md):

- Read-only: run `state` and `file` over a repo with a review, a `meta.json`, an unstaged change and an
  untracked file. Compare `review.jsonl`, `meta.json`, the work tree files and `.git/index` byte for byte
  before and after. Run it again with no store directory and check that none was created.
- Tokens: a row with a non-ASCII character (an e acute and an emoji before the coloured parts) has offsets in UTF-16
  units, and the mod's decoder test reads the same row. A row with no tokens has no `tokens` key. A span that is not on
  characters is left out. A block comment that opens above the hunk colours its context row. A file longer than
  the line budget has plain rows past it. A build without the `syntax` feature prints no `tokens` at all.
  The golden `file.json` has tokens and is compared only with the feature on, and the other build has its own test.
  On the mod side: a missing or odd `tokens` is none, and each kind of bad part is dropped.
- Marks: a pair of a removed and an added row that differ in a word marks that word on both, a rewritten pair and a
  row with no partner have none, and a row with a non-ASCII character (an e acute and an emoji before the word) has
  offsets in UTF-16 units. Every row has a `marks` key, `[]` when there are none. The golden `file.json` has a pair
  with marks. On the mod side: a missing or odd `marks` is none, each kind of bad mark is dropped, and the runs of a
  row are cut at the marks.
- Archivable: over a review with an open thread, a thread the user resolved, an agent's resolve nobody has
  seen, an agent's resolve the user has seen, and a resolved orphan, `state` says 3. A review with nothing says 0.
  `unread_resolved` says 1 for that review (the agent's resolve nobody has seen), 0 for a review with nothing, and
  it still says 1 after `archive` moved the rest.
  After `archive` moves them, `state` says 0, and the count before an archive is what `archive --json` reports.
  On the mod side: a missing or non-integer `archivable` or `unread_resolved` is a `ContractMismatch`.
- A reply that decodes into the shapes of `shapes.md`, with `schema` 1.
- Golden files: the Rust test compares what `state` and `file` print for a fixed repo, after the root and the
  store directory are replaced by fixed text (the timestamps are those of the events the test writes), with the
  checked-in `herdr-app-mod/test/fixtures/state.json` and `file.json`, and the mod's decoder tests read the
  same two files. A change to a field fails a test on both sides. The fixtures are written in the order of
  `shapes.md`, and `REVIEW_UPDATE_FIXTURES=1 cargo test` rewrites them, so the change that changes a field
  carries the new files in the same commit.
