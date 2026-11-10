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
| [cli](design/cli.md) | Adding or changing a command, an exit code, or how the agent's name is found. Also the commands the panel calls and the backend choice |
| [send](design/send.md) | Changing the prompt the agent receives, or how the target agent is chosen, under herdr or under hc |
| [tui](design/tui.md) | Adding a key, or changing the sidebar, the status line or the cursor |
| [tui-layout](design/tui-layout.md) | Side by side view, the mouse, syntax colours |
| [tui-comments](design/tui-comments.md) | Cards, the editor, visual mode, archive and send inside the pane |
| [config](design/config.md) | `config.toml`: keys, theme, sidebar |
| [plugin](design/plugin.md) | The Herdr manifest, the skill, install |
| [herdr-facts](design/herdr-facts.md) | Before assuming what Herdr does |
| [gui](design/gui.md) | Anything about the herdr-app panel: its shapes, errors, tests, code layout, how it finds the binary and opens, and what it needs from the repo. Read [herdr-app-facts](design/herdr-app-facts.md) before assuming what herdr-app does |
| [herdr-app-facts](design/herdr-app-facts.md) | Before assuming what herdr-app, its mod runtime or `hc` does. It has what was run, what was seen and what is still unknown |
| [message-review](design/message-review.md) | Anything about reviewing the agent's last message. It holds that feature whole: scope, transcript, pane, prompt, shapes, graphs and tests |

## Design of an operation

The types, the call graph of each operation, how each step fails, and what each step receives. Read
[method](design/method.md) once, then these before writing a function.

| Read this | When |
|---|---|
| [method](design/method.md) | Reading a graph for the first time: how Effect maps onto Rust, and the notation. [gui](design/gui.md) has the same map onto plain TypeScript |
| [shapes](design/shapes.md) | Adding a type, or asking why something is an enum and not a flag. Also the JSON `state`, `file` and `send --json` print, and the `Target` shape |
| [boundary](design/boundary.md) | Anything that crosses in from a file, `git`, Herdr, the environment or stdin |
| [requirements](design/requirements.md) | Deciding what a function receives as parameters |
| [errors](design/errors.md) | Deciding where a new failure is handled and what it exits with |
| [tests](design/tests.md) | Writing a test, and asking what to swap for it |

### Call graphs

One file per group of operations. An `E:` line in a graph is one test.

| File | Operations |
|---|---|
| [store](design/graphs/store.md) | Write to the store, archive |
| [commands](design/graphs/commands.md) | The agent CLI (`comment apply`, `reply`, `resolve`, `reopen`), the user CLI (`--as user`, `edit`, `delete`, `seen`, `archive`, `view`), `open` |
| [pane](design/graphs/pane.md) | TUI start, load the diff, the loop, a user comment |
| [send](design/graphs/send.md) | Send, under the herdr backend and under hc |
| [json](design/graphs/json.md) | `state` and `file`, the read-only JSON the panel reads |
| [gui-open](design/graphs/gui-open.md) | The panel opens: root, target, binary, close |
| [gui-load](design/graphs/gui-load.md) | The panel loads the state: triggers, single flight, spec switch, file switch |
| [gui-stream](design/graphs/gui-stream.md) | The panel's stream of every file: items and heights, fetching the files as they show, the selected file, scrolling, folding |
| [gui-comment](design/graphs/gui-comment.md) | The panel hovers a line for its [+], drags a range, comments from a [+], and writes the comment |
| [gui-thread](design/graphs/gui-thread.md) | The panel replies, resolves, reopens, edits, deletes, marks seen |
| [gui-keys](design/graphs/gui-keys.md) | The panel's keyboard: a cursor on the code rows (`j k`, `[ ]`, `h l`), `v` for a range, `c` to comment, `r e d x` on a thread |
| [gui-send](design/graphs/gui-send.md) | The panel sends, resends one thread, and picks among agents |
| [gui-message](design/graphs/gui-message.md) | The panel shows the agent's last message, comments on its lines, and sends them |
| [gui-archive](design/graphs/gui-archive.md) | The panel archives resolved threads and marks a file viewed |

The graphs of message review are in [message-review](design/message-review.md).

A new operation is a graph in one of these files, its failures in `errors.md`, and its tests in
`tests.md`. A graph that exists in only one of those places is drift.
