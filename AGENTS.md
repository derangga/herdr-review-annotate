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
| `PLAN.md` section 12 | Before writing any function. It has the types, the call graph of each operation, how each step fails, and what each step receives as parameters |
| `PLAN.md`, the sections a bead names | Before starting that bead. The plan is the source of truth, the bead only points at it |
| `docs/adr/` | When a bead cites an ADR, or when a design choice looks odd |
| `CONTEXT.md` | For the meaning of thread, anchor, outdated, unsent, resend, target agent |

A finding such as B3, G8 or W13 comes from `RESEARCH.review.md`, removed from the tree. Read it with
`git show c1fb97a^:RESEARCH.review.md`. `PLAN.md` supersedes `RESEARCH.md`.

## Rules the code follows

- Functions below `main.rs` receive `git`, `herdr`, `dir`, `now` and `env` as parameters. Tests pass
  closures and a temp directory in their place.
- Inner functions use `?`. Each error is matched once, at the edge named in `PLAN.md` section 12.6.
- A graph's `E:` lines are its test list. One test per line.
- When the plan and what you find disagree, update `PLAN.md` in the same change and say so in the
  bead's close note.
- Bead ids stay inside `bd`. Code comments and commit messages describe the change in their own
  words and never name a bead id.

## Reference clones

`hunk/`, `zeron/` and `herdr-annotate/` are read-only reference code. Copy from `herdr-annotate/rust/src/`
only the files `PLAN.md` section 2.1 lists.
