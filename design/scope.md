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

A second front end, the herdr-app panel: a TypeScript mod that draws the files, the diff and the threads,
and runs this binary for every read and write (`gui.md`, ADR 0011). It adds the commands in `cli.md` that
the panel calls, an hc backend for `send`, and an integer `schema` on their JSON. The TUI does not change.

Not built yet, in this order: paste mode, Codex and pi, staged and single-commit
specs, a reviewer agent. For the panel, each a follow-up: syntax colours, side-by-side layout, changed
words, the agent-message review, vim motions, a filter box, the whole-file view, collapsing a file.

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
    width.rs           display width helpers, over unicode-width
    words.rs           the words that differ between a removed line and the added line paired with it
    agent_delivery.rs  readiness check and prompt  (from herdr-annotate)
    herdr.rs           herdr CLI wrapper           (from herdr-annotate)
    hc.rs              hc CLI wrapper, and the hc backend of target resolution and delivery
    json.rs            the JSON of `state`, `file`, `send --json` and `archive --json`, and their schema number
    state.rs           `state` and `file`: the read-only reads the panel makes
    user.rs            the user CLI: `--as user`, body files, `edit`, `delete`, `seen`, `archive`, `view`
    termination.rs     the flag that SIGTERM and SIGHUP set (from herdr-annotate)
    message.rs         find the agent's newest message, format its prompt (`message-review.md`)
    message_tui.rs     the message pane (`message-review.md`)
    message_action.rs  the `message` action: find the agent, then focus or open the pane (`message-review.md`)
  herdr-app-mod/       the panel (`gui.md`)
    package.json  tsconfig.json  mod.json  package-lock.json
    src/               types, decode, host, cli, update, rows, runner, view, main
    test/              node:test files and the golden JSON the Rust tests share
  skills/herdr-review/SKILL.md
  scripts/fetch-herdr-review.sh   prebuilt binary and SHA-256, for `plugin install`
  scripts/stage-local.sh          build and stage, for `plugin link`
  scripts/stage-app-mod.sh        build the panel and the binary, copy both to the herdr-app mod folder
  scripts/ci.sh                   the CI steps, in CI's order: `ci.sh test`, `ci.sh build`, or both
  tests/*.rs           integration tests: store processes, live pickup, a temporary repository
  tests/fixtures/*.patch
  src/<module>/tests.rs  unit tests of the modules that have one
  .github/workflows/ci.yml, release.yml
```

Dependencies: `ratatui` 0.30 (brings crossterm), `serde`, `serde_json`, `toml` (for the keymap file,
`config.md`), `chrono` with the `clock` feature. On Unix, `signal-hook` for clean terminal restore.
`unicode-width`, the crate ratatui measures text with, so `width.rs` agrees with what is drawn. Behind
the cargo feature `syntax`, which is on by default: `syntect` with `regex-fancy` and `default-syntaxes` and no
default features, and `two-face` with `syntect-fancy` for TypeScript, TSX, TOML and the other grammars `bat`
ships. Neither builds C code. `cargo build --no-default-features` leaves both out. No `uuid`, `notify`, `tokio`, `similar`,
`rustix` or git library. Lints and the release profile are copied from
`herdr-annotate/rust/Cargo.toml`. `rust-version` is 1.89 or newer, for `std::fs::File::lock`.

The panel is TypeScript and adds a second toolchain, kept small:

- Node 22 or newer, for `node --test` with type stripping. No test runner package.
- `esbuild`, a dev dependency: bundles `src/main.ts` to one `main.js`, with `gpui-kit`, `gpui-base` and
  `herdr` left out, because herdr-app provides them.
- `typescript`, a dev dependency, for the local typecheck. `@types/node`, a dev dependency, for the tests'
  `node:test` types.
- No runtime dependency. The bundle holds only the mod's own code. No Effect (its `Schema` does not load in
  the mod runtime), no UI library, no schema library.
- Not in the repo: `gpui-kit.d.ts` and `herdr.d.ts`, which herdr-app writes on every start. A local setup
  step copies `gpui-kit.d.ts` next to `tsconfig.json`, git-ignored.
- CI runs `npm ci`, the tests and the bundle. The typecheck runs locally, before a bead closes.

The panel never calls `git`, `herdr` or `hc` itself. It needs `exec`, `writeTextFile` and `watchFolder` from
the mod runtime, and `hc` and the herdr CLI stay behind the binary. So a change in what either CLI prints
reaches one place, `hc.rs` or `herdr.rs`.

