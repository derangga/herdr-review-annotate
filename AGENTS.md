# herdr-review

A Rust Herdr plugin for reviewing a diff and sending the comments to an agent. Work is tracked in
beads (`bd`).

## Work loop

1. `bd ready` lists beads with no open blocker. Take one task or spike, not an epic.
2. `bd show <id>`, then `bd update <id> --status in_progress`.
3. Read every section the bead names before writing code.
4. The bead is done when each line of its acceptance criteria is true and `cargo clippy` and
   `cargo test` are clean. Then `bd close <id>`.
5. Work found along the way that the bead does not cover becomes a new bead:
   `bd create "<title>" --deps discovered-from:<id>`.

Commit only when the user asks.

## Where things are

| Read | When |
|---|---|
| `DESIGN.md` | First. It is an index with a "when" for each file in `design/`. Load the two or three files the task needs, not the directory |
| `design/shapes.md`, `design/graphs/`, `design/errors.md` | Before writing any function. They have the types, the call graph of each operation, how each step fails, and what each step receives as parameters |
| The `design/` files a bead names | Before starting that bead. The design is the source of truth, the bead only points at it |
| `docs/adr/` | When a bead cites an ADR, or when a design choice looks odd |
| `CONTEXT.md` | For the meaning of thread, anchor, outdated, unsent, resend, target agent |

## Rules the code follows

- Functions below `main.rs` receive `git`, `herdr`, `dir`, `now` and `env` as parameters. Tests pass
  closures and a temp directory in their place.
- Inner functions use `?`. Each error is matched once, at the edge named in `design/errors.md`.
- A graph's `E:` lines are its test list. One test per line.
- When the design and what you find disagree, update the `design/` file in the same change and say so in the
  bead's close note.
- Bead ids stay inside `bd`. Code comments and commit messages describe the change in their own
  words and never name a bead id.

