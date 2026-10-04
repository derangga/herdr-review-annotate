---
status: accepted
---

# Keep review state outside the repo, keyed by worktree root

Review state lives in `${XDG_STATE_HOME:-~/.local/state}/herdr-review/<hash of the canonical worktree
root>/` on both macOS and Linux. One worktree has one review, whichever diff spec is shown. Two
worktrees of one repository are two reviews, which matches Herdr's worktree workspaces. Each comment
records the diff spec it was written against, because old-side line numbers mean different things in
different specs.

## Considered options

- **`HERDR_PLUGIN_STATE_DIR`.** Rejected. Herdr sets it only for commands it launches as plugin
  commands, and the agent CLI is not one of them.
- **Inside the repo, such as `.git/herdr-review/`.** Rejected. It puts tool state in the user's
  repository, and agent sandboxes commonly protect `.git`.
- **Key by root plus diff spec.** Rejected. Switching the spec would hide the user's comments.

## Consequences

- A sandboxed agent may not be allowed to write outside its workspace. This is unverified for Codex
  and is a known risk for the pass after Claude Code.
- Deleting or moving a worktree leaves its state directory behind. Nothing cleans it up in v1.
- Two agents in one worktree share one review. The target for send is chosen as described in ADR 0006.
