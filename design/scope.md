# Scope and layout

Part of the [herdr-review design](../DESIGN.md).

The core loop:

1. The user opens a review pane beside the agent and reads the diff.
2. The user comments on a line, a range, or a file.
3. One key sends the unsent comments to the agent.
4. The agent fixes, then replies and resolves through the CLI.
5. The user sees the replies and resolved marks without restarting the pane.
6. The agent can also start a review by adding its own comments and opening the pane.

Built around it: both diff specs, replies, resolve and reopen by either side, the `new` marker, the
`outdated` tag, resend, the quit prompt, edit and delete of your own comments, archive, a side-by-side
layout, syntax colours, the changed words of a line marked, and message review, the agent's newest message reviewed like a diff (`message-review.md`).

Not built yet, in this order: paste mode, Codex and pi, staged and single-commit
specs, a reviewer agent.

Not planned: daemon, file watcher, outbox file, startup hook, generated skill, highlight marks, markup,
agent-driven navigation.


## Repository layout

```
herdr-review/
  herdr-plugin.toml
  Cargo.toml
  src/
    main.rs            argument parsing and dispatch
    lib.rs             module list, so the copied files keep their `pub` items
    open.rs            the `open` action
    store.rs           event log: lock, append, read, fold
    meta.rs            meta.json: load, locked save, find the state directory
    diff.rs            git runner, unified diff parser, anchor matching
    agent.rs           the agent name for a `comment` command
    apply.rs           `comment apply`: decode, check, read the anchored lines, append
    cli.rs             arguments into a Command, a Command into output and an exit code
    comment.rs         what the `comment` subcommands read and print
    env.rs             Env: the process variables, read once in main.rs
    send.rs            prompt format, target resolution, mark sent
    tui.rs             review pane: layout, render, actions
    keymap.rs          default keys, config.toml overrides, key parsing
    theme.rs           colour roles, the Catppuccin flavors, the scope table for syntax colours
    syntax.rs          tokens per line of a file, and the cache of the files on screen
    view.rs            the stream of rows, the cursor, the sidebar, drawing
    cards.rs           a thread as the lines of its box
    actions.rs         what the user's actions write: comment, reply, edit, delete, resolve, reopen
    editor.rs          multi-line comment editor   (adapted from herdr-annotate)
    edit_keys.rs       editor key map              (from herdr-annotate)
    icons.rs           the Nerd Font glyph for a file name, for the sidebar rows
    width.rs           display width helpers       (from herdr-annotate)
    words.rs           the words that differ between a removed line and the added line paired with it
    agent_delivery.rs  readiness check and prompt  (from herdr-annotate)
    herdr.rs           herdr CLI wrapper           (from herdr-annotate)
    termination.rs     the flag that SIGTERM and SIGHUP set (from herdr-annotate)
    message.rs         find the agent's newest message, format its prompt (`message-review.md`)
    message_tui.rs     the message pane (`message-review.md`)
    message_action.rs  the `message` action: find the agent, then focus or open the pane (`message-review.md`)
  skills/herdr-review/SKILL.md
  scripts/fetch-herdr-review.sh   prebuilt binary and SHA-256, for `plugin install`
  scripts/stage-local.sh          build and stage, for `plugin link`
  scripts/ci.sh                   the CI steps, in CI's order
  tests/*.rs           integration tests: store processes, live pickup, a temporary repository
  tests/fixtures/*.patch
  src/<module>/tests.rs  unit tests of the modules that have one
  .github/workflows/ci.yml, release.yml
```

Dependencies: `ratatui` 0.30 (brings crossterm), `serde`, `serde_json`, `toml` (for the keymap file,
`config.md`), `chrono` with the `clock` feature. On Unix, `signal-hook` for clean terminal restore. Behind
the cargo feature `syntax`, which is on by default: `syntect` with `regex-fancy` and `default-syntaxes` and no
default features, and `two-face` with `syntect-fancy` for TypeScript, TSX, TOML and the other grammars `bat`
ships. Neither builds C code. `cargo build --no-default-features` leaves both out. No `uuid`, `notify`, `tokio`, `similar`,
`rustix` or git library. Lints and the release profile are copied from
`herdr-annotate/rust/Cargo.toml`. `rust-version` is 1.89 or newer, for `std::fs::File::lock`.
