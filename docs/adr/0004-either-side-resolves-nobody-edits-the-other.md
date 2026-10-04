---
status: accepted
---

# Either side resolves any thread, nobody edits the other side's words

The user and the agent have the same rights. Either can add comments and replies, and either can
resolve or reopen any thread at once, with no approval step. Each can edit or delete only its own
comments. There is one resolved state, and the event records who resolved it. A thread the agent
resolved shows a `new` marker until the user has looked at it.

We chose this over a two-step "agent proposes, user confirms" flow because the confirm step would put
back a manual action for every comment, which is the cost the plugin exists to remove. This is the
opposite of hunk, where agents may delete user notes but cannot create or edit them.

An agent resolve must carry a reply (`comment resolve <id> --reply -`), so the `new` marker always has
the agent's explanation beside it.

## Consequences

- An agent can resolve a thread it did not fix. The reply and the `new` marker are the only checks.
  The user reopens it, and a reopened thread goes out with the next send, marked as reopened.
- There is no automatic loop, because the agent acts only after the user sends.
- Status belongs to the thread's root comment. Replies have no status of their own.
