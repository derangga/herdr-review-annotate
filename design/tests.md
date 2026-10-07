# Tests that the graphs give

Part of the [herdr-review design](../DESIGN.md).

Each `E:` line above is one test, and each test swaps only the parameters listed under `R:`.

| Graph | Swapped | Tests |
|---|---|---|
| Store write | temp `dir`, fixed `now` | Lock held by another process gives `Busy` after 2 s. A killed holder frees the lock. `build` returning an error writes nothing |
| Archive | temp `dir`, fixed `now` | A held lock gives `Busy` and changes neither file. A bad line stays. Nothing to archive writes nothing. An archive that cannot be appended to and a rename that fails each leave the log as it was. Other processes appending beside it lose nothing and get no id twice |
| Agent CLI | fixture `git`, recording `herdr`, temp `dir` | Unknown id, a second `resolve` is a no-op, a line past the end of the file, an oversized body, a `herdr` failure does not fail the command |
| `open` | recording `herdr`, `Env` literal | No context JSON, review pane already open, stale review pane, no agent |
| Load diff | fixture `git` | Every row of the cases in `diff.md`, and a failed reload keeps the old diff |
| TUI loop | `TestBackend`, key list, temp `dir` | A write from another process appears, a failed save keeps the editor text, termination with a draft saves it |
| Filter the files | `TestBackend`, key list | Every `E:` line of the graph, each decision of `filter` in `tui.md`, and `matches` and `sidebar_rows` directly. The fixture is a diff of four files in two directories |
| Highlight | recording `git`, temp repo dir, a budget that counts lines | A side that cannot be read falls back to its hunks, so does one over 1 MiB, an unknown extension reads nothing, steps of 50 lines add up to the tokens of one whole pass, the loop reads no file while keys are waiting |
| Send | recording `herdr`, temp `dir` | Each `Refusal`, a stale pane id falls through to the list, `Ambiguous`, `sent` not recorded still reports success |

If a test needs a real `git`, a real Herdr or a real terminal to exercise one of these graphs, the
function has a hidden dependency and the design is wrong. The temporary-repository tests of the diff engine
and the manual checks in `message-review.md` are the only places that use the real tools, and they test the tools'
behaviour, not our graphs.
