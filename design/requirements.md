# Requirements

Part of the [herdr-review design](../DESIGN.md).

Six dependencies exist. Every function below `main.rs` receives the ones it needs as parameters.

| Name | Production | Tests |
|---|---|---|
| `git` | `Command::new("git")` with `GIT_OPTIONAL_LOCKS=0` | A closure that returns fixture bytes |
| `herdr` | `herdr.rs`, through `HERDR_BIN_PATH` | A closure that records calls and returns canned JSON, as `agent_delivery.rs` tests do today |
| `dir` | The state directory from `data.md` | A temp directory |
| `now` | `chrono::Utc::now()` formatted once per operation | A fixed string |
| `env` | `Env::from_process()` | An `Env` literal |
| `term` | crossterm on stdout | ratatui `TestBackend` and a list of key events |

`GIT_OPTIONAL_LOCKS=0` stops `git diff` from taking `index.lock` to refresh the index. Without it the
review pane can make the agent's own `git` commands fail while both run in one worktree.
