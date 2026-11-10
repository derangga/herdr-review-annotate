# Context: herdr-review

Glossary only. Decisions are in `docs/adr/`, the design is in `DESIGN.md`.

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
| Unsent | An open thread with a user comment or reply that no send has carried, or that the user reopened since its last send. A resolved thread is never unsent: send skips it and no count shows it, and reopening it makes it unsent again. `archive` still reports the resolved threads it took that were never sent. |
| Edited since sent | A mark on a user comment whose text changed after its last send. It stays sent until the user resends it. |
| Resend | Send an open thread again although it was already sent. |
| New | A marker on a thread an agent resolved that the user has not looked at yet. |
| Unread | The word the panel uses for a resolved thread that is still new: it is what `archive` leaves. `state --json` counts them as `unread_resolved`. |
| Target agent | The agent that receives a send for this review. |
| Archive | Move resolved threads out of the review, into `archive.jsonl`. Only the user archives. A thread that is still new stays. |
| Agent message | The text of the newest assistant message in the agent's Claude Code transcript: the text blocks of one message id. Not to be confused with a reply. |
| Message review | The pane that shows an agent message as Markdown lines for the user to comment on. It has no threads and writes nothing to the review. |
| Message comment | A user comment on a line or range of an agent message. It lives only in the message pane until it is sent or the pane closes. |
| Panel | The herdr-app mod that shows a review: the files, the diff and the threads. A second front end to the same review as the TUI, and it runs the binary for every read and write. |
| Backend | The terminal CLI that `send` and target resolution talk to: `herdr` in plain Herdr, `hc` under herdr-app. |
| Terminal address | hc's one name for a terminal (`t7`). The target agent under the hc backend is stored as an address, and it is assumed not to survive a restart of herdr-core. |

