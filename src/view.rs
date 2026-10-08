//! The review body: one stream of every file's rows, the cursor in it, the sidebar, and how they
//! are drawn.
//!
//! The stream is a flat list of rows numbered from 0: each file's header, then its hunks, each a
//! header and its lines. `Stream` keeps only where each file starts, so a row is looked up when it
//! is drawn and nothing is laid out for files that are off screen.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::ops::Range;
use std::path::Path;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::cards::{Card, Look, card};
use crate::diff::{Change, Diff, DiffFile, Hunk, Placement, Row, RowKind, place};
use crate::icons::icon;
use crate::keymap::{Action, Keymap};
use crate::store::{Anchor, AnchorTarget, CommentId, RelPath, Review, Side, Spec, Warning};
use crate::syntax::{Cache, FileTokens, side_of};
use crate::theme::Theme;
use crate::tui::sanitize_terminal_text;
use crate::width::{char_width, string_width, tail_to_width, truncate_to_width};
use crate::words::{Marks, changed};

/// Rows the mouse wheel moves per notch.
const WHEEL_ROWS: usize = 3;

/// Cells of code one press of a scroll key, or one notch of the sideways wheel, moves.
const HSCROLL_COLS: usize = 8;

/// Rows at the top of the sidebar the filter's box takes, drawn whether or not there is a filter: its
/// two borders and the line of the query.
const FILTER_BOX: u16 = 3;

/// Cells a unified row spends before its code: two line numbers, the sign, and the spaces.
const UNIFIED_GUTTER: usize = 12;

/// Cells one half of a split row spends before its code: a line number, the sign, and the spaces.
const SPLIT_GUTTER: usize = 7;

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

/// A stream narrower than this does not dock a note to one half of a split row.
const NOTE_DOCK_MIN: usize = 84;

/// Cells to the left of a note that is not docked.
const NOTE_INDENT: usize = 4;

/// A note that is not docked is never narrower than this, unless the stream is.
const NOTE_MIN: usize = 28;

/// Where the box of a note goes in a stream `width` cells wide: the cells to its left and its
/// width. Side by side, it is the half its line is on. Otherwise, and for a file comment, it is
/// indented and runs to the right edge.
pub fn note_box(width: usize, layout: DiffLayout, side: Option<Side>) -> (usize, usize) {
    let (old, new) = split_widths(width);
    match (layout, side) {
        (DiffLayout::Split, Some(Side::New)) if width >= NOTE_DOCK_MIN => (width - new, new),
        (DiffLayout::Split, Some(Side::Old)) if width >= NOTE_DOCK_MIN => (0, old),
        _ => {
            let wide = width.saturating_sub(NOTE_INDENT).max(NOTE_MIN).min(width);
            (width - wide, wide)
        }
    }
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
        /// The row of the hunk this one is paired with: the added line that replaced a removed
        /// one, or the removed line an added one replaced.
        pair: Option<usize>,
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
                // The same pairs as side by side, so both layouts mark the same words.
                let mut pair = vec![None; hunk.rows.len()];
                for row in pairs(index, &hunk.rows) {
                    if let FileRow::Pair {
                        old: Some(old),
                        new: Some(new),
                        ..
                    } = row
                        && old != new
                    {
                        for (at, other) in [(old, new), (new, old)] {
                            if let Some(slot) = pair.get_mut(at) {
                                *slot = Some(other);
                            }
                        }
                    }
                }
                let lines = pair.into_iter().enumerate();
                rows.extend(lines.map(|(row, pair)| FileRow::Line {
                    hunk: index,
                    row,
                    pair,
                }));
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

/// The colour of a file's status letter.
const fn glyph_color(change: Change, theme: &Theme) -> Color {
    match change {
        Change::Modified => theme.warning,
        Change::Added => theme.added,
        Change::Deleted | Change::Unparsed => theme.removed,
        Change::Renamed => theme.accent,
        Change::Untracked => theme.agent,
        Change::Binary | Change::Submodule | Change::TooLarge => theme.subtle,
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
        FileRow::Line { hunk, row, .. } => line_of(hunk, Some(row)) == Some(line),
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
    /// Cells in the widest line of code of the diff.
    widest: usize,
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
    /// The rows of each file, for the layout the stream was built for. A collapsed file has its
    /// header only.
    files: Vec<Vec<FileRow>>,
    layout: DiffLayout,
    /// Per file, whether it is collapsed to its header.
    folded: Vec<bool>,
    /// Per file, the threads a collapsed file hides, in the order their cards would be in. Their
    /// row is the file's header, so a thread jump reaches them. An open file hides none.
    hidden: Vec<Vec<usize>>,
    marks: MarkCache,
}

/// The changed words of the code rows that have been drawn, by stream row: those of the removed
/// line, or the old half, then those of the added line, or the new half. Drawing fills it, so a
/// frame compares a pair of lines once and looks the answer up afterwards. A new layout starts
/// empty, and it takes no part in comparing two streams.
#[derive(Debug, Clone, Default)]
struct MarkCache(RefCell<HashMap<usize, (Marks, Marks)>>);

impl PartialEq for MarkCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for MarkCache {}

/// One row of the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SideRow {
    Heading(String),
    File(usize),
}

/// The query of the file filter, and whether keys go to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub query: String,
    /// Keys edit the query. Once applied, the query stays and the keys are the keymap's again.
    pub typing: bool,
}

/// The query's characters appear in `path` in order. A query with no upper case letter ignores
/// case.
fn matches(query: &str, path: &str) -> bool {
    let fold = !query.chars().any(char::is_uppercase);
    let mut path = path.chars().map(|c| {
        if fold {
            c.to_lowercase().next().unwrap_or(c)
        } else {
            c
        }
    });
    query.chars().all(|wanted| path.any(|c| c == wanted))
}

/// Group the files by directory, in the diff's order. A directory that comes again later, as
/// untracked files do, gets a second heading. With a `query`, a file that does not match is left
/// out, and so is the heading of a directory with no match.
fn sidebar_rows(diff: &Diff, query: Option<&str>) -> Vec<SideRow> {
    let mut rows = Vec::new();
    let mut last = None;
    for (index, file) in diff.files.iter().enumerate() {
        if query.is_some_and(|query| !matches(query, file.path.as_str())) {
            continue;
        }
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
    /// Lay out `diff` and the threads of `review` for a stream `width` cells wide. A file whose
    /// path is in `collapsed` is its header alone, and the cards of its threads are left out.
    pub fn build(
        diff: &Diff,
        review: &Review,
        width: usize,
        layout: DiffLayout,
        look: &Look,
        collapsed: &HashSet<RelPath>,
    ) -> Self {
        let folded = diff
            .files
            .iter()
            .map(|file| collapsed.contains(&file.path))
            .collect::<Vec<_>>();
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
            .map(|(thread, placement)| {
                let side = thread
                    .anchor
                    .side()
                    .filter(|_| *placement != Placement::NotInDiff);
                let (left, wide) = note_box(width, layout, side);
                card(thread, *placement, wide, look).indented(left)
            })
            .collect::<Vec<_>>();
        let mut stream = Self {
            card_rows: vec![0; cards.len()],
            ids: review
                .threads
                .iter()
                .map(|thread| thread.root.id.clone())
                .collect(),
            width,
            side: sidebar_rows(diff, None),
            unsent: vec![false; diff.files.len()],
            layout,
            ..Self::default()
        };
        let mut block = Vec::new();
        let mut hung = vec![Vec::<(usize, usize)>::new(); diff.files.len()];
        let mut tucked = hung.clone();
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
            let lists = if folded.get(file) == Some(&true) {
                &mut tucked
            } else {
                &mut hung
            };
            if let Some(list) = lists.get_mut(file) {
                list.push((offset, index));
            }
        }
        // The offsets above are rows of the whole file, so a collapsed file's threads keep the
        // order of their lines.
        let files = files
            .into_iter()
            .zip(&folded)
            .map(|(rows, folded)| if *folded { vec![FileRow::Header] } else { rows })
            .collect::<Vec<_>>();
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
        for ((rows, mut list), mut tucked) in files.iter().zip(hung).zip(tucked) {
            tucked.sort_unstable();
            for (_, thread) in &tucked {
                stream.set_card_row(*thread, total);
            }
            stream
                .hidden
                .push(tucked.into_iter().map(|(_, thread)| thread).collect());
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
            // The cards hung above a row push it down. `slots` is sorted by `at`, so one walk
            // keeps the sum of the heights above each row.
            let mut hung_above = slots.iter().peekable();
            let mut shift = 0;
            for (offset, row) in rows.iter().enumerate() {
                while let Some(slot) = hung_above.next_if(|slot| slot.at < offset) {
                    shift += slot.height;
                }
                if matches!(row, FileRow::Hunk(_)) {
                    stream.hunk_rows.push(start + offset + shift);
                }
            }
            total = start + rows.len() + above;
            stream.slots.push(slots);
        }
        stream.total = total;
        stream.widest = diff
            .files
            .iter()
            .flat_map(|file| &file.hunks)
            .flat_map(|hunk| &hunk.rows)
            .map(|row| string_width(&row.text))
            .max()
            .unwrap_or(0);
        stream.thread_rows.clone_from(&stream.card_rows);
        stream.thread_rows.sort_unstable();
        stream.cards = cards;
        stream.placements = placements;
        stream.files = files;
        stream.folded = folded;
        stream
    }

    /// Whether file `file` is collapsed to its header.
    pub fn folded(&self, file: usize) -> bool {
        self.folded.get(file) == Some(&true)
    }

    /// The threads file `file` hides while it is collapsed.
    fn hidden(&self, file: usize) -> &[usize] {
        self.hidden.get(file).map_or(&[], Vec::as_slice)
    }

    /// The collapsed file whose header is `row`, when it hides a thread.
    fn hiding_at(&self, row: usize) -> Option<usize> {
        match self.at(row)? {
            At::Base { file, offset: 0 } if !self.hidden(file).is_empty() => Some(file),
            _ => None,
        }
    }

    /// What a collapsed file hides, for its header: its lines of code and its threads.
    fn fold_of(&self, file: usize, diff: &DiffFile) -> Option<(usize, usize)> {
        let lines = diff.hunks.iter().map(|hunk| hunk.rows.len()).sum();
        self.folded(file).then(|| (lines, self.hidden(file).len()))
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

    /// The sidebar row that holds `file`, when the filter has not left it out.
    fn side_row_of(&self, file: usize) -> Option<usize> {
        self.side.iter().position(|row| *row == SideRow::File(file))
    }

    /// The files the sidebar lists, in the diff's order.
    fn side_files(&self) -> Vec<usize> {
        self.side
            .iter()
            .filter_map(|row| match row {
                SideRow::File(file) => Some(*file),
                SideRow::Heading(_) => None,
            })
            .collect()
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
            FileRow::Line { hunk, row: at, .. } => RowRef::Line(row(hunk, Some(at))?),
            FileRow::Pair { hunk, old, new } => RowRef::Pair {
                old: row(hunk, old),
                new: row(hunk, new),
            },
        })
    }

    /// The words to mark on stream row `row`: those of the removed line of its pair, then those of
    /// the added line. A split row draws both and a unified row the one that is its own. They are
    /// compared the first time the row is drawn and kept until the stream is laid out again.
    fn marks(&self, diff: &Diff, row: usize) -> (Marks, Marks) {
        let mut cache = self.marks.0.borrow_mut();
        let compare = || self.compare(diff, row).unwrap_or_default();
        cache.entry(row).or_insert_with(compare).clone()
    }

    /// `marks`, compared now. `None` for a row that is no pair of a removed and an added line.
    fn compare(&self, diff: &Diff, row: usize) -> Option<(Marks, Marks)> {
        let At::Base { file, offset } = self.at(row)? else {
            return None;
        };
        let hunks = &diff.files.get(file)?.hunks;
        let line = |hunk: usize, row: usize| hunks.get(hunk)?.rows.get(row);
        let (first, second) = match *self.files.get(file)?.get(offset)? {
            FileRow::Line { hunk, row, pair } => (line(hunk, row)?, line(hunk, pair?)?),
            FileRow::Pair { hunk, old, new } => (line(hunk, old?)?, line(hunk, new?)?),
            _ => return None,
        };
        match (first.kind, second.kind) {
            (RowKind::Removed, RowKind::Added) => changed(&first.text, &second.text),
            (RowKind::Added, RowKind::Removed) => changed(&second.text, &first.text),
            _ => None,
        }
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

/// Where each part goes in a pane of `area`. `sidebar` is whether the user wants the sidebar. It
/// is drawn when they do and the pane is wide enough, and this is the one place that decides.
pub fn areas(area: Rect, sidebar: bool) -> Areas {
    let [body, warnings, status] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    if !sidebar || area.width < SIDEBAR_MIN_TOTAL {
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

/// What `[sidebar]` in the `config.toml` sets: whether the sidebar starts open, and whether its file
/// rows show an icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarConfig {
    pub open: bool,
    pub icons: bool,
}

impl Default for SidebarConfig {
    fn default() -> Self {
        Self {
            open: true,
            icons: true,
        }
    }
}

/// The `[sidebar]` table of the `config.toml` at `path`, and what was wrong with it. A file that
/// is missing, unreadable or not TOML gives the defaults with no warning here, because the keymap
/// reads the same file and reports it.
pub fn sidebar_config(path: Option<&Path>) -> (SidebarConfig, Vec<Warning>) {
    path.and_then(|path| std::fs::read_to_string(path).ok())
        .map_or_else(
            || (SidebarConfig::default(), Vec::new()),
            |text| sidebar_config_from_toml(&text),
        )
}

/// The sidebar config for the text of a `config.toml`. Each value that is not a boolean is one
/// warning and falls back to its default alone.
pub fn sidebar_config_from_toml(text: &str) -> (SidebarConfig, Vec<Warning>) {
    let mut config = SidebarConfig::default();
    let mut warnings = Vec::new();
    let mut warn = |message: &str| warnings.push(Warning::Config(message.to_owned()));
    let Ok(table) = text.parse::<toml::Table>() else {
        return (config, warnings);
    };
    match table.get("sidebar") {
        None => {}
        Some(toml::Value::Table(sidebar)) => {
            match sidebar.get("open") {
                None => {}
                Some(toml::Value::Boolean(open)) => config.open = *open,
                Some(_) => warn("[sidebar] open is not true or false, showing the sidebar"),
            }
            match sidebar.get("icons") {
                None => {}
                Some(toml::Value::Boolean(icons)) => config.icons = *icons,
                Some(_) => warn("[sidebar] icons is not true or false, showing icons"),
            }
        }
        Some(_) => warn("[sidebar] is not a table, showing the sidebar"),
    }
    (config, warnings)
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

/// Where a range being selected started: the row, and the half of a split row it started on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Select {
    pub row: usize,
    pub half: Option<Side>,
}

/// The cursor, the scroll position, the focus, and the help overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags: help is open, the sidebar is wanted, icons are on, a drag is held"
)]
pub struct View {
    pub stream: Stream,
    pub cursor: usize,
    pub scroll: usize,
    pub panel: Panel,
    pub help: bool,
    pub area: Rect,
    /// The user wants the sidebar. A pane under 50 columns leaves it out all the same.
    pub sidebar: bool,
    /// Each file row of the sidebar shows an icon. Set once from the config.
    pub icons: bool,
    /// Where a range started, while one is being selected. This is visual mode.
    pub select: Option<Select>,
    /// The sidebar lists only the files that match. `None` lists them all.
    pub filter: Option<Filter>,
    /// The files collapsed to their header, by path, so they stay collapsed through a reload.
    pub collapsed: HashSet<RelPath>,
    /// The layout the user chose with the toggle key. `None` follows the pane's width.
    forced: Option<DiffLayout>,
    /// Cells of code hidden on the left of every row, and the file they were set for.
    pub hscroll: usize,
    hfile: usize,
    /// The half of a split row the mouse last clicked, until the cursor moves another way.
    half: Option<Side>,
    /// A left press in the stream is held, so a drag selects a range until the button comes up.
    drag: bool,
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
            sidebar: true,
            icons: false,
            select: None,
            filter: None,
            collapsed: HashSet::new(),
            forced: None,
            hscroll: 0,
            hfile: 0,
            half: None,
            drag: false,
            pointer: None,
        }
    }
}

impl View {
    fn areas(&self) -> Areas {
        areas(self.area, self.sidebar)
    }

    /// The sidebar is on screen: the user wants it and the pane is wide enough.
    pub fn sidebar_drawn(&self) -> bool {
        self.areas().sidebar.is_some()
    }

    /// No sidebar fits in a pane this narrow, whatever the user wants.
    pub fn too_narrow(&self) -> bool {
        areas(self.area, true).sidebar.is_none()
    }

    /// Show the sidebar, or hide it and give the stream its columns. The caller lays the stream
    /// out again. The focus cannot stay on a sidebar that is gone.
    pub fn toggle_sidebar(&mut self) {
        self.sidebar = !self.sidebar;
        self.leave_hidden_sidebar();
    }

    fn leave_hidden_sidebar(&mut self) {
        if !self.sidebar_drawn() {
            self.panel = Panel::Stream;
            if let Some(filter) = &mut self.filter {
                filter.typing = false;
            }
        }
    }

    /// Open the query on the sidebar, with the query an applied filter has. The caller has shown
    /// the sidebar.
    pub fn open_filter(&mut self) {
        let query = self
            .filter
            .take()
            .map(|filter| filter.query)
            .unwrap_or_default();
        self.filter = Some(Filter {
            query,
            typing: true,
        });
        self.panel = Panel::Sidebar;
    }

    /// Drop the filter and list every file again.
    pub fn clear_filter(&mut self, diff: &Diff) {
        self.filter = None;
        self.refilter(diff);
    }

    /// List the files that match the query, or all of them when there is no filter.
    pub fn refilter(&mut self, diff: &Diff) {
        let query = self.filter.as_ref().map(|filter| filter.query.as_str());
        self.stream.side = sidebar_rows(diff, query);
    }

    /// A key while the query takes the keys. Anything it does not use is ignored.
    pub fn filter_key(&mut self, key: KeyEvent, diff: &Diff) {
        let Some(filter) = self.filter.as_mut().filter(|filter| filter.typing) else {
            return;
        };
        let control = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match key.code {
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                filter.query.clear();
            }
            KeyCode::Char(c) if !control => filter.query.push(c),
            KeyCode::Backspace => {
                filter.query.pop();
            }
            KeyCode::Esc => return self.clear_filter(diff),
            KeyCode::Enter => return self.apply_filter(diff),
            _ => return,
        }
        self.refilter(diff);
    }

    /// `enter` in the query: keep the filter and put the cursor on a match. An empty query is no
    /// filter, and a query nothing matches stays open.
    fn apply_filter(&mut self, diff: &Diff) {
        if self
            .filter
            .as_ref()
            .is_some_and(|filter| filter.query.is_empty())
        {
            return self.clear_filter(diff);
        }
        let files = self.stream.side_files();
        let Some(&first) = files.first() else { return };
        if let Some(filter) = &mut self.filter {
            filter.typing = false;
        }
        if !files.contains(&self.stream.file_at(self.cursor)) {
            self.move_to_file(first);
        }
    }

    /// Where the sidebar lists its files, and the first sidebar row it shows: under the query
    /// when there is one, with the cursor's file in the middle. Drawing and clicks both use it.
    fn side_list(&self, area: Rect) -> (Rect, usize) {
        let mut list = Block::new().borders(Borders::RIGHT).inner(area);
        list.y += FILTER_BOX;
        list.height = list.height.saturating_sub(FILTER_BOX);
        let selected = self.stream.side_row_of(self.stream.file_at(self.cursor));
        let top = sidebar_top(
            selected.unwrap_or(0),
            usize::from(list.height),
            self.stream.side.len(),
        );
        (list, top)
    }

    fn height(&self) -> usize {
        usize::from(self.areas().stream.height).max(1)
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
        let stream = self.areas().stream;
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

    /// The files that have a row in the window, as indices into the diff's files.
    pub fn visible_files(&self) -> std::ops::RangeInclusive<usize> {
        let last = (self.scroll + self.height()).saturating_sub(1);
        self.stream.file_at(self.scroll)..=self.stream.file_at(last)
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

    /// Collapse the cursor's file to its header, or open it again. From inside the file the
    /// cursor goes to the header. False when the cursor is on no file. The caller lays the stream
    /// out again.
    pub fn toggle_collapse(&mut self, diff: &Diff) -> bool {
        let file = self.stream.file_at(self.cursor);
        let on_file = self.cursor >= self.stream.top || self.panel == Panel::Sidebar;
        let (true, Some(start), Some(file)) =
            (on_file, self.stream.file_start(file), diff.files.get(file))
        else {
            return false;
        };
        if !self.collapsed.remove(&file.path) {
            self.collapsed.insert(file.path.clone());
            self.move_to(start);
        }
        true
    }

    /// Open file `file` again. The caller lays the stream out again.
    pub fn expand(&mut self, diff: &Diff, file: usize) {
        if let Some(file) = diff.files.get(file) {
            self.collapsed.remove(&file.path);
        }
    }

    /// `next_thread` and `prev_thread`. The threads a collapsed file hides are at its header, and
    /// going forward from that header reaches them too. When the jump lands there, this is the
    /// file to open and the thread to go to: its first going forward, its last going back.
    pub fn jump_thread(&mut self, forward: bool) -> Option<(usize, CommentId)> {
        let (stream, cursor) = (&self.stream, self.cursor);
        let row = if forward {
            let hides = |row| row == cursor && stream.hiding_at(row).is_some();
            stream
                .thread_rows
                .iter()
                .find(|&&row| row > cursor || hides(row))
        } else {
            stream.thread_rows.iter().rev().find(|&&row| row < cursor)
        };
        let row = *row?;
        self.move_to_card(row);
        let hidden = self.stream.hidden(self.stream.hiding_at(row)?);
        let thread = if forward {
            hidden.first()
        } else {
            hidden.last()
        };
        let id = self.stream.ids.get(*thread?)?.clone();
        Some((self.stream.file_at(row), id))
    }

    /// Start a range at the cursor, or drop the one being selected.
    pub fn toggle_select(&mut self) {
        self.select = match self.select {
            Some(_) => None,
            None => self.base_row().map(|row| Select {
                row,
                half: self.half,
            }),
        };
    }

    /// What a comment written now would point at, read from the rows under the cursor, or from the
    /// rows between the cursor and where a range started. The result is kept by the caller, so a
    /// reload after this cannot move it.
    pub fn capture(&self, diff: &Diff) -> Result<Anchor, &'static str> {
        let row = self
            .base_row()
            .ok_or("that thread's file is not in the diff, reply to it instead")?;
        let start = self.select.map_or(row, |select| select.row);
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
        // The half the selection started on counts, else the side a click chose for the row under
        // the cursor, when the first row has a line there.
        let clicked = self
            .select
            .and_then(|select| select.half)
            .or_else(|| self.half.filter(|_| first_at == self.cursor))
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

    fn card_row(&self, id: &CommentId) -> Option<usize> {
        let thread = self.stream.ids.iter().position(|other| other == id)?;
        self.stream.card_rows.get(thread).copied()
    }

    /// Put the cursor on the first row of the card of the thread whose root is `id`.
    pub fn focus_thread(&mut self, id: &CommentId) {
        if let Some(row) = self.card_row(id) {
            self.move_to(row);
        }
    }

    /// `focus_thread` for a jump: a card that was off screen is brought to the top of the window.
    pub fn jump_to_thread(&mut self, id: &CommentId) {
        if let Some(row) = self.card_row(id) {
            self.move_to_card(row);
        }
    }

    fn stream_width(&self) -> usize {
        usize::from(self.areas().stream.width)
    }

    /// Lay the new diff out. The cursor returns to its spot when it is still there: its line of
    /// the same card, else its row in the same file.
    pub fn rebuild(&mut self, diff: &Diff, review: &Review, spot: Option<Spot>, look: &Look) {
        self.stream = Stream::build(
            diff,
            review,
            self.stream_width(),
            self.layout(),
            look,
            &self.collapsed,
        );
        self.refilter(diff);
        self.select = None;
        self.drag = false;
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
        self.leave_hidden_sidebar();
        self.ensure_visible();
    }

    /// The stream is wider or narrower than when it was laid out, so its cards wrap differently.
    pub fn needs_rebuild(&self) -> bool {
        self.stream.width != self.stream_width() || self.stream.layout != self.layout()
    }

    /// Cells of code a row can show, the narrower half in a split row.
    fn code_room(&self) -> usize {
        let width = self.stream_width();
        match self.layout() {
            DiffLayout::Unified => width.saturating_sub(UNIFIED_GUTTER),
            DiffLayout::Split => split_widths(width).0.saturating_sub(SPLIT_GUTTER),
        }
    }

    fn scroll_x(&mut self, right: bool) {
        self.hscroll = if right {
            self.hscroll + HSCROLL_COLS
        } else {
            self.hscroll.saturating_sub(HSCROLL_COLS)
        };
        self.sync_hscroll();
    }

    /// The offset starts over in another file, and never runs past the widest line.
    fn sync_hscroll(&mut self) {
        let file = self.stream.file_at(self.cursor);
        if file != self.hfile {
            self.hfile = file;
            self.hscroll = 0;
        }
        self.hscroll = self
            .hscroll
            .min(self.stream.widest.saturating_sub(self.code_room()));
    }

    fn ensure_visible(&mut self) {
        self.sync_hscroll();
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

    /// The cursor to the first row of a card. When that scrolls the window, the line the card hangs
    /// from goes on the top row so the whole card is on screen, not just its first row at the bottom.
    fn move_to_card(&mut self, row: usize) {
        let before = self.scroll;
        self.move_to(row);
        if self.scroll != before {
            let max_scroll = self.stream.len().saturating_sub(self.height());
            self.scroll = self.cursor.saturating_sub(1).min(max_scroll);
        }
    }

    /// The cursor to the header of file `file`, on the top row so the file's changes are below it.
    fn move_to_file(&mut self, file: usize) {
        let file = file.min(self.stream.files().saturating_sub(1));
        if let Some(start) = self.stream.file_start(file) {
            self.move_to(start);
            self.scroll = start.min(self.stream.len().saturating_sub(self.height()));
        }
    }

    /// The sidebar's `up`, `down` and page keys: `by` matching files on from the cursor's file. A
    /// cursor on a file the filter left out goes to the next match after it, or the last before it.
    fn step_files(&mut self, down: bool, by: usize) {
        let files = self.stream.side_files();
        let last = files.len().saturating_sub(1);
        let target = match (files.binary_search(&self.stream.file_at(self.cursor)), down) {
            (Ok(at), true) => at + by,
            (Ok(at), false) => at.saturating_sub(by),
            (Err(at), true) if at < files.len() => at + by - 1,
            (Err(at), false) if at > 0 => at.saturating_sub(by),
            (Err(_), _) => return,
        };
        if let Some(&file) = files.get(target.min(last)) {
            self.move_to_file(file);
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

    /// Move to the next or previous hunk header.
    fn jump_hunk(&mut self, forward: bool) {
        let rows = &self.stream.hunk_rows;
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
        match action {
            Action::Up if files => self.step_files(false, 1),
            Action::Down if files => self.step_files(true, 1),
            Action::Up => self.move_to(self.cursor.saturating_sub(1)),
            Action::Down => self.move_to(self.cursor + 1),
            Action::PageUp if files => self.step_files(false, self.height()),
            Action::PageDown if files => self.step_files(true, self.height()),
            Action::PageUp => self.page(false),
            Action::PageDown => self.page(true),
            Action::PrevHunk => self.jump_hunk(false),
            Action::NextHunk => self.jump_hunk(true),
            Action::PrevThread | Action::NextThread => {
                self.jump_thread(action == Action::NextThread);
            }
            Action::SwitchPanel if !self.sidebar_drawn() => {}
            Action::SwitchPanel => {
                self.panel = match self.panel {
                    Panel::Sidebar => Panel::Stream,
                    Panel::Stream => Panel::Sidebar,
                };
            }
            Action::ScrollLeft if !files => self.scroll_x(false),
            Action::ScrollRight if !files => self.scroll_x(true),
            Action::ScrollReset if !files => self.hscroll = 0,
            Action::Help => self.help = !self.help,
            _ => return false,
        }
        self.sync_hscroll();
        true
    }

    /// The first drag of a gesture starts a selection at the press row. Each drag then moves the
    /// cursor to the row it is on, or scrolls one row when it is above or below the stream.
    fn drag_to(&mut self, screen_row: u16, stream: Rect) {
        if self.select.is_none() {
            self.select = Some(Select {
                row: self.cursor,
                half: self.half,
            });
        }
        let height = self.height();
        if screen_row < stream.y {
            self.scroll = self.scroll.saturating_sub(1);
            self.move_to(self.scroll);
        } else if screen_row >= stream.y + stream.height {
            let max_scroll = self.stream.len().saturating_sub(height);
            self.scroll = (self.scroll + 1).min(max_scroll);
            self.move_to(self.scroll + height - 1);
        } else {
            self.move_to(self.scroll + usize::from(screen_row - stream.y));
        }
    }

    /// The wheel scrolls and a click moves the cursor. Neither can be remapped. A drag from a press
    /// in the stream selects a range.
    pub fn mouse(&mut self, event: MouseEvent) -> bool {
        let areas = self.areas();
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
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
                self.scroll_x(event.kind == MouseEventKind::ScrollRight);
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
                // A press on the `[+]` keeps the selection, so the comment covers it.
                if !plus_clicked {
                    self.select = None;
                }
                self.drag = false;
                if row < self.stream.len() {
                    self.drag = !plus_clicked;
                    self.panel = Panel::Stream;
                    self.move_to(row);
                    if matches!(self.stream.file_row(row), Some(FileRow::Pair { .. })) {
                        let left = split_widths(usize::from(areas.stream.width)).0;
                        let column = usize::from(event.column - areas.stream.x);
                        self.half = Some(if column < left { Side::Old } else { Side::New });
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if self.drag => {
                if self.pointer.is_some() {
                    self.pointer = Some((event.column, event.row));
                }
                self.drag_to(event.row, areas.stream);
            }
            MouseEventKind::Up(MouseButton::Left) => self.drag = false,
            MouseEventKind::Down(MouseButton::Left) => {
                self.drag = false;
                if let Some(sidebar) = areas.sidebar.filter(|rect| at(*rect)) {
                    let (list, top) = self.side_list(sidebar);
                    let row = top + usize::from(event.row.saturating_sub(list.y));
                    let file = self.stream.side.get(row);
                    if let (true, Some(SideRow::File(file))) = (event.row >= list.y, file) {
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

/// The text of a code row in exactly `room` cells, over the row's tint, starting `skip` cells in.
/// A row that was highlighted is drawn token by token, in the theme's colour for each, and what
/// lies between tokens is plain text. A row that was not is drawn whole in `plain`, the colour of
/// its kind. A `‹` takes the first cell when the row is cut off on the left, and a `›` the last
/// when it is cut off on the right. `marks` are the byte ranges of the row's text that the row it
/// is paired with does not have, and they have a stronger green or red behind them than the tint.
fn code_text(
    row: &Row,
    room: usize,
    skip: usize,
    plain: Style,
    tint: Style,
    tokens: Option<&FileTokens>,
    marks: &[Range<usize>],
    theme: &Theme,
) -> Vec<Span<'static>> {
    let side = side_of(row);
    let spans = tokens.and_then(|tokens| tokens.line(side, row.line(side)?));
    let mut parts = Vec::new();
    let mut at = 0;
    for (range, token) in spans.unwrap_or_default() {
        parts.push((at..range.start, Style::new()));
        parts.push((range.clone(), Style::new().fg(theme.token(*token))));
        at = range.end;
    }
    let rest = if spans.is_some() { Style::new() } else { plain };
    parts.push((at..row.text.len(), rest));
    let word = Style::new().bg(match row.kind {
        RowKind::Removed => theme.removed_word,
        RowKind::Added | RowKind::Context => theme.added_word,
    });
    let mut pieces = Vec::new();
    let mut push = |range: Range<usize>, style: Style| {
        let piece = row.text.get(range).unwrap_or_default();
        pieces.push((sanitize_terminal_text(piece), style));
    };
    for (range, style) in parts {
        let mut at = range.start;
        let inside = |mark: &&Range<usize>| mark.start < range.end && mark.end > range.start;
        for mark in marks.iter().filter(inside) {
            let (from, to) = (mark.start.max(at), mark.end.min(range.end));
            push(at..from, style.patch(tint));
            push(from..to, style.patch(word));
            at = to;
        }
        push(at..range.end, style.patch(tint));
    }
    if row.no_newline {
        pieces.push(("  [no newline at end of file]".to_owned(), rest.patch(tint)));
    }
    let total: usize = pieces.iter().map(|(text, _)| string_width(text)).sum();
    let left = skip > 0 && total > skip;
    let right = total > skip + room;
    let (start, end) = (
        skip + usize::from(left),
        (skip + room).saturating_sub(usize::from(right)),
    );
    let marker = |text: &'static str| Span::styled(text, theme.dim().patch(tint));
    let mut out = Vec::new();
    let mut used = 0;
    if left && room > 0 {
        out.push(marker("‹"));
        used += 1;
    }
    let mut pos = 0;
    for (text, style) in pieces {
        let kept: String = text
            .chars()
            .filter(|character| {
                let width = char_width(*character);
                pos += width;
                pos - width >= start && pos <= end
            })
            .collect();
        used += string_width(&kept);
        if !kept.is_empty() {
            out.push(Span::styled(kept, style));
        }
    }
    if right && room > 0 {
        out.push(marker("›"));
        used += 1;
    }
    out.push(Span::styled(" ".repeat(room.saturating_sub(used)), tint));
    out
}

/// One half of a split row: the line number, the sign and the text on `side`, or an empty half
/// when the other side has a line and this one does not.
fn half_spans(
    row: Option<&Row>,
    side: Side,
    width: usize,
    skip: usize,
    theme: &Theme,
    tokens: Option<&FileTokens>,
    marks: &[Range<usize>],
) -> Vec<Span<'static>> {
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
    let room = width.saturating_sub(gutter.len());
    let mut spans = vec![Span::styled(
        truncate_to_width(&gutter, width),
        theme.dim().patch(tint),
    )];
    spans.extend(code_text(
        row, room, skip, text_style, tint, tokens, marks, theme,
    ));
    spans
}

/// A file's header: a `▾`, or a `▸` while it is collapsed, its name, and the added and removed
/// counts on the right. `fold` is what a collapsed file hides, its lines and its threads, which
/// the header says after the name.
fn file_header(
    file: &DiffFile,
    width: usize,
    theme: &Theme,
    fold: Option<(usize, usize)>,
) -> Line<'static> {
    let mut text = sanitize_terminal_text(file.path.as_str());
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
    let count = |n: usize, noun: &str| {
        let plural = if n == 1 { "" } else { "s" };
        (n > 0).then(|| format!("{n} {noun}{plural}"))
    };
    let hidden = fold.map_or_else(String::new, |(lines, threads)| {
        let parts = [count(lines, "line"), count(threads, "thread")];
        let parts = parts.into_iter().flatten().collect::<Vec<_>>();
        if parts.is_empty() {
            String::new()
        } else {
            format!("  {}", parts.join(", "))
        }
    });
    // The marker, the letter and their two spaces come before the name.
    let name = truncate_to_width(&text, room.saturating_sub(4));
    let hidden = fitted(&hidden, room.saturating_sub(4 + string_width(&name)));
    let bold = Style::new().bg(theme.header).add_modifier(Modifier::BOLD);
    let marker = if fold.is_some() { "▸ " } else { "▾ " };
    let mut spans = vec![
        Span::styled(
            truncate_to_width(marker, room),
            theme.dim().bg(theme.header),
        ),
        Span::styled(
            truncate_to_width(&glyph(file.change).to_string(), room.saturating_sub(2)),
            bold.fg(glyph_color(file.change, theme)),
        ),
        Span::styled(truncate_to_width(" ", room.saturating_sub(3)), bold),
        Span::styled(name, bold),
        Span::styled(hidden, theme.dim().bg(theme.header)),
    ];
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

fn row_line(
    stream: &Stream,
    row: RowRef,
    width: usize,
    skip: usize,
    theme: &Theme,
    tokens: Option<&FileTokens>,
    (was, now): &(Marks, Marks),
    index: usize,
) -> Line<'static> {
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
        RowRef::File(file) => file_header(file, width, theme, stream.fold_of(index, file)),
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
            let mut spans = half_spans(old, Side::Old, left, skip, theme, tokens, was);
            spans.push(Span::styled("│", Style::new().fg(theme.border)));
            spans.extend(half_spans(new, Side::New, right, skip, theme, tokens, now));
            Line::from(spans)
        }
        RowRef::Line(row) => {
            let number =
                |line: Option<u32>| line.map_or_else(|| "     ".to_owned(), |n| format!("{n:>5}"));
            let gutter = format!("{}{} ", number(row.old), number(row.new));
            let (sign, style, tint) = code_style(row.kind, theme);
            let room = width.saturating_sub(gutter.len() + 1);
            let mut spans = vec![
                Span::styled(gutter, theme.dim().patch(tint)),
                Span::styled(sign.to_string(), style.patch(tint)),
            ];
            let marks = if row.kind == RowKind::Removed {
                was
            } else {
                now
            };
            spans.extend(code_text(
                row, room, skip, style, tint, tokens, marks, theme,
            ));
            Line::from(spans)
        }
    }
}

fn empty_message(spec: &Spec) -> String {
    match spec {
        Spec::WorkTree => "No changes in the working tree.".to_owned(),
        Spec::Branch { base } => format!("No changes against {}.", sanitize_terminal_text(base)),
    }
}

pub(crate) fn highlight(buffer: &mut Buffer, area: Rect, row: usize, style: Style) {
    let y = area.y + u16::try_from(row).unwrap_or(0);
    buffer.set_style(Rect::new(area.x, y, area.width, 1), style);
}

/// `highlight` for a row of the stream: a cell behind a changed word keeps its colour, so the
/// cursor and a selection do not hide which words changed.
fn bar(buffer: &mut Buffer, area: Rect, row: usize, style: Style, theme: &Theme) {
    let y = area.y + u16::try_from(row).unwrap_or(0);
    for x in area.left()..area.right() {
        if let Some(cell) = buffer.cell_mut((x, y))
            && cell.bg != theme.added_word
            && cell.bg != theme.removed_word
        {
            cell.set_style(style);
        }
    }
}

/// Draw the sidebar and the stream, and the help overlay when it is open. Only the rows in the
/// window are built. `mark` is the rows the open editor comments on. While it is set they are
/// marked and no `[+]` is offered, since the mouse does nothing until the editor closes. `syntax`
/// holds the tokens of the files that were highlighted, and a file that is not in it draws plain.
pub fn draw(
    frame: &mut Frame,
    view: &View,
    diff: &Diff,
    keymap: &Keymap,
    theme: &Theme,
    mark: Option<(usize, usize)>,
    syntax: &Cache,
) {
    let areas = areas(frame.area(), view.sidebar);
    let cursor_style = Style::new().bg(theme.cursor);
    let height = usize::from(areas.stream.height);
    let width = usize::from(areas.stream.width);
    let lines = (view.scroll..view.scroll + height)
        .filter_map(|at| Some((at, view.stream.locate(diff, at)?)))
        .map(|(at, row)| {
            let index = view.stream.file_at(at);
            let tokens = diff.files.get(index);
            let tokens = tokens.and_then(|file| syntax.file(file.path.as_str()));
            let marks = view.stream.marks(diff, at);
            let skip = view.hscroll;
            row_line(&view.stream, row, width, skip, theme, tokens, &marks, index)
        })
        .collect::<Vec<_>>();
    frame.render_widget(Clear, areas.stream);
    frame.render_widget(Paragraph::new(lines), areas.stream);
    if let Some(select) = view.select {
        let (low, high) = (select.row.min(view.cursor), select.row.max(view.cursor));
        for row in low.max(view.scroll)..=high.min(view.scroll + height.saturating_sub(1)) {
            bar(
                frame.buffer_mut(),
                areas.stream,
                row - view.scroll,
                Style::new().bg(theme.selection),
                theme,
            );
        }
    }
    if view.cursor >= view.scroll && view.cursor < view.scroll + height {
        // The same bar whichever panel has the keys, so the file chosen in the sidebar stands out.
        bar(
            frame.buffer_mut(),
            areas.stream,
            view.cursor - view.scroll,
            cursor_style,
            theme,
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
        let hint = if keymap.keys(Action::Filter).is_empty() {
            "filter".to_owned()
        } else {
            format!("filter ({})", keymap.label(Action::Filter))
        };
        draw_sidebar(frame, sidebar, view, diff, theme, &hint);
    }
    if view.help {
        draw_help(frame, keymap, theme, &Action::ALL);
    }
}

/// Mark rows `low..=high` of the stream as what the open editor comments on: a tint across the
/// row and a bar in its first cell. A cell that holds a digit of a line number keeps it.
fn draw_mark(frame: &mut Frame, view: &View, (low, high): (usize, usize), theme: &Theme) {
    let area = areas(frame.area(), view.sidebar).stream;
    let last = view.scroll + usize::from(area.height).saturating_sub(1);
    for row in low.max(view.scroll)..=high.min(last) {
        let at = row - view.scroll;
        bar(
            frame.buffer_mut(),
            area,
            at,
            Style::new().bg(theme.selection),
            theme,
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
/// it: under that row, else above it, else at the bottom. It is as wide as `stream`, which the
/// caller narrows to where a note goes.
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

/// One file of the sidebar: a mark for unsent comments, the letter, an icon when `icons` is on, the
/// name, and the counts pushed to the right edge. The name gives way first. The name of a `folded`
/// file, one collapsed in the stream, is in the subtle colour.
fn side_file_line(
    file: &DiffFile,
    unsent: bool,
    icons: bool,
    width: usize,
    theme: &Theme,
    folded: bool,
) -> Line<'static> {
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
    let letter = glyph(file.change);
    let room = width.saturating_sub(counts_width + usize::from(counts_width > 0));
    // The mark, the letter and a space, then the icon and a space. All of them are one cell each.
    let prefix_width = if icons { 5 } else { 3 };
    let mut spans = if room < prefix_width {
        let plain = format!("{mark}{letter} {}", if icons { icon(name) } else { ' ' });
        vec![Span::raw(truncate_to_width(&plain, room))]
    } else {
        let mut spans = vec![
            Span::raw(mark.to_string()),
            Span::styled(
                letter.to_string(),
                Style::new().fg(glyph_color(file.change, theme)),
            ),
            Span::raw(" "),
        ];
        if icons {
            spans.push(Span::styled(icon(name).to_string(), theme.dim()));
            spans.push(Span::raw(" "));
        }
        let name = sanitize_terminal_text(name);
        let style = if folded { theme.dim() } else { Style::new() };
        spans.push(Span::styled(
            head_to_width(&name, room - prefix_width),
            style,
        ));
        spans
    };
    let used: usize = spans.iter().map(|span| string_width(&span.content)).sum();
    spans.push(Span::raw(
        " ".repeat(width.saturating_sub(used + counts_width)),
    ));
    spans.extend(counts);
    Line::from(spans)
}

/// The filter's query in a rounded box at the top of the sidebar, always drawn so the user sees
/// there is a filter: a `>` prompt, the query cut from the left so its end shows (or `hint` while
/// it is empty), and how many of the diff's files match on the right. The border is in the accent
/// colour while the query takes keys.
fn draw_filter_box(
    frame: &mut Frame,
    inner: Rect,
    view: &View,
    diff: &Diff,
    theme: &Theme,
    hint: &str,
) {
    let typing = view.filter.as_ref().is_some_and(|filter| filter.typing);
    let query = view.filter.as_ref().map_or("", |filter| &filter.query);
    let area = Rect::new(inner.x, inner.y, inner.width, inner.height.min(FILTER_BOX));
    let border = if typing { theme.accent } else { theme.border };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(border));
    let room = usize::from(block.inner(area).width);
    frame.render_widget(block.clone(), area);
    let count = format!("{}/{}", view.stream.side_files().len(), diff.files.len());
    let cursor = usize::from(typing);
    let query_room = room.saturating_sub(2 + string_width(&count) + 1 + cursor);
    let mut spans = vec![Span::styled("> ", key_style(theme))];
    let mut used = 2 + cursor;
    if query.is_empty() && !typing {
        let hint = tail_to_width(hint, query_room);
        used += string_width(&hint);
        spans.push(Span::styled(hint, theme.dim()));
    } else {
        let query = tail_to_width(&sanitize_terminal_text(query), query_room);
        used += string_width(&query);
        spans.push(Span::raw(query));
    }
    if typing {
        spans.push(Span::styled(" ", Style::new().bg(theme.cursor)));
    }
    spans.push(Span::raw(
        " ".repeat(room.saturating_sub(used + string_width(&count))),
    ));
    spans.push(Span::styled(count, theme.dim()));
    frame.render_widget(Paragraph::new(Line::from(spans)), block.inner(area));
}

fn draw_sidebar(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    diff: &Diff,
    theme: &Theme,
    hint: &str,
) {
    let block = Block::new()
        .borders(Borders::RIGHT)
        .border_style(Style::new().fg(theme.border));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let (list, top) = view.side_list(area);
    let height = usize::from(list.height);
    let width = usize::from(list.width);
    draw_filter_box(frame, inner, view, diff, theme, hint);
    if view.filter.is_some() && view.stream.side.is_empty() {
        frame.render_widget(Paragraph::new(Line::styled("no match", theme.dim())), list);
    }
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
                let folded = view.stream.folded(*index);
                side_file_line(file, unsent, view.icons, width, theme, folded)
            }),
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), list);
    // The cursor's file has a bar either way, a dimmer one while the keys go to the stream.
    let style = if view.panel == Panel::Sidebar {
        Style::new().bg(theme.cursor)
    } else {
        Style::new().bg(theme.header).add_modifier(Modifier::BOLD)
    };
    let selected = view.stream.side_row_of(view.stream.file_at(view.cursor));
    if let Some(selected) = selected.filter(|row| (top..top + height).contains(row)) {
        highlight(frame.buffer_mut(), list, selected - top, style);
    }
}

/// The box of the help overlay and the prompts: an accent border and a bold accent `title`.
pub fn popup_block(theme: &Theme, title: String) -> Block<'static> {
    Block::bordered()
        .border_style(Style::new().fg(theme.accent))
        .title(Line::styled(title, key_style(theme)))
}

/// What a key is drawn in, in every popup.
pub fn key_style(theme: &Theme) -> Style {
    Style::new().fg(theme.accent).add_modifier(Modifier::BOLD)
}

/// `actions` with their current keys, drawn from the effective keymap.
pub(crate) fn draw_help(frame: &mut Frame, keymap: &Keymap, theme: &Theme, actions: &[Action]) {
    let area = frame.area();
    let width = area.width.saturating_sub(4).min(64);
    let height = (u16::try_from(actions.len()).unwrap_or(0) + 2).min(area.height.saturating_sub(2));
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );
    let lines = actions
        .iter()
        .map(|action| {
            Line::from(vec![
                Span::styled(
                    format!("{:<16}", sanitize_terminal_text(&keymap.label(*action))),
                    key_style(theme),
                ),
                Span::raw(format!(" {}", action.describe())),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .block(popup_block(theme, " keys, any key closes ".to_owned()))
            .style(Style::new().bg(theme.popup)),
        popup,
    );
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests;
