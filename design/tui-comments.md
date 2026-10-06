# The review pane: cards, comments, archive and send

Part of the [herdr-review design](../DESIGN.md).

## How the cards behave

- A card is drawn under the line its thread is placed at, or under the file header for a file comment. An
  outdated thread is drawn under the nearest row, or under the file header when the file has no row on that
  side. Several threads on one line stack in the review's order. Where the box goes and how wide it is
  are in `tui-layout.md`.
- An open card is a rounded box, in the theme's warning colour for the user's thread and its agent colour for
  an agent's. The top border reads `● <author> · <age> · <path> R<line>`: the author is `Your note` for the
  user and the agent's name otherwise (`agent` when it has none), the side letter is `L` or `R`, a range is
  `R101-110`, and a file comment has the bare path. Then come the badges `[outdated]` (on an open thread
  only), `[new]`, `(edited since sent)` and `[unsent]`. A path that does not fit is cut from the left with `…`,
  and is left out when there is no room for it. The id is not on an open card.
- Inside the box are an empty row, then the line text as it was when the comment was written, for a thread
  that is outdated or not in the diff, then the body wrapped to the box, then each reply indented under it.
  The text keeps a cell clear of each side.
- The bottom border holds the keys on the right, read from the effective keymap: `reply`, `edit` and `delete`
  for the user's thread, and only `reply` for an agent's. Each is the action's first key as the keymap prints
  it, so the defaults read `r reply  e edit  d delete`, and an action with no key is left out.
- The age is `now` under a minute, then `2m`, `3h`, `2d`, counted from the `at` of the comment's `add` event
  to the time the stream was laid out. No timer runs, so the age on an idle pane is as old as its last layout.
  A comment whose time is not RFC 3339 has no age.
- A resolved thread is one line: a green `✓` in place of the bullet, the id, `[new]`, who resolved it and the
  first line of its last comment, with no `outdated` tag.
- Threads that are not in the diff, because their file is not in it or they were written against the other
  spec, are listed in a block at the top of the stream under "Comments not in this diff". A resolved one
  also says the path and line it pointed at, which an open card has on its border. With an empty diff the block is shown above the message. A
  thread in the block is reached with next and previous thread, like any other.
- The thread a key acts on is the one whose card holds the cursor, or the first one hung under the line the
  cursor is on. A card's height is its number of lines, so the stream is laid out again when the pane's
  width changes. Both borders of a box belong to its root comment, so `edit` and `delete` on the bottom
  border act on the root.

## How comments are written

- `comment` reads what the comment points at when the key is pressed and keeps it until the text is saved. On
  a diff line it is that line, on the new side, or on the old side for a removed row. On a file header, or the
  note of a file with no rows, it is the file. A hunk header is refused. On a card it is the row the card
  hangs under, and a card in the not-in-diff block is refused.
- `select_range` enters and leaves visual mode. Visual mode is a selection being open, `View::select`, and has
  no flag of its own. It starts at the cursor's row and the half the last click chose, and a second press
  drops it. `Esc` also leaves it. `Esc` is a fixed key like the editor's and does nothing outside visual
  mode. Every other action still works there, and motions grow the range. The range runs to the cursor, stays
  inside one file, takes the side of its first line, ends at the last line on that side, and keeps the text
  of its first line. On a split row, the half the selection started on is that side, whichever way the
  cursor moves, provided the first row has a line there. A reload, `switch_spec`, `toggle_layout` and a
  resize drop it. `comment` uses it and ends it. When the range cannot be commented on, `comment` says why
  and visual mode stays.
- In visual mode the status line has the selection colour behind the whole line. The left is a ` VISUAL `
  chip in the visual colour (peach, on the base colour), then where a comment would point and how many lines
  it covers, as `src/a.rs R12-18 (7 lines)`, `(1 line)` for one line, and the bare path for a file. When
  `comment` would be refused, the reason takes that place in the removed colour. The spec chip, the agent and
  the unsent chip are hidden until visual mode ends. A message still takes the left side. The right side is
  `c comment  v/esc cancel`, with the first key of `comment` and `select_range` from the keymap, and it drops
  off from the left like the other keys.
- The editor is a rounded box in the theme's warning colour, placed and sized as the card of the saved
  comment will be. Its text has an empty row above it and a cell clear of each side. It is drawn under the
  cursor row, or under the last row of the range a new comment points at when that is lower,
  and above that row when there is no room below. It grows with its text up to two thirds of the stream's
  height. `Ctrl+S` saves and `Esc` cancels.
- The top border reads `Draft note - <path> R<line>` for a new comment: `L` for the old side, `R101-110` for a
  range, and the bare path for a file comment. A path that does not fit is cut from the left with `…`. A reply
  reads `Reply to <id>` and an edit reads `Edit <id>`. The bottom border holds `^S save  Esc cancel` on the
  right. An empty editor shows a dim `Write a note…`.
- While the editor of a new comment is open, the row or range it points at has the selection tint and a bar in
  its first cell, and no `[+]` is drawn. The bar gives way to a digit of a line number.
- A failed save keeps the editor open with its text, and the reason replaces the keys on the bottom border and
  is on the status line. The diff is not reloaded while the editor is open: a store change, `reload` and
  regaining focus wait until it closes. Visual mode follows the same rule for a store change and for
  regaining focus, which do nothing until it ends, so the range is not dropped under the cursor. `reload`
  is a key the user presses, so it still works there and ends visual mode.
- A saved comment, reply, edit, delete, resolve or reopen is written with `actions.rs`, then the pane reads the
  log again and lays the stream out. It does not reload the diff. A new comment or reply moves the cursor to
  its thread's card.
- `reply` replies to the thread the cursor is on. `edit` and `delete` act on the comment whose line of the card
  the cursor is on, or on the root when it is on the line above the card. They refuse an agent's comment.
  Deleting a root deletes its thread and nothing asks first. `resolve` flips any thread, whoever wrote it.
- A store change is read within one tick, 250 ms, whoever wrote it, and the diff is reloaded with it. A log
  that is shorter than the one read, or gone, is read again from the start.
- A resolved card says `[new]` while an agent's resolve has not been looked at. After a key or a click that
  leaves the cursor on a new thread, on its card or on the line it hangs under, one `seen` event is written.
  The marker stays cleared after a restart, and moving around the thread writes nothing more. A write that
  fails is a warning, and the next key tries again.
- When the pane is told to end, or its terminal fails, with text in the editor, the text is written as the
  comment, reply or edit it was for.

## How archive behaves

- Resolved threads leave the pane only through `archive`. Nothing is archived or hidden on its own.
- `archive` takes every resolved thread except the ones still marked `[new]`, so an agent's resolve the
  user has not looked at stays. A resolved thread that was never sent goes too.
- It asks first, in a box like the quit prompt: `archive 3 resolved threads (1 never sent)?`, then
  `[y] yes` and `[n] no`. The part in brackets is there only when some were never sent. These two
  keys are not remappable, and any other key leaves the question open. The numbers are counted from the
  review when the box is drawn.
- With nothing to archive the status line says `nothing to archive` and no box opens.
- After `y` the status line says `archived 3`, with the number the store moved, and the pane reads the log
  again. The threads are gone from the pane, from the unsent count and from the agent's `comment list`.
  A busy review or a failed write is a failure on the status line, and the threads stay.
- There is no viewer, no unarchive and no agent command for it. Only the user archives.

## How send behaves

- `send` sends the unsent threads and `resend` sends the thread under the cursor again, with the text it has
  now. A resolved thread is not resent. Both draw "sending" before the Herdr call, since the call blocks the
  pane.
- Several matching agents open a picker (`up`, `down` or `k`, `j`, then `enter`, `esc` cancels). The choice is
  saved in `meta.json`, so the next send does not ask, and the send that asked runs at once.
- A refusal is on the status line and in a notification, and nothing is marked sent.
- `quit` with unsent comments asks: `s` sends and then quits (a refusal keeps the pane open), `k` quits and
  keeps them unsent, `esc` stays. These keys are not remappable.
- The status line names the target as `→ claude w1:p2`, or `→ no agent`. The target is resolved when the
  pane starts and after each send, and what is found is saved for the `send` action.
