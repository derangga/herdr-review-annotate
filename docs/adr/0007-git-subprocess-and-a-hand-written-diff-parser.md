---
status: accepted
---

# Read diffs from a `git` subprocess and parse them by hand

The plugin has a memory and binary-size budget, and it must show the same renames and the same hunks
the user sees from `git` itself. We run `git` as a subprocess and parse the unified diff with our own
parser. We do not link libgit2 or gix.

There are two diff specs. The working-tree spec is `git diff HEAD` plus untracked files. The branch
spec compares the working tree with the merge base, `git diff $(git merge-base <base> HEAD)`.

## Considered options

- **libgit2 or gix.** Rejected for binary size, and because their rename detection does not match
  `git`'s exactly.
- **`git diff <base>...HEAD` for the branch spec.** Rejected. It ignores the working tree, so a fix
  the agent has not committed never appears and the user cannot see what changed.

## Consequences

- The parser must not depend on the user's git config. Every call passes
  `-c core.quotePath=false --no-color --no-ext-diff --no-textconv --src-prefix=a/ --dst-prefix=b/ -M`,
  and paths come from the `---`, `+++` and `rename` lines, not from the `diff --git` header.
- Untracked files need a synthesized patch and a size check before the file is read.
- The plugin needs `git` on `PATH` and reports clearly when the directory is not a repository, has no
  commits, or has no usable base.
