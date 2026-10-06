# herdr-review: design

Domain vocabulary is in [CONTEXT.md](./CONTEXT.md). Decisions that are hard to reverse are in
[docs/adr/](./docs/adr/).

This file is an index. Nothing is defined here. Every section lives in `design/`, so a task loads the
two or three files it needs instead of all of them. Working on one feature means its behaviour file, then
`shapes.md`, `graphs/`, `errors.md` and `tests.md` for the operation it touches. It does not mean the
whole directory.

## Behaviour

What the plugin does, one file per area.

| Read this | When |
|---|---|
| [scope](design/scope.md) | Adding a module or a dependency, or asking whether a feature is in scope |
| [data](design/data.md) | Changing the event log, a fold rule, how a write is locked, archive, or `meta.json` |
| [diff](design/diff.md) | Changing the `git` commands, a case the diff must handle, or how a comment finds its line |
| [cli](design/cli.md) | Adding or changing a command, an exit code, or how the agent's name is found |
| [send](design/send.md) | Changing the prompt the agent receives, or how the target agent is chosen |
| [tui](design/tui.md) | Adding a key, or changing the sidebar, the status line or the cursor |
| [tui-layout](design/tui-layout.md) | Side by side view, the mouse, syntax colours |
| [tui-comments](design/tui-comments.md) | Cards, the editor, visual mode, archive and send inside the pane |
| [config](design/config.md) | `config.toml`: keys, theme, sidebar |
| [plugin](design/plugin.md) | The Herdr manifest, the skill, install |
| [herdr-facts](design/herdr-facts.md) | Before assuming what Herdr does |
| [message-review](design/message-review.md) | Anything about reviewing the agent's last message. It holds that feature whole: scope, transcript, pane, prompt, shapes, graphs and tests |

## Design of an operation

The types, the call graph of each operation, how each step fails, and what each step receives. Read
[method](design/method.md) once, then these before writing a function.

| Read this | When |
|---|---|
| [method](design/method.md) | Reading a graph for the first time: how Effect maps onto Rust, and the notation |
| [shapes](design/shapes.md) | Adding a type, or asking why something is an enum and not a flag |
| [boundary](design/boundary.md) | Anything that crosses in from a file, `git`, Herdr, the environment or stdin |
| [requirements](design/requirements.md) | Deciding what a function receives as parameters |
| [errors](design/errors.md) | Deciding where a new failure is handled and what it exits with |
| [tests](design/tests.md) | Writing a test, and asking what to swap for it |

### Call graphs

One file per group of operations. An `E:` line in a graph is one test.

| File | Operations |
|---|---|
| [store](design/graphs/store.md) | Write to the store, archive |
| [commands](design/graphs/commands.md) | The agent CLI (`comment apply`, `reply`, `resolve`, `reopen`), `open` |
| [pane](design/graphs/pane.md) | TUI start, load the diff, the loop, a user comment |
| [send](design/graphs/send.md) | Send |

The graphs of message review are in [message-review](design/message-review.md).

A new operation is a graph in one of these files, its failures in `errors.md`, and its tests in
`tests.md`. A graph that exists in only one of those places is drift.
