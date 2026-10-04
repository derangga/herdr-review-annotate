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
- Claude Code queues a prompt that arrives during a turn. Tested on 2026-10-04 with Claude Code 2.1.285 and
  Herdr 0.9.1, by running `spike-send` while the agent was `working` through a 250-line answer:
  - The prompt appeared under the running turn as a queued message ("ctrl+enter to send now",
    "Press up to edit queued messages"). The agent stayed `working`.
  - When the turn finished, Claude Code submitted the queued prompt as its own next turn and
    answered it. Nothing was merged into the running turn.
  - With the user pressing Esc while the prompt was queued, Claude Code printed "Interrupted",
    stopped the running turn, and then submitted the queued prompt as the next turn. The prompt was
    not lost and not returned to the input box.
  - Not tested: editing the queued message with the up key. A comment batch edited there would reach the agent changed, and the `sent` event would not show it.
- The first pass covers Claude Code only. Codex and pi are checked afterwards.
