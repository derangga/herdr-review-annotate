# The review pane

Part of the [herdr-review design](../DESIGN.md).

One thread, one loop: `event::poll(250 ms)`, then the store length check. Layout: file sidebar, one stream of all files, comment cards under their lines, unified view
in a unified or a side-by-side layout.

| Action name | Default keys | What it does |
|---|---|---|
| `up`, `down` | `k` `up`, `j` `down` | Move the cursor one row |
| `page_up`, `page_down` | `ctrl+u`/`pageup`, `ctrl+d`/`pagedown` | Scroll one page |
| `prev_hunk`, `next_hunk` | `[`, `]` | Previous or next hunk |
| `prev_thread`, `next_thread` | `shift+n`, `n` | Previous or next thread |
| `scroll_left`, `scroll_right` | `h` `left`, `l` `right` | Scroll the code of the diff 8 cells sideways |
| `scroll_reset` | `0` | Scroll back to the start of the lines |
| `switch_panel` | `tab` | Switch between sidebar and stream |
| `toggle_sidebar` | `f` | Show or hide the sidebar |
| `comment` | `c` | Comment on the line, the selected range, or the file when the cursor is on a file header |
| `select_range` | `v` | Start or leave visual mode, to select a range |
| `reply` | `r` | Reply to the focused thread |
| `edit`, `delete` | `e`, `d` | Edit or delete your own focused comment |
| `resolve` | `x` | Resolve or reopen the focused thread |
| `archive` | `shift+a` | Move the resolved threads out of the review, after asking |
| `send` | `shift+s` | Send unsent |
| `resend` | `s` | Resend the focused thread |
| `reload` | `shift+r` | Reload the diff |
| `switch_spec` | `b` | Switch diff spec |
| `toggle_layout` | `t` | Side by side or unified |
| `help` | `?` | Show every action with its current keys |
| `quit` | `q` | Quit. With unsent comments, ask send, keep, or stay |

The mouse wheel scrolls (the sideways wheel scrolls the code) and a click moves the cursor. These are not remappable.

## How the body behaves

- The sidebar groups the files under a heading for their directory (`./` for the repository root), in the
  diff's order, so a directory that comes again later gets a second heading. A file row is a `•` when a thread
  hung in it is unsent, a letter for how it changed (M modified, A added, D deleted, R renamed, ? untracked,
  B binary, S submodule, L too large, ! unparsed) in the colour of the change (M warning, A added,
  D and ! removed, R accent, ? agent, B S L subtle), the file's name, and its added and removed line counts
  against the right edge. The name is cut with `…` before the counts. The sidebar is left out below 50
  columns and follows the file under the cursor. With the sidebar focused, `up` and `down` move to the
  previous and next file, and the page keys move by a page of files. A click on a heading does nothing.
- `toggle_sidebar` hides the sidebar and shows it again. While it is hidden the stream has the whole
  width and is laid out again, as after a resize, so cards and the editor box are as wide as the pane.
  Hiding it moves the focus to the stream. While the sidebar is not drawn, because it is hidden or the
  pane is under 50 columns, `switch_panel` does nothing and the focus stays on the stream. In a pane
  under 50 columns the key still flips the state, which shows once the pane is wider, and the status
  line says the pane is too narrow to show the sidebar. The state is not saved: `areas` in `view.rs` is
  the one place that decides whether the sidebar is drawn, from the state and the width.
- Next and previous hunk and thread move to the next row after, or the previous row before, the cursor. From
  inside a hunk, previous hunk goes to that hunk's own header. A thread is at the first row of its card.
- A file with no hunks has one row that says why (binary, too large, unreadable, mode changed).
- The sideways keys move the code of every diff row 8 cells at a time, in the unified and the split layout
  alike, and stop where the widest line of the diff ends. The line numbers, the sign, hunk headers and cards
  stay where they are. A `‹` takes the first cell of a row cut off on the left, and a `›` the last cell of a
  row cut off on the right. The offset starts over when the cursor moves to another file, and the sidebar
  ignores the keys.
- The wheel moves three rows. A click in the stream or the sidebar moves the cursor there and focuses it.
- The status line is a bar in the theme's header colour across the whole line. Its left part is the state:
  the diff on screen as a chip (` WORKING TREE ` or ` VS MAIN `, upper case, base colour on the accent
  colour), then `→ claude w8G:p1` in the text colour or `→ no agent` in the removed colour, then a chip
  ` 3 unsent ` on the warning colour when a thread is unsent. A chip is coloured cells with one space of
  padding on each side and no border glyphs.
- Against the right edge the line names the keys of `toggle_sidebar`, `send`, `resolve`, `reload`,
  `help` and `quit` as the keymap has them: `f sidebar  S send  x resolve  R reload  ? help  q quit`. The six
  need 54 cells. `sidebar` comes first, so it is the first to drop: an 80 column pane with an agent
  named on the left shows the keys from `send` on. A key is bold in the accent colour and
  its label is in the subtle colour. The keys keep two cells clear of the left part. When they do not fit,
  they drop off from the left, so `help` and `quit` go last.
- A message takes the place of the state and the keys stay. It is a notice in the warning colour for
  something that happened or that the cursor's place does not allow (`sending`, `deleted u2`, `no thread
  here`), and a failure in the removed colour for something that went wrong (a reload error, a refused
  send, a busy review, a write that failed). The next action clears it.
- `help` opens an overlay of every action, and any key closes it without doing anything else.
- `switch_spec` flips between the working tree and the branch spec and saves the choice, with the base
  (`meta.base`, else `origin/HEAD`, `main`, `master`), in `meta.json`. With no base it says so and stays.
- A reload puts the cursor back on its row in the same file, or on its line of the same card.

## Rules

- Every string from the store or from `git` passes through `sanitize_terminal_text` before it is drawn.
- `c` captures the anchor when it is pressed. The diff is frozen while the editor is open.
- A thread is marked seen, with one `seen` event, when the cursor lands on it while it is `new`.
- The `outdated` tag is drawn on open threads only.
- The status line is the state on the left and the keys on the right, drawn in roles of `Theme` and
  never reversed. Every message that replaces the state is a notice or a failure, and the place that
  sets it says which.
- The target is resolved at start and after each send, not on every frame.
- Rendering builds rows for the visible window only. Each file keeps its row count so scrolling does
  not lay out files that are off screen.
