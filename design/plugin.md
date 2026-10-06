# Plugin manifest and skill

Part of the [herdr-review design](../DESIGN.md).

```toml
id = "review"
name = "Review"
version = "0.1.0"
min_herdr_version = "0.9.1"
platforms = ["macos", "linux"]

[[build]]
command = ["bash", "scripts/fetch-herdr-review.sh"]

[[actions]]
id = "open"
title = "Review: open diff"
contexts = ["workspace", "pane"]
command = ["./bin/herdr-review", "open"]

[[actions]]
id = "send"
title = "Review: send comments to agent"
contexts = ["pane"]
command = ["./bin/herdr-review", "send"]

[[panes]]
id = "tui"
title = "Review"
placement = "split"
command = ["sh", "-c", "exec \"$HERDR_PLUGIN_ROOT/bin/herdr-review\" tui"]
```

Herdr keys. `prefix+r` and `prefix+shift+r` are Herdr's defaults for resize mode and reload config, so
the README suggests `prefix+i` to open and `prefix+shift+i` to send. These bindings live in the user's
own Herdr `config.toml` as `plugin_action` entries, so the user can pick any other key there.

`skills/herdr-review/SKILL.md` is a static file. It says:

- Use it only when `HERDR_ENV=1`.
- Find the binary: `herdr plugin list --plugin review --json`, field `plugin_root`, plus
  `/bin/herdr-review`. Read it each time, do not remember it (ADR 0005).
- When a prompt lists review comments, follow the commands in that prompt.
- To review your own changes: pipe a JSON batch to `comment apply --stdin`, run `open`, then end the
  turn. The user's replies arrive as the next message.

Install: `plugin install` runs the fetch script. `plugin link` does not, so `stage-local.sh` builds and
copies the binary to `bin/`. On this machine plugins are linked from the Nix store, so the Nix package
must build the binary itself.
