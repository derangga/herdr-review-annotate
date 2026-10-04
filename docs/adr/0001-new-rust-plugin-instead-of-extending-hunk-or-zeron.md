---
status: accepted
---

# Build a new Rust plugin instead of extending hunk or zeron

The user wants to review a diff, comment, and send the comments to the agent with one key. hunk already
has the review UI and an agent CLI, and zeron already has the "comment, then send" flow. We build a new
single Rust binary, `herdr-review`, that runs as a Herdr plugin pane, because neither reference can
deliver the send step inside a terminal multiplexer at an acceptable cost.

## Considered options

- **Extend hunk.** It is TypeScript on Bun with a loopback daemon, a caller credential and signed
  responses (`hunk/docs/agent-workflows.md:24-28`). Nothing in it pushes user notes to an agent, and
  adding that means teaching the daemon about Herdr. The daemon and runtime are the cost the user wants
  to avoid.
- **Extend zeron.** It is a gpui desktop app and cannot run in a terminal pane.
- **New binary, chosen.** `herdr-annotate` already proves the shape: a Rust binary shipped as a Herdr
  plugin that delivers text with `herdr agent prompt`. We copy its delivery code and port zeron's prompt
  block format.

## Consequences

We rebuild a diff viewer that hunk already has. Features hunk has and we do not plan (highlight marks,
markup notes, agent-driven navigation and reload) stay out unless the user asks for them.
