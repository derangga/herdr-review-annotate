# Command line

Part of the [herdr-review design](../DESIGN.md).

```
herdr-review tui    [--repo <root>]
herdr-review open   [--repo <root>] [--base <ref>]
herdr-review send   [--repo <root>] [--all-open]
herdr-review comment apply   [--repo <root>] [--name <agent>] --stdin
herdr-review comment list    [--repo <root>] [--status open|resolved] [--author user|agent] [--json]
herdr-review comment reply   [--repo <root>] [--name <agent>] <id> -
herdr-review comment resolve [--repo <root>] [--name <agent>] <id> --reply -
herdr-review comment reopen  [--repo <root>] <id>
```

- `--repo` defaults to `git rev-parse --show-toplevel` of the current directory. Sent prompts always
  include it, because an agent's `cwd` can be a parent of the repository.
- `-` reads the text from stdin. There is no form that takes the text as an argument, so shell
  substitution in agent text cannot run.
- `comment apply` reads `{"comments":[{"path","side","line","end_line","body"} | {"reply_to","body"}]}`
  and validates the whole batch before writing. Limits are in `boundary.md`.
- The CLI reads the anchored line's text itself: from the worktree file for the new side, from
  `git show <base>:<path>` for the old side. A line that does not exist rejects the batch.
- `resolve` on a resolved thread and `reopen` on an open thread exit 0 and write nothing.
- Exit codes: 0 done, 1 failed and worth retrying later (busy, disk, git), 2 the request is wrong
  (usage, unknown id, not allowed, invalid batch).
- Everything the `comment` subcommands write is authored by an agent. The user writes through the TUI.
- `side` defaults to `new` when a `line` is given. `end_line` equal to `line` is a one-line comment. A
  file comment (`path` only) is not checked against the worktree, so a deleted file can be commented.
  An old-side line is read at `HEAD`, or at the merge base for the branch spec, under the path the
  agent gives. A renamed file therefore needs its old path.
- Agent name: `--name` wins. Otherwise the CLI runs `herdr agent list` and takes the `agent` field of
  the entry whose `pane_id` is `$HERDR_PANE_ID`, or else of the single entry whose `cwd` matches the
  current directory, and writes `agent:<name>`. Outside Herdr, or with no single match, it writes
  `agent`. A failed Herdr call never fails the command.
- A wrong id prints the open ids. Every failure prints one line on stderr.
- `comment apply` ends with `herdr notification show "N review comments from <name>"`.

`open`:

1. Find the root. From an action, use `focused_pane_cwd` in `HERDR_PLUGIN_CONTEXT_JSON`. From an
   agent's shell, use the current directory.
2. Find the agent pane: the focused pane when it hosts an agent, else `$HERDR_PANE_ID` when
   `herdr agent get` accepts it, else resolution as in `send.md`, target resolution.
3. If `meta.json` has a `review_pane_id` that still exists, run `herdr plugin pane focus <id>` and stop.
4. Otherwise run `herdr plugin pane open --plugin review --entrypoint tui --placement split
   --direction right --target-pane <agent pane> --cwd <root> --env REVIEW_DELIVER_TO=<pane id>
   --env REVIEW_DELIVER_TERM=<terminal id> --focus`.

The TUI writes its own `HERDR_PANE_ID` to `meta.json` as `review_pane_id` when it starts.
