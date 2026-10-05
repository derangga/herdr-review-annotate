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
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::cards::{Card, Look, card};
use crate::diff::{Change, Diff, DiffFile, Hunk, Placement, Row, RowKind, place};
use crate::keymap::{Action, Keymap};
use crate::store::{Anchor, AnchorTarget, CommentId, RelPath, Review, Side, Spec};
use crate::theme::Theme;
use crate::tui::sanitize_terminal_text;
use crate::width::{string_width, tail_to_width, truncate_to_width};

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
    /// The side-by-side form of a line: the old half and the new half. A context line is both.
    Pair {
        old: Option<&'a Row>,
        new: Option<&'a Row>,
    },
    /// Lines between two hunks, or above the first one, that the diff does not show.
    Gap(u32),
    /// The heading of the block of threads that are not in the diff, with their number.
    BlockHeader(usize),
    /// Line `line` of the card of thread `thread`, counting threads in the review's order.
    Card {
        thread: usize,
        line: usize,
    },
    /// The one row that says the diff has no files.
    Empty(&'a Spec),
}

/// How the diff is drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DiffLayout {
    /// One column, a removed line above the added one that replaced it.
    #[default]
    Unified,
    /// Old on the left and new on the right.
    Split,
}

impl DiffLayout {
    const fn other(self) -> Self {
        match self {
            Self::Unified => Self::Split,
            Self::Split => Self::Unified,
        }
    }
}

/// A pane this wide or wider draws the diff side by side, unless the user chose.
const SPLIT_MIN_TOTAL: u16 = 120;

/// The widths of the old and the new half of a split row, with one cell between them.
fn split_widths(width: usize) -> (usize, usize) {
    let left = width.saturating_sub(1) / 2;
    (left, width.saturating_sub(1 + left))
}

/// What one row inside a file is. Indices point into the file's hunks and their rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileRow {
    Header,
    Note,
    Gap(u32),
    Hunk(usize),
    Line {
        hunk: usize,
        row: usize,
    },
    Pair {
        hunk: usize,
        old: Option<usize>,
        new: Option<usize>,
    },
}

/// Lines the diff leaves out before `hunk`: above the first one, or between it and `before`.
fn gap(before: Option<&Hunk>, hunk: &Hunk) -> Option<u32> {
    let first = |side| hunk.rows.iter().find_map(|row| row.line(side));
    let Some(before) = before else {
        let line = first(Side::New).or_else(|| first(Side::Old))?;
        return Some(line.saturating_sub(1));
    };
    [Side::New, Side::Old].into_iter().find_map(|side| {
        let last = before.rows.iter().rev().find_map(|row| row.line(side))?;
        Some(first(side)?.saturating_sub(last + 1))
    })
}

/// The rows of one hunk side by side. A run of removed lines pairs, line by line, with the run of
/// added lines after it, and the longer run's extra lines sit opposite an empty half.
fn pairs(hunk_index: usize, rows: &[Row]) -> Vec<FileRow> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(row) = rows.get(at) {
        if row.kind == RowKind::Context {
            out.push(FileRow::Pair {
                hunk: hunk_index,
                old: Some(at),
                new: Some(at),
            });
            at += 1;
            continue;
        }
        let mut run = |kind| {
            let start = at;
            while rows.get(at).is_some_and(|row| row.kind == kind) {
                at += 1;
            }
            start..at
        };
        let removed = run(RowKind::Removed);
        let added = run(RowKind::Added);
        for k in 0..removed.len().max(added.len()) {
            out.push(FileRow::Pair {
                hunk: hunk_index,
                old: (k < removed.len()).then(|| removed.start + k),
                new: (k < added.len()).then(|| added.start + k),
            });
        }
    }
    out
}

/// Every row of a file, from its header: the hunks, or one note when it has none. The split
/// layout also marks the unchanged lines between hunks.
fn file_rows(file: &DiffFile, layout: DiffLayout) -> Vec<FileRow> {
    let mut rows = vec![FileRow::Header];
    if file.hunks.is_empty() {
        rows.push(FileRow::Note);
        return rows;
    }
    let mut before = None;
    for (index, hunk) in file.hunks.iter().enumerate() {
        if layout == DiffLayout::Split {
            rows.extend(gap(before, hunk).filter(|n| *n > 0).map(FileRow::Gap));
        }
        rows.push(FileRow::Hunk(index));
        match layout {
            DiffLayout::Unified => {
                rows.extend((0..hunk.rows.len()).map(|row| FileRow::Line { hunk: index, row }));
            }
            DiffLayout::Split => rows.extend(pairs(index, &hunk.rows)),
        }
        before = Some(hunk);
    }
    rows
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

/// A card hung under base row `at` of a file, counting the file's own rows from its header at 0.
/// In the not-in-diff block `at` is unused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    at: usize,
    thread: usize,
    height: usize,
}

/// What a stream row is, before it is looked up in the diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum At {
    BlockHeader,
    Empty,
    /// A card line. `owner` is the file and base row it hangs from, or `None` in the block.
    Card {
        thread: usize,
        line: usize,
        owner: Option<(usize, usize)>,
    },
    /// Row `offset` of file `file`, not counting the cards above it in the file.
    Base {
        file: usize,
        offset: usize,
    },
}

/// The half of a code row on `side`, which is nothing when the row has no line there.
fn on_side<'a>(
    side: Side,
    (_, old, new): (usize, Option<&'a Row>, Option<&'a Row>),
) -> Option<&'a Row> {
    if side == Side::Old { old } else { new }
}

/// The row inside a file where its line `line` on `side` is, counting the header as 0.
fn offset_of(file: &DiffFile, rows: &[FileRow], side: Side, line: u32) -> Option<usize> {
    let line_of = |hunk: usize, row: Option<usize>| {
        let row = file.hunks.get(hunk)?.rows.get(row?)?;
        row.line(side)
    };
    rows.iter().position(|row| match *row {
        FileRow::Line { hunk, row } => line_of(hunk, Some(row)) == Some(line),
        FileRow::Pair { hunk, old, new } => {
            let on_side = if side == Side::Old { old } else { new };
            line_of(hunk, on_side) == Some(line)
        }
        _ => false,
    })
}

/// Where the rows of the diff start, where the cards go, and the rows the navigation keys jump to.
///
/// The rows are numbered from 0: first the not-in-diff block (its heading, then one card per
/// thread), then each file's header, its hunks, and under every line the cards of the threads
/// placed there. Only the lines of the cards are kept. The diff rows are looked up when drawn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stream {
    /// The row each file starts at.
    starts: Vec<usize>,
    total: usize,
    hunk_rows: Vec<usize>,
    /// The first row of each card, in row order.
    thread_rows: Vec<usize>,
    /// One per thread of the review, in the review's order.
    pub placements: Vec<Placement>,
    /// The rows of the block of threads not in the diff. Zero when there are none.
    top: usize,
    block: Vec<Slot>,
    /// The cards of each file, in row order.
    slots: Vec<Vec<Slot>>,
    /// The lines of each thread's card, in the review's order.
    cards: Vec<Card>,
    /// The first row of each thread's card, in the review's order.
    card_rows: Vec<usize>,
    ids: Vec<CommentId>,
    /// The width the cards were wrapped for.
    width: usize,
    /// The sidebar's rows: a heading for each directory, and the files under it.
    side: Vec<SideRow>,
    /// Per file, whether a thread hung in it has not been sent.
    unsent: Vec<bool>,
    /// The rows of each file, for the layout the stream was built for.
    files: Vec<Vec<FileRow>>,
    layout: DiffLayout,
}

/// One row of the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SideRow {
    Heading(String),
    File(usize),
}

/// Group the files by directory, in the diff's order. A directory that comes again later, as
/// untracked files do, gets a second heading.
fn sidebar_rows(diff: &Diff) -> Vec<SideRow> {
    let mut rows = Vec::new();
    let mut last = None;
    for (index, file) in diff.files.iter().enumerate() {
        let dir = file
            .path
            .as_str()
            .rsplit_once('/')
            .map_or("./", |(dir, _)| dir);
        if last != Some(dir) {
            let shown = if dir == "./" {
                dir.to_owned()
            } else {
                format!("{dir}/")
            };
            rows.push(SideRow::Heading(sanitize_terminal_text(&shown)));
            last = Some(dir);
        }
        rows.push(SideRow::File(index));
    }
    rows
}

impl Stream {
    /// Lay out `diff` and the threads of `review` for a stream `width` cells wide.
    pub fn build(
        diff: &Diff,
        review: &Review,
        width: usize,
        layout: DiffLayout,
        look: &Look,
    ) -> Self {
        let files = diff
            .files
            .iter()
            .map(|file| file_rows(file, layout))
            .collect::<Vec<_>>();
        let placements = review
            .threads
            .iter()
            .map(|thread| place(thread, diff))
            .collect::<Vec<_>>();
        let cards = review
            .threads
            .iter()
            .zip(&placements)
            .map(|(thread, placement)| card(thread, *placement, width, look))
            .collect::<Vec<_>>();
        let mut stream = Self {
            card_rows: vec![0; cards.len()],
            ids: review
                .threads
                .iter()
                .map(|thread| thread.root.id.clone())
                .collect(),
            width,
            side: sidebar_rows(diff),
            unsent: vec![false; diff.files.len()],
            layout,
            ..Self::default()
        };
        let mut block = Vec::new();
        let mut hung = vec![Vec::<(usize, usize)>::new(); diff.files.len()];
        for (index, (thread, placement)) in review.threads.iter().zip(&placements).enumerate() {
            let line = match placement {
                Placement::NotInDiff => None,
                Placement::Matched { line } => Some(*line),
                Placement::Outdated { near } => Some(*near),
            };
            let file = line.and(diff.file_index(&thread.anchor.path));
            let Some((file, line)) = file.zip(line) else {
                block.push(index);
                continue;
            };
            if let Some(mark) = stream.unsent.get_mut(file) {
                *mark |= thread.unsent;
            }
            let side = match &thread.anchor.target {
                AnchorTarget::Line { side, .. } | AnchorTarget::Range { side, .. } => *side,
                AnchorTarget::File => Side::New,
            };
            let offset = diff
                .files
                .get(file)
                .zip(files.get(file))
                .zip(line)
                .and_then(|((file, rows), line)| offset_of(file, rows, side, line))
                .unwrap_or(0);
            if let Some(list) = hung.get_mut(file) {
                list.push((offset, index));
            }
        }
        let height = |thread: usize| cards.get(thread).map_or(0, |card| card.lines.len());
        let mut total = 0;
        if !block.is_empty() {
            total = 1;
            for &thread in &block {
                stream.block.push(Slot {
                    at: 0,
                    thread,
                    height: height(thread),
                });
                stream.set_card_row(thread, total);
                total += height(thread);
            }
        }
        stream.top = total;
        if diff.files.is_empty() {
            total += 1;
        }
        for (rows, mut list) in files.iter().zip(hung) {
            list.sort_by_key(|(offset, _)| *offset);
            let slots = list
                .into_iter()
                .map(|(at, thread)| Slot {
                    at,
                    thread,
                    height: height(thread),
                })
                .collect::<Vec<_>>();
            let start = total;
            stream.starts.push(start);
            let mut above = 0;
            for slot in &slots {
                stream.set_card_row(slot.thread, start + slot.at + 1 + above);
                above += slot.height;
            }
            let shift = |offset: usize| {
                slots
                    .iter()
                    .filter(|slot| slot.at < offset)
                    .map(|slot| slot.height)
                    .sum::<usize>()
            };
            for (offset, row) in rows.iter().enumerate() {
                if matches!(row, FileRow::Hunk(_)) {
                    stream.hunk_rows.push(start + offset + shift(offset));
                }
            }
            total = start + rows.len() + above;
            stream.slots.push(slots);
        }
        stream.total = total;
        stream.thread_rows.clone_from(&stream.card_rows);
        stream.thread_rows.sort_unstable();
        stream.cards = cards;
        stream.placements = placements;
        stream.files = files;
        stream
    }

    fn set_card_row(&mut self, thread: usize, row: usize) {
        if let Some(slot) = self.card_rows.get_mut(thread) {
            *slot = row;
        }
    }

    pub const fn len(&self) -> usize {
        self.total
    }

    pub const fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// The sidebar row that holds `file`.
    fn side_row_of(&self, file: usize) -> usize {
        self.side
            .iter()
            .position(|row| *row == SideRow::File(file))
            .unwrap_or(0)
    }

    pub const fn files(&self) -> usize {
        self.starts.len()
    }

    /// The file a row belongs to. Rows above the first file belong to it too.
    pub fn file_at(&self, row: usize) -> usize {
        self.starts
            .partition_point(|&start| start <= row)
            .saturating_sub(1)
    }

    pub fn file_start(&self, file: usize) -> Option<usize> {
        self.starts.get(file).copied()
    }

    /// The row of base row `offset` of file `file`, with the cards above it counted.
    fn row_of(&self, file: usize, offset: usize) -> Option<usize> {
        let above = self
            .slots
            .get(file)?
            .iter()
            .filter(|slot| slot.at < offset)
            .map(|slot| slot.height)
            .sum::<usize>();
        Some(self.starts.get(file)? + offset + above)
    }

    fn at(&self, row: usize) -> Option<At> {
        if row >= self.total {
            return None;
        }
        if row < self.top {
            if row == 0 {
                return Some(At::BlockHeader);
            }
            let mut first = 1;
            for slot in &self.block {
                if row < first + slot.height {
                    return Some(At::Card {
                        thread: slot.thread,
                        line: row - first,
                        owner: None,
                    });
                }
                first += slot.height;
            }
            return None;
        }
        if self.starts.is_empty() {
            return Some(At::Empty);
        }
        let file = self.file_at(row);
        let offset = row - self.starts.get(file)?;
        let mut shift = 0;
        for slot in self.slots.get(file)? {
            let first = slot.at + 1 + shift;
            if offset < first {
                break;
            }
            if offset < first + slot.height {
                return Some(At::Card {
                    thread: slot.thread,
                    line: offset - first,
                    owner: Some((file, slot.at)),
                });
            }
            shift += slot.height;
        }
        Some(At::Base {
            file,
            offset: offset - shift,
        })
    }

    pub fn locate<'a>(&self, diff: &'a Diff, row: usize) -> Option<RowRef<'a>> {
        let (index, offset) = match self.at(row)? {
            At::BlockHeader => return Some(RowRef::BlockHeader(self.block.len())),
            At::Empty => return Some(RowRef::Empty(&diff.spec)),
            At::Card { thread, line, .. } => return Some(RowRef::Card { thread, line }),
            At::Base { file, offset } => (file, offset),
        };
        let file = diff.files.get(index)?;
        let row = |hunk: usize, row: Option<usize>| file.hunks.get(hunk)?.rows.get(row?);
        Some(match *self.files.get(index)?.get(offset)? {
            FileRow::Header => RowRef::File(file),
            FileRow::Note => RowRef::Note(file),
            FileRow::Gap(count) => RowRef::Gap(count),
            FileRow::Hunk(hunk) => RowRef::Hunk(file.hunks.get(hunk)?),
            FileRow::Line { hunk, row: at } => RowRef::Line(row(hunk, Some(at))?),
            FileRow::Pair { hunk, old, new } => RowRef::Pair {
                old: row(hunk, old),
                new: row(hunk, new),
            },
        })
    }

    /// The kind of file row a stream row is, when it is one.
    fn file_row(&self, row: usize) -> Option<FileRow> {
        match self.at(row)? {
            At::Base { file, offset } => self.files.get(file)?.get(offset).copied(),
            _ => None,
        }
    }

    /// The thread a row belongs to: the one whose card holds it, or the first one hung under it.
    pub fn thread_at(&self, row: usize) -> Option<usize> {
        match self.at(row)? {
            At::Card { thread, .. } => Some(thread),
            At::Base { file, offset } => self
                .slots
                .get(file)?
                .iter()
                .find(|slot| slot.at == offset)
                .map(|slot| slot.thread),
            At::BlockHeader | At::Empty => None,
        }
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

/// Where the cursor was, to put it back after the rows change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spot {
    /// The file the row is in, or `None` in the block of threads not in the diff.
    path: Option<RelPath>,
    /// The row of the file the cursor is on, or the one the card hangs from. The header is 0.
    offset: usize,
    /// The thread and the line of its card, when the cursor is on a card.
    card: Option<(CommentId, usize)>,
    /// The layout `offset` counts in, and the diff line the cursor is on, so a change of layout
    /// can put the cursor back on that line.
    layout: DiffLayout,
    line: Option<(Side, u32)>,
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
    /// The row a range started at, while one is being selected.
    pub select: Option<usize>,
    /// The layout the user chose with the toggle key. `None` follows the pane's width.
    forced: Option<DiffLayout>,
    /// The half of a split row the mouse last clicked, until the cursor moves another way.
    half: Option<Side>,
    /// Where the mouse last moved, as a column and a row of the screen. `None` until Herdr
    /// delivers a motion event, which tells the pane hover works.
    pointer: Option<(u16, u16)>,
}

/// The `[+]` that opens the comment editor on a line: the stream row it is on and the column of the
/// stream where its three cells start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plus {
    pub row: usize,
    pub col: usize,
}

/// Cells in the `[+]` marker.
const PLUS_WIDTH: usize = 3;

impl Default for View {
    fn default() -> Self {
        Self {
            stream: Stream::default(),
            cursor: 0,
            scroll: 0,
            panel: Panel::Stream,
            help: false,
            area: Rect::default(),
            select: None,
            forced: None,
            half: None,
            pointer: None,
        }
    }
}

impl View {
    fn height(&self) -> usize {
        usize::from(areas(self.area).stream.height).max(1)
    }

    /// How the diff is drawn: what the user chose, else side by side in a wide pane.
    pub fn layout(&self) -> DiffLayout {
        self.forced
            .unwrap_or(if self.area.width >= SPLIT_MIN_TOTAL {
                DiffLayout::Split
            } else {
                DiffLayout::Unified
            })
    }

    /// The `[+]` to draw: on the row under the mouse, or on the cursor's row until the mouse has
    /// moved once, so a pane whose Herdr drops motion still offers it. Only a code row has one. In
    /// a split row it sits on the half under the mouse, in the gutter where the sign is.
    pub fn plus(&self) -> Option<Plus> {
        let stream = areas(self.area).stream;
        let (row, half) = match self.pointer {
            Some((column, line)) => {
                let inside = column >= stream.x
                    && column < stream.x + stream.width
                    && line >= stream.y
                    && line < stream.y + stream.height;
                if !inside {
                    return None;
                }
                let left = split_widths(usize::from(stream.width)).0;
                let half = if usize::from(column - stream.x) < left {
                    Side::Old
                } else {
                    Side::New
                };
                (self.scroll + usize::from(line - stream.y), Some(half))
            }
            None => (self.cursor, self.half),
        };
        match self.stream.file_row(row)? {
            FileRow::Line { .. } => Some(Plus { row, col: 0 }),
            FileRow::Pair { old, new, .. } => {
                let old_side = (half == Some(Side::Old) && old.is_some()) || new.is_none();
                let side = if old_side { Side::Old } else { Side::New };
                let left = split_widths(usize::from(stream.width)).0;
                let col = if side == Side::Old { 4 } else { left + 1 + 4 };
                Some(Plus { row, col })
            }
            _ => None,
        }
    }

    /// Switch to the other layout until toggled again. The caller lays the stream out again.
    pub fn toggle_layout(&mut self) {
        self.forced = Some(self.layout().other());
    }

    /// Where the cursor is, so a rebuild can put it back: a row of a file, or a line of a card.
    pub fn spot(&self, diff: &Diff) -> Option<Spot> {
        let (file, offset, card) = match self.stream.at(self.cursor)? {
            At::Base { file, offset } => (Some(file), offset, None),
            At::Card {
                thread,
                line,
                owner,
            } => (
                owner.map(|(file, _)| file),
                owner.map_or(0, |(_, at)| at),
                self.stream.ids.get(thread).map(|id| (id.clone(), line)),
            ),
            At::BlockHeader | At::Empty => return None,
        };
        let path = match file {
            Some(file) => Some(diff.files.get(file)?.path.clone()),
            None => None,
        };
        let line = match self.stream.locate(diff, self.cursor) {
            Some(RowRef::Line(row)) => Some(row),
            Some(RowRef::Pair { old, new }) => new.or(old),
            _ => None,
        }
        .and_then(|row| {
            row.new
                .map(|n| (Side::New, n))
                .or_else(|| row.old.map(|n| (Side::Old, n)))
        });
        Some(Spot {
            path,
            offset,
            card,
            layout: self.stream.layout,
            line,
        })
    }

    /// The thread the cursor is on, as an index into the review's threads.
    pub fn focused(&self) -> Option<usize> {
        self.stream.thread_at(self.cursor)
    }

    /// The row of the diff the cursor is on, or the one the card under the cursor hangs from.
    fn base_row(&self) -> Option<usize> {
        match self.stream.at(self.cursor)? {
            At::Base { .. } => Some(self.cursor),
            At::Card {
                owner: Some((file, at)),
                ..
            } => self.stream.row_of(file, at),
            _ => None,
        }
    }

    /// Start a range at the cursor, or drop the one being selected.
    pub fn toggle_select(&mut self) {
        self.select = match self.select {
            Some(_) => None,
            None => self.base_row(),
        };
    }

    /// What a comment written now would point at, read from the rows under the cursor, or from the
    /// rows between the cursor and where a range started. The result is kept by the caller, so a
    /// reload after this cannot move it.
    pub fn capture(&self, diff: &Diff) -> Result<Anchor, &'static str> {
        let row = self
            .base_row()
            .ok_or("that thread's file is not in the diff, reply to it instead")?;
        let start = self.select.unwrap_or(row);
        let (low, high) = (start.min(row), start.max(row));
        let file = self.stream.file_at(row);
        if self.stream.file_at(low) != file || self.stream.file_at(high) != file {
            return Err("a range stays inside one file");
        }
        let file = diff.files.get(file).ok_or("no file here to comment on")?;
        let target = self.capture_target(diff, low, high)?;
        Ok(Anchor {
            path: file.path.clone(),
            old_path: file.old_path.clone(),
            target,
            spec: diff.spec.clone(),
        })
    }

    fn capture_target(
        &self,
        diff: &Diff,
        low: usize,
        high: usize,
    ) -> Result<AnchorTarget, &'static str> {
        // Each code row, as the pair of halves it has. A unified row is one half or the other.
        let rows = (low..=high)
            .filter_map(|at| match self.stream.locate(diff, at)? {
                RowRef::Line(row) => match row.kind {
                    RowKind::Removed => Some((at, Some(row), None)),
                    RowKind::Added => Some((at, None, Some(row))),
                    RowKind::Context => Some((at, Some(row), Some(row))),
                },
                RowRef::Pair { old, new } => Some((at, old, new)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let Some(&(first_at, old, new)) = rows.first() else {
            return match self.stream.locate(diff, low) {
                Some(RowRef::File(_) | RowRef::Note(_)) if low == high => Ok(AnchorTarget::File),
                _ if low == high => Err("comment on a line or a file header"),
                _ => Err("select lines to comment on"),
            };
        };
        // The side a click chose counts for the row under the cursor, when the row has it.
        let clicked = self
            .half
            .filter(|_| first_at == self.cursor)
            .filter(|side| if *side == Side::Old { old } else { new }.is_some());
        let side = clicked.unwrap_or(if new.is_some() { Side::New } else { Side::Old });
        let first = on_side(side, (first_at, old, new)).ok_or("no line number here")?;
        let start = first.line(side).ok_or("no line number here")?;
        let end = rows
            .iter()
            .rev()
            .find_map(|row| on_side(side, *row)?.line(side))
            .unwrap_or(start);
        let text = first.text.clone();
        Ok(if end > start {
            AnchorTarget::Range {
                side,
                start,
                end,
                text,
            }
        } else {
            AnchorTarget::Line {
                side,
                line: start,
                text,
            }
        })
    }

    /// The stream rows a comment at `anchor` points at: its line, the ends of its range, or the
    /// header of its file. It is looked up when drawn, so it is right after the stream is laid out
    /// again.
    pub fn rows_of(&self, diff: &Diff, anchor: &Anchor) -> Option<(usize, usize)> {
        let index = diff.file_index(&anchor.path)?;
        let (file, rows) = (diff.files.get(index)?, self.stream.files.get(index)?);
        let row = |side, line| {
            self.stream
                .row_of(index, offset_of(file, rows, side, line)?)
        };
        match anchor.target {
            AnchorTarget::File => self.stream.row_of(index, 0).map(|row| (row, row)),
            AnchorTarget::Line { side, line, .. } => row(side, line).map(|row| (row, row)),
            AnchorTarget::Range {
                side, start, end, ..
            } => row(side, start).zip(row(side, end)),
        }
    }

    /// The thread and the comment of it the cursor is on: the comment whose line of the card it is
    /// on, or the root when it is on the line the card hangs under.
    pub fn focused_comment(&self) -> Option<(usize, usize)> {
        match self.stream.at(self.cursor)? {
            At::Card { thread, line, .. } => {
                Some((thread, *self.stream.cards.get(thread)?.owners.get(line)?))
            }
            At::Base { .. } => Some((self.focused()?, 0)),
            At::BlockHeader | At::Empty => None,
        }
    }

    /// The root id of thread `thread`, counting threads in the review's order as the stream was
    /// laid out.
    pub fn thread_id(&self, thread: usize) -> Option<&CommentId> {
        self.stream.ids.get(thread)
    }

    /// Put the cursor on the first row of the card of the thread whose root is `id`.
    pub fn focus_thread(&mut self, id: &CommentId) {
        let row = self
            .stream
            .ids
            .iter()
            .position(|other| other == id)
            .and_then(|thread| self.stream.card_rows.get(thread));
        if let Some(&row) = row {
            self.move_to(row);
        }
    }

    fn stream_width(&self) -> usize {
        usize::from(areas(self.area).stream.width)
    }

    /// Lay the new diff out. The cursor returns to its spot when it is still there: its line of
    /// the same card, else its row in the same file.
    pub fn rebuild(&mut self, diff: &Diff, review: &Review, spot: Option<Spot>, look: &Look) {
        self.stream = Stream::build(diff, review, self.stream_width(), self.layout(), look);
        self.select = None;
        let stream = &self.stream;
        let on_card = |spot: &Spot| {
            let (id, line) = spot.card.as_ref()?;
            let thread = stream.ids.iter().position(|other| other == id)?;
            let height = stream.cards.get(thread)?.lines.len();
            Some(stream.card_rows.get(thread)? + (*line).min(height.saturating_sub(1)))
        };
        let in_file = |spot: &Spot| {
            let index = diff
                .files
                .iter()
                .position(|file| Some(&file.path) == spot.path.as_ref())?;
            let rows = stream.files.get(index)?;
            let by_line = spot
                .line
                .filter(|_| spot.layout != stream.layout)
                .and_then(|(side, line)| offset_of(diff.files.get(index)?, rows, side, line));
            let last = rows.len().saturating_sub(1);
            stream.row_of(index, by_line.unwrap_or_else(|| spot.offset.min(last)))
        };
        self.cursor = spot
            .and_then(|spot| on_card(&spot).or_else(|| in_file(&spot)))
            .unwrap_or(self.cursor)
            .min(self.stream.len().saturating_sub(1));
        self.scroll = self.scroll.min(self.stream.len().saturating_sub(1));
        self.ensure_visible();
    }

    pub fn resize(&mut self, area: Rect) {
        self.area = area;
        self.ensure_visible();
    }

    /// The stream is wider or narrower than when it was laid out, so its cards wrap differently.
    pub fn needs_rebuild(&self) -> bool {
        self.stream.width != self.stream_width() || self.stream.layout != self.layout()
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
        self.half = None;
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
    pub fn mouse(&mut self, event: MouseEvent) -> bool {
        let areas = areas(self.area);
        let at = |rect: Rect| {
            event.column >= rect.x
                && event.column < rect.x + rect.width
                && event.row >= rect.y
                && event.row < rect.y + rect.height
        };
        let mut plus_clicked = false;
        match event.kind {
            MouseEventKind::Moved => self.pointer = Some((event.column, event.row)),
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
                let column = usize::from(event.column - areas.stream.x);
                // The marker is where the mouse is, or on the cursor's row when nothing moved it.
                if self.pointer.is_some() {
                    self.pointer = Some((event.column, event.row));
                }
                plus_clicked = self
                    .plus()
                    .is_some_and(|p| p.row == row && (p.col..p.col + PLUS_WIDTH).contains(&column));
                if row < self.stream.len() {
                    self.panel = Panel::Stream;
                    self.move_to(row);
                    if matches!(self.stream.file_row(row), Some(FileRow::Pair { .. })) {
                        let left = split_widths(usize::from(areas.stream.width)).0;
                        let column = usize::from(event.column - areas.stream.x);
                        self.half = Some(if column < left { Side::Old } else { Side::New });
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(sidebar) = areas.sidebar.filter(|rect| at(*rect)) {
                    let selected = self.stream.side_row_of(self.stream.file_at(self.cursor));
                    let top = sidebar_top(
                        selected,
                        usize::from(sidebar.height),
                        self.stream.side.len(),
                    );
                    let row = top + usize::from(event.row - sidebar.y);
                    if let Some(SideRow::File(file)) = self.stream.side.get(row) {
                        self.panel = Panel::Sidebar;
                        self.move_to_file(*file);
                    }
                }
            }
            _ => {}
        }
        plus_clicked
    }
}

/// The first file the sidebar shows, so the selected one is in the middle when the list is longer
/// than the sidebar.
fn sidebar_top(selected: usize, height: usize, files: usize) -> usize {
    selected
        .saturating_sub(height / 2)
        .min(files.saturating_sub(height))
}

/// The first `width` cells of `text`, with a trailing `…` when something was cut.
fn head_to_width(text: &str, width: usize) -> String {
    if string_width(text) <= width {
        return text.to_owned();
    }
    let mut kept = truncate_to_width(text, width.saturating_sub(1));
    kept.push('…');
    kept
}

/// The sign of a code row, the colour of its text, and the tint behind it.
fn code_style(kind: RowKind, theme: &Theme) -> (char, Style, Style) {
    match kind {
        RowKind::Context => (' ', Style::new(), Style::new()),
        RowKind::Added => (
            '+',
            Style::new().fg(theme.added),
            Style::new().bg(theme.added_bg),
        ),
        RowKind::Removed => (
            '-',
            Style::new().fg(theme.removed),
            Style::new().bg(theme.removed_bg),
        ),
    }
}

/// `text` cut to `width` cells and padded with spaces to exactly `width`.
fn fitted(text: &str, width: usize) -> String {
    let mut text = truncate_to_width(text, width);
    text.push_str(&" ".repeat(width.saturating_sub(string_width(&text))));
    text
}

/// One half of a split row: the line number, the sign and the text on `side`, or an empty half
/// when the other side has a line and this one does not.
fn half_spans(row: Option<&Row>, side: Side, width: usize, theme: &Theme) -> Vec<Span<'static>> {
    let Some(row) = row else {
        return vec![Span::styled(
            " ".repeat(width),
            Style::new().bg(theme.filler),
        )];
    };
    let (sign, text_style, tint) = code_style(row.kind, theme);
    let number = row
        .line(side)
        .map_or_else(|| "    ".to_owned(), |n| format!("{n:>4}"));
    let gutter = format!("{number} {sign} ");
    let mut text = sanitize_terminal_text(&row.text);
    if row.no_newline {
        text.push_str("  [no newline at end of file]");
    }
    let room = width.saturating_sub(gutter.len());
    vec![
        Span::styled(truncate_to_width(&gutter, width), theme.dim().patch(tint)),
        Span::styled(fitted(&text, room), text_style.patch(tint)),
    ]
}

/// A file's header: its name on the left, the added and removed counts on the right.
fn file_header(file: &DiffFile, width: usize, theme: &Theme) -> Line<'static> {
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
    let (added, removed) = file.stat();
    let counts = match (added, removed) {
        (0, 0) => String::new(),
        (a, 0) => format!("+{a}"),
        (0, r) => format!("-{r}"),
        (a, r) => format!("+{a} -{r}"),
    };
    let room = width.saturating_sub(string_width(&counts) + 1);
    let head = fitted(&text, room);
    let bold = Style::new().bg(theme.header).add_modifier(Modifier::BOLD);
    let mut spans = vec![Span::styled(head, bold)];
    let counts_width = string_width(&counts);
    spans.push(Span::styled(
        " ".repeat(width.saturating_sub(room + counts_width)),
        bold,
    ));
    if added > 0 {
        spans.push(Span::styled(format!("+{added}"), bold.fg(theme.added)));
        if removed > 0 {
            spans.push(Span::styled(" ", bold));
        }
    }
    if removed > 0 {
        spans.push(Span::styled(format!("-{removed}"), bold.fg(theme.removed)));
    }
    Line::from(spans)
}

fn row_line(stream: &Stream, row: RowRef, width: usize, theme: &Theme) -> Line<'static> {
    match row {
        RowRef::BlockHeader(count) => Line::styled(
            truncate_to_width(&format!("Comments not in this diff ({count})"), width),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        RowRef::Card { thread, line } => stream
            .cards
            .get(thread)
            .and_then(|card| card.lines.get(line))
            .cloned()
            .unwrap_or_default(),
        RowRef::Empty(spec) => Line::from(truncate_to_width(&empty_message(spec), width)),
        RowRef::File(file) => file_header(file, width, theme),
        RowRef::Note(file) => Line::styled(format!("  {}", note(file)), theme.dim()),
        RowRef::Hunk(hunk) => Line::styled(
            truncate_to_width(&sanitize_terminal_text(&hunk.header), width),
            Style::new().fg(theme.accent),
        ),
        RowRef::Gap(count) => {
            let noun = if count == 1 { "line" } else { "lines" };
            let text = format!("▾ {count} unchanged {noun}");
            Line::styled(fitted(&text, width), theme.dim().bg(theme.header))
        }
        RowRef::Pair { old, new } => {
            let (left, right) = split_widths(width);
            let mut spans = half_spans(old, Side::Old, left, theme);
            spans.push(Span::styled("│", Style::new().fg(theme.border)));
            spans.extend(half_spans(new, Side::New, right, theme));
            Line::from(spans)
        }
        RowRef::Line(row) => {
            let number =
                |line: Option<u32>| line.map_or_else(|| "     ".to_owned(), |n| format!("{n:>5}"));
            let gutter = format!("{}{} ", number(row.old), number(row.new));
            let (sign, style, tint) = code_style(row.kind, theme);
            let mut text = format!("{sign}{}", sanitize_terminal_text(&row.text));
            if row.no_newline {
                text.push_str("  [no newline at end of file]");
            }
            let room = width.saturating_sub(gutter.len());
            Line::from(vec![
                Span::styled(gutter, theme.dim().patch(tint)),
                Span::styled(fitted(&text, room), style.patch(tint)),
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
/// window are built. `mark` is the rows the open editor comments on. While it is set they are
/// marked and no `[+]` is offered, since the mouse does nothing until the editor closes.
pub fn draw(
    frame: &mut Frame,
    view: &View,
    diff: &Diff,
    keymap: &Keymap,
    theme: &Theme,
    mark: Option<(usize, usize)>,
) {
    let areas = areas(frame.area());
    let cursor_style = Style::new().bg(theme.cursor);
    let height = usize::from(areas.stream.height);
    let width = usize::from(areas.stream.width);
    let lines = (view.scroll..view.scroll + height)
        .filter_map(|row| view.stream.locate(diff, row))
        .map(|row| row_line(&view.stream, row, width, theme))
        .collect::<Vec<_>>();
    frame.render_widget(Clear, areas.stream);
    frame.render_widget(Paragraph::new(lines), areas.stream);
    if let Some(start) = view.select {
        let (low, high) = (start.min(view.cursor), start.max(view.cursor));
        for row in low.max(view.scroll)..=high.min(view.scroll + height.saturating_sub(1)) {
            highlight(
                frame.buffer_mut(),
                areas.stream,
                row - view.scroll,
                Style::new().bg(theme.selection),
            );
        }
    }
    if view.cursor >= view.scroll && view.cursor < view.scroll + height {
        let style = if view.panel == Panel::Stream {
            cursor_style
        } else {
            theme.dim()
        };
        highlight(
            frame.buffer_mut(),
            areas.stream,
            view.cursor - view.scroll,
            style,
        );
    }
    if let Some(rows) = mark {
        draw_mark(frame, view, rows, theme);
    } else if let Some(plus) = view.plus() {
        let shown = plus.row >= view.scroll && plus.row < view.scroll + height;
        if shown && plus.col + PLUS_WIDTH <= width {
            let x = areas.stream.x + u16::try_from(plus.col).unwrap_or(0);
            let y = areas.stream.y + u16::try_from(plus.row - view.scroll).unwrap_or(0);
            let style = Style::new()
                .fg(theme.base)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD);
            frame.buffer_mut().set_string(x, y, "[+]", style);
        }
    }
    if let Some(sidebar) = areas.sidebar {
        draw_sidebar(frame, sidebar, view, diff, theme);
    }
    if view.help {
        draw_help(frame, keymap, theme);
    }
}

/// Mark rows `low..=high` of the stream as what the open editor comments on: a tint across the
/// row and a bar in its first cell. A cell that holds a digit of a line number keeps it.
fn draw_mark(frame: &mut Frame, view: &View, (low, high): (usize, usize), theme: &Theme) {
    let area = areas(frame.area()).stream;
    let last = view.scroll + usize::from(area.height).saturating_sub(1);
    for row in low.max(view.scroll)..=high.min(last) {
        let at = row - view.scroll;
        highlight(
            frame.buffer_mut(),
            area,
            at,
            Style::new().bg(theme.selection),
        );
        let y = area.y + u16::try_from(at).unwrap_or(0);
        if let Some(cell) = frame.buffer_mut().cell_mut((area.x, y))
            && cell.symbol() == " "
        {
            cell.set_symbol("▌").set_fg(theme.warning);
        }
    }
}

/// Where an editor `height` rows tall goes in `stream` when the row it belongs to is row `at` of
/// it: under that row, else above it, else at the bottom. It is as wide as the stream.
pub fn editor_rect(stream: Rect, at: usize, height: u16) -> Rect {
    let height = height.min(stream.height);
    let at = u16::try_from(at)
        .unwrap_or(0)
        .min(stream.height.saturating_sub(1));
    let y = if at + 1 + height <= stream.height {
        at + 1
    } else if at >= height {
        at - height
    } else {
        stream.height - height
    };
    Rect::new(stream.x, stream.y + y, stream.width, height)
}

/// One file of the sidebar: a mark for unsent comments, the letter, the name, and the counts
/// pushed to the right edge. The name gives way first.
fn side_file_line(file: &DiffFile, unsent: bool, width: usize, theme: &Theme) -> Line<'static> {
    let (added, removed) = file.stat();
    let mut counts = Vec::new();
    if added > 0 {
        counts.push(Span::styled(
            format!("+{added}"),
            Style::new().fg(theme.added),
        ));
    }
    if removed > 0 {
        if !counts.is_empty() {
            counts.push(Span::raw(" "));
        }
        counts.push(Span::styled(
            format!("-{removed}"),
            Style::new().fg(theme.removed),
        ));
    }
    let counts_width: usize = counts.iter().map(|span| string_width(&span.content)).sum();
    let name = file
        .path
        .as_str()
        .rsplit_once('/')
        .map_or(file.path.as_str(), |(_, name)| name);
    let mark = if unsent { '•' } else { ' ' };
    let head = format!(
        "{mark}{} {}",
        glyph(file.change),
        sanitize_terminal_text(name)
    );
    let room = width.saturating_sub(counts_width + usize::from(counts_width > 0));
    let head = head_to_width(&head, room);
    let pad = width.saturating_sub(string_width(&head) + counts_width);
    let mut spans = vec![Span::raw(head), Span::raw(" ".repeat(pad))];
    spans.extend(counts);
    Line::from(spans)
}

fn draw_sidebar(frame: &mut Frame, area: Rect, view: &View, diff: &Diff, theme: &Theme) {
    let block = Block::new()
        .borders(Borders::RIGHT)
        .border_style(Style::new().fg(theme.border));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let selected = view.stream.side_row_of(view.stream.file_at(view.cursor));
    let height = usize::from(inner.height);
    let top = sidebar_top(selected, height, view.stream.side.len());
    let width = usize::from(inner.width);
    let lines = view
        .stream
        .side
        .iter()
        .skip(top)
        .take(height)
        .map(|row| match row {
            SideRow::Heading(text) => Line::styled(tail_to_width(text, width), theme.dim()),
            SideRow::File(index) => diff.files.get(*index).map_or_else(Line::default, |file| {
                let unsent = view.stream.unsent.get(*index).copied().unwrap_or(false);
                side_file_line(file, unsent, width, theme)
            }),
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), inner);
    let style = if view.panel == Panel::Sidebar {
        Style::new().bg(theme.cursor)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    };
    if selected >= top && selected < top + height {
        highlight(frame.buffer_mut(), inner, selected - top, style);
    }
}

/// The border of the help overlay and the prompts.
pub fn popup_block(theme: &Theme) -> Block<'static> {
    Block::bordered().border_style(Style::new().fg(theme.border))
}

/// Every action with its current keys, drawn from the effective keymap.
fn draw_help(frame: &mut Frame, keymap: &Keymap, theme: &Theme) {
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
        Paragraph::new(lines)
            .block(popup_block(theme).title(" keys, any key closes "))
            .style(Style::new().bg(theme.popup)),
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
                at: String::new(),
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
        view.resize(Rect::new(0, 0, width, height));
        view.rebuild(diff, review, None, &Look::test());
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
            .draw(|frame| draw(frame, view, diff, keymap, &Theme::default(), None))
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
            RowRef::Line(_) | RowRef::Pair { .. } => "line",
            RowRef::Gap(_) => "gap",
            RowRef::BlockHeader(_) => "block",
            RowRef::Card { .. } => "card",
            RowRef::Empty(_) => "empty",
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

    // With `review()` at 80 columns the stream is 60 wide. A card is a box: two borders around
    // its body, and the old line text when it is outdated or not in the diff.
    // The block of threads not in the diff is rows 0..=4: the heading, and u4's four rows.
    // a.rs starts at 5: header 5, hunk 6, a1 7, a2 8, A2 9, then u3 (rows 10-12), a3 13, then
    // u1 (14-17), hunk 18, a10 19, a11 20, a12 21. b.rs starts at 22, and u2 is rows 23-25.
    // Its hunk is 26, b1 27, b2 28, img.png 29, and its note 30.

    #[test]
    fn cards_take_rows_under_the_lines_they_are_placed_at() {
        let diff = diff_of(PATCH);
        let review = review();
        let view = view(&diff, &review, 80, 12);
        assert_eq!(view.stream.len(), 31);
        assert_eq!(view.stream.hunk_rows, [6, 18, 26]);
        assert_eq!(view.stream.thread_rows, [1, 10, 14, 23]);
        assert_eq!(view.stream.file_start(1), Some(22));
        assert_eq!(
            view.stream.placements,
            [
                Placement::Outdated { near: Some(3) },
                Placement::Matched { line: None },
                Placement::Matched { line: Some(2) },
                Placement::NotInDiff,
            ]
        );
        let kinds = (0..31)
            .map(|row| match view.stream.locate(&diff, row).unwrap() {
                RowRef::BlockHeader(_) => "block".to_owned(),
                RowRef::Card { thread, line } => format!("t{thread}.{line}"),
                RowRef::File(_) => "file".to_owned(),
                RowRef::Hunk(_) => "hunk".to_owned(),
                RowRef::Line(row) => row.text.clone(),
                RowRef::Pair { .. } | RowRef::Gap(_) => "split".to_owned(),
                RowRef::Note(_) => "note".to_owned(),
                RowRef::Empty(_) => "empty".to_owned(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            kinds.join(" "),
            "block t3.0 t3.1 t3.2 t3.3 file hunk a1 a2 A2 t2.0 t2.1 t2.2 a3 t0.0 t0.1 t0.2 t0.3 \
             hunk a10 a11 a12 file t1.0 t1.1 t1.2 hunk b1 b2 file note"
        );
        assert!(view.stream.locate(&diff, 31).is_none());
    }

    #[test]
    fn next_and_previous_thread_jump_between_cards() {
        let diff = diff_of(PATCH);
        let review = review();
        let mut view = view(&diff, &review, 80, 12);
        let mut seen = Vec::new();
        for _ in 0..5 {
            view.apply(Action::NextThread);
            seen.push(view.cursor);
        }
        assert_eq!(seen, [1, 10, 14, 23, 23]);
        let mut back = Vec::new();
        for _ in 0..5 {
            view.apply(Action::PrevThread);
            back.push(view.cursor);
        }
        assert_eq!(back, [14, 10, 1, 1, 1]);
    }

    #[test]
    fn the_focused_thread_is_the_card_the_cursor_is_in_or_the_first_under_its_line() {
        let diff = diff_of(PATCH);
        let review = review();
        let mut view = view(&diff, &review, 80, 12);
        let focused = |view: &mut View, row| {
            view.move_to(row);
            view.focused()
        };
        // u3's card, and the A2 line it hangs under.
        assert_eq!(focused(&mut view, 11), Some(2));
        assert_eq!(focused(&mut view, 9), Some(2));
        // The file header with a file comment, and the line u1 hangs under.
        assert_eq!(focused(&mut view, 22), Some(1));
        assert_eq!(focused(&mut view, 13), Some(0));
        // The block, its heading, a line with no card, and a hunk header.
        assert_eq!(focused(&mut view, 3), Some(3));
        assert_eq!(focused(&mut view, 0), None);
        assert_eq!(focused(&mut view, 7), None);
        assert_eq!(focused(&mut view, 18), None);
    }

    #[test]
    fn a_card_is_drawn_for_each_placement_case() {
        let diff = diff_of(PATCH);
        let review = review();
        let mut view = view(&diff, &review, 80, 30);
        let screen = fresh(&view, &diff);
        let rows = screen
            .lines()
            .map(|row| row.split_once('│').map_or(row, |(_, rest)| rest).to_owned())
            .collect::<Vec<_>>();
        let at = |needle: &str| rows.iter().position(|row| row.contains(needle)).unwrap();
        // Not in the diff: the block comes first, with where it pointed.
        assert_eq!(at("Comments not in this diff (1)"), 0);
        assert!(at("● Your note · gone.rs R1 ") < at("M a.rs"));
        assert!(screen.contains("was: x"), "{screen}");
        // Matched: under the line, with no tag.
        assert_eq!(at("· a.rs R3 "), at("+A2") + 1);
        assert!(!rows[at("· a.rs R3 ")].contains("outdated"));
        // Outdated: under the nearest line, tagged, with the old text.
        assert_eq!(at("· a.rs R4 [outdated]"), at(" a3") + 1);
        assert!(screen.contains("was: not there anymore"), "{screen}");
        // A file comment: under the file header, with the bare path.
        assert_eq!(at("· b.rs [unsent]"), at("A b.rs") + 1);
        // Every box closes with the keys that act on it, against the stream's right edge.
        assert!(rows[at("· a.rs R3 ") + 2].ends_with(" r reply  e edit  d delete ╯"));
        // The cursor row is highlighted whether it is a card or a diff row.
        view.move_to(10);
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        terminal
            .draw(|frame| {
                draw(
                    frame,
                    &view,
                    &diff,
                    &Keymap::default(),
                    &Theme::default(),
                    None,
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(30, 10)].bg, Theme::default().cursor);
        assert_ne!(buffer[(30, 9)].bg, Theme::default().cursor);
    }

    #[test]
    fn a_card_is_a_box_across_the_whole_stream_in_the_side_by_side_layout() {
        let diff = diff_of(PATCH);
        let review = review();
        let view = view(&diff, &review, 130, 40);
        assert_eq!(view.stream.layout, DiffLayout::Split);
        let screen = fresh(&view, &diff);
        // The sidebar is 32 columns and the stream is the other 98.
        let boxes = screen
            .lines()
            .map(|row| row.chars().skip(32).collect::<String>())
            .filter(|row| row.starts_with('╭'))
            .collect::<Vec<_>>();
        assert_eq!(boxes.len(), 4, "{screen}");
        for top in &boxes {
            assert!(top.starts_with("╭ ● Your note · "), "{top}");
            assert!(top.ends_with("─╮"), "{top}");
            assert_eq!(string_width(top), 98, "{top}");
        }
        assert_eq!(
            screen
                .lines()
                .filter(|row| row.ends_with(" r reply  e edit  d delete ╯"))
                .count(),
            4,
            "{screen}"
        );
    }

    #[test]
    fn a_resolved_outdated_thread_is_one_line_with_no_tag() {
        let diff = diff_of(PATCH);
        let mut review = review();
        review.threads[0].status = Status::Resolved {
            by: Author::Agent(Some("claude".into())),
        };
        review.threads[0].replies.push(Comment {
            id: CommentId::parse("a1").unwrap(),
            parent: None,
            author: Author::Agent(Some("claude".into())),
            at: String::new(),
            body: "Added with_capacity".into(),
            sent_batch: None,
            edited_since_sent: false,
        });
        let view = view(&diff, &review, 120, 30);
        // The thread is still placed as outdated, and its card is one row.
        assert_eq!(
            view.stream.placements[0],
            Placement::Outdated { near: Some(3) }
        );
        assert_eq!(view.stream.len(), 28);
        let screen = fresh(&view, &diff);
        assert!(
            screen.contains("✓ u1 resolved by agent:claude: Added with_capacity"),
            "{screen}"
        );
        assert!(!screen.contains("outdated"), "{screen}");
        assert!(!screen.contains("was: not there anymore"), "{screen}");
    }

    #[test]
    fn with_an_empty_diff_the_block_still_lists_the_threads() {
        let diff = diff_of("");
        let review = Review {
            threads: vec![
                thread("u1", "a.rs", line(1, "fn main() {}")),
                thread("u2", "b.rs", AnchorTarget::File),
            ],
            ..Review::default()
        };
        let view = view(&diff, &review, 80, 20);
        let screen = fresh(&view, &diff);
        assert!(screen.contains("Comments not in this diff (2)"), "{screen}");
        assert!(screen.contains("· a.rs R1 "), "{screen}");
        assert!(screen.contains("· b.rs [unsent]"), "{screen}");
        assert!(
            screen.contains("No changes in the working tree."),
            "{screen}"
        );
        assert_eq!(view.stream.thread_rows, [1, 5]);
        // With no threads either, the message is the only row.
        let bare = fresh(&view_of(&diff), &diff);
        assert!(!bare.contains("not in this diff"));
    }

    #[test]
    fn a_comment_on_a_renamed_files_old_name_hangs_under_the_new_file() {
        let patch = "diff --git a/old.rs b/new.rs\nsimilarity index 90%\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -1 +1 @@\n-a\n+b\n";
        let diff = diff_of(patch);
        let mut review = Review {
            threads: vec![thread("a1", "old.rs", AnchorTarget::File)],
            ..Review::default()
        };
        review.threads[0].root.author = Author::Agent(None);
        let view = view(&diff, &review, 80, 12);
        assert_eq!(view.stream.len(), 7);
        assert_eq!(view.stream.thread_rows, [1]);
    }

    #[test]
    fn a_narrow_stream_wraps_a_long_body_into_more_rows() {
        let diff = diff_of(PATCH);
        let mut review = review();
        review.threads[2].root.body = "word ".repeat(30);
        let wide = view(&diff, &review, 120, 12);
        let narrow = view(&diff, &review, 60, 12);
        assert!(narrow.stream.len() > wide.stream.len());
        assert!(!wide.needs_rebuild() && !narrow.needs_rebuild());
        let mut changed = narrow.clone();
        changed.resize(Rect::new(0, 0, 120, 12));
        assert!(changed.needs_rebuild());
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
        // A click in the sidebar on the second file. Row 0 is the directory heading.
        view.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 2));
        assert_eq!((view.cursor, view.panel), (10, Panel::Sidebar));
        // A click on the heading, or below the last file, does nothing.
        view.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 0));
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
    fn a_long_name_is_cut_at_its_end_in_the_sidebar_and_long_lines_are_cut() {
        let long = "x".repeat(60);
        let patch = format!(
            "diff --git a/dir/{long}.rs b/dir/{long}.rs\n--- a/dir/{long}.rs\n+++ b/dir/{long}.rs\n@@ -1 +1 @@\n-{long}{long}\n+new\n"
        );
        let diff = diff_of(&patch);
        let view = view_of(&diff);
        let screen = fresh(&view, &diff);
        assert!(screen.contains("dir/"), "{screen}");
        assert!(
            screen.contains(" M xx") && screen.contains("… +1 -1│"),
            "{screen}"
        );
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
        let anchor = view.spot(&diff);
        // The first file lost its second hunk, so b.rs starts earlier.
        let smaller = diff_of(&PATCH.replace("@@ -10,2 +10,3 @@\n a10\n+a11\n a12\n", ""));
        view.rebuild(&smaller, &Review::default(), anchor, &Look::test());
        assert_eq!(view.cursor, 8);
        assert!(
            matches!(view.stream.locate(&smaller, 8), Some(RowRef::Line(row)) if row.text == "b1")
        );
        // The file is gone: the cursor stays on the nearest row that exists.
        let anchor = view.spot(&smaller);
        view.rebuild(&diff_of(""), &Review::default(), anchor, &Look::test());
        assert_eq!(view.cursor, 0);
    }

    #[test]
    fn a_rebuild_keeps_the_cursor_on_its_line_of_the_same_card() {
        let diff = diff_of(PATCH);
        let mut review = review();
        let mut view = view(&diff, &review, 80, 12);
        // Line 1 of u3's card, which is row 11.
        view.move_to(11);
        // A new thread above it pushes everything down by its rows.
        review.threads.push(thread("u5", "a.rs", line(1, "a1")));
        let spot = view.spot(&diff);
        view.rebuild(&diff, &review, spot, &Look::test());
        assert!(matches!(
            view.stream.locate(&diff, view.cursor),
            Some(RowRef::Card { thread: 2, line: 1 })
        ));
        assert_eq!(view.cursor, 11 + 3);
        // The thread is deleted: the cursor goes to the line it hung under.
        review.threads.remove(2);
        let spot = view.spot(&diff);
        view.rebuild(&diff, &review, spot, &Look::test());
        assert!(
            matches!(view.stream.locate(&diff, view.cursor), Some(RowRef::Line(row)) if row.text == "A2")
        );
    }

    fn anchor_at(view: &mut View, diff: &Diff, row: usize) -> Result<Anchor, &'static str> {
        view.move_to(row);
        view.capture(diff)
    }

    fn target_at(view: &mut View, diff: &Diff, row: usize) -> Result<AnchorTarget, &'static str> {
        anchor_at(view, diff, row).map(|anchor| anchor.target)
    }

    #[test]
    fn a_comment_on_a_line_points_at_its_side_number_and_text() {
        let (diff, mut view) = plain();
        assert_eq!(target_at(&mut view, &diff, 4), Ok(line(2, "A2")));
        let removed = AnchorTarget::Line {
            side: Side::Old,
            line: 2,
            text: "a2".into(),
        };
        assert_eq!(target_at(&mut view, &diff, 3), Ok(removed));
        // A context row is on the new side.
        assert_eq!(target_at(&mut view, &diff, 2), Ok(line(1, "a1")));
        let anchor = anchor_at(&mut view, &diff, 4).unwrap();
        assert_eq!(anchor.path.as_str(), "a.rs");
        assert_eq!((anchor.old_path, anchor.spec), (None, Spec::WorkTree));
    }

    #[test]
    fn a_comment_on_a_file_header_or_its_note_points_at_the_file() {
        let (diff, mut view) = plain();
        assert_eq!(target_at(&mut view, &diff, 0), Ok(AnchorTarget::File));
        let binary = anchor_at(&mut view, &diff, 15).unwrap();
        assert_eq!(binary.target, AnchorTarget::File);
        assert_eq!(binary.path.as_str(), "img.png");
        assert_eq!(
            target_at(&mut view, &diff, 1),
            Err("comment on a line or a file header")
        );
    }

    #[test]
    fn a_range_runs_from_where_it_started_to_the_cursor_on_the_side_of_its_first_line() {
        let (diff, mut view) = plain();
        view.move_to(2);
        view.toggle_select();
        let range = |side, start, end, text: &str| AnchorTarget::Range {
            side,
            start,
            end,
            text: text.into(),
        };
        // From a1 to a3 on the new side, over the removed row, which has no new line.
        assert_eq!(
            target_at(&mut view, &diff, 5),
            Ok(range(Side::New, 1, 3, "a1"))
        );
        // Upwards is the same range.
        let mut upwards = View::default();
        upwards.resize(Rect::new(0, 0, 80, 12));
        upwards.rebuild(&diff, &Review::default(), None, &Look::test());
        upwards.move_to(5);
        upwards.toggle_select();
        assert_eq!(
            target_at(&mut upwards, &diff, 2),
            Ok(range(Side::New, 1, 3, "a1"))
        );
        // Starting on a removed row, the range is on the old side.
        let mut old = View::default();
        old.resize(Rect::new(0, 0, 80, 12));
        old.rebuild(&diff, &Review::default(), None, &Look::test());
        old.move_to(3);
        old.toggle_select();
        assert_eq!(
            target_at(&mut old, &diff, 5),
            Ok(range(Side::Old, 2, 3, "a2"))
        );
        // Across hunks, the end is the last line on that side.
        assert_eq!(
            target_at(&mut old, &diff, 9),
            Ok(range(Side::Old, 2, 11, "a2"))
        );
        // Where it started is where it ends: one line.
        assert_eq!(
            target_at(&mut old, &diff, 3).map(|t| matches!(t, AnchorTarget::Line { .. })),
            Ok(true)
        );
    }

    #[test]
    fn a_range_cannot_cross_files_or_hold_no_lines() {
        let (diff, mut view) = plain();
        view.move_to(2);
        view.toggle_select();
        assert_eq!(
            target_at(&mut view, &diff, 12),
            Err("a range stays inside one file")
        );
        // The file header and the hunk header under it hold no line.
        view.select = Some(0);
        assert_eq!(
            target_at(&mut view, &diff, 1),
            Err("select lines to comment on")
        );
        // Selecting again drops the range, and a rebuild does too.
        view.toggle_select();
        assert_eq!(view.select, None);
        view.toggle_select();
        assert_eq!(view.select, Some(1));
        view.rebuild(&diff, &Review::default(), None, &Look::test());
        assert_eq!(view.select, None);
    }

    #[test]
    fn a_comment_written_from_a_card_points_where_the_card_hangs() {
        let diff = diff_of(PATCH);
        let review = review();
        let mut view = view(&diff, &review, 80, 12);
        // u3's card hangs under A2, and u2's under the b.rs header.
        assert_eq!(target_at(&mut view, &diff, 11), Ok(line(2, "A2")));
        assert_eq!(target_at(&mut view, &diff, 22), Ok(AnchorTarget::File));
        // A thread in the block has no file in the diff to point into.
        let block = anchor_at(&mut view, &diff, 3).unwrap_err();
        assert!(block.contains("reply"), "{block}");
        assert!(anchor_at(&mut view, &diff, 0).is_err());
    }

    #[test]
    fn a_comment_on_a_renamed_file_keeps_the_old_name_and_the_spec_on_screen() {
        let patch = "diff --git a/old.rs b/new.rs\nsimilarity index 90%\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -1 +1 @@\n-a\n+b\n";
        let mut diff = diff_of(patch);
        diff.spec = Spec::Branch {
            base: "main".into(),
        };
        let mut view = view_of(&diff);
        let anchor = anchor_at(&mut view, &diff, 2).unwrap();
        assert_eq!(anchor.path.as_str(), "new.rs");
        assert_eq!(anchor.old_path.unwrap().as_str(), "old.rs");
        assert_eq!(anchor.spec, diff.spec);
    }

    #[test]
    fn the_focused_comment_is_the_one_whose_line_of_the_card_the_cursor_is_on() {
        let diff = diff_of(PATCH);
        let mut review = review();
        review.threads[2].replies.push(Comment {
            id: CommentId::parse("a1").unwrap(),
            parent: None,
            author: Author::Agent(None),
            at: String::new(),
            body: "done".into(),
            sent_batch: None,
            edited_since_sent: false,
        });
        let mut view = view(&diff, &review, 80, 14);
        let at = |view: &mut View, row| {
            view.move_to(row);
            view.focused_comment()
        };
        // u3's card is rows 10 to 13 now: the top border, the body, the reply, the bottom border.
        assert_eq!(at(&mut view, 10), Some((2, 0)));
        assert_eq!(at(&mut view, 11), Some((2, 0)));
        assert_eq!(at(&mut view, 12), Some((2, 1)));
        // The bottom border names the keys that act on the root, so it is the root.
        assert_eq!(at(&mut view, 13), Some((2, 0)));
        // Past the box is a3, the line u1 hangs under.
        assert_eq!(at(&mut view, 14), Some((0, 0)));
        // The line the card hangs under counts as the root.
        assert_eq!(at(&mut view, 9), Some((2, 0)));
        assert_eq!(at(&mut view, 7), None);
        assert_eq!(at(&mut view, 0), None);
        assert_eq!(view.thread_id(2).unwrap().as_str(), "u3");
    }

    #[test]
    fn focusing_a_thread_puts_the_cursor_on_its_card() {
        let diff = diff_of(PATCH);
        let review = review();
        let mut view = view(&diff, &review, 80, 12);
        view.focus_thread(&CommentId::parse("u2").unwrap());
        assert_eq!(view.cursor, 23);
        view.focus_thread(&CommentId::parse("u9").unwrap());
        assert_eq!(view.cursor, 23);
    }

    #[test]
    fn the_editor_goes_under_the_cursor_then_above_it_then_to_the_bottom() {
        let stream = Rect::new(20, 0, 60, 20);
        // It is as wide as the stream, with no indent under the code.
        assert_eq!(editor_rect(stream, 5, 4), Rect::new(20, 6, 60, 4));
        assert_eq!(editor_rect(stream, 17, 4), Rect::new(20, 13, 60, 4));
        assert_eq!(editor_rect(stream, 2, 19), Rect::new(20, 1, 60, 19));
        assert_eq!(
            editor_rect(Rect::new(0, 3, 30, 8), 1, 20),
            Rect::new(0, 3, 30, 8)
        );
    }

    #[test]
    fn a_comment_s_rows_are_its_line_its_range_or_its_file_header_in_both_layouts() {
        let diff = diff_of(PATCH);
        let anchor = |path: &str, target| Anchor {
            path: RelPath::parse(path).unwrap(),
            old_path: None,
            target,
            spec: Spec::WorkTree,
        };
        let range = |side, start, end| AnchorTarget::Range {
            side,
            start,
            end,
            text: String::new(),
        };
        let old = AnchorTarget::Line {
            side: Side::Old,
            line: 2,
            text: "a2".into(),
        };
        // Unified: a.rs is rows 0..=9, with `-a2` on row 3 and `+A2` on row 4.
        let unified = view(&diff, &Review::default(), 80, 12);
        assert_eq!(
            unified.rows_of(&diff, &anchor("a.rs", line(2, "A2"))),
            Some((4, 4))
        );
        assert_eq!(
            unified.rows_of(&diff, &anchor("a.rs", old.clone())),
            Some((3, 3))
        );
        assert_eq!(
            unified.rows_of(&diff, &anchor("a.rs", range(Side::New, 2, 11))),
            Some((4, 8))
        );
        assert_eq!(
            unified.rows_of(&diff, &anchor("b.rs", AnchorTarget::File)),
            Some((10, 10))
        );
        assert_eq!(
            unified.rows_of(&diff, &anchor("gone.rs", line(1, "x"))),
            None
        );
        assert_eq!(unified.rows_of(&diff, &anchor("a.rs", line(99, "x"))), None);
        // Side by side pairs `-a2` with `+A2`, so both sides of line 2 are one row.
        let split = view(&diff, &Review::default(), 130, 12);
        let new = split.rows_of(&diff, &anchor("a.rs", line(2, "A2")));
        assert!(new.is_some());
        assert_eq!(new, split.rows_of(&diff, &anchor("a.rs", old)));
    }

    #[test]
    fn the_mark_tints_its_rows_and_puts_a_bar_where_no_digit_is() {
        let (diff, view) = plain();
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal
            .draw(|frame| {
                draw(
                    frame,
                    &view,
                    &diff,
                    &Keymap::default(),
                    &theme,
                    Some((3, 4)),
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        // The stream starts at column 20.
        for y in [3, 4] {
            assert_eq!(buffer[(20, y)].symbol(), "▌", "row {y}");
            assert_eq!(buffer[(20, y)].fg, theme.warning, "row {y}");
            assert_eq!(buffer[(60, y)].bg, theme.selection, "row {y}");
        }
        assert_ne!(buffer[(20, 2)].symbol(), "▌");
        assert_ne!(buffer[(60, 5)].bg, theme.selection);
        assert!(!screen(&terminal).contains("[+]"));
    }

    #[test]
    fn a_selected_range_is_drawn_with_the_cursor_on_its_end() {
        let (diff, mut view) = plain();
        view.move_to(2);
        view.toggle_select();
        view.move_to(4);
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal
            .draw(|frame| {
                draw(
                    frame,
                    &view,
                    &diff,
                    &Keymap::default(),
                    &Theme::default(),
                    None,
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        // The stream starts at column 20. Rows 2 and 3 are selected, and row 4 is the cursor.
        assert_ne!(buffer[(40, 1)].bg, Theme::default().selection);
        assert_eq!(buffer[(40, 2)].bg, Theme::default().selection);
        assert_eq!(buffer[(40, 3)].bg, Theme::default().selection);
        assert_eq!(buffer[(40, 4)].bg, Theme::default().cursor);
        assert_ne!(buffer[(40, 5)].bg, Theme::default().cursor);
    }

    const TREE: &str = "diff --git a/app/a.js b/app/a.js
--- a/app/a.js
+++ b/app/a.js
@@ -1,2 +1,3 @@
 keep
-gone
+one
+two
diff --git a/app/b.js b/app/b.js
--- a/app/b.js
+++ b/app/b.js
@@ -1 +1 @@
-x
+y
diff --git a/top.md b/top.md
--- a/top.md
+++ b/top.md
@@ -1 +1,2 @@
 t
+u
";

    #[test]
    fn the_sidebar_groups_files_under_their_directory_with_letters_and_counts() {
        let diff = diff_of(TREE);
        let view = view(&diff, &Review::default(), 80, 12);
        assert_eq!(
            view.stream.side,
            [
                SideRow::Heading("app/".into()),
                SideRow::File(0),
                SideRow::File(1),
                SideRow::Heading("./".into()),
                SideRow::File(2),
            ]
        );
        let screen = fresh(&view, &diff);
        let rows = screen
            .lines()
            .map(|row| row.split('│').next().unwrap_or("").to_owned());
        let rows = rows.take(5).collect::<Vec<_>>();
        assert_eq!(rows[0].trim_end(), "app/");
        assert_eq!(rows[1], format!("{:<14}+2 -1", " M a.js"));
        assert_eq!(rows[2], format!("{:<14}+1 -1", " M b.js"));
        assert_eq!(rows[3].trim_end(), "./");
        assert_eq!(rows[4], format!("{:<17}+1", " M top.md"));
    }

    #[test]
    fn a_directory_that_comes_back_gets_a_second_heading() {
        let patch = format!(
            "{TREE}diff --git a/app/z.js b/app/z.js\n--- a/app/z.js\n+++ b/app/z.js\n@@ -1 +1 @@\n-1\n+2\n"
        );
        let diff = diff_of(&patch);
        let headings = sidebar_rows(&diff)
            .into_iter()
            .filter(|row| matches!(row, SideRow::Heading(_)))
            .count();
        assert_eq!(headings, 3);
    }

    #[test]
    fn a_file_with_an_unsent_thread_carries_a_mark_and_the_others_do_not() {
        let diff = diff_of(TREE);
        let mut review = Review {
            threads: vec![thread("u1", "app/b.js", line(1, "y"))],
            ..Review::default()
        };
        review.threads[0].unsent = true;
        let view = view(&diff, &review, 80, 12);
        assert_eq!(view.stream.unsent, [false, true, false]);
        let screen = fresh(&view, &diff);
        assert!(
            screen.contains("•M b.js") && screen.contains(" M a.js"),
            "{screen}"
        );
        review.threads[0].unsent = false;
        let sent = self::view(&diff, &review, 80, 12);
        assert_eq!(sent.stream.unsent, [false, false, false]);
    }

    #[test]
    fn the_sidebar_follows_the_cursor_through_headings() {
        let diff = diff_of(TREE);
        let mut view = view(&diff, &Review::default(), 80, 4);
        view.apply(Action::SwitchPanel);
        view.apply(Action::Down);
        view.apply(Action::Down);
        assert_eq!(view.stream.file_at(view.cursor), 2);
        assert!(fresh(&view, &diff).contains("M top.md"));
    }

    const SPLIT_PATCH: &str = "diff --git a/s.rs b/s.rs
--- a/s.rs
+++ b/s.rs
@@ -3,5 +3,4 @@
 c3
-old4
-old5
+new4
 c6
 c7
@@ -20,2 +19,3 @@
 c20
+add21
 c22
";
    // Split rows of s.rs: header 0, gap 1, hunk 2, c3 3, old4/new4 4, old5/- 5, c6 6, c7 7, gap 8,
    // hunk 9, c20 10, -/add21 11, c22 12.

    fn kinds_of(view: &View, diff: &Diff) -> Vec<String> {
        (0..view.stream.len())
            .map(|row| match view.stream.locate(diff, row).unwrap() {
                RowRef::File(_) => "file".to_owned(),
                RowRef::Hunk(_) => "hunk".to_owned(),
                RowRef::Gap(n) => format!("gap{n}"),
                RowRef::Pair { old, new } => format!(
                    "{}|{}",
                    old.map_or("-", |row| row.text.as_str()),
                    new.map_or("-", |row| row.text.as_str())
                ),
                RowRef::Line(row) => row.text.clone(),
                _ => "other".to_owned(),
            })
            .collect()
    }

    #[test]
    fn a_wide_pane_pairs_removed_with_added_and_marks_the_unchanged_lines() {
        let diff = diff_of(SPLIT_PATCH);
        let view = view(&diff, &Review::default(), 130, 14);
        assert_eq!(view.layout(), DiffLayout::Split);
        assert_eq!(
            kinds_of(&view, &diff),
            [
                "file",
                "gap2",
                "hunk",
                "c3|c3",
                "old4|new4",
                "old5|-",
                "c6|c6",
                "c7|c7",
                "gap12",
                "hunk",
                "c20|c20",
                "-|add21",
                "c22|c22"
            ]
        );
        assert_eq!(view.stream.hunk_rows, [2, 9]);
    }

    #[test]
    fn a_narrow_pane_stays_unified_without_gap_rows() {
        let diff = diff_of(SPLIT_PATCH);
        let view = view(&diff, &Review::default(), 119, 14);
        assert_eq!(view.layout(), DiffLayout::Unified);
        assert_eq!(
            kinds_of(&view, &diff),
            [
                "file", "hunk", "c3", "old4", "old5", "new4", "c6", "c7", "hunk", "c20", "add21",
                "c22"
            ]
        );
    }

    #[test]
    fn the_toggle_forces_the_other_layout_and_the_width_no_longer_decides() {
        let diff = diff_of(SPLIT_PATCH);
        let mut view = view(&diff, &Review::default(), 130, 14);
        view.toggle_layout();
        assert_eq!(view.layout(), DiffLayout::Unified);
        assert!(view.needs_rebuild());
        view.rebuild(&diff, &Review::default(), None, &Look::test());
        assert!(!view.needs_rebuild());
        view.resize(Rect::new(0, 0, 200, 14));
        assert_eq!(view.layout(), DiffLayout::Unified);
        view.toggle_layout();
        assert_eq!(view.layout(), DiffLayout::Split);
        let mut narrow = self::view(&diff, &Review::default(), 90, 14);
        narrow.toggle_layout();
        assert_eq!(narrow.layout(), DiffLayout::Split);
    }

    #[test]
    fn a_split_row_draws_old_and_new_on_one_line_with_an_empty_half_opposite() {
        let diff = diff_of(SPLIT_PATCH);
        let view = view(&diff, &Review::default(), 130, 14);
        let screen = fresh(&view, &diff);
        assert!(screen.contains("▾ 2 unchanged lines"), "{screen}");
        assert!(screen.contains("▾ 12 unchanged lines"));
        let row = screen.lines().find(|row| row.contains("old4")).unwrap();
        let (old, new) = row.split_once("old4").unwrap();
        assert!(old.ends_with("4 - ") && new.contains("new4"), "{row}");
        assert!(row.matches('│').count() >= 2, "{row}");
        let alone = screen.lines().find(|row| row.contains("old5")).unwrap();
        assert!(!alone.contains("new"), "{alone}");
        assert!(screen.lines().all(|row| string_width(row) <= 130));
        let unified = fresh(&self::view(&diff, &Review::default(), 100, 14), &diff);
        assert!(!unified.contains("unchanged"));
    }

    #[test]
    fn the_file_header_shows_the_counts_on_the_right() {
        let diff = diff_of(SPLIT_PATCH);
        let view = view(&diff, &Review::default(), 130, 14);
        let screen = fresh(&view, &diff);
        let header = screen.lines().find(|row| row.contains("M s.rs")).unwrap();
        let stream = header.split('│').nth(1).unwrap_or(header);
        assert!(stream.trim_end().ends_with("+2 -2"), "{header}");
    }

    fn click(view: &mut View, column: u16, row: u16) {
        view.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
    }

    #[test]
    fn a_comment_on_a_split_row_takes_the_new_side_unless_a_click_chose_the_old_one() {
        let diff = diff_of(SPLIT_PATCH);
        let mut view = view(&diff, &Review::default(), 130, 14);
        let line = |side, line, text: &str| AnchorTarget::Line {
            side,
            line,
            text: text.into(),
        };
        assert_eq!(
            target_at(&mut view, &diff, 4),
            Ok(line(Side::New, 4, "new4"))
        );
        // A removed line with nothing opposite is the old side, and so is a change in a hunk's end.
        assert_eq!(
            target_at(&mut view, &diff, 5),
            Ok(line(Side::Old, 5, "old5"))
        );
        assert_eq!(
            target_at(&mut view, &diff, 11),
            Ok(line(Side::New, 20, "add21"))
        );
        // Clicks: the left half is the old side, the right half the new side.
        let stream = areas(view.area).stream;
        let (left, _) = split_widths(usize::from(stream.width));
        let y = stream.y + 4;
        click(&mut view, stream.x + 3, y);
        assert_eq!(
            view.capture(&diff).unwrap().target,
            line(Side::Old, 4, "old4")
        );
        click(&mut view, stream.x + u16::try_from(left).unwrap() + 3, y);
        assert_eq!(
            view.capture(&diff).unwrap().target,
            line(Side::New, 4, "new4")
        );
        // A click on the empty half of a row falls back to the side that has the line.
        click(
            &mut view,
            stream.x + u16::try_from(left).unwrap() + 3,
            stream.y + 5,
        );
        assert_eq!(
            view.capture(&diff).unwrap().target,
            line(Side::Old, 5, "old5")
        );
        // Moving the cursor forgets the click.
        click(&mut view, stream.x + 3, y);
        view.apply(Action::Down);
        view.apply(Action::Up);
        assert_eq!(
            view.capture(&diff).unwrap().target,
            line(Side::New, 4, "new4")
        );
    }

    #[test]
    fn a_range_over_split_rows_follows_the_side_of_its_first_row() {
        let diff = diff_of(SPLIT_PATCH);
        let mut view = view(&diff, &Review::default(), 130, 14);
        view.move_to(5);
        view.toggle_select();
        let got = target_at(&mut view, &diff, 7).unwrap();
        // Starting on a removed-only row the range is old: old5, c6 (old 6), c7 (old 7).
        assert_eq!(
            got,
            AnchorTarget::Range {
                side: Side::Old,
                start: 5,
                end: 7,
                text: "old5".into()
            }
        );
        let mut view = self::view(&diff, &Review::default(), 130, 14);
        view.move_to(3);
        view.toggle_select();
        let got = target_at(&mut view, &diff, 7).unwrap();
        assert_eq!(
            got,
            AnchorTarget::Range {
                side: Side::New,
                start: 3,
                end: 6,
                text: "c3".into()
            }
        );
    }

    #[test]
    fn a_card_hangs_under_its_split_row_and_the_cursor_survives_a_layout_change() {
        let diff = diff_of(SPLIT_PATCH);
        let review = Review {
            threads: vec![thread("u1", "s.rs", line(4, "new4"))],
            ..Review::default()
        };
        let mut view = view(&diff, &review, 130, 14);
        // The card sits under old4|new4, which is row 4, so its first row is 5.
        assert_eq!(view.stream.card_rows, [5]);
        let add21 = 11 + view.stream.cards[0].lines.len();
        view.move_to(add21);
        assert!(matches!(
            view.stream.locate(&diff, view.cursor),
            Some(RowRef::Pair { new: Some(row), .. }) if row.text == "add21"
        ));
        let spot = view.spot(&diff);
        view.toggle_layout();
        view.rebuild(&diff, &review, spot, &Look::test());
        assert!(matches!(
            view.stream.locate(&diff, view.cursor),
            Some(RowRef::Line(row)) if row.text == "add21"
        ));
        let spot = view.spot(&diff);
        view.toggle_layout();
        view.rebuild(&diff, &review, spot, &Look::test());
        assert!(matches!(
            view.stream.locate(&diff, view.cursor),
            Some(RowRef::Pair { new: Some(row), .. }) if row.text == "add21"
        ));
    }

    fn motion(view: &mut View, column: u16, row: u16) {
        view.mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
    }

    fn plus_rows(screen: &str) -> Vec<usize> {
        let rows = screen.lines().enumerate();
        rows.filter(|(_, row)| row.contains("[+]"))
            .map(|(at, _)| at)
            .collect()
    }

    #[test]
    fn hovering_a_code_row_shows_a_plus_on_it_and_nowhere_else() {
        let (diff, mut view) = plain();
        let stream = areas(view.area).stream;
        // Row 2 is a1, a context line. Rows 0 and 1 are the file header and the hunk header.
        motion(&mut view, stream.x + 12, stream.y + 2);
        assert_eq!(view.plus(), Some(Plus { row: 2, col: 0 }));
        assert_eq!(plus_rows(&fresh(&view, &diff)), [2]);
        motion(&mut view, stream.x + 12, stream.y + 3);
        assert_eq!(plus_rows(&fresh(&view, &diff)), [3]);
        for header in [0, 1] {
            motion(&mut view, stream.x + 12, stream.y + header);
            assert_eq!(view.plus(), None, "row {header}");
        }
        // Off the stream, in the sidebar, there is none.
        motion(&mut view, 2, 3);
        assert_eq!(view.plus(), None);
    }

    #[test]
    fn without_any_motion_the_plus_is_on_the_cursor_row() {
        let (diff, mut view) = plain();
        view.move_to(3);
        assert_eq!(view.plus(), Some(Plus { row: 3, col: 0 }));
        assert_eq!(plus_rows(&fresh(&view, &diff)), [3]);
        view.move_to(1);
        assert_eq!(view.plus(), None);
        // Once the mouse has moved, the cursor no longer shows one.
        let stream = areas(view.area).stream;
        view.move_to(3);
        motion(&mut view, 2, 0);
        assert_eq!(view.plus(), None);
        motion(&mut view, stream.x + 12, stream.y + 4);
        assert_eq!(view.plus().map(|p| p.row), Some(4));
    }

    #[test]
    fn the_plus_follows_the_wheel_while_the_mouse_stands_still() {
        let (_, mut view) = plain();
        let stream = areas(view.area).stream;
        motion(&mut view, stream.x + 12, stream.y + 2);
        assert_eq!(view.plus().map(|p| p.row), Some(2));
        view.mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: stream.x + 12,
            row: stream.y + 2,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(view.plus().map(|p| p.row), Some(2 + view.scroll));
    }

    #[test]
    fn a_click_on_the_plus_asks_for_a_comment_and_a_click_elsewhere_only_moves_the_cursor() {
        let (_, mut view) = plain();
        let stream = areas(view.area).stream;
        let down = |view: &mut View, column: u16, row: u16| {
            view.mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            })
        };
        motion(&mut view, stream.x + 1, stream.y + 3);
        assert!(down(&mut view, stream.x + 1, stream.y + 3));
        assert_eq!(view.cursor, 3);
        assert!(!down(&mut view, stream.x + 12, stream.y + 3));
        assert!(
            !down(&mut view, stream.x + 3, stream.y + 3),
            "the cell after the marker"
        );
        // The header row has no marker to click.
        motion(&mut view, stream.x + 1, stream.y);
        assert!(!down(&mut view, stream.x + 1, stream.y));
    }

    #[test]
    fn in_a_split_row_the_plus_sits_on_the_hovered_half_and_a_click_on_it_chooses_that_half() {
        let diff = diff_of(SPLIT_PATCH);
        let mut view = view(&diff, &Review::default(), 130, 14);
        let stream = areas(view.area).stream;
        let (left, _) = split_widths(usize::from(stream.width));
        let y = stream.y + 4;
        motion(&mut view, stream.x + 10, y);
        assert_eq!(view.plus(), Some(Plus { row: 4, col: 4 }));
        let screen = fresh(&view, &diff);
        let row = screen.lines().nth(4).unwrap();
        assert!(row.contains("   4[+]old4"), "the number stays: {row}");
        let right = u16::try_from(left).unwrap() + 10;
        motion(&mut view, stream.x + right, y);
        assert_eq!(
            view.plus(),
            Some(Plus {
                row: 4,
                col: left + 5
            })
        );
        let cell = stream.x + u16::try_from(left + 5).unwrap();
        let hit = view.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: cell,
            row: y,
            modifiers: KeyModifiers::NONE,
        });
        assert!(hit);
        assert_eq!(
            view.capture(&diff).unwrap().target,
            AnchorTarget::Line {
                side: Side::New,
                line: 4,
                text: "new4".into()
            }
        );
        // On the old half of the same row it is the old line.
        motion(&mut view, stream.x + 10, y);
        let hit = view.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: stream.x + 5,
            row: y,
            modifiers: KeyModifiers::NONE,
        });
        assert!(hit);
        assert!(matches!(
            view.capture(&diff).unwrap().target,
            AnchorTarget::Line {
                side: Side::Old,
                line: 4,
                ..
            }
        ));
        // Over the empty right half of old5, the marker falls to the half that has the line.
        motion(&mut view, stream.x + right, stream.y + 5);
        assert_eq!(view.plus(), Some(Plus { row: 5, col: 4 }));
    }
}
