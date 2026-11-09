# Requirements

Part of the [herdr-review design](../DESIGN.md).

Seven dependencies exist. Every function below `main.rs` receives the ones it needs as parameters.

| Name | Production | Tests |
|---|---|---|
| `git` | `Command::new("git")` with `GIT_OPTIONAL_LOCKS=0` | A closure that returns fixture bytes |
| `herdr` | `herdr.rs`, through `HERDR_BIN_PATH` | A closure that records calls and returns canned JSON, as `agent_delivery.rs` tests do today |
| `hc` | `hc.rs`, the `hc` binary on `PATH`. It stands with `herdr` as one of the two backends behind the `get`, `list` and `prompt` operations of `send.md` | The same recording fake, returning canned `hc get` and `hc ls` JSON and `hc send` exits |
| `dir` | The state directory from `data.md` | A temp directory |
| `now` | `chrono::Utc::now()` formatted once per operation | A fixed string |
| `env` | `Env::from_process()` | An `Env` literal |
| `term` | crossterm on stdout | ratatui `TestBackend` and a list of key events |

`GIT_OPTIONAL_LOCKS=0` stops `git diff` from taking `index.lock` to refresh the index. Without it the
review pane can make the agent's own `git` commands fail while both run in one worktree.

`Env` also holds `HC_TERMINAL_ID` and `exe`, the path of the running binary from `std::env::current_exe()`,
read once in `main.rs`. The prompt's `<bin>` under hc comes from `exe`, so a test sets it and no function
below `main.rs` asks the process. A body file named by `--file` or `--body-file` is read and deleted in the
function that decodes it, on a path it receives, and a test gives it a file in the temp directory.

