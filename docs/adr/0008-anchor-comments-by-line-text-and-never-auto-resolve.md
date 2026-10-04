---
status: accepted
---

# Anchor comments by line text and never resolve them automatically

The diff changes while a review is open, because the agent is fixing what the comments point at. A
comment stores its path, side, line number and the text of that line. After a reload a comment matches
when the same position holds the same text. Otherwise we look for the nearest line in that file's diff
with identical text. Otherwise the comment is outdated.

An outdated comment is kept, keeps its original line text, and is never resolved or deleted by the
tool. Only the user or the agent resolves. A comment whose file is no longer in the diff is listed in a
"comments not in this diff" block at the top of the review, so it stays reachable after a commit
empties the diff.

## Considered options

- **Resolve a comment when its line changes.** Rejected. A changed line is not proof of a correct fix.
- **A hash of the line plus surrounding context.** Rejected. An edit to a neighbouring line changes
  the hash although the commented line is untouched, and the line text has to be stored anyway to
  show it on an outdated comment.

## Consequences

- A correct fix changes the anchored line, so most handled comments become outdated. The `outdated`
  tag is shown on open threads only. On an open thread it means "the code changed and nobody
  resolved this", which is worth the user's attention.
- Only lines inside diff hunks can be searched. A commented line that moves outside every hunk
  becomes outdated.
