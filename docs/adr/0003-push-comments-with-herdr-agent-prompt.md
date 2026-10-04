---
status: accepted
---

# Push comments to the agent with `herdr agent prompt`

With hunk the user finishes a review and then types "check my comment in hunk". We remove that step.
`send` formats the unsent user comments into one prompt and submits it to the agent pane with
`herdr agent prompt`. The prompt carries the comment text, the ids, and the exact commands to reply and
resolve, so the agent does not have to poll or list anything first.

Send is refused only when the agent is `blocked`, missing, or still launching. A `working` agent is
sent to anyway and the TUI says the comments are queued.

## Considered options

- **Pull.** The agent runs `comment list` when told to. This is hunk's model and the step we are
  removing.
- **Paste without submitting.** Kept as a second mode for later. Submit is the default.
- **Refuse a working agent.** Rejected, because the user would have to wait and press send again.

## Consequences

- A comment is marked `sent` after Herdr accepts the prompt. Herdr accepting it does not prove the
  agent read it. If the TUI dies between the two steps the next send repeats the batch. Delivery is
  at-least-once, and the TUI has a resend key for comments that were sent and never handled.
- What Claude Code does with a prompt that arrives during a turn is not documented by Herdr and has
  not been tested. Milestone 1 tests it before anything is built on it.
- The first pass covers Claude Code only. Codex and pi are checked afterwards.
