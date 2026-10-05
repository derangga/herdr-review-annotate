//! The review body: one stream of every file's rows, the cursor in it, the sidebar, and how they
//! are drawn.
//!
//! The stream is a flat list of rows numbered from 0: each file's header, then its hunks, each a
//! header and its lines. `Stream` keeps only where each file starts, so a row is looked up when it
//! is drawn and nothing is laid out for files that are off screen.

use std::fmt::Write as _;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::diff::{Change, Diff, DiffFile, Hunk, Placement, Row, RowKind, place};
use crate::keymap::{Action, Keymap};
use crate::store::{AnchorTarget, RelPath, Review, Side, Spec};
use crate::tui::sanitize_terminal_text;
use crate::width::{char_width, string_width, truncate_to_width};

/// Rows the mouse wheel moves per notch.
const WHEEL_ROWS: usize = 3;

/// Which half of the body takes the navigation keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Sidebar,
    Stream,
}

/// What one row of the stream is.
#[derive(Debug, Clone, Copy)]
pub enum RowRef<'a> {
    File(&'a DiffFile),
    /// The one row of a file with no hunks, which says why.
    Note(&'a DiffFile),
    Hunk(&'a Hunk),
    Line(&'a Row),
}

/// How many rows a file takes: its header, and then its hunks or one note.
fn file_len(file: &DiffFile) -> usize {
    1 + if file.hunks.is_empty() {
        1
    } else {
        file.hunks.iter().map(|hunk| 1 + hunk.rows.len()).sum()
    }
}

/// Why a file has no rows.
fn note(file: &DiffFile) -> &'static str {
    match file.change {
        Change::Binary => "binary file, not shown",
        Change::TooLarge => "too large, not shown",
        Change::Unparsed => "could not read this change",
        Change::Untracked => "untracked, nothing to show",
        Change::Renamed => "renamed, no other change",
        _ if file.flags.mode_changed => "mode changed",
        _ => "no changes to show",
    }
}

fn glyph(change: Change) -> char {
    match change {
        Change::Modified => 'M',
        Change::Added => 'A',
        Change::Deleted => 'D',
        Change::Renamed => 'R',
        Change::Untracked => '?',
        Change::Binary => 'B',
        Change::Submodule => 'S',
        Change::TooLarge => 'L',
        Change::Unparsed => '!',
    }
}

/// Where the rows of the diff start, and the rows the navigation keys jump to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stream {
    /// The row each file starts at.
    starts: Vec<usize>,
    total: usize,
    hunk_rows: Vec<usize>,
    /// The row each thread is drawn at, in row order.
    thread_rows: Vec<usize>,
    /// One per thread of the review, in the review's order.
    pub placements: Vec<Placement>,
}

impl Stream {
    pub fn build(diff: &Diff, review: &Review) -> Self {
        let mut stream = Self::default();
        for file in &diff.files {
            stream.starts.push(stream.total);
            let mut at = stream.total + 1;
            if file.hunks.is_empty() {
                at += 1;
            }
            for hunk in &file.hunks {
                stream.hunk_rows.push(at);
                at += 1 + hunk.rows.len();
            }
            stream.total = at;
        }
        stream.placements = review
            .threads
            .iter()
            .map(|thread| place(thread, diff))
            .collect();
        let mut rows = review
            .threads
            .iter()
            .zip(&stream.placements)
            .filter_map(|(thread, placement)| {
                let file = diff.file_index(&thread.anchor.path)?;
                let line = match placement {
                    Placement::Matched { line } => *line,
                    Placement::Outdated { near } => *near,
                    Placement::NotInDiff => return None,
                };
                let side = match &thread.anchor.target {
                    AnchorTarget::Line { side, .. } | AnchorTarget::Range { side, .. } => *side,
                    AnchorTarget::File => Side::New,
                };
                Some(
                    line.and_then(|line| stream.row_of(diff, file, side, line))
                        .unwrap_or_else(|| stream.starts.get(file).copied().unwrap_or(0)),
                )
            })
            .collect::<Vec<_>>();
        rows.sort_unstable();
        rows.dedup();
        stream.thread_rows = rows;
        stream
    }

    pub const fn len(&self) -> usize {
        self.total
    }

    pub const fn is_empty(&self) -> bool {
        self.total == 0
    }

    pub const fn files(&self) -> usize {
        self.starts.len()
    }

    /// The row of line `line` on `side` of file `file`.
    fn row_of(&self, diff: &Diff, file: usize, side: Side, line: u32) -> Option<usize> {
        let mut at = self.starts.get(file)? + 1;
        for hunk in &diff.files.get(file)?.hunks {
            at += 1;
            for (offset, row) in hunk.rows.iter().enumerate() {
                if row.line(side) == Some(line) {
                    return Some(at + offset);
                }
            }
            at += hunk.rows.len();
        }
        None
    }

    /// The file a row belongs to.
    pub fn file_at(&self, row: usize) -> usize {
        self.starts
            .partition_point(|&start| start <= row)
            .saturating_sub(1)
    }

    pub fn file_start(&self, file: usize) -> Option<usize> {
        self.starts.get(file).copied()
    }

    pub fn locate<'a>(&self, diff: &'a Diff, row: usize) -> Option<RowRef<'a>> {
        if row >= self.total {
            return None;
        }
        let index = self.file_at(row);
        let file = diff.files.get(index)?;
        let mut offset = row - self.starts.get(index)?;
        if offset == 0 {
            return Some(RowRef::File(file));
        }
        offset -= 1;
        if file.hunks.is_empty() {
            return Some(RowRef::Note(file));
        }
        for hunk in &file.hunks {
            if offset == 0 {
                return Some(RowRef::Hunk(hunk));
            }
            offset -= 1;
            if let Some(line) = hunk.rows.get(offset) {
                return Some(RowRef::Line(line));
            }
            offset -= hunk.rows.len();
        }
        None
    }
}

/// Where each part of the screen goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Areas {
    pub sidebar: Option<Rect>,
    pub stream: Rect,
    pub warnings: Rect,
    pub status: Rect,
}

/// Narrower than this and the sidebar is left out.
const SIDEBAR_MIN_TOTAL: u16 = 50;

pub fn areas(area: Rect) -> Areas {
    let [body, warnings, status] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    if area.width < SIDEBAR_MIN_TOTAL {
        return Areas {
            sidebar: None,
            stream: body,
            warnings,
            status,
        };
    }
    let [sidebar, stream] = Layout::horizontal([
        Constraint::Length((area.width / 4).clamp(14, 32)),
        Constraint::Min(1),
    ])
    .areas(body);
    Areas {
        sidebar: Some(sidebar),
        stream,
        warnings,
        status,
    }
}

/// The cursor, the scroll position, the focus, and the help overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub stream: Stream,
    pub cursor: usize,
    pub scroll: usize,
    pub panel: Panel,
    pub help: bool,
    pub area: Rect,
}

impl Default for View {
    fn default() -> Self {
        Self {
            stream: Stream::default(),
            cursor: 0,
            scroll: 0,
            panel: Panel::Stream,
            help: false,
            area: Rect::default(),
        }
    }
}

impl View {
    fn height(&self) -> usize {
        usize::from(areas(self.area).stream.height).max(1)
    }

    /// Where the cursor is, as a file and a row inside it, so a reload can put it back.
    pub fn anchor(&self, diff: &Diff) -> Option<(RelPath, usize)> {
        let file = diff.files.get(self.stream.file_at(self.cursor))?;
        let start = self.stream.file_start(self.stream.file_at(self.cursor))?;
        Some((file.path.clone(), self.cursor - start))
    }

    /// Lay the new diff out. The cursor returns to its row in the same file when it is still there.
    pub fn rebuild(&mut self, diff: &Diff, review: &Review, anchor: Option<(RelPath, usize)>) {
        self.stream = Stream::build(diff, review);
        self.cursor = anchor
            .and_then(|(path, offset)| {
                let index = diff.files.iter().position(|file| file.path == path)?;
                let start = self.stream.file_start(index)?;
                Some(start + offset.min(file_len(diff.files.get(index)?) - 1))
            })
            .unwrap_or(self.cursor)
            .min(self.stream.len().saturating_sub(1));
        self.scroll = self.scroll.min(self.stream.len().saturating_sub(1));
        self.ensure_visible();
    }

    pub fn resize(&mut self, area: Rect) {
        self.area = area;
        self.ensure_visible();
    }

    fn ensure_visible(&mut self) {
        let height = self.height();
        let last = self.stream.len().saturating_sub(1);
        self.cursor = self.cursor.min(last);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + height {
            self.scroll = self.cursor + 1 - height;
        }
    }

    fn move_to(&mut self, row: usize) {
        self.cursor = row.min(self.stream.len().saturating_sub(1));
        self.ensure_visible();
    }

    /// The cursor to the header of file `file`.
    fn move_to_file(&mut self, file: usize) {
        let file = file.min(self.stream.files().saturating_sub(1));
        if let Some(start) = self.stream.file_start(file) {
            self.move_to(start);
        }
    }

    fn page(&mut self, down: bool) {
        let height = self.height();
        let last = self.stream.len().saturating_sub(1);
        let max_scroll = self.stream.len().saturating_sub(height);
        self.scroll = if down {
            (self.scroll + height).min(max_scroll)
        } else {
            self.scroll.saturating_sub(height)
        };
        let moved = if down {
            self.cursor + height
        } else {
            self.cursor.saturating_sub(height)
        };
        self.cursor = moved
            .min(last)
            .clamp(self.scroll, (self.scroll + height - 1).min(last));
    }

    /// Move to the next or previous row of the hunk headers or the thread rows.
    fn jump(&mut self, hunks: bool, forward: bool) {
        let rows = if hunks {
            &self.stream.hunk_rows
        } else {
            &self.stream.thread_rows
        };
        let target = if forward {
            rows.iter().find(|&&row| row > self.cursor)
        } else {
            rows.iter().rev().find(|&&row| row < self.cursor)
        };
        if let Some(&row) = target {
            self.move_to(row);
        }
    }

    /// Apply a navigation action. Returns false for the actions the body does not handle.
    pub fn apply(&mut self, action: Action) -> bool {
        let files = self.panel == Panel::Sidebar;
        let file = self.stream.file_at(self.cursor);
        match action {
            Action::Up if files => self.move_to_file(file.saturating_sub(1)),
            Action::Down if files => self.move_to_file(file + 1),
            Action::Up => self.move_to(self.cursor.saturating_sub(1)),
            Action::Down => self.move_to(self.cursor + 1),
            Action::PageUp if files => self.move_to_file(file.saturating_sub(self.height())),
            Action::PageDown if files => self.move_to_file(file + self.height()),
            Action::PageUp => self.page(false),
            Action::PageDown => self.page(true),
            Action::PrevHunk => self.jump(true, false),
            Action::NextHunk => self.jump(true, true),
            Action::PrevThread => self.jump(false, false),
            Action::NextThread => self.jump(false, true),
            Action::SwitchPanel => {
                self.panel = match self.panel {
                    Panel::Sidebar => Panel::Stream,
                    Panel::Stream => Panel::Sidebar,
                };
            }
            Action::Help => self.help = !self.help,
            _ => return false,
        }
        true
    }

    /// The wheel scrolls and a click moves the cursor. Neither can be remapped.
    pub fn mouse(&mut self, event: MouseEvent) {
        let areas = areas(self.area);
        let at = |rect: Rect| {
            event.column >= rect.x
                && event.column < rect.x + rect.width
                && event.row >= rect.y
                && event.row < rect.y + rect.height
        };
        match event.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let down = event.kind == MouseEventKind::ScrollDown;
                let height = self.height();
                let max_scroll = self.stream.len().saturating_sub(height);
                self.scroll = if down {
                    (self.scroll + WHEEL_ROWS).min(max_scroll)
                } else {
                    self.scroll.saturating_sub(WHEEL_ROWS)
                };
                let last = (self.scroll + height - 1).min(self.stream.len().saturating_sub(1));
                self.cursor = self.cursor.clamp(self.scroll, last);
            }
            MouseEventKind::Down(MouseButton::Left) if at(areas.stream) => {
                let row = self.scroll + usize::from(event.row - areas.stream.y);
                if row < self.stream.len() {
                    self.panel = Panel::Stream;
                    self.move_to(row);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(sidebar) = areas.sidebar.filter(|rect| at(*rect)) {
                    let top = sidebar_top(
                        self.stream.file_at(self.cursor),
                        usize::from(sidebar.height),
                        self.stream.files(),
                    );
                    let file = top + usize::from(event.row - sidebar.y);
                    if file < self.stream.files() {
                        self.panel = Panel::Sidebar;
                        self.move_to_file(file);
                    }
                }
            }
            _ => {}
        }
    }
}

/// The first file the sidebar shows, so the selected one is in the middle when the list is longer
/// than the sidebar.
fn sidebar_top(selected: usize, height: usize, files: usize) -> usize {
    selected
        .saturating_sub(height / 2)
        .min(files.saturating_sub(height))
}

/// The last `width` cells of `text`, with a leading `…` when something was cut.
fn tail_to_width(text: &str, width: usize) -> String {
    if string_width(text) <= width {
        return text.to_owned();
    }
    let mut used = 1;
    let mut kept = Vec::new();
    for character in text.chars().rev() {
        used += char_width(character);
        if used > width {
            break;
        }
        kept.push(character);
    }
    kept.push('…');
    kept.into_iter().rev().collect()
}

fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

fn row_line(row: RowRef, width: usize) -> Line<'static> {
    match row {
        RowRef::File(file) => {
            let mut text = format!(
                "{} {}",
                glyph(file.change),
                sanitize_terminal_text(file.path.as_str())
            );
            if let Some(old) = &file.old_path {
                let _ = write!(text, " <- {}", sanitize_terminal_text(old.as_str()));
            }
            if file.flags.mode_changed && !file.hunks.is_empty() {
                text.push_str(" (mode changed)");
            }
            Line::styled(
                truncate_to_width(&text, width),
                Style::new().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )
        }
        RowRef::Note(file) => Line::styled(format!("  {}", note(file)), dim()),
        RowRef::Hunk(hunk) => Line::styled(
            truncate_to_width(&sanitize_terminal_text(&hunk.header), width),
            Style::new().fg(Color::Cyan),
        ),
        RowRef::Line(row) => {
            let number =
                |line: Option<u32>| line.map_or_else(|| "     ".to_owned(), |n| format!("{n:>5}"));
            let gutter = format!("{}{} ", number(row.old), number(row.new));
            let (sign, style) = match row.kind {
                RowKind::Context => (' ', Style::new()),
                RowKind::Added => ('+', Style::new().fg(Color::Green)),
                RowKind::Removed => ('-', Style::new().fg(Color::Red)),
            };
            let mut text = format!("{sign}{}", sanitize_terminal_text(&row.text));
            if row.no_newline {
                text.push_str("  [no newline at end of file]");
            }
            let room = width.saturating_sub(gutter.len());
            Line::from(vec![
                Span::styled(gutter, dim()),
                Span::styled(truncate_to_width(&text, room), style),
            ])
        }
    }
}

fn empty_message(spec: &Spec) -> String {
    match spec {
        Spec::WorkTree => "No changes in the working tree.".to_owned(),
        Spec::Branch { base } => format!("No changes against {}.", sanitize_terminal_text(base)),
    }
}

fn highlight(buffer: &mut Buffer, area: Rect, row: usize, style: Style) {
    let y = area.y + u16::try_from(row).unwrap_or(0);
    buffer.set_style(Rect::new(area.x, y, area.width, 1), style);
}

/// Draw the sidebar and the stream, and the help overlay when it is open. Only the rows in the
/// window are built.
pub fn draw(frame: &mut Frame, view: &View, diff: &Diff, keymap: &Keymap) {
    let areas = areas(frame.area());
    if diff.files.is_empty() {
        frame.render_widget(Paragraph::new(empty_message(&diff.spec)), areas.stream);
        return;
    }
    let cursor_style = Style::new().bg(Color::DarkGray);
    let height = usize::from(areas.stream.height);
    let width = usize::from(areas.stream.width);
    let lines = (view.scroll..view.scroll + height)
        .filter_map(|row| view.stream.locate(diff, row))
        .map(|row| row_line(row, width))
        .collect::<Vec<_>>();
    frame.render_widget(Clear, areas.stream);
    frame.render_widget(Paragraph::new(lines), areas.stream);
    if view.cursor >= view.scroll && view.cursor < view.scroll + height {
        let style = if view.panel == Panel::Stream {
            cursor_style
        } else {
            dim()
        };
        highlight(
            frame.buffer_mut(),
            areas.stream,
            view.cursor - view.scroll,
            style,
        );
    }
    if let Some(sidebar) = areas.sidebar {
        draw_sidebar(frame, sidebar, view, diff);
    }
    if view.help {
        draw_help(frame, keymap);
    }
}

fn draw_sidebar(frame: &mut Frame, area: Rect, view: &View, diff: &Diff) {
    let block = Block::new().borders(Borders::RIGHT).border_style(dim());
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let selected = view.stream.file_at(view.cursor);
    let height = usize::from(inner.height);
    let top = sidebar_top(selected, height, diff.files.len());
    let width = usize::from(inner.width);
    let lines = diff
        .files
        .iter()
        .skip(top)
        .take(height)
        .map(|file| {
            let name = sanitize_terminal_text(file.path.as_str());
            let text = tail_to_width(&format!("{} {name}", glyph(file.change)), width);
            Line::from(text)
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), inner);
    let style = if view.panel == Panel::Sidebar {
        Style::new().bg(Color::DarkGray)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    };
    if selected >= top && selected < top + height {
        highlight(frame.buffer_mut(), inner, selected - top, style);
    }
}

/// Every action with its current keys, drawn from the effective keymap.
fn draw_help(frame: &mut Frame, keymap: &Keymap) {
    let area = frame.area();
    let width = area.width.saturating_sub(4).min(64);
    let height =
        (u16::try_from(Action::ALL.len()).unwrap_or(0) + 2).min(area.height.saturating_sub(2));
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );
    let lines = Action::ALL
        .iter()
        .map(|action| {
            Line::from(format!(
                "{:<16} {}",
                sanitize_terminal_text(&keymap.label(*action)),
                action.describe()
            ))
        })
        .collect::<Vec<_>>();
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" keys, any key closes ")),
        popup,
    );
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyModifiers;

    use super::*;
    use crate::diff::{MAX_PATCH, parse};
    use crate::store::{Anchor, Author, Comment, CommentId, Status, Thread};

    const PATCH: &str = "diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,3 +1,3 @@
 a1
-a2
+A2
 a3
@@ -10,2 +10,3 @@
 a10
+a11
 a12
diff --git a/b.rs b/b.rs
new file mode 100644
--- /dev/null
+++ b/b.rs
@@ -0,0 +1,2 @@
+b1
+b2
diff --git a/img.png b/img.png
index 1111111..2222222 100644
Binary files a/img.png and b/img.png differ
";
    // Rows: a.rs 0..=9 (header, hunk 1, a1, a2, A2, a3, hunk 6, a10, a11, a12), b.rs 10..=13,
    // img.png 14..=15.

    fn diff_of(patch: &str) -> Diff {
        Diff {
            files: parse(patch.as_bytes(), MAX_PATCH),
            spec: Spec::WorkTree,
            notices: Vec::new(),
        }
    }

    fn thread(id: &str, path: &str, target: AnchorTarget) -> Thread {
        Thread {
            root: Comment {
                id: CommentId::parse(id).unwrap(),
                parent: None,
                author: Author::User,
                body: "fix".into(),
                sent_batch: None,
                edited_since_sent: false,
            },
            anchor: Anchor {
                path: RelPath::parse(path).unwrap(),
                old_path: None,
                target,
                spec: Spec::WorkTree,
            },
            replies: Vec::new(),
            status: Status::Open,
            is_new: false,
            reopened: false,
            unsent: true,
        }
    }

    fn line(line: u32, text: &str) -> AnchorTarget {
        AnchorTarget::Line {
            side: Side::New,
            line,
            text: text.into(),
        }
    }

    fn review() -> Review {
        Review {
            threads: vec![
                thread("u1", "a.rs", line(4, "not there anymore")),
                thread("u2", "b.rs", AnchorTarget::File),
                thread("u3", "a.rs", line(3, "A2")),
                thread("u4", "gone.rs", line(1, "x")),
            ],
            ..Review::default()
        }
    }

    fn view(diff: &Diff, review: &Review, width: u16, height: u16) -> View {
        let mut view = View::default();
        view.rebuild(diff, review, None);
        view.resize(Rect::new(0, 0, width, height));
        view
    }

    fn plain() -> (Diff, View) {
        let diff = diff_of(PATCH);
        let view = view(&diff, &Review::default(), 80, 12);
        (diff, view)
    }

    fn screen(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        buffer
            .content
            .chunks(usize::from(buffer.area.width))
            .map(|row| {
                row.iter()
                    .map(ratatui::buffer::Cell::symbol)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn render(
        terminal: &mut Terminal<TestBackend>,
        view: &View,
        diff: &Diff,
        keymap: &Keymap,
    ) -> String {
        terminal
            .draw(|frame| draw(frame, view, diff, keymap))
            .unwrap();
        screen(terminal)
    }

    fn fresh(view: &View, diff: &Diff) -> String {
        let mut terminal =
            Terminal::new(TestBackend::new(view.area.width, view.area.height)).unwrap();
        render(&mut terminal, view, diff, &Keymap::default())
    }

    #[test]
    fn the_stream_numbers_every_row_of_every_file() {
        let (diff, view) = plain();
        assert_eq!(view.stream.len(), 16);
        assert_eq!(view.stream.hunk_rows, [1, 6, 11]);
        assert_eq!(view.stream.file_at(9), 0);
        assert_eq!(view.stream.file_at(10), 1);
        assert_eq!(view.stream.file_at(15), 2);
        let kind = |row| match view.stream.locate(&diff, row).unwrap() {
            RowRef::File(_) => "file",
            RowRef::Note(_) => "note",
            RowRef::Hunk(_) => "hunk",
            RowRef::Line(_) => "line",
        };
        let kinds = (0..16).map(kind).collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [
                "file", "hunk", "line", "line", "line", "line", "hunk", "line", "line", "line",
                "file", "hunk", "line", "line", "file", "note"
            ]
        );
        assert!(view.stream.locate(&diff, 16).is_none());
    }

    #[test]
    fn up_and_down_move_one_row_and_stop_at_the_ends() {
        let (_, mut view) = plain();
        view.apply(Action::Up);
        assert_eq!(view.cursor, 0);
        view.apply(Action::Down);
        view.apply(Action::Down);
        assert_eq!(view.cursor, 2);
        view.apply(Action::Up);
        assert_eq!(view.cursor, 1);
        for _ in 0..40 {
            view.apply(Action::Down);
        }
        assert_eq!(view.cursor, 15);
        assert_eq!(view.scroll, 6);
    }

    #[test]
    fn page_down_and_page_up_scroll_a_page() {
        let (_, mut view) = plain();
        assert_eq!(view.height(), 10);
        view.apply(Action::PageDown);
        assert_eq!((view.scroll, view.cursor), (6, 10));
        view.apply(Action::PageDown);
        assert_eq!((view.scroll, view.cursor), (6, 15));
        view.apply(Action::PageUp);
        assert_eq!((view.scroll, view.cursor), (0, 5));
        view.apply(Action::PageUp);
        assert_eq!((view.scroll, view.cursor), (0, 0));
    }

    #[test]
    fn next_and_previous_hunk_jump_between_hunk_headers_across_files() {
        let (_, mut view) = plain();
        let mut seen = Vec::new();
        for _ in 0..4 {
            view.apply(Action::NextHunk);
            seen.push(view.cursor);
        }
        assert_eq!(seen, [1, 6, 11, 11]);
        view.apply(Action::PrevHunk);
        assert_eq!(view.cursor, 6);
        // From inside a hunk, previous goes to that hunk's own header.
        view.move_to(8);
        view.apply(Action::PrevHunk);
        assert_eq!(view.cursor, 6);
        view.move_to(0);
        view.apply(Action::PrevHunk);
        assert_eq!(view.cursor, 0);
    }

    #[test]
    fn next_and_previous_thread_jump_between_the_rows_threads_are_placed_at() {
        let diff = diff_of(PATCH);
        let review = review();
        let mut view = view(&diff, &review, 80, 12);
        // u3 asked for A2 at line 3, and A2 moved to line 2, row 4. u1 is outdated, and the closest
        // line to its line 4 is line 3, row 5. u2 is the file b.rs. u4 is not in the diff.
        assert_eq!(view.stream.thread_rows, [4, 5, 10]);
        assert_eq!(
            view.stream.placements,
            [
                Placement::Outdated { near: Some(3) },
                Placement::Matched { line: None },
                Placement::Matched { line: Some(2) },
                Placement::NotInDiff,
            ]
        );
        let mut seen = Vec::new();
        for _ in 0..4 {
            view.apply(Action::NextThread);
            seen.push(view.cursor);
        }
        assert_eq!(seen, [4, 5, 10, 10]);
        let mut back = Vec::new();
        for _ in 0..4 {
            view.apply(Action::PrevThread);
            back.push(view.cursor);
        }
        assert_eq!(back, [5, 4, 4, 4]);
    }

    #[test]
    fn the_switch_panel_key_moves_up_and_down_between_files_in_the_sidebar() {
        let (_, mut view) = plain();
        assert_eq!(view.panel, Panel::Stream);
        view.apply(Action::SwitchPanel);
        assert_eq!(view.panel, Panel::Sidebar);
        view.apply(Action::Down);
        assert_eq!(view.cursor, 10);
        view.apply(Action::Down);
        assert_eq!(view.cursor, 14);
        view.apply(Action::Down);
        assert_eq!(view.cursor, 14);
        view.apply(Action::Up);
        assert_eq!(view.cursor, 10);
        view.apply(Action::PageUp);
        assert_eq!(view.cursor, 0);
        view.apply(Action::PageDown);
        assert_eq!(view.cursor, 14);
        view.apply(Action::SwitchPanel);
        assert_eq!(view.panel, Panel::Stream);
    }

    #[test]
    fn help_toggles_and_the_other_actions_are_not_the_bodys() {
        let (_, mut view) = plain();
        assert!(view.apply(Action::Help));
        assert!(view.help);
        assert!(view.apply(Action::Help));
        assert!(!view.help);
        for action in [
            Action::Reload,
            Action::SwitchSpec,
            Action::Quit,
            Action::Comment,
            Action::Send,
        ] {
            assert!(!view.apply(action), "{action:?}");
        }
    }

    #[test]
    fn the_wheel_scrolls_and_a_click_moves_the_cursor() {
        let (_, mut view) = plain();
        let mouse = |kind, column, row| MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        view.mouse(mouse(MouseEventKind::ScrollDown, 40, 3));
        assert_eq!((view.scroll, view.cursor), (3, 3));
        view.mouse(mouse(MouseEventKind::ScrollDown, 40, 3));
        view.mouse(mouse(MouseEventKind::ScrollDown, 40, 3));
        assert_eq!(view.scroll, 6);
        view.mouse(mouse(MouseEventKind::ScrollUp, 40, 3));
        assert_eq!(view.scroll, 3);
        // A click in the stream.
        let stream = areas(view.area).stream;
        view.mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            stream.x + 4,
            stream.y + 5,
        ));
        assert_eq!((view.cursor, view.panel), (8, Panel::Stream));
        // A click in the sidebar on the second file.
        view.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 1));
        assert_eq!((view.cursor, view.panel), (10, Panel::Sidebar));
        // A click below the last file does nothing.
        view.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 6));
        assert_eq!(view.cursor, 10);
    }

    #[test]
    fn the_stream_and_the_sidebar_are_drawn() {
        let (diff, view) = plain();
        let screen = fresh(&view, &diff);
        assert!(screen.contains("M a.rs"));
        assert!(screen.contains("A b.rs"));
        assert!(screen.contains("B img.png"));
        assert!(screen.contains("@@ -1,3 +1,3 @@"));
        assert!(screen.contains("    1    1  a1"), "{screen}");
        assert!(screen.contains("    2      -a2"), "{screen}");
        assert!(screen.contains("         2 +A2"), "{screen}");
    }

    #[test]
    fn a_file_with_no_rows_says_why() {
        let (diff, mut view) = plain();
        view.move_to(15);
        let screen = fresh(&view, &diff);
        assert!(screen.contains("binary file, not shown"));
    }

    #[test]
    fn scrolling_to_the_end_and_back_leaves_no_stale_rows() {
        let (diff, mut view) = plain();
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        let keymap = Keymap::default();
        let top = render(&mut terminal, &view, &diff, &keymap);
        for _ in 0..40 {
            view.apply(Action::Down);
        }
        let bottom = render(&mut terminal, &view, &diff, &keymap);
        assert_ne!(top, bottom);
        assert_eq!(bottom, fresh(&view, &diff));
        for _ in 0..40 {
            view.apply(Action::Up);
        }
        assert_eq!(render(&mut terminal, &view, &diff, &keymap), top);
        // A shorter diff over the same terminal leaves nothing of the longer one behind.
        let short = diff_of(&PATCH[..PATCH.find("diff --git a/b.rs").unwrap()]);
        let short_view = view_of(&short);
        assert_eq!(
            render(&mut terminal, &short_view, &short, &keymap),
            fresh(&short_view, &short)
        );
    }

    fn view_of(diff: &Diff) -> View {
        view(diff, &Review::default(), 80, 12)
    }

    #[test]
    fn a_file_name_with_an_escape_byte_draws_without_control_characters() {
        let patch = "diff --git \"a/evil\\033[2Jname.rs\" \"b/evil\\033[2Jname.rs\"\n--- \"a/evil\\033[2Jname.rs\"\n+++ \"b/evil\\033[2Jname.rs\"\n@@ -1 +1 @@\n-old\\u{1b}x\n+new\n";
        let diff = diff_of(patch);
        assert_eq!(diff.files.len(), 1);
        assert!(diff.files[0].path.as_str().contains('\u{1b}'));
        let view = view_of(&diff);
        let screen = fresh(&view, &diff);
        assert!(screen.contains("evil[2Jname.rs"));
        assert!(!screen.chars().any(|c| c != '\n' && c.is_control()));
    }

    #[test]
    fn a_long_path_keeps_its_tail_in_the_sidebar_and_long_lines_are_cut() {
        let long = "x".repeat(60);
        let patch = format!(
            "diff --git a/dir/{long}.rs b/dir/{long}.rs\n--- a/dir/{long}.rs\n+++ b/dir/{long}.rs\n@@ -1 +1 @@\n-{long}{long}\n+new\n"
        );
        let diff = diff_of(&patch);
        let view = view_of(&diff);
        let screen = fresh(&view, &diff);
        assert!(screen.contains("…x") && screen.contains(".rs│"), "{screen}");
        assert!(screen.lines().all(|row| string_width(row) <= 80));
    }

    #[test]
    fn an_empty_diff_says_so_for_each_spec() {
        let mut diff = diff_of("");
        let view = view_of(&diff);
        assert!(fresh(&view, &diff).contains("No changes in the working tree."));
        diff.spec = Spec::Branch {
            base: "main".into(),
        };
        assert!(fresh(&view, &diff).contains("No changes against main."));
    }

    #[test]
    fn the_help_overlay_lists_every_action_with_its_current_keys() {
        let (diff, mut view) = plain();
        view.help = true;
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        view.resize(Rect::new(0, 0, 80, 30));
        let defaults = render(&mut terminal, &view, &diff, &Keymap::default());
        assert!(
            defaults.contains("R                reload the diff"),
            "{defaults}"
        );
        assert!(defaults.contains("q                quit"));
        let rebound = Keymap::from_toml("[keys]\nreload = \"r\"\nquit = [\"q\", \"ctrl+c\"]\n");
        let after = render(&mut terminal, &view, &diff, &rebound);
        assert!(after.contains("r                reload the diff"));
        assert!(after.contains("ctrl+c, q"));
        assert!(after.contains("-                reply to the thread"));
    }

    #[test]
    fn a_reload_puts_the_cursor_back_on_its_row_in_the_same_file() {
        let (diff, mut view) = plain();
        view.move_to(12);
        let anchor = view.anchor(&diff);
        // The first file lost its second hunk, so b.rs starts earlier.
        let smaller = diff_of(&PATCH.replace("@@ -10,2 +10,3 @@\n a10\n+a11\n a12\n", ""));
        view.rebuild(&smaller, &Review::default(), anchor);
        assert_eq!(view.cursor, 8);
        assert!(
            matches!(view.stream.locate(&smaller, 8), Some(RowRef::Line(row)) if row.text == "b1")
        );
        // The file is gone: the cursor stays on the nearest row that exists.
        let anchor = view.anchor(&smaller);
        view.rebuild(&diff_of(""), &Review::default(), anchor);
        assert_eq!(view.cursor, 0);
    }
}
