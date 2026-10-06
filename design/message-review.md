# Message review

Part of the [herdr-review design](../DESIGN.md).

Decided in the grilling session of 2026-10-06. Plannotator's `herdr last` (plannotator-tui v0.9.4) is
the model, read for this section and not copied. The user comments on the agent's newest message
the way they comment on a diff, and one key sends the comments back as the agent's next prompt.
ADR 0010 says why the comments are not threads.

## Scope

| Decision | Choice |
|---|---|
| Where | A `message` action and a `message` pane entrypoint in this plugin and binary. It reuses the editor, keymap, theme, syntax colours and `agent_delivery.rs`, and has its own line view in place of the diff stream |
| Which message | The newest assistant message of the agent's Claude Code transcript: the text blocks of one `message.id`. Interim text between tool calls belongs to earlier ids and is not shown. There is no picker |
| Agents | Claude Code only. Another agent gets a notification and nothing opens |
| Comments | Kept in the pane's memory. Nothing is written to `review.jsonl`. Closing the pane without sending loses them |
| Git | Not needed. The pane works in any folder an agent runs in |
| Pane | Split to the right of the agent pane. A second press focuses the open pane |
| Agent still working | The pane opens on the newest message and the status line says the agent is working |
| After send | The pane closes. A refused send keeps the pane open with the reason |
| Started by the agent | No. The skill is unchanged |
| Herdr key | The README suggests `prefix+o`. The user's config binds it to herdr-annotate today and will drop that plugin once this ships |

## Finding the message

`herdr agent get <pane>` gives `agent`, `agent_status`, `terminal_id`, `cwd` and `agent_session`. On
Herdr 0.9.3 with Claude Code, `agent_session` is `{"kind": "id", "value": "<session uuid>"}`, and the
value is the transcript's file name.

1. `agent` is not `claude`: `Unsupported(agent)`.
2. No `agent_session`: `NoSession`.
3. `kind` `path`: use the value when the file exists.
4. `kind` `id`: look for `projects/*/<id>.jsonl` under each root, in order: `$CLAUDE_CONFIG_DIR`, then
   `$HOME/.claude`, then every other directory in `$HOME` whose name starts with `.claude`. The id is a
   uuid, so the first file found is the session. The plugin's environment is Herdr's, not the agent's,
   so `CLAUDE_CONFIG_DIR` is often unset there. The `$HOME/.claude*` scan finds a session kept under
   another config directory, as on this machine (`~/.claude-work`).
5. No file: `NoTranscript { id }`.

Reading the transcript:

- The file is read whole. A long session is a few MB. `ponytail:` read from the end if this ever shows.
- Each line is parsed on its own. A line that is not JSON is skipped.
- An entry counts when `type` is `assistant`, `isSidechain` is not `true`, and `message.model` is not
  `<synthetic>`. Its text is every content block of `type` `text` whose text is not blank. Thinking and
  `tool_use` blocks are dropped.
- Claude Code writes each content block of one API message as its own line, all with the same
  `message.id`. The newest message is the `message.id` of the last entry with text. Its text is every
  text block with that id, in file order, joined by a blank line.
- Newest means last in the file. A rewind can leave a newer branch above an older one. Plannotator
  follows the `parentUuid` chain for this. We do not until it shows.
- No entry with text: `NoMessage { path }`. This is the state right after `/clear`.

## Command line and manifest

```
herdr-review message        the action: find the agent, then focus or open the pane
herdr-review message-tui    the pane
```

```toml
[[actions]]
id = "message"
title = "Review: agent's last message"
contexts = ["pane"]
command = ["./bin/herdr-review", "message"]

[[panes]]
id = "message"
title = "Message"
placement = "split"
command = ["sh", "-c", "exec \"$HERDR_PLUGIN_ROOT/bin/herdr-review\" message-tui"]
```

`herdr pane list` does not say which plugin entrypoint a pane runs. So the pane writes its own
`HERDR_PANE_ID` to `<state base>/message/<agent terminal id>` when it starts, and the action reads that
file to find it. This is the only file the feature writes. It is keyed by the agent's terminal, not by
a repository, so it needs no git. A stale id fails `herdr pane get` and the action opens a new pane.

`Env` keeps `CLAUDE_CONFIG_DIR` next to the `HERDR_*` and `REVIEW_*` variables.

## Pane

- One row per source line of the message, as raw Markdown, wrapped to the pane width. A wrapped line
  is several screen rows with one cursor position. The cursor, visual mode and comments work on source
  lines. A comment on a paragraph quotes the whole paragraph. A column of line numbers sits on the
  left, on the first row of each line, because the prompt cites lines by number.
- Colours come from `syntax::highlight` with the Markdown grammar. Without the `syntax` feature the
  text draws plain.
- A comment draws as a note box under the last line it covers, like a diff thread's card. Several
  comments may cover the same lines. `edit` and `delete` act on the comment whose box hangs under the
  cursor's line, and `next_thread` and `prev_thread` land on one: they go through the comments in
  line order, then in the order written, so with several on one line they step through them. The
  box of that comment is tinted like the cursor's row. `delete` asks nothing.
- Keys come from the same `[keys]` table as the diff pane. The pane acts on `up`, `down`, `page_up`,
  `page_down`, `prev_thread`, `next_thread` (move between comments), `comment`, `select_range`, `edit`,
  `delete`, `send`, `reload`, `help` and `quit`. The rest do nothing, and the footer and help overlay
  leave them out. The mouse wheel scrolls, and a click or a drag selects lines, as in the diff pane.
- `reload` loads the newest message again. With comments it first asks "discard N comments and load
  the newest message?". A failed reload keeps the message on screen and shows the error, and the
  comments too: they are dropped only once the new message is loaded.
- `quit` with comments asks send, discard, or stay.
- Status line: a ` MESSAGE ` chip, `→ claude w8M:p1`, ` 2 comments ` on the warning colour when there
  are any, and ` working ` when `agent_status` was `working` at load. On the right:
  `c comment  S send  R reload  ? help  q quit` with the keys the keymap has.

## Prompt

```
Feedback on your last message. Address each point.

- lines 12-14:
  > ## Step 2: migrate the store
  > Move events into the new file and drop the old one.
  > The fold reads only the new file.
  Why not keep the old file and read both?
- line 30:
  > Use a global lock.
  Fine for now, add a ponytail note.
```

- Comments go in line order, and in the order they were written when they start on the same line.
- A quote holds at most 6 lines. A longer range shows its first 5 lines and then
  `  > … (N more lines)`. The agent has the whole message in its context, so the quote only has to
  locate the comment.
- Each body line is indented by two spaces. `deliver_to_agent` removes ESC, as for diff comments.
- There are no commands in the prompt. The agent answers in its next message, which the user can
  review again.

## Shapes

| Type | Fields or cases |
|---|---|
| `Session` | `Id(String)`, `Path(PathBuf)` |
| `MessageAgent` | `pane: PaneId`, `terminal: TerminalId`, `cwd`, `status: AgentStatus`, `session: Option<Session>`, `name: String` |
| `AgentMessage` | `id: String`, `lines: Vec<String>`, `transcript: PathBuf` |
| `MessageComment` | `start: u32`, `end: u32` (source lines, 1-based, inclusive), `body: String` |
| `MessageError` | `NotAnAgent`, `Unsupported(String)`, `NoSession`, `NoTranscript { id }`, `NoMessage { path }`, `Io { path, kind }`, `Herdr(HerdrError)` |

## Call graphs

### `message` action

```
-> agent pane from HERDR_PLUGIN_CONTEXT_JSON       R: env     E: absent -> notify "focus the agent's pane", exit 1
-> herdr agent get <pane>                          R: herdr   E: not an agent -> NotAnAgent, notify, exit 1
                                                              E: not claude -> Unsupported, notify, exit 1
-> message pane already open?
  -> read <state base>/message/<terminal>          R: env, fs E: absent, unreadable -> continue to open
  -> herdr pane get, herdr plugin pane focus       R: herdr   E: any -> continue to open
-> locate transcript, read newest message          R: env, fs E: NoSession, NoTranscript, NoMessage, Io -> notify, exit 1
-> herdr plugin pane open --entrypoint message
   --placement split --direction right
   --target-pane <pane> --cwd <agent cwd>
   --env REVIEW_DELIVER_TO --env REVIEW_DELIVER_TERM --focus
                                                   R: herdr   E: HerdrError -> notify, exit 1
```

The action reads the message only to refuse early. The pane reads it again.

### Pane start and load

```
-> load keymap, theme                              R: env     E: as in TUI start
-> REVIEW_DELIVER_TO, REVIEW_DELIVER_TERM          R: env     E: absent -> message screen "open it with the message action"
-> write <state base>/message/<terminal>           R: env, fs E: Io -> escape, warning
-> enter raw mode and alternate screen             R: term    scope: restored by the guard, as in the diff pane
-> load
  -> herdr agent get <pane>                        R: herdr   E: gone, terminal differs -> message screen "the agent is gone"
  -> locate transcript, read newest message        R: env, fs E: any MessageError -> message screen with the reason
  -> highlight as Markdown                         (pure)     E: no syntax feature -> plain
```

`reload` runs `load` again. Its failure keeps the old message and shows the error on the status line.

### Loop

The diff pane's loop without the store check: poll keys every 250 ms, apply the action, draw only on a
change. On the termination flag the pane leaves the loop and the comments are lost (ADR 0010).

### Send

```
-> no comments                                     (pure)     -> "nothing to send"
-> format prompt (Prompt)                          (pure)
-> herdr agent get, terminal still matches         R: herdr   E: mismatch -> Refusal::AgentGone
-> deliver_to_agent(Send, pane, text)              R: herdr   E: Refusal, HerdrError -> status line and notification, comments kept
-> leave the loop, exit 0                                     -> Herdr closes the pane
```

## Tests that the graphs give

| Graph | Swapped | Tests |
|---|---|---|
| Find the message | temp `HOME` with transcript fixtures | Not claude, no session, a `path` session, the id under `CLAUDE_CONFIG_DIR`, under `~/.claude`, under `~/.claude-work`, no file, a bad JSON line skipped, sidechain and `<synthetic>` skipped, thinking and `tool_use` dropped, one id over three lines joined, the last id with text wins over a later `tool_use`-only line, no text at all |
| `message` action | recording `herdr`, `Env` literal, temp state base | No context, not an agent, open pane focused, stale pane id opens a new one, no message notifies and opens nothing |
| Pane | `TestBackend`, key list, recording `herdr` | Wrapped line moves as one, a range comment draws under its last line, reload asks when comments exist, a failed reload keeps the message, quit asks with comments |
| Prompt | (pure) | Line order, a 7-line range cut to 5 plus the count, a multi-line body indented |
| Send | recording `herdr` | Refused keeps the comments and the pane, success exits, a changed terminal refuses |

Manual checks in Herdr:

- With Claude Code idle, `prefix+o` opens a split on the agent's final message. A second press focuses
  it.
- Comment on one line and on a range, then press `S`. The agent receives the prompt and the pane
  closes.
- With the agent working, the pane opens with ` working ` and `S` queues the prompt.
- With Claude Code started under `CLAUDE_CONFIG_DIR=~/.claude-work`, the message is found.
- In a Codex or shell pane, `prefix+o` shows a notification and opens nothing.
- After `/clear`, `prefix+o` says the session has no message yet.
