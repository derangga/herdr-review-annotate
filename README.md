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
2. Move as in vim: `j` and `k` by screen row, so they step through a wrapped paragraph, `h` and `l` by
   character, `w`, `b` and `e` by word, `0` and `$` to the ends of the line. Press `c`, write the
   comment, and save it with `ctrl+s`. `c` alone comments on the cursor's line. To point at a few
   words, press `v`, move to the last one (`e` is handy), then press `c`. `V` selects whole lines, and
   a drag of the mouse selects characters. The comment is a box under the last line it covers, and the
   words it is about are underlined. `e` edits and `d` deletes the comment under the cursor, and `n`
   and `N` jump between comments.
3. Press `S`. Every comment goes to the agent as one prompt, and the pane closes. The agent answers in
   its next message, and `prefix+o` opens that one if you want to go on. If the agent is waiting on a
   permission prompt, the send is refused, the pane stays, and the comments are kept.

The prompt quotes each comment's lines, at most six, so the agent can find them. A comment on some
words names them too, cut at 60 characters:

```
Feedback on your last message. Address each point.

- lines 12-14:
  > ## Step 2: migrate the store
  > Move events into the new file and drop the old one.
  > The fold reads only the new file.
  Why not keep the old file and read both?
- line 30, "global lock":
  > Use a global lock.
  Fine for now, add a ponytail note.
```

`R` loads the newest message again, and `q` leaves. Both ask first when there are comments. The comments
live only in the pane: nothing is written to disk, and closing the pane without sending loses them.

The pane reads the same `[keys]` table as the diff pane and acts on `up`, `down`, `page_up`,
`page_down`, `prev_thread`, `next_thread`, `comment`, `select_range`, `edit`, `delete`, `send`,
`reload`, `help` and `quit`. The other actions do nothing there, and `?` lists only these. In visual
mode it acts only on the moves, `comment`, `select_range`, `help` and `quit`. Any key not bound to one
of these is read as a vim motion (`h j k l w b e 0 $`, `V`), so `e` edits in normal mode and moves to
the end of a word in visual mode.

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
| `filter` | `/` | narrow the sidebar to the files whose path matches what you type; `enter` keeps it, `esc` clears it |
| `comment` | `c` | comment on the line, range or file |
| `select_range` | `v` | select a range (visual mode) |
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
| `collapse` | `z` | collapse or expand the file |
| `viewed` | `m` | mark the file viewed and collapse it, or take the mark off |
| `whole_file` | `o` | show every line of the file, or only its hunks |
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
icons = false                 # hide the file-type glyph on each file row, which needs a Nerd Font
```

- Spell keys as Herdr does: `ctrl+`, `shift+`, `alt+`, and names such as `enter`, `tab`, `pageup`.
  `shift+s` and `S` are the same key.
- An action you do not list keeps its default keys. A key you assign takes priority over another
  action's default, and the pane prints a warning naming each action left with no key.
- A mistake in the file is a warning in the pane, never a crash. A file that is not valid TOML is ignored.

## The herdr-app panel

A second way to review, for [herdr-app](https://github.com/herdrdev/herdr) (the desktop app). It is a mod
that draws a native panel beside the agent: the changed files, the unified diff of the selected file, and the
comment threads under their lines. It holds no review logic. Every read and write runs the `herdr-review`
binary, so the panel and the pane above show the same review, stored in the same place. The agent
answers the same way, with `herdr-review comment ...` from its shell, and the panel updates by itself.

### Install the panel

From a checkout, with `cargo`, `node` and `npm` installed:

```sh
bash scripts/stage-app-mod.sh
```

The script builds the mod and the release binary, and copies them into
`~/.config/herdr-app/mods/herdr-review` (or `$HERDR_APP_HOME/mods/herdr-review`): `mod.json`, `main.js`, and
the binary at `bin/herdr-review`. Nothing else under the app folder is touched. It stages its own binary
because the panel needs `herdr-review state`, which a copy on `PATH` may be too old to have. The panel runs
the staged binary first and `herdr-review` on `PATH` second. The prompt it sends names the binary that sent it,
so the agent replies with the same version.

Run `herdr-app check ~/.config/herdr-app/mods/herdr-review`. Run the script again after an update. Saving
the mod folder reloads a panel that is open.

The script never edits `keys.json`. To open the panel, add this line under `"global"` in
`~/.config/herdr-app/keys.json`:

```json
"cmd-shift-r": {"command": "pane.split", "args": {"direction": "right", "mod": "herdr-review"}}
```

Pick another key if you like. Without a key, run the same command from a shell:

```sh
herdr-app do pane.split '{"direction": "right", "mod": "herdr-review"}'
```

Focus the pane where your agent runs first. The panel opens to its right and reviews the folder that pane is in,
and sends to the agent in it.

### Use the panel

1. Click a line of the diff to select it. Click a second line to select the range between the two. A bar under
   the diff shows what is selected, with `Comment` and `Cancel`. `Comment on file` is there too, and it
   comments on the whole file.
2. Write the comment in the editor and save it. The comment shows as a card under its line, marked `unsent`.
3. Click `Send`. It shows the number of unsent comments. Every unsent comment goes to the agent as one prompt.
   If the folder has more than one agent, the bar lists them and you click one.
4. Click a card to focus its thread. The bar then shows `Reply`, `Resolve` or `Reopen`, `Resend` (that thread
   only), and for your own comments `Edit` and `Delete` (it asks for a second click). A thread the agent resolved
   shows `new` until you look at it.
5. `Archive` moves the resolved threads to `archive.jsonl` after a second click. `Mark viewed` marks the
   selected file as read, and `Mark unviewed` takes the mark off.

The header has `Working tree` and `Branch`, which are the two diff specs of the pane above (`b` there), and
`Refresh`. The panel also reloads when the repository or the review state changes, and every 10 seconds. The
choice of spec lasts as long as the panel is open.

### Limits

- A range is two clicks. herdr-app gives a list row no drag.
- The thread buttons are in the bar under the diff, not on the card, for the same reason.
- The diff is unified and has no syntax colours or marks on changed words. There are no vim keys, no filter
  box, and no review of the agent's last message. Use the pane above for those.
- A frame can lag a change by a moment. `Refresh` loads again at once.
- With no staged binary and an old `herdr-review` on `PATH`, the panel shows a banner and does nothing else.
  Run the stage script.

## Let the agent start a review

Install the skill so Claude Code knows the commands:

```sh
mkdir -p ~/.claude/skills
cp -R "$(herdr plugin list --plugin review --json | jq -r '.result.plugins[0].plugin_root')/skills/herdr-review" ~/.claude/skills/
```

Then ask: "review your changes with herdr-review". The agent posts its comments, opens the pane, and
ends its turn. Your replies come back as its next message when you press `S`. The skill finds the
binary through `herdr plugin list` on every run, so it survives plugin updates.

Under herdr-app there is no plugin to list. The skill is the same file, copied from a checkout with
`cp -R skills/herdr-review ~/.claude/skills/`. The agent takes the binary from the review prompt it was
sent, and for comments of its own, from `herdr-review` on `PATH` or the staged copy. It posts its comments, and
you read them in the panel.

## Develop

```sh
cargo test
cargo clippy --all-targets
```

`DESIGN.md` indexes the design in `design/`, `docs/adr/` has the decisions, and `CONTEXT.md` defines the terms.

## License

MIT. See `LICENSE`.
