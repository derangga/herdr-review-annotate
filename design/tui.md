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
| `filter` | `/` | Narrow the sidebar to the files whose path matches a query |
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
| `collapse` | `z` | Collapse the cursor's file to its header, or open it again |
| `help` | `?` | Show every action with its current keys |
| `quit` | `q` | Quit. With unsent comments, ask send, keep, or stay |

The mouse wheel scrolls (the sideways wheel scrolls the code) and a click moves the cursor. These are not remappable.

## How the body behaves

- The sidebar groups the files under a heading for their directory (`./` for the repository root), in the
  diff's order, so a directory that comes again later gets a second heading. A file row is a `•` when a thread
  hung in it is unsent, a letter for how it changed (M modified, A added, D deleted, R renamed, ? untracked,
  B binary, S submodule, L too large, ! unparsed) in the colour of the change (M warning, A added,
  D and ! removed, R accent, ? agent, B S L subtle), with `[sidebar] icons` on a Nerd Font glyph for the file's type after the letter (`icons.rs`:
  a whole name such as `Cargo.toml` first, then the lower-cased extension, else a generic file glyph) in the
  subtle colour, the file's name, and its added and removed line counts
  against the right edge. The name is cut with `…` before the counts, and the icon always stays. The sidebar is left out below 50
  columns and follows the file under the cursor. That file's row has a bar behind it: the cursor colour
  while the sidebar is focused, and the header colour with bold text while the stream is. With the sidebar focused, `up` and `down` move to the
  previous and next file, and the page keys move by a page of files. A click on a heading does nothing.
  Selecting a file from the sidebar, by key, click or `enter` in the filter, puts its header on the top
  row of the stream, or as near the top as the end of the stream allows.
  The cursor's row of the stream has the cursor colour behind it whichever panel is focused, and its
  text keeps its own colours.
- `filter` puts the cursor in the query box at the top of the sidebar, and the sidebar lists only the files whose path
  matches it. The stream, its cursor, cards, hunk and thread jumps and send are not filtered.
  - A path matches when the query's characters appear in it in order. A query with no upper case letter
    ignores case, and one upper case letter makes it exact. A space is a character of the query. There is no
    score and no highlight. The files keep the diff's order and their directory headings, and a directory
    with no matching file has no heading.
  - While the query takes keys, a character goes into it (so `q`, `j` and `f` do not run their actions),
    `backspace` removes one, `ctrl+u` empties it, and each key narrows the list while the stream stays
    where it is. These keys are fixed and not in `[keys]`. Any other key is ignored.
  - `enter` keeps the filter and the keys are the keymap's again, with the focus on the sidebar. The cursor
    goes to the first match unless its file matches. With no match it is ignored, so the user keeps typing
    or presses `esc`. On an empty query it clears the filter.
  - `esc` clears the filter, while typing and afterwards, from either panel. A visual selection is
    cancelled first and the filter stays. `filter` with a filter applied reopens the same query.
  - With a filter on, the sidebar's `up`, `down` and page keys visit the matching files only, and a click
    selects the file drawn on its row. A cursor on a file that does not match has no highlighted row, and
    `down` goes to the first match after that file and `up` to the last match before it. The mouse never
    changes the filter.
  - The query is a rounded box of three rows at the top of the sidebar. It is drawn whether or not there
    is a filter, so the user sees that the files can be filtered, and it reads as an input field and not
    as a file. Inside it are a `>` prompt in the accent colour, the text cut from the left so its end stays
    in view, a cursor cell while it takes keys, and on the right how many of the diff's files match, as
    `2/4`, in the subtle colour. With an empty query that is not taking keys, the text is the hint
    `filter (/)` in the subtle colour, with the key from the keymap (`filter` alone when the action is
    unbound). The border is the accent colour while the query takes keys and the plain border colour
    otherwise. A list with no match shows `no match` in the subtle colour under the box.
  - `filter` with the sidebar hidden shows it first. In a pane under 50 columns it opens nothing and the
    status line says the pane is too narrow to show the sidebar. A reload, a spec switch or a change from
    an agent keeps the query and matches it against the new files. The filter is not saved. A pane resized under
    50 columns while the query takes keys stops taking them, since the query is no longer drawn.
- `toggle_sidebar` hides the sidebar and shows it again. While it is hidden the stream has the whole
  width and is laid out again, as after a resize, so cards and the editor box are as wide as the pane.
  Hiding it moves the focus to the stream. While the sidebar is not drawn, because it is hidden or the
  pane is under 50 columns, `switch_panel` does nothing and the focus stays on the stream. In a pane
  under 50 columns the key still flips the state, which shows once the pane is wider, and the status
  line says the pane is too narrow to show the sidebar. The state is not saved: `areas` in `view.rs` is
  the one place that decides whether the sidebar is drawn, from the state and the width.
- `collapse` folds a file of the stream to its header row, to get a lockfile or a generated file out of the
  way. It acts on the file the cursor is in, from any of its rows and cards, and on the file chosen in the
  sidebar while the sidebar has the keys. From inside the file the cursor goes to the header. The same key
  on a collapsed file opens it.
  - Every header starts with `▾`, and a collapsed one with `▸`. After its name a collapsed header says
    what it hides, in the subtle colour, as `120 lines, 2 threads`: the rows of its hunks and the threads
    placed in it. A part that is zero is left out, and the text gives way to the name and the counts in a
    narrow stream. The sidebar draws a collapsed file's name in the subtle colour.
  - The cards of its threads are hidden with its rows. The threads are still in the review: `send` sends
    them, the unsent chip counts them and the sidebar keeps its `•`. `reply`, `edit`, `delete`, `resolve`
    and `resend` on the header find no thread.
  - Three things open a collapsed file: the key, a thread jump, and saving a file comment written on its
    header. The hidden threads are at the header for `next_thread` and `prev_thread`. A jump that lands
    there opens the file and goes to a card: the file's first thread going forward, also from the header
    itself, and its last going back. An agent's reply, a reload and a mouse click do not open it.
  - The collapsed files are kept by path in `View::collapsed` for the pane's session. A reload, a spec
    switch, a layout change and a store change leave them collapsed, and a file that leaves the diff and
    comes back is still collapsed. Nothing is written to `meta.json`, so a new pane has every file open.
  - Hunk jumps pass over a collapsed file, since it has no hunk row. `comment` on its header writes a file
    comment, as on any header. The key ends visual mode, as every new layout does. With the cursor on the
    block of comments not in the diff, or with no file in the diff, the status line says
    `no file here to collapse`.
  - A collapsed file is not read for syntax colours while it is collapsed.
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
