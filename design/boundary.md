# Boundaries

Part of the [herdr-review design](../DESIGN.md).

Untrusted data enters at eight places. Each is parsed once, into the shapes above, and code past the
boundary never sees a raw string or `serde_json::Value`.

| Boundary | Parsed into | On bad input |
|---|---|---|
| `review.jsonl` line | `Event` | `Warning::SkippedLine`, the read continues |
| `meta.json` | `Meta` | `Warning::MetaUnreadable`, an empty `Meta` is used and rewritten on the next save |
| `config.toml` | `Keymap`, `Theme`, the sidebar's starting state | `Warning::Config`, defaults are used |
| `git` stdout | `Diff` | `Change::Unparsed` for that file |
| `herdr` stdout and stderr | `Target`, `AgentStatus`, `HerdrError` | `Refusal::AgentGone` or the raw message |
| Environment | `Env` | A missing value is `None`. A pane id is trusted only after `agent get` confirms it |
| CLI arguments | `Command` enum | Usage text, exit 2 |
| Agent stdin (batch, reply text) | `Vec<NewComment>`, `String` | `CommandError::InvalidBatch`, nothing is written |

Limits checked at the agent stdin boundary: a body is at most 16 KiB after trimming and must not be
empty, a batch holds at most 200 comments, a path must parse as `RelPath`, a line must be 1 or more
and exist in the file. Every body is stored as written. Control characters are removed when it is
drawn and ESC is removed when it is sent.
