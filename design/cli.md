# Command line

Part of the [herdr-review design](../DESIGN.md).

```
herdr-review tui    [--repo <root>]
herdr-review open   [--repo <root>] [--base <ref>]
herdr-review send   [--repo <root>] [--all-open] [--backend herdr|hc] [--target <t7>] [--thread <id>]... [--json]
herdr-review state  [--repo <root>] [--spec worktree|branch] [--base <ref>] --json
herdr-review file   [--repo <root>] [--spec worktree|branch] [--base <ref>] --path <p> --json
herdr-review archive [--repo <root>] [--json]
herdr-review view   [--repo <root>] [--spec ..] [--base ..] --path <p> --set | --unset
herdr-review comment apply   [--repo <root>] [--name <agent>] [--spec worktree|branch] [--base <ref>] --stdin
herdr-review comment apply   --as user [--repo <root>] [--spec ..] [--base ..] --file <f>
herdr-review comment list    [--repo <root>] [--status open|resolved] [--author user|agent] [--json]
herdr-review comment reply   [--repo <root>] [--name <agent>] <id> -
herdr-review comment resolve [--repo <root>] [--name <agent>] <id> --reply -
herdr-review comment reopen  [--repo <root>] <id>
herdr-review comment reply   --as user [--repo <root>] <id> --body-file <f>
herdr-review comment resolve --as user [--repo <root>] <id> [--body-file <f>]
herdr-review comment reopen  --as user [--repo <root>] <id>
herdr-review comment edit    [--repo <root>] <id> --body-file <f>
herdr-review comment delete  [--repo <root>] <id>
herdr-review comment seen    [--repo <root>] <id>
```

The first block is what agents and the TUI plugin use, and it works as it did in v0.5.0. The commands from
`state` down, and `--as user`, are the ones the herdr-app panel uses ([gui](gui.md)). Their graphs are in
`graphs/json.md` and `graphs/commands.md`.

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
- A command that is not in the binary, or a flag it does not know, prints `unknown command '<x>'`,
  `unknown option <x>` or `unknown comment command '<x>'` and exits 2. The panel reads that as a binary
  that is too old, so those three messages are fixed text and a test holds them.
- Without `--as user`, everything the `comment` subcommands write is authored by an agent. The user writes
  through the TUI, or through the commands below.
- `side` defaults to `new` when a `line` is given. `end_line` equal to `line` is a one-line comment. A
  file comment (`path` only) is not checked against the worktree, so a deleted file can be commented.
  An old-side line is read at `HEAD`, or at the merge base for the branch spec, under the path the
  agent gives. A renamed file therefore needs its old path.
- Agent name: `--name` wins. Otherwise the CLI runs `herdr agent list` and takes the `agent` field of
  the entry whose `pane_id` is `$HERDR_PANE_ID`, or else of the single entry whose `cwd` matches the
  current directory, and writes `agent:<name>`. Outside Herdr, or with no single match, it writes
  `agent`. A failed Herdr call never fails the command.
- A wrong id prints the open ids. Every failure prints one line on stderr.
- `comment apply` ends with `herdr notification show "N review comments from <name>"`. Under the hc backend
  there is no such command in hc, so it ends silently.
- `--spec` and `--base` on `comment apply` choose the spec the anchored lines are read under, and the spec
  recorded in each anchor. Without them the spec is the one in `meta.json`, as before. The panel passes its
  own, because switching the spec in the panel saves nothing.
- A batch entry takes an optional `old_path`. An old-side anchor on a renamed file reads the line from
  `git show <base>:<old_path>`, and the anchor keeps `path` as the new path and records `old_path`, so the
  thread is placed in the renamed file. Without it the old path is the one the agent gives in `path`, as
  before.

### Backend

`send` and the agent name talk to one of two terminal CLIs. The backend is `--backend`, else hc when
`--target` is given or hc is the nearest host, else herdr. hc is the nearest host when `HC_TERMINAL_ID` is set
and `HERDR_PANE_ID` is not: a Herdr pane started inside an hc terminal inherits `HC_TERMINAL_ID`, while an hc
terminal carries no `HERDR_*` variable, so `HERDR_PANE_ID` means Herdr owns the shell. `--target` takes an hc address (`t7`) and implies hc. The
panel runs outside any terminal, so it passes `--target` when it knows the terminal and `--backend hc` when it
does not. The herdr backend is the one the TUI plugin uses in plain Herdr, and it is unchanged.

- `--target` with `--backend herdr` is a usage error, and so is `--thread` with `--all-open`. `--thread` may
  repeat.
- `comment` has no `--backend`. It is under hc when hc is the nearest host, and then it neither lists agents
  through herdr nor notifies.
- Agent name under hc: when hc is the nearest host, `hc get $HC_TERMINAL_ID --json` and its `status.app`
  (`claude-code`). A failed call, or no `status`, gives plain `agent`. `--name` still wins.
- `<bin>` in the prompt under hc is `std::env::current_exe()`, so the agent replies with the binary that sent
  the prompt, and not the older one on `PATH` ([send](send.md)).

### The panel's commands

- `state` and `file` print JSON ([shapes](shapes.md)) and write nothing: no event, no `meta.json`, no lock,
  and `.git/index` is as it was. `--json` is required, since there is no other form.
- `--repo` may be any folder inside the work tree. The reply names the root.
- `--spec branch` without `--base` uses the base in `meta.json`, else the default base ([diff](diff.md)).
  A base that does not resolve falls back to the working tree with a notice. `--base` without `--spec` is the
  branch spec, as in `open`. With neither flag the spec is the one in `meta.json`, else the working tree.
- `--file <f>` (on `apply`) and `--body-file <f>`: the CLI opens the file, reads it, deletes it, and then
  uses the text. The file must be a regular file and not a symlink. A body is never an argument. A body file
  holds at most 16 KiB. A batch file holds at most 256 KiB, so that a body of 16 KiB fits with its anchor and
  the JSON escapes, and a body that is too long is refused by the batch check with its index, as on stdin.
  The file is deleted as soon as it is open, so a body that is refused (too long, not UTF-8, empty) is gone
  too. A file refused before it is opened (missing, a symlink, a directory) is left alone. A delete that
  fails is ignored. Both flags are accepted with `--as user` and on `edit`, and refused with an agent's
  `reply` and `apply`, which keep stdin.
- `--as user` may also read stdin: `apply --as user --stdin`, `reply --as user <id> -` and `resolve --as user
  <id> --reply -` work as the agent's forms do, with the author changed. A file and stdin on one command is a
  usage error. `--name` with `--as user` is a usage error, since the user has no agent name.
- `--as user` writes `by: user`, never calls `herdr` or `hc`, and never sends a notification. Only `user` is
  accepted. `resolve --as user` takes an optional body, which becomes a reply.
- `comment edit` changes the text of a comment the user wrote, and `comment delete` removes one (a root
  takes its thread). Both exit 2 on an agent's comment. `comment seen` marks a resolved thread seen, and
  writes nothing for a thread that is not new.
- `archive` moves the resolved threads that are not new into `archive.jsonl` ([data](data.md)). With
  `--json` it prints `{ schema, archived, unsent }`. It exits 0 with zeros when there is nothing to move.
  Without `--json` it prints `nothing to archive` or `archived N threads`, with `, M never sent` when M is
  above 0.
- `view --set` stores the fingerprint of the file as it is in the diff of the spec, and `--unset` removes
  the mark. `--set` for a file that is not in the diff exits 2. It touches that one entry of `viewed` and
  nothing else in `meta.json`, and it writes nothing when the mark is already as asked. It prints nothing.
- `--spec` and `--base` on `comment apply` and on `view` are chosen as `state` chooses them: the flags win,
  then `meta.json`, then the working tree, and `--base` alone is the branch spec. A branch spec with no base
  that resolves reads the working tree, and `apply` prints the notice on stderr.
- `send --thread <id>` sends the named open threads, whether or not they were sent, and nothing else. A
  named id that is not an open thread exits 2 and lists the open ids. Without `--thread`, `send` sends the
  unsent threads (open ones), as before. `--all-open` stays for the TUI.
- `send --json` prints one JSON object on stdout in every outcome, and exits 0 when a send happened or there
  was nothing to send, and 1 when it failed: `ambiguous` (with the candidates), `no_agent`, `blocked`,
  `not_ready`, `agent_gone`, `busy`, `backend`. A usage error still exits 2 with a line on stderr.

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
