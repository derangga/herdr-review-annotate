---
status: accepted
---

# Message comments live in the pane, not in the review

A diff comment becomes a thread in `review.jsonl`, and the agent replies and resolves it through the
CLI. A comment on the agent's newest message does not. The message pane keeps its comments in memory,
sends them as one prompt, and closes. The agent answers in its next message, and the user reviews that
one if they want to.

A thread works because its anchor outlives the turn: a line of a file is still there after the agent
edits it, or shows up as outdated. A message is gone from view one turn later. A stored thread would
point at text the user cannot open again from the diff pane, and a resolve would mean only "I read
it", which the next message already says.

## Considered options

- **Threads in `review.jsonl` with a message anchor.** Gives replies, resolve and a history. It costs
  a new anchor kind, changes to the fold and the diff pane's stream, and a git repository for a
  feature that does not need one.
- **A send log, like plannotator's feedback archive.** History that nothing reads. Add it when someone
  asks for it.

## Consequences

- Closing the pane, or Herdr killing it, loses unsent comments. `quit` asks first, and nothing else
  protects them.
- A send cannot be repeated. The pane closes when Herdr accepts the prompt.
- The prompt carries no ids and no commands, so the skill does not change.
- The pane writes one file, the pointer to its own pane id that lets a second key press focus it
  (PLAN.md section 13.3).
