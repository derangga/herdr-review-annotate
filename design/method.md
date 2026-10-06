# Method: Effect mapped onto Rust

Part of the [herdr-review design](../DESIGN.md).

This section applies the design-thinking method (shapes, happy path, failures, dependencies) to every
operation. It uses the Effect model `Effect<A, E, R>` and maps it onto Rust:

| Effect | Rust in this project |
|---|---|
| `A`, the success value | The `Ok` type of a function |
| `E`, the error channel | The `Err` type, one enum per module. Errors are values until the edge |
| `R`, the requirements | Function parameters: a `git` closure, a `herdr` closure, a state directory, a clock value, an `Env` struct. No globals, no `std::env` or `Command` calls below `main.rs` |
| `gen` body is A, `pipe` is E | Inner functions use `?` only. Each error is matched once, at the edge that owns the reaction: `main.rs` for the CLI, the action dispatcher for the TUI |
| Retry, escape, die | Retry is a bounded loop at the node. Escape returns a fallback value and records a warning. Die is `panic!`, reserved for a broken invariant in our own code |
| Layer swap for tests | Pass a closure that returns canned output and a temp directory |
| Scope | A guard value whose `Drop` releases the resource |

Graph notation: steps start with `->`, nested steps are indented, `R:` names what the step needs and
`E:` names how it fails and what happens then.
