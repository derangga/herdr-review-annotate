# The review pane: layout and syntax colours

Part of the [herdr-review design](../DESIGN.md).

## How the layout behaves

- A pane 120 columns wide or more draws the diff side by side, old on the left and new on the right, and a
  narrower one draws it unified. `toggle_layout` forces the other layout until it is pressed again or the pane
  restarts, and the width stops deciding. The cursor stays on its line across the change.
- Side by side pairs a run of removed lines with the run of added lines after it, line by line. The longer
  run's extra lines sit opposite an empty half. A context line is on both halves with its own numbers.
- Side by side marks the lines the diff leaves out, above the first hunk and between hunks, with a row that
  says `▾ N unchanged lines`. Unified does not, so its rows are the diff's rows.
- A file header has a `▾`, the letter of its change and its name on the left, and the added and removed
  counts on the right. A collapsed file's header is in `tui.md`. Removed and added
  rows are tinted across the row.
- A removed line and the added line that replaced it show which words changed, in both layouts. The pairs
  are the ones side by side draws on one row: a run of removed lines pairs with the run of added lines
  after it, the first with the first. A line of the longer run with no partner has no marks.
  - `changed` in `words.rs` cuts both lines into tokens: a run of letters, digits and underscores, a run of
    spaces, and every other character on its own. The tokens of a line that are not in the longest
    sequence the two lines share are marked. Marked tokens next to each other are one mark, and a change
    of indent marks the spaces.
  - A mark is a stronger background than the tint: the flavor's green or red mixed 35 parts in a hundred
    into its base (`added_word` and `removed_word` in `Theme`). The text keeps its colour, from the syntax
    tokens or from the row's kind.
  - A pair gets no marks when under half of its tokens that are not spaces are common to both lines,
    counted against the line that has more of them. Such a line was rewritten, and it draws as a row with
    no partner does. A line of more than 200 tokens gets none either, since the comparison builds a table
    with a cell for every pair of tokens.
  - The cursor's row, the rows of a selection and the rows the open editor comments on keep the marks: the
    bar colours every cell of the row but the marked ones (`bar` in `view.rs`).
  - The marks are computed when a row is drawn, for the rows in the window, and nothing is kept. They
    follow the sideways scroll with the code. There is no key and no config entry to turn them off, and
    they do not need the `syntax` feature.
- A row of a split diff has two halves. `comment` and `select_range` use the new half, or the old half of a
  row with no new line. A click on a half chooses it for that row until the cursor moves. A range takes the
  side of its first row and ends at the last line on that side.
- A card and the editor box hang under their row where `note_box` in `view.rs` puts a note, as hunk does.
  Side by side, in a stream of 84 cells or more, the box is the half its line is on: the old half for a
  comment on the old side and the new half for one on the new side. In the unified layout, in a narrower
  split stream, for a file comment and in the block of threads not in the diff, the box starts four cells
  into the stream and runs to its right edge. It is never narrower than 28 cells unless the stream is.
- A code row under the mouse shows `[+]` in its gutter, in both layouts. In a split row it is on the half under
  the mouse, where the sign is, so the line number stays visible. A click on it opens the comment editor on that
  line and half, as `comment` does. Until the mouse has moved once, which tells the pane that Herdr delivers
  motion, the `[+]` is on the cursor's row instead, and a click elsewhere only moves the cursor.
- A drag selects a range, as in Neovim. A left press in the stream moves the cursor and starts a gesture. The
  first `Drag(Left)` of the gesture enters visual mode with the press row as the start, even when it stays
  on that row, and each later drag moves the cursor to the row it is on, whatever the column. Terminals send
  a drag only when the cell changes, so a click that does not move never selects. A drag above the stream
  scrolls up one row and puts the cursor on the top visible row, and one below scrolls down one row and
  puts it on the bottom visible row. That is one row per event, with no timer, and it works while the pointer
  is over the footer or outside the pane, since Herdr keeps sending drags. The selection follows the mouse
  onto cards, file headers and other files as `j` and `k` do, and the footer shows the reason a range
  across files cannot be commented. `Up(Left)` ends the gesture and visual mode stays. A drag with no
  gesture behind it, because the press was in the sidebar or outside the stream, does nothing. A plain press
  in the stream while in visual mode leaves it and moves the cursor. A press on the `[+]` keeps the selection
  and the cursor moves to that row, so `comment` covers the range from where it started to that row. A
  reload ends the gesture too.

## How syntax colours behave (behind the `syntax` feature)

- A code row is drawn token by token. `syntect` only splits a line into scopes. The colour of a token comes
  from the theme: `SCOPES` in `theme.rs` maps a TextMate scope and everything under it to one of seven tokens
  (comment, string, number, keyword, operator, function, type), and the theme has one colour for each, so the
  colours follow the flavor. The innermost scope with an entry decides. Text between tokens is the theme's
  text colour.
- A token's colour replaces the green or red of an added or removed row. The sign keeps that colour, and the
  tint stays behind the whole row, but for the words marked as changed.
- The language comes from the file's name, then its extension. A file in a language `syntect` and `two-face`
  do not know is drawn as before, each row in the colour of its kind.
- A file is highlighted against its whole text, so a row inside a block comment or a multi-line string is
  coloured as one. The new side is read from the work tree. The old side is `git show <rev>:<path>`, with the
  old path of a renamed file, where `rev` is what the diff was taken against and `Diff` keeps. A side with no
  row of its own is not read: context rows use the new side.
- A row takes the tokens of its line only when that line of the file still reads as the row does. A file
  edited after the diff was loaded draws the rows that moved plain until the next reload.
- A side that cannot be read, is not UTF-8, or is over 1 MiB is highlighted hunk by hunk instead, each hunk
  as a snippet of its rows on that side. A snippet cannot know it starts inside a comment or a string.
- A file is highlighted after one of its rows first came into the window, and never before a frame. The
  frame of a key is drawn with what is highlighted so far, so a file that was just reached draws plain. While
  no key is waiting the loop then highlights the files on screen in slices of 8 ms, the cursor's file first,
  and draws again after each slice. A file is read in its first slice, and its lines are parsed from the top,
  the new side before the old, so its colours arrive from the top down. A slice parses at least one line, so
  one very long line can take longer than a slice.
- A reload against the same `rev` keeps the tokens of each file whose `DiffFile` did not change, half
  highlighted or whole, and forgets the rest. A reload against another `rev` forgets them all. Only the tokens
  of the lines the diff shows are kept.
- A build without the feature knows no language, reads nothing, and draws every row as before.
