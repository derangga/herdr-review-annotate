# herdr-review

Review a diff in a [Herdr](https://github.com/herdrdev/herdr) pane, comment on lines, and send the
comments to your coding agent. The agent replies and resolves threads from its own shell, and the pane
picks the changes up live. The agent can also start a review of its own changes. A second pane does
the same for the agent's last message, with no diff and no repository.

Needs Herdr 0.9.1 or later, on macOS or Linux, and `git`. Claude Code is the agent v1 supports.

## Install

```sh
herdr plugin install derangga/herdr-review-annotate
```

Herdr runs `scripts/fetch-herdr-review.sh`, which downloads the prebuilt `herdr-review` binary for your
platform from the GitHub release and refuses it when its SHA-256 does not match the `.sha256` file next
to it.

To work on the plugin, link a checkout instead. `plugin link` skips the fetch script, so build and stage
the binary yourself:

```sh
bash scripts/stage-local.sh
herdr plugin link .
```

If your plugins come from the Nix store, the Nix package has to build the binary itself and put it at
`bin/herdr-review`. The fetch script does not run there.

## Bind the keys

Add two entries to your Herdr `config.toml`. `prefix+r` and `prefix+shift+r` are Herdr's own
(resize mode and reload config), so these use `i`. Pick any other key you like.

```toml
[[keys.command]]
key = "prefix+i"
type = "plugin_action"
command = "review.open"
description = "review the diff"

[[keys.command]]
key = "prefix+shift+i"
type = "plugin_action"
command = "review.send"
description = "send review comments to the agent"
```

To review the agent's last message too, add a third entry. `prefix+o` is the key herdr-annotate uses.
Herdr runs one action per key, so remove that entry, or pick another key.

```toml
[[keys.command]]
key = "prefix+o"
type = "plugin_action"
command = "review.message"
description = "review the agent's last message"
```

Run `herdr config check`, then reload the config. `check` does not verify that the action exists, so a
typo in `review.open` shows up only when you press the key.

## Run a review

1. Focus the pane where your agent runs and press `prefix+i`. The review opens in a split to its right.
   Pressing it again focuses the open review pane.
2. Move to a line with `j` and `k`, press `c`, write the comment, and save it. `v` starts visual mode first:
   move with `j` and `k`, or drag the mouse over the lines, to grow the range, then press `c`. `Esc` or
   `v` leaves visual mode. `c` on a file header comments on the whole file.
3. Press `S` to send every unsent comment to the agent as one prompt. `s` sends the thread under the
   cursor again. You can also press `prefix+shift+i` from the agent's pane.
4. The agent replies or resolves each thread with `herdr-review comment ...`. A thread it resolved shows
   a `new` marker until you look at it. Press `x` to reopen it, or `r` to reply.
5. `A` archives the resolved threads to `archive.jsonl` after a yes or no.

`b` switches between the working tree against `HEAD` and the working tree against the merge base with
your base branch. `herdr-review open --base <ref>` picks the base. Without it, the base is `origin/HEAD`,
else `main`, else `master`.

Review state lives outside the repository, keyed by the worktree root, so nothing lands in `git status`.

## Review the agent's last message

Use this when the agent just explained something and you want to answer it line by line. It reads the
agent's Claude Code transcript, so it works in any folder, and it supports Claude Code only. In a pane
that runs another agent, or none, the key shows a notification and opens nothing.

1. Focus the pane where Claude Code runs and press `prefix+o`. A pane opens to its right with the
   agent's newest message as Markdown source, one numbered row per line, wrapped to the width. Pressing
   the key again focuses it. Text the agent wrote between tool calls belongs to earlier messages and is
   not shown. If the agent is still working, the status line says `working` and the pane shows the
   newest message so far. After `/clear` there is no message yet, and the notification says so.
2. Move with `j` and `k`. A wrapped line is one stop. Press `c`, write the comment, and save it with
   `ctrl+s`. For a range, press `v`, move or drag the mouse over the lines, then press `c`. The comment
   is a box under the last line it covers. `e` edits and `d` deletes the comment under the cursor, and
   `n` and `N` jump between comments.
3. Press `S`. Every comment goes to the agent as one prompt, and the pane closes. The agent answers in
   its next message, and `prefix+o` opens that one if you want to go on. If the agent is waiting on a
   permission prompt, the send is refused, the pane stays, and the comments are kept.

The prompt quotes each comment's lines, at most six, so the agent can find them:

```
Feedback on your last message. Address each point.

- lines 12-14:
  > ## Step 2: migrate the store
  > Move events into the new file and drop the old one.
  > The fold reads only the new file.
  Why not keep the old file and read both?
- line 30:
  > Use a global lock.
  Fine for now, add a ponytail note.
```

`R` loads the newest message again, and `q` leaves. Both ask first when there are comments. The comments
live only in the pane: nothing is written to disk, and closing the pane without sending loses them.

The pane reads the same `[keys]` table as the diff pane and acts on `up`, `down`, `page_up`,
`page_down`, `prev_thread`, `next_thread`, `comment`, `select_range`, `edit`, `delete`, `send`,
`reload`, `help` and `quit`. The other actions do nothing there, and `?` lists only these.

To find the transcript the pane takes the session id from `herdr agent get`, then looks under
`$CLAUDE_CONFIG_DIR`, `~/.claude` and every other `~/.claude*` directory. Herdr's own environment is
not the agent's, so the scan is what finds sessions kept elsewhere, such as `~/.claude-work`.

## Keys

Every key below can be changed in `config.toml`. The footer and the `?` overlay always show the keys in
effect.

| Action | Default | Does |
|---|---|---|
| `up` | `k`, `up` | move up one row |
| `down` | `j`, `down` | move down one row |
| `page_up` | `ctrl+u`, `pageup` | scroll up one page |
| `page_down` | `ctrl+d`, `pagedown` | scroll down one page |
| `prev_hunk` | `[` | previous hunk |
| `next_hunk` | `]` | next hunk |
| `prev_thread` | `N` | previous thread |
| `next_thread` | `n` | next thread |
| `scroll_left` | `h`, `left` | scroll the code left |
| `scroll_right` | `l`, `right` | scroll the code right |
| `scroll_reset` | `0` | scroll back to the start of the lines |
| `switch_panel` | `tab` | switch between sidebar and stream |
| `toggle_sidebar` | `f` | show or hide the sidebar |
| `comment` | `c` | comment on the line, range or file |
| `select_range` | `v` | select lines (visual mode) |
| `reply` | `r` | reply to the thread |
| `edit` | `e` | edit your comment |
| `delete` | `d` | delete your comment |
| `resolve` | `x` | resolve or reopen the thread |
| `archive` | `A` | archive the resolved threads |
| `send` | `S` | send unsent comments |
| `resend` | `s` | resend the thread |
| `reload` | `R` | reload the diff |
| `switch_spec` | `b` | switch diff spec |
| `toggle_layout` | `t` | side by side or unified |
| `help` | `?` | show this help |
| `quit` | `q` | quit |

Keys inside the comment editor are fixed.

## Configure

The file is `config.toml` in the directory `herdr plugin config-dir review` prints. The pane reads it
once at start, so restart the pane after an edit.

```toml
[keys]
send = "ctrl+s"               # one key
next_hunk = ["]", "ctrl+n"]   # several keys
switch_spec = ""              # unbound

[theme]
name = "catppuccin-latte"     # mocha (default), macchiato, frappe, latte

[sidebar]
open = false                  # start with the sidebar hidden
```

- Spell keys as Herdr does: `ctrl+`, `shift+`, `alt+`, and names such as `enter`, `tab`, `pageup`.
  `shift+s` and `S` are the same key.
- An action you do not list keeps its default keys. A key you assign takes priority over another
  action's default, and the pane prints a warning naming each action left with no key.
- A mistake in the file is a warning in the pane, never a crash. A file that is not valid TOML is ignored.

## Let the agent start a review

Install the skill so Claude Code knows the commands:

```sh
mkdir -p ~/.claude/skills
cp -R "$(herdr plugin list --plugin review --json | jq -r '.result.plugins[0].plugin_root')/skills/herdr-review" ~/.claude/skills/
```

Then ask: "review your changes with herdr-review". The agent posts its comments, opens the pane, and
ends its turn. Your replies come back as its next message when you press `S`. The skill finds the
binary through `herdr plugin list` on every run, so it survives plugin updates.

## Develop

```sh
cargo test
cargo clippy --all-targets
```

`DESIGN.md` indexes the design in `design/`, `docs/adr/` has the decisions, and `CONTEXT.md` defines the terms.
