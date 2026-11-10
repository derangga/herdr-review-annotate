---
name: herdr-review
description: Review your own changes with the human, in a Herdr pane or the herdr-app panel. Use when asked to "review your changes with herdr-review", or when a prompt lists herdr-review comments to address.
---

# herdr-review

Run this under one of two hosts. Otherwise tell the human what changed and ask them to review it.

- Herdr: `HERDR_ENV=1`. The human reviews in a pane.
- herdr-app: `HC_TERMINAL_ID` is set and `HERDR_ENV` is not. The human reviews in the app's panel, which they open themselves.

## Find the binary

Under Herdr, read the path each time you need it. It changes when the plugin updates.

```bash
HR="$(herdr plugin list --plugin review --json | jq -r '.result.plugins[0].plugin_root')/bin/herdr-review"
```

If `herdr plugin list` shows no `review` plugin, tell the human to install it and stop.

Under herdr-app, set `HR` to the absolute path in the review prompt you received. For comments of your own, before any prompt, use `herdr-review` on `PATH`, else `~/.config/herdr-app/mods/herdr-review/bin/herdr-review`. If neither runs, tell the human to run `scripts/stage-app-mod.sh` and stop.

## Review your own changes

1. Pipe one JSON batch of comments to `comment apply`. Put a comment on each change you want the human to check: a decision you were unsure of, a tradeoff, a place you cut a corner.

   ```bash
   "$HR" comment apply --stdin <<'EOF'
   {"comments": [
     {"path": "src/store.rs", "line": 42, "body": "Takes the lock for the whole append. Held across the fsync on purpose."},
     {"path": "src/cli.rs", "line": 10, "end_line": 18, "body": "These arms repeat. Left alone to keep the diff small."},
     {"path": "src/lib.rs", "side": "old", "line": 7, "body": "Removed: nothing calls it."}
   ]}
   EOF
   ```

   - `path` is relative to the repo root. `line` is a line of the changed file (`side` `new`, the default). `side` `old` takes a line of the original file. `end_line` makes a range.
   - Leave out `line` for a comment on the whole file.
   - The CLI reads the line text itself. One bad entry rejects the whole batch (exit 2) with a message naming it. Fix it and send the batch again.
2. Under Herdr, open the pane:

   ```bash
   "$HR" open
   ```

   A second `open` focuses the pane that exists, so run it whenever you add comments. Under herdr-app there is nothing to open: tell the human the comments are in the panel.
3. **End your turn.** Do not wait, poll, or read the pane. The human's replies arrive as your next message.

## Address a prompt that lists comments

The prompt gives the exact `resolve` and `reply` commands, with the binary path filled in. Run those. Resolve a comment after you change the code. Reply and leave it open when you disagree or are unsure.

To read the open threads again: `"$HR" comment list --status open`.
