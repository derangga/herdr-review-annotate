# Context: herdr-review

Glossary only. Decisions are in `docs/adr/`, the build plan is in `PLAN.md`.

| Term | Meaning |
|---|---|
| Review | All comments for one worktree. One worktree has exactly one review. |
| Diff spec | Which diff the review pane shows. Either the working tree against `HEAD`, or the working tree against the merge base with a base ref. |
| Comment | A piece of text written by the user or an agent, attached to a line, a line range on one side, or a whole file. |
| Thread | A root comment and its replies. |
| Reply | A comment with a parent. It has no anchor and no status of its own. |
| Anchor | Where a root comment points: path, side, line, and the text of that line when the comment was written. |
| Outdated | A computed tag on a thread whose anchor no longer matches the current diff. It is never stored and never changes the status. |
| Status | `open` or `resolved`. It belongs to the thread, on its root comment. |
| Resolve, reopen | Change a thread's status. Either side may do it to any thread. |
| Send | Deliver the user's unsent comments and replies to the target agent as one prompt. |
| Unsent | A user comment or reply that no send has carried, or a thread the user reopened since its last send. |
| Edited since sent | A mark on a user comment whose text changed after its last send. It stays sent until the user resends it. |
| Resend | Send an open thread again although it was already sent. |
| New | A marker on a thread an agent resolved that the user has not looked at yet. |
| Target agent | The agent that receives a send for this review. |
| Archive | Move resolved threads out of the review, into `archive.jsonl`. Only the user archives. A thread that is still new stays. |
| Agent message | The text of the newest assistant message in the agent's Claude Code transcript: the text blocks of one message id. Not to be confused with a reply. |
| Message review | The pane that shows an agent message as Markdown lines for the user to comment on. It has no threads and writes nothing to the review. |
| Message comment | A user comment on a line or range of an agent message. It lives only in the message pane until it is sent or the pane closes. |
