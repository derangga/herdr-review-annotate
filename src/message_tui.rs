//! The message pane: the agent's newest message as Markdown lines the user can comment on.
//!
//! The pane takes its terminal and its input as parameters, as the review pane does, so tests
//! drive it with a `TestBackend` and a list of events (design/message-review.md, pane and graphs).

use std::io;
use std::ops::Range;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::layout::{Constraint, Layout as Split, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::agent_delivery::{Delivery, deliver_to_agent};
use crate::cards::{Card, message_note, wrap};
use crate::editor::{Editor, Outcome};
use crate::env::Env;
use crate::keymap::{Action, Keymap};
use crate::message::{
    AgentMessage, AgentStatus, MessageAgent, MessageComment, find_transcript, format_prompt,
    parse_agent, pointer_path, read_message, write_pointer,
};
use crate::motion::{Pos, word_end, word_next, word_prev};
use crate::store::{PaneId, TerminalId, Warning};
use crate::syntax::{self, Span as Token};
use crate::termination::Termination;
use crate::theme::Theme;
use crate::tui::{
    Exit, Guard, HerdrCall, Screen, TICK, Tone, draw_notice, draw_popup, enter_terminal,
    restore_terminal, restoring_panic_hook, sanitize_terminal_text, status_bar,
};
use crate::view::{DiffLayout, draw_help, editor_rect, highlight, key_style, note_box};
use crate::width::{char_width, string_width, truncate_to_width};

/// The actions the pane acts on, in the order of the help overlay. The other keys do nothing, and
/// neither the footer nor the help overlay lists them.
pub const ACTIONS: [Action; 14] = [
    Action::Up,
    Action::Down,
    Action::PageUp,
    Action::PageDown,
    Action::PrevThread,
    Action::NextThread,
    Action::Comment,
    Action::SelectRange,
    Action::Edit,
    Action::Delete,
    Action::Send,
    Action::Reload,
    Action::Help,
    Action::Quit,
];

/// The keys the status line names, in the order they are drawn.
const FOOTER: [(Action, &str); 5] = [
    (Action::Comment, "comment"),
    (Action::Send, "send"),
    (Action::Reload, "reload"),
    (Action::Help, "help"),
    (Action::Quit, "quit"),
];

/// The actions the pane still takes in visual mode. The other keys fall to the vim table, so `e`
/// moves to the end of a word there instead of editing.
const VISUAL_ACTIONS: [Action; 8] = [
    Action::Up,
    Action::Down,
    Action::PageUp,
    Action::PageDown,
    Action::Comment,
    Action::SelectRange,
    Action::Help,
    Action::Quit,
];

/// Rows the wheel scrolls by.
const WHEEL_ROWS: usize = 3;

/// What a screen row of the message shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    /// The bytes of the line this row shows.
    Text(Range<usize>),
    /// Row `at` of the box of comment `comment`.
    Note { comment: usize, at: usize },
}

/// One screen row of the message: part of a source line, or of the box of a comment that hangs
/// under it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    line: usize,
    kind: Kind,
}

/// The message wrapped to a width: a row for each piece of each line, and the boxes of the
/// comments under the last line they cover.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Layout {
    width: usize,
    rows: Vec<Row>,
    /// The rows of the text of each line.
    text: Vec<Range<usize>>,
    /// The rows of the box of each comment.
    boxes: Vec<Range<usize>>,
}

impl Layout {
    /// `notes` is the line each comment hangs under, from 0, and its box.
    fn build(lines: &[String], width: usize, notes: &[(usize, Card)]) -> Self {
        let mut rows = Vec::new();
        let mut text = Vec::new();
        let mut boxes = vec![0..0; notes.len()];
        for (line, source) in lines.iter().enumerate() {
            let first = rows.len();
            let mut at = 0;
            let mut pieces = wrap(source, width);
            // A long word after leading spaces leaves an empty piece before it.
            pieces.retain(|piece| !piece.is_empty());
            if pieces.is_empty() {
                pieces.push(String::new());
            }
            for piece in pieces {
                // `wrap` drops the spaces it breaks at, so find where the piece starts.
                while !source
                    .get(at..)
                    .is_some_and(|rest| rest.starts_with(&piece))
                    && source.get(at..).is_some_and(|rest| rest.starts_with(' '))
                {
                    at += 1;
                }
                rows.push(Row {
                    line,
                    kind: Kind::Text(at..at + piece.len()),
                });
                at += piece.len();
            }
            text.push(first..rows.len());
            for (comment, (_, card)) in notes.iter().enumerate().filter(|(_, (to, _))| *to == line)
            {
                let first = rows.len();
                for at in 0..card.lines.len() {
                    rows.push(Row {
                        line,
                        kind: Kind::Note { comment, at },
                    });
                }
                if let Some(slot) = boxes.get_mut(comment) {
                    *slot = first..rows.len();
                }
            }
        }
        Self {
            width,
            rows,
            text,
            boxes,
        }
    }

    /// The rows the text of line `line` takes.
    fn rows_of(&self, line: usize) -> Range<usize> {
        self.text.get(line).cloned().unwrap_or_default()
    }

    /// The rows of the box of comment `comment`.
    fn box_of(&self, comment: usize) -> Range<usize> {
        self.boxes.get(comment).cloned().unwrap_or_default()
    }

    /// The line a row belongs to: its own, or the one its box hangs under.
    fn line_at(&self, row: usize) -> Option<usize> {
        self.rows.get(row).map(|row| row.line)
    }

    /// The row of line `line` that byte `col` is drawn on. A space the wrap dropped belongs to the
    /// row before it.
    fn row_at(&self, line: usize, col: usize) -> usize {
        let rows = self.rows_of(line);
        rows.clone()
            .rev()
            .find(|at| {
                matches!(self.rows.get(*at), Some(Row { kind: Kind::Text(range), .. }) if range.start <= col)
            })
            .unwrap_or(rows.start)
    }

    fn is_text(&self, row: usize) -> bool {
        matches!(
            self.rows.get(row),
            Some(Row {
                kind: Kind::Text(_),
                ..
            })
        )
    }

    /// A text row for `row`: itself, or for a box row the last row of the line it hangs under.
    fn text_row(&self, row: usize) -> usize {
        match self.rows.get(row) {
            Some(Row {
                line,
                kind: Kind::Note { .. },
            }) => self.rows_of(*line).end.saturating_sub(1),
            _ => row,
        }
    }
}

/// Where a selection started, and whether it covers whole lines (`V`) or characters (`v`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Select {
    pub at: Pos,
    pub lines: bool,
}

/// A key of the fixed vim table, for a key the keymap gives no action this pane takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Motion {
    Left,
    Right,
    Up,
    Down,
    Start,
    End,
    WordNext,
    WordPrev,
    WordEnd,
    Lines,
}

impl Motion {
    fn of(key: &KeyEvent) -> Option<Self> {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return None;
        }
        Some(match key.code {
            KeyCode::Char('h') | KeyCode::Left => Self::Left,
            KeyCode::Char('l') | KeyCode::Right => Self::Right,
            KeyCode::Char('k') | KeyCode::Up => Self::Up,
            KeyCode::Char('j') | KeyCode::Down => Self::Down,
            KeyCode::Char('0') | KeyCode::Home => Self::Start,
            KeyCode::Char('$') | KeyCode::End => Self::End,
            KeyCode::Char('w') => Self::WordNext,
            KeyCode::Char('b') => Self::WordPrev,
            KeyCode::Char('e') => Self::WordEnd,
            KeyCode::Char('V') => Self::Lines,
            _ => return None,
        })
    }
}

/// The help overlay's line for the vim table.
const MOTIONS: [(&str, &str); 2] = [
    ("h j k l w b e 0 $", "move by character, row and word"),
    ("V", "select whole lines"),
];

/// The editor, and what its text becomes.
#[derive(Debug)]
struct Compose {
    /// The comment being edited, or `None` for a new one.
    edit: Option<usize>,
    /// The lines it covers, from 0, the end not included.
    lines: Range<usize>,
    /// The characters it covers, as in `MessageComment::chars`.
    chars: Option<(usize, usize)>,
    editor: Editor,
    /// What the top border of the editor says first, then where the comment points.
    title: &'static str,
    place: String,
}

/// A question the pane asks before it goes on. It takes every key until it is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// Reloading with comments: they would be lost.
    Reload,
    /// Quitting with comments: they would be lost.
    Quit,
}

/// The line, from 0, that the box of `comment` hangs under: the last it covers.
fn end_line(comment: &MessageComment) -> usize {
    (comment.end as usize).saturating_sub(1)
}

/// `line 12` or `lines 12-14`, for lines from 0 and the end not included.
fn place(lines: &Range<usize>) -> String {
    if lines.len() == 1 {
        format!("line {}", lines.start + 1)
    } else {
        format!("lines {}-{}", lines.start + 1, lines.end)
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// The state of the pane.
#[derive(Debug)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags: a mouse button is held, help is open, a send is queued, quit was asked"
)]
pub struct Pane {
    pub env: Env,
    pub keymap: Keymap,
    pub theme: Theme,
    pub herdr: HerdrCall,
    /// The agent pane and terminal the message action started this pane for.
    deliver: Option<(PaneId, TerminalId)>,
    pub screen: Screen,
    pub agent: Option<MessageAgent>,
    /// The newest message, with its lines made safe to draw.
    pub message: Option<AgentMessage>,
    /// The Markdown tokens of each line. Empty when no grammar is built in.
    tokens: Vec<Vec<Token>>,
    layout: Layout,
    /// The comments, in the order they were written.
    pub comments: Vec<MessageComment>,
    /// The line each comment hangs under, from 0, and its box. Drawn again when the width changes.
    notes: Vec<(usize, Card)>,
    /// The comment that `edit` and `delete` act on, when several hang under the cursor's line.
    focus: Option<usize>,
    compose: Option<Compose>,
    pub prompt: Option<Prompt>,
    /// A send is waiting for the loop to draw "sending" and then make the Herdr call.
    pending: bool,
    /// The line the cursor is on.
    pub cursor: usize,
    /// The byte of the cursor's character in its line.
    pub col: usize,
    /// The cells from the start of its row that `j` and `k` keep the cursor at.
    want: usize,
    /// The first row on screen.
    pub scroll: usize,
    /// Where a selection started, while one is being made. This is visual mode.
    pub select: Option<Select>,
    /// A left press is held, so a drag selects characters until the button comes up.
    drag: bool,
    pub area: Rect,
    pub help: bool,
    /// What the last action said, in place of the state on the status line.
    pub status: Option<(Tone, String)>,
    pub warnings: Vec<Warning>,
    pub quit: bool,
}

impl Pane {
    /// Read the keymap, the theme and the pane and terminal of the agent. Nothing else is read
    /// until `load`.
    pub fn new(env: Env) -> Self {
        let config = env
            .get("HERDR_PLUGIN_CONFIG_DIR")
            .map(|dir| Path::new(dir).join("config.toml"));
        let keymap = Keymap::load(config.as_deref());
        let (theme, theme_warnings) = Theme::load(config.as_deref());
        let mut warnings = keymap.warnings.clone();
        warnings.extend(theme_warnings);
        let deliver = env
            .get("REVIEW_DELIVER_TO")
            .and_then(PaneId::parse)
            .zip(env.get("REVIEW_DELIVER_TERM").and_then(TerminalId::parse));
        Self {
            env,
            keymap,
            theme,
            herdr: HerdrCall::new(crate::herdr::run_herdr_output),
            deliver,
            screen: Screen::Message("loading".to_owned()),
            agent: None,
            message: None,
            tokens: Vec::new(),
            layout: Layout::default(),
            comments: Vec::new(),
            notes: Vec::new(),
            focus: None,
            compose: None,
            prompt: None,
            pending: false,
            cursor: 0,
            col: 0,
            want: 0,
            scroll: 0,
            select: None,
            drag: false,
            area: Rect::default(),
            help: false,
            status: None,
            warnings,
            quit: false,
        }
    }

    fn notice(&mut self, text: impl Into<String>) {
        self.status = Some((Tone::Notice, text.into()));
    }

    fn fail(&mut self, text: impl Into<String>) {
        self.status = Some((Tone::Failure, text.into()));
    }

    fn warn(&mut self, warning: Warning) {
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }

    /// Record this pane's id for the terminal of the agent, so a second press of the key focuses
    /// it instead of splitting again. A failure is a warning.
    pub fn save_pointer(&mut self) {
        let Some((_, terminal)) = &self.deliver else {
            return;
        };
        let Some(pane) = self.env.get("HERDR_PANE_ID").and_then(PaneId::parse) else {
            return;
        };
        let Some(path) = pointer_path(&self.env, terminal) else {
            return;
        };
        if let Err(error) = write_pointer(&path, &pane) {
            self.warn(Warning::Config(format!(
                "could not save {}: {error}",
                path.display()
            )));
        }
    }

    /// The agent and its newest message, or why they cannot be read.
    fn read(
        &mut self,
        pane: &PaneId,
        terminal: &TerminalId,
    ) -> Result<(MessageAgent, AgentMessage), String> {
        let got = self.herdr.call(&[
            "agent".to_owned(),
            "get".to_owned(),
            pane.as_str().to_owned(),
        ]);
        let agent = parse_agent(&got).map_err(|error| error.to_string())?;
        if agent.terminal != *terminal {
            return Err("The agent is gone: that pane runs something else now.".to_owned());
        }
        let path = find_transcript(&agent, &self.env).map_err(|error| error.to_string())?;
        let message = read_message(&path).map_err(|error| error.to_string())?;
        Ok((agent, message))
    }

    /// Start-up and `reload`: read the agent and its newest message. A failure with a message on
    /// screen is the status line, and the message stays. Without one it is the message screen.
    /// True when the message was read.
    pub fn load(&mut self) -> bool {
        let Some((pane, terminal)) = self.deliver.clone() else {
            self.screen = Screen::Message(
                "This pane has no agent to read. Open it with the message action.".to_owned(),
            );
            return false;
        };
        match self.read(&pane, &terminal) {
            Ok((agent, mut message)) => {
                message.lines = message
                    .lines
                    .iter()
                    .map(|line| sanitize_terminal_text(line))
                    .collect();
                let same = self
                    .message
                    .as_ref()
                    .is_some_and(|old| old.id == message.id);
                self.tokens = markdown_tokens(&message.lines);
                self.agent = Some(agent);
                self.message = Some(message);
                if !same {
                    (self.cursor, self.col, self.want) = (0, 0, 0);
                    (self.scroll, self.select) = (0, None);
                }
                self.relayout();
                self.screen = Screen::Review;
                self.status = None;
                true
            }
            Err(text) => {
                if self.message.is_some() {
                    self.fail(text);
                } else {
                    self.screen = Screen::Message(text);
                }
                false
            }
        }
    }

    /// The words of the message on the status line.
    pub fn message_text(&self) -> Option<&str> {
        self.status.as_ref().map(|(_, text)| text.as_str())
    }

    /// Herdr said the agent was working when the message was loaded.
    pub fn working(&self) -> bool {
        self.agent
            .as_ref()
            .is_some_and(|agent| agent.status == AgentStatus::Working)
    }

    fn lines(&self) -> &[String] {
        self.message
            .as_ref()
            .map_or(&[][..], |message| &message.lines)
    }

    /// The columns of line numbers, and the space after them.
    fn gutter(&self) -> usize {
        self.lines().len().max(1).to_string().len().max(2) + 1
    }

    fn body(&self) -> Rect {
        split(self.area)[0]
    }

    fn height(&self) -> usize {
        usize::from(self.body().height).max(1)
    }

    /// The cells a line of text may use, after the line numbers.
    fn text_width(&self) -> usize {
        usize::from(self.body().width)
            .saturating_sub(self.gutter())
            .max(1)
    }

    /// Tell the pane how big it is. The message wraps to the width of the text.
    pub fn resize(&mut self, area: Rect) {
        self.area = area;
        if self.layout.width == self.text_width() {
            self.ensure_visible();
        } else {
            self.relayout();
        }
    }

    /// Wrap the message to the current width, keeping the first line on screen where it was.
    fn relayout(&mut self) {
        let top = self.layout.line_at(self.scroll);
        let width = self.text_width();
        let (left, wide) = note_box(width, DiffLayout::Unified, None);
        self.notes = self
            .comments
            .iter()
            .map(|comment| {
                let range = (comment.start as usize, comment.end as usize);
                let card = message_note(range, &comment.body, wide, &self.theme, &self.keymap);
                (end_line(comment), card.indented(left))
            })
            .collect();
        self.layout = Layout::build(self.lines(), width, &self.notes);
        if let Some(top) = top {
            self.scroll = self.layout.rows_of(top).start;
        }
        self.cursor = self.cursor.min(self.lines().len().saturating_sub(1));
        self.clamp_col();
        self.ensure_visible();
    }

    fn pos(&self) -> Pos {
        (self.cursor, self.col)
    }

    /// Keep the cursor on a character of its line: the last one when it was past it.
    fn clamp_col(&mut self) {
        let text = self.lines().get(self.cursor).map_or("", String::as_str);
        let mut col = self.col.min(last_char(text));
        while !text.is_char_boundary(col) {
            col -= 1;
        }
        self.col = col;
    }

    /// The cells from the start of its row to the character at `pos`.
    fn cells_of(&self, (line, col): Pos) -> usize {
        let start = match self.layout.rows.get(self.layout.row_at(line, col)) {
            Some(Row {
                kind: Kind::Text(range),
                ..
            }) => range.start,
            _ => 0,
        };
        self.lines()
            .get(line)
            .and_then(|text| text.get(start..col.max(start)))
            .map_or(0, string_width)
    }

    /// The character of text row `row` that covers cell `cells`, or its last when the row is
    /// shorter.
    fn byte_at(&self, row: usize, cells: usize) -> Option<Pos> {
        let Some(Row {
            line,
            kind: Kind::Text(range),
        }) = self.layout.rows.get(row)
        else {
            return None;
        };
        let text = self.lines().get(*line)?.get(range.clone())?;
        let mut used = 0;
        let mut at = 0;
        for (byte, character) in text.char_indices() {
            at = byte;
            used += char_width(character);
            if used > cells {
                break;
            }
        }
        Some((*line, range.start + at))
    }

    /// Put the cursor on `pos`, aim `j` and `k` at its column, and scroll to it.
    fn set_pos(&mut self, (line, col): Pos) {
        (self.cursor, self.col) = (line, col);
        self.want = self.cells_of((line, col));
        self.ensure_visible();
    }

    /// Put the cursor on row `row`, at the column `j` and `k` aim at.
    fn put_row(&mut self, row: usize) {
        if let Some(pos) = self.byte_at(self.layout.text_row(row), self.want) {
            (self.cursor, self.col) = pos;
        }
    }

    /// `j` and `k`: the next or previous row of text, the boxes skipped.
    fn vertical(&mut self, down: bool) {
        let here = self.layout.row_at(self.cursor, self.col);
        let next = if down {
            (here + 1..self.layout.rows.len()).find(|at| self.layout.is_text(*at))
        } else {
            (0..here).rev().find(|at| self.layout.is_text(*at))
        };
        if let Some(row) = next {
            self.put_row(row);
        }
        self.ensure_visible();
    }

    /// A key of the vim table.
    fn motion(&mut self, motion: Motion) {
        let lines = self.lines();
        let Some(text) = lines.get(self.cursor) else {
            return;
        };
        let (line, col) = self.pos();
        let pos = match motion {
            Motion::Up | Motion::Down => return self.vertical(motion == Motion::Down),
            Motion::Lines => return self.visual(true),
            Motion::Left => (line, text.get(..col).map_or(0, last_char)),
            Motion::Right => {
                let next = text
                    .get(col..)
                    .and_then(|rest| rest.char_indices().nth(1))
                    .map(|(at, _)| col + at);
                (line, next.unwrap_or(col))
            }
            Motion::Start => (line, 0),
            Motion::End => (line, last_char(text)),
            Motion::WordNext => word_next(lines, (line, col)),
            Motion::WordPrev => word_prev(lines, (line, col)),
            Motion::WordEnd => word_end(lines, (line, col)),
        };
        self.set_pos(pos);
    }

    /// Scroll so the cursor's row, and the box of the comment on its line, are on screen.
    fn ensure_visible(&mut self) {
        let row = self.layout.row_at(self.cursor, self.col);
        let mut rows = row..row + 1;
        if let Some(comment) = self.focused() {
            rows.end = rows.end.max(self.layout.box_of(comment).end);
        }
        let height = self.height();
        if rows.start < self.scroll {
            self.scroll = rows.start;
        } else if rows.end > self.scroll + height {
            self.scroll = (rows.end - height).min(rows.start);
        }
        self.scroll = self.scroll.min(self.layout.rows.len().saturating_sub(1));
    }

    /// Scroll a page of rows, and move the cursor a page with it, onto the line at that row. The
    /// cursor stays in the window.
    fn page(&mut self, down: bool) {
        let height = self.height();
        let rows = self.layout.rows.len();
        let last = rows.saturating_sub(1);
        let was = self.layout.row_at(self.cursor, self.col);
        self.scroll = if down {
            (self.scroll + height).min(rows.saturating_sub(height))
        } else {
            self.scroll.saturating_sub(height)
        };
        let moved = if down {
            was + height
        } else {
            was.saturating_sub(height)
        };
        let target = moved
            .min(last)
            .clamp(self.scroll, (self.scroll + height - 1).min(last));
        self.put_row(target);
    }

    /// Keep the cursor on a row of text in the window, after the window moved.
    fn scroll_cursor_into_window(&mut self) {
        let here = self.layout.row_at(self.cursor, self.col);
        let end = (self.scroll + self.height()).min(self.layout.rows.len());
        let mut window = (self.scroll..end).filter(|at| self.layout.is_text(*at));
        let row = if here < self.scroll {
            window.next()
        } else if here >= end {
            window.next_back()
        } else {
            None
        };
        if let Some(row) = row {
            self.put_row(row);
        }
    }

    /// The comment that `edit` and `delete` act on: the one `n` and `N` last landed on, when it
    /// hangs under the cursor's line, else the first written there.
    fn focused(&self) -> Option<usize> {
        let here = |at: &usize| {
            self.comments
                .get(*at)
                .is_some_and(|comment| end_line(comment) == self.cursor)
        };
        self.focus
            .filter(here)
            .or_else(|| (0..self.comments.len()).find(here))
    }

    /// `n` and `N`: the cursor to the line the next or the previous comment hangs under, in line
    /// order and then the order they were written.
    fn jump(&mut self, forward: bool) {
        let mut order = (0..self.comments.len()).collect::<Vec<_>>();
        order.sort_by_key(|at| {
            let line = self.comments.get(*at).map(end_line);
            (line, *at)
        });
        let line = |at: usize| self.comments.get(at).map_or(0, end_line);
        let here = self
            .focused()
            .and_then(|focus| order.iter().position(|at| *at == focus));
        let target = match (here, forward) {
            (Some(at), true) => order.get(at + 1),
            (Some(at), false) => at.checked_sub(1).and_then(|at| order.get(at)),
            (None, true) => order.iter().find(|at| line(**at) > self.cursor),
            (None, false) => order.iter().rev().find(|at| line(**at) < self.cursor),
        };
        match target.copied() {
            Some(at) => self.land_on(at),
            None if self.comments.is_empty() => self.notice("no comments yet"),
            None if forward => self.notice("no next comment"),
            None => self.notice("no previous comment"),
        }
    }

    /// `comment`: open the editor for the lines under the cursor, or the selected range.
    fn start_comment(&mut self) {
        if self.lines().is_empty() {
            return;
        }
        let (lines, chars) = (self.range(), self.chars());
        self.select = None;
        self.compose = Some(Compose {
            edit: None,
            place: place(&lines),
            lines,
            chars,
            editor: Editor::default(),
            title: "Draft note - ",
        });
    }

    /// `edit`: open the editor on the text of the comment under the cursor.
    fn start_edit(&mut self) {
        let Some((at, comment)) = self
            .focused()
            .and_then(|at| Some((at, self.comments.get(at)?)))
        else {
            self.notice("no comment here, n finds the next one");
            return;
        };
        let lines = (comment.start as usize).saturating_sub(1)..comment.end as usize;
        self.compose = Some(Compose {
            edit: Some(at),
            place: place(&lines),
            lines,
            chars: comment.chars,
            editor: Editor::with_text(&comment.body),
            title: "Edit note - ",
        });
    }

    /// `delete`: remove the comment under the cursor.
    fn delete(&mut self) {
        let Some(at) = self.focused() else {
            self.notice("no comment here, n finds the next one");
            return;
        };
        self.comments.remove(at);
        self.focus = None;
        self.relayout();
        self.notice("deleted the comment");
    }

    /// The editor saved: keep the comment, and put the cursor and the focus on it.
    fn commit(&mut self, body: String) {
        let Some(compose) = self.compose.take() else {
            return;
        };
        let at = if let Some(at) = compose.edit {
            if let Some(comment) = self.comments.get_mut(at) {
                comment.body = body;
            }
            at
        } else {
            self.comments.push(MessageComment {
                start: u32::try_from(compose.lines.start + 1).unwrap_or(u32::MAX),
                end: u32::try_from(compose.lines.end).unwrap_or(u32::MAX),
                chars: compose.chars,
                body,
            });
            self.comments.len() - 1
        };
        self.relayout();
        self.land_on(at);
    }

    /// Put the cursor on the last character comment `at` covers, or on its last line, and focus it.
    fn land_on(&mut self, at: usize) {
        let Some(comment) = self.comments.get(at) else {
            return;
        };
        let line = end_line(comment);
        let col = match comment.chars {
            Some((_, to)) => self
                .lines()
                .get(line)
                .and_then(|text| text.get(..to))
                .map_or(0, last_char),
            None if line == self.cursor => self.col,
            None => 0,
        };
        self.focus = Some(at);
        self.set_pos((line, col));
    }

    /// A key while the editor is open.
    fn compose_key(&mut self, key: KeyEvent) {
        let Some(compose) = &mut self.compose else {
            return;
        };
        match compose.editor.handle_key(key) {
            Outcome::Continue => {}
            Outcome::Cancel => self.compose = None,
            Outcome::Save(body) => self.commit(body),
        }
    }

    /// `reload`: with comments, ask first. Without, load the newest message.
    fn reload(&mut self) {
        if self.comments.is_empty() {
            self.load();
        } else {
            self.prompt = Some(Prompt::Reload);
        }
    }

    /// The prompt said yes: load the newest message and drop the comments. A load that fails
    /// keeps them, since nothing replaced the message they are on.
    fn discard_and_reload(&mut self) {
        let kept = std::mem::take(&mut self.comments);
        if !self.load() {
            self.comments = kept;
            self.relayout();
        }
        self.focus = None;
    }

    /// `send`: queue the comments as the agent's next prompt. The loop draws "sending" and then
    /// runs it, because the Herdr call blocks.
    fn request_send(&mut self) {
        if self.comments.is_empty() {
            self.notice("nothing to send");
        } else {
            self.notice("sending");
            self.pending = true;
        }
    }

    /// Run the queued send. `draw` shows the frame first, which says "sending".
    pub fn run_pending(&mut self, draw: impl FnOnce(&Self)) {
        if std::mem::take(&mut self.pending) {
            draw(self);
            self.send();
        }
    }

    /// Deliver the prompt. When Herdr takes it the pane is done and leaves. When it does not, the
    /// pane and the comments stay, and the reason is on the status line and in a notification.
    fn send(&mut self) {
        let (Some((pane, terminal)), Some(message)) = (self.deliver.clone(), &self.message) else {
            return;
        };
        let text = format_prompt(&message.lines, &self.comments);
        match self.deliver_prompt(&pane, &terminal, &text) {
            Ok(()) => self.quit = true,
            Err(reason) => {
                let args = [
                    "notification",
                    "show",
                    "review: not sent",
                    "--body",
                    &reason,
                ];
                let _ = self.herdr.call(&args.map(str::to_owned));
                self.fail(reason);
            }
        }
    }

    /// Check that `pane` still runs the agent of `terminal`, then prompt it with `text`.
    fn deliver_prompt(
        &mut self,
        pane: &PaneId,
        terminal: &TerminalId,
        text: &str,
    ) -> Result<(), String> {
        let refused =
            |reason: String| format!("{reason} Nothing was sent; your comments are still unsent.");
        let got = self.herdr.call(&[
            "agent".to_owned(),
            "get".to_owned(),
            pane.as_str().to_owned(),
        ]);
        let agent = parse_agent(&got).map_err(|error| refused(error.to_string()))?;
        if agent.terminal != *terminal {
            return Err(refused(
                "The agent is gone: that pane runs something else now.".to_owned(),
            ));
        }
        deliver_to_agent(Delivery::Send, Some(pane.as_str()), text, |args| {
            self.herdr.call(args)
        })
    }

    /// `quit`: leave, or with comments ask first.
    fn quit_or_ask(&mut self) {
        if self.comments.is_empty() {
            self.quit = true;
        } else {
            self.prompt = Some(Prompt::Quit);
        }
    }

    /// A key while a prompt is open.
    fn prompt_key(&mut self, key: KeyEvent) {
        match (self.prompt, key.code) {
            (Some(Prompt::Reload), KeyCode::Char('y')) => {
                self.prompt = None;
                self.discard_and_reload();
            }
            (Some(Prompt::Quit), KeyCode::Char('s')) => {
                self.prompt = None;
                self.request_send();
            }
            (Some(Prompt::Quit), KeyCode::Char('d')) => self.quit = true,
            (Some(_), KeyCode::Esc | KeyCode::Char('n')) => self.prompt = None,
            _ => {}
        }
    }

    /// `v` (`lines` false) and `V`: start a selection at the cursor, switch the one being made to
    /// this kind, or drop it when it is this kind already.
    fn visual(&mut self, lines: bool) {
        self.select = match self.select {
            Some(select) if select.lines == lines => None,
            Some(select) => Some(Select { lines, ..select }),
            None if self.lines().is_empty() => None,
            None => Some(Select {
                at: self.pos(),
                lines,
            }),
        };
    }

    /// The lines a comment written now would cover: the selected range, or the cursor's line.
    pub fn range(&self) -> Range<usize> {
        let start = self.select.map_or(self.cursor, |select| select.at.0);
        start.min(self.cursor)..start.max(self.cursor) + 1
    }

    /// The characters a comment written now would cover, as in `MessageComment::chars`: `None` with
    /// no character selection, or one that covers its lines whole.
    pub fn chars(&self) -> Option<(usize, usize)> {
        let select = self.select.filter(|select| !select.lines)?;
        let (from, to) = if select.at <= self.pos() {
            (select.at, self.pos())
        } else {
            (self.pos(), select.at)
        };
        let last = self.lines().get(to.0)?;
        let end = to.1 + last.get(to.1..)?.chars().next().map_or(0, char::len_utf8);
        (from.1 > 0 || end < last.len()).then_some((from.1, end))
    }

    /// Whether a key bound to `action` acts here: one of the pane's, and in visual mode one of
    /// those it keeps.
    fn acts_on(&self, action: Action) -> bool {
        ACTIONS.contains(&action) && (self.select.is_none() || VISUAL_ACTIONS.contains(&action))
    }

    /// Apply a key's action. Any action clears the warning line and the last error.
    pub fn handle(&mut self, action: Action) {
        self.warnings.clear();
        self.status = None;
        match action {
            Action::Quit => self.quit_or_ask(),
            Action::Reload => self.reload(),
            Action::SelectRange => self.visual(false),
            Action::Send => self.request_send(),
            Action::Comment => self.start_comment(),
            Action::Edit => self.start_edit(),
            Action::Delete => self.delete(),
            Action::NextThread => self.jump(true),
            Action::PrevThread => self.jump(false),
            Action::Help => self.help = !self.help,
            Action::Up => self.vertical(false),
            Action::Down => self.vertical(true),
            Action::PageUp => self.page(false),
            Action::PageDown => self.page(true),
            _ => {}
        }
    }

    /// Apply a key press: to the editor when one is open, then to a prompt, then to the help
    /// overlay, then to the keymap, else to the vim table.
    pub fn key(&mut self, key: KeyEvent) {
        if self.compose.is_some() {
            self.compose_key(key);
        } else if self.prompt.is_some() {
            self.prompt_key(key);
        } else if self.help {
            self.help = false;
        } else if key.code == KeyCode::Esc && self.select.is_some() {
            self.warnings.clear();
            self.status = None;
            self.select = None;
        } else if let Some(action) = self
            .keymap
            .action(&key)
            .filter(|action| self.acts_on(*action))
        {
            self.handle(action);
        } else if self.screen == Screen::Review
            && let Some(motion) = Motion::of(&key)
        {
            self.warnings.clear();
            self.status = None;
            self.motion(motion);
        }
    }

    /// The first drag of a gesture starts a selection at the press. Each drag then moves the
    /// cursor to the character it is on, or scrolls a row when it is above or below the text.
    fn drag_to(&mut self, column: u16, screen_row: u16) {
        let body = self.body();
        if self.select.is_none() {
            self.select = Some(Select {
                at: self.pos(),
                lines: false,
            });
        }
        let height = self.height();
        let rows = self.layout.rows.len();
        if screen_row < body.y {
            self.scroll = self.scroll.saturating_sub(1);
            self.point_at(self.scroll, column);
        } else if screen_row >= body.y + body.height {
            self.scroll = (self.scroll + 1).min(rows.saturating_sub(height));
            self.point_at(self.scroll + height - 1, column);
        } else {
            self.point_at(self.scroll + usize::from(screen_row - body.y), column);
        }
    }

    /// The cursor to the character drawn at screen column `column` of row `row`.
    fn point_at(&mut self, row: usize, column: u16) {
        let row = row.min(self.layout.rows.len().saturating_sub(1));
        let cells = usize::from(column.saturating_sub(self.body().x)).saturating_sub(self.gutter());
        if let Some(pos) = self.byte_at(self.layout.text_row(row), cells) {
            self.set_pos(pos);
        }
    }

    /// The wheel scrolls and a click moves the cursor, as in the review pane. A drag from a press
    /// selects characters.
    pub fn mouse(&mut self, event: MouseEvent) {
        if self.screen != Screen::Review
            || self.help
            || self.compose.is_some()
            || self.prompt.is_some()
        {
            return;
        }
        let body = self.body();
        let inside = event.column >= body.x
            && event.column < body.x + body.width
            && event.row >= body.y
            && event.row < body.y + body.height;
        match event.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let height = self.height();
                let rows = self.layout.rows.len();
                self.scroll = if event.kind == MouseEventKind::ScrollDown {
                    (self.scroll + WHEEL_ROWS).min(rows.saturating_sub(height))
                } else {
                    self.scroll.saturating_sub(WHEEL_ROWS)
                };
                self.scroll_cursor_into_window();
            }
            MouseEventKind::Down(MouseButton::Left) if inside => {
                let row = self.scroll + usize::from(event.row - body.y);
                self.select = None;
                self.drag = row < self.layout.rows.len();
                self.point_at(row, event.column);
            }
            MouseEventKind::Drag(MouseButton::Left) if self.drag => {
                self.drag_to(event.column, event.row);
            }
            MouseEventKind::Up(MouseButton::Left) => self.drag = false,
            _ => {}
        }
    }

    /// The left of the status line: the pane's chip, the agent, and whether it was working.
    fn state(&self) -> Vec<Span<'static>> {
        let theme = &self.theme;
        let chip =
            |text: &str, bg| Span::styled(text.to_owned(), Style::new().fg(theme.base).bg(bg));
        let mut spans = vec![chip(" MESSAGE ", theme.accent), " ".into()];
        spans.push(self.agent.as_ref().map_or_else(
            || Span::styled("\u{2192} no agent", Style::new().fg(theme.removed)),
            |agent| {
                let text = format!("\u{2192} {} {}", agent.name, agent.pane);
                Span::styled(sanitize_terminal_text(&text), Style::new().fg(theme.text))
            },
        ));
        let count = self.comments.len();
        if count > 0 {
            spans.push("  ".into());
            spans.push(chip(
                &format!(" {count} comment{} ", plural(count)),
                theme.warning,
            ));
        }
        if self.working() {
            spans.push("  ".into());
            spans.push(chip(" working ", theme.warning));
        }
        spans
    }

    /// The left of the status line in visual mode: the chip, the lines the selection covers, and
    /// how many lines or characters it holds.
    fn visual_state(&self) -> Vec<Span<'static>> {
        let theme = &self.theme;
        let range = self.range();
        let chars = self.chars();
        let place = place(&range);
        let count = match chars {
            Some(chars) => {
                let comment = MessageComment {
                    start: u32::try_from(range.start + 1).unwrap_or(u32::MAX),
                    end: u32::try_from(range.end).unwrap_or(u32::MAX),
                    chars: Some(chars),
                    body: String::new(),
                };
                let count = comment
                    .text(self.lines())
                    .map_or(0, |text| text.chars().count());
                format!("{count} char{}", plural(count))
            }
            None => format!("{} line{}", range.len(), plural(range.len())),
        };
        let chip = if self.select.is_some_and(|select| select.lines) {
            " VISUAL LINE "
        } else {
            " VISUAL "
        };
        vec![
            Span::styled(chip, Style::new().fg(theme.base).bg(theme.visual)),
            Span::styled(format!(" {place} ({count})"), Style::new().fg(theme.text)),
        ]
    }

    /// The status line, `width` cells wide.
    fn status_line(&self, width: usize) -> Line<'static> {
        let theme = &self.theme;
        let spans = match &self.status {
            Some((tone, text)) => {
                let colour = match tone {
                    Tone::Notice => theme.warning,
                    Tone::Failure => theme.removed,
                };
                let text = format!(" {}", sanitize_terminal_text(text));
                vec![Span::styled(
                    truncate_to_width(&text, width),
                    Style::new().fg(colour),
                )]
            }
            None if self.select.is_some() => self.visual_state(),
            None => self.state(),
        };
        let label = |action| self.keymap.label(action);
        let keys = if self.select.is_some() {
            vec![
                (label(Action::Comment), "comment"),
                (format!("{}/esc", label(Action::SelectRange)), "cancel"),
            ]
        } else {
            FOOTER
                .iter()
                .filter(|(action, _)| !self.keymap.keys(*action).is_empty())
                .map(|(action, what)| (label(*action), *what))
                .collect()
        };
        status_bar(spans, keys, width, theme)
    }

    /// One row of the message. A text row has the line number on a line's first row, then the text
    /// in the colours of its tokens. A box row is indented under the line numbers.
    fn row_line(&self, at: usize, gutter: usize) -> Option<Line<'static>> {
        let row = self.layout.rows.get(at)?;
        match &row.kind {
            Kind::Text(range) => {
                let text = self.lines().get(row.line)?;
                let number = if self.layout.rows_of(row.line).start == at {
                    format!("{:>1$} ", row.line + 1, gutter - 1)
                } else {
                    " ".repeat(gutter)
                };
                let tokens = self.tokens.get(row.line).map_or(&[][..], Vec::as_slice);
                let mut spans = vec![Span::styled(number, self.theme.dim())];
                spans.extend(text_spans(text, range, tokens, &self.theme));
                Some(Line::from(spans))
            }
            Kind::Note { comment, at } => {
                let mut line = self.notes.get(*comment)?.1.lines.get(*at)?.clone();
                line.spans.insert(0, Span::raw(" ".repeat(gutter)));
                Some(line)
            }
        }
    }

    /// Draw the rows in the window, then tint the selected lines, the cursor's line and the box of
    /// the comment `edit` and `delete` would act on.
    fn draw_text(&self, frame: &mut Frame, body: Rect) {
        let gutter = self.gutter();
        let height = usize::from(body.height);
        let lines = (self.scroll..self.scroll + height)
            .filter_map(|at| self.row_line(at, gutter))
            .collect::<Vec<_>>();
        frame.render_widget(Clear, body);
        frame.render_widget(Paragraph::new(lines), body);
        let range = self.range();
        let chars = self.chars();
        let focused = self.focused();
        let selection = Style::new().bg(self.theme.selection);
        let underline = Style::new().add_modifier(Modifier::UNDERLINED);
        for at in self.scroll..(self.scroll + height).min(self.layout.rows.len()) {
            let Some(row) = self.layout.rows.get(at) else {
                continue;
            };
            let style = |color| Style::new().bg(color);
            let mut tint =
                |color| highlight(frame.buffer_mut(), body, at - self.scroll, style(color));
            match row.kind {
                Kind::Text(_) => {
                    let whole = self.select.is_some() && chars.is_none();
                    if whole && range.contains(&row.line) {
                        tint(self.theme.selection);
                    }
                    if row.line == self.cursor {
                        tint(self.theme.cursor);
                    }
                    if self.select.is_some()
                        && chars.is_some()
                        && let Some(bytes) = self.covered(at, &range, chars)
                    {
                        self.paint(frame.buffer_mut(), body, at, bytes, selection);
                    }
                    for comment in self.comments.iter().filter(|c| c.chars.is_some()) {
                        let lines =
                            (comment.start as usize).saturating_sub(1)..comment.end as usize;
                        if let Some(bytes) = self.covered(at, &lines, comment.chars) {
                            self.paint(frame.buffer_mut(), body, at, bytes, underline);
                        }
                    }
                    let (line, col) = self.pos();
                    if self.compose.is_none() && self.layout.row_at(line, col) == at {
                        let text = self.lines().get(line).map_or("", String::as_str);
                        let next = text.get(col..).and_then(|rest| rest.chars().next());
                        let end = col + next.map_or(0, char::len_utf8);
                        let reversed = Style::new().add_modifier(Modifier::REVERSED);
                        self.paint(frame.buffer_mut(), body, at, col..end, reversed);
                    }
                }
                Kind::Note { comment, .. } if focused == Some(comment) => tint(self.theme.cursor),
                Kind::Note { .. } => {}
            }
        }
        if let Some(compose) = &self.compose {
            self.draw_compose(frame, body, compose);
        }
    }

    /// The bytes of text row `at` that `lines` and `chars` cover, as in `MessageComment`. An empty
    /// line in the range is covered, with no bytes.
    fn covered(
        &self,
        at: usize,
        lines: &Range<usize>,
        chars: Option<(usize, usize)>,
    ) -> Option<Range<usize>> {
        let Some(Row {
            line,
            kind: Kind::Text(range),
        }) = self.layout.rows.get(at)
        else {
            return None;
        };
        if !lines.contains(line) {
            return None;
        }
        let len = self.lines().get(*line).map_or(0, String::len);
        let (from, to) = chars.unwrap_or((0, len));
        let start = if *line == lines.start { from } else { 0 };
        let end = if *line + 1 == lines.end { to } else { len };
        let (left, right) = (start.max(range.start), end.min(range.end));
        let empty = range.is_empty() && start <= range.start && range.end <= end;
        (left < right || empty).then_some(left..right.max(left))
    }

    /// Patch `style` onto the cells of the bytes `bytes` of text row `at`, at least one cell.
    fn paint(&self, buffer: &mut Buffer, body: Rect, at: usize, bytes: Range<usize>, style: Style) {
        let Some(Row {
            line,
            kind: Kind::Text(range),
        }) = self.layout.rows.get(at)
        else {
            return;
        };
        let text = self.lines().get(*line).map_or("", String::as_str);
        let width = |from: usize, to: usize| text.get(from..to).map_or(0, string_width);
        let skip = self.gutter() + width(range.start, bytes.start);
        let cells = width(bytes.start, bytes.end).max(1);
        let x = body
            .x
            .saturating_add(u16::try_from(skip).unwrap_or(u16::MAX));
        if x >= body.right() || at < self.scroll {
            return;
        }
        let y = body.y + u16::try_from(at - self.scroll).unwrap_or(0);
        let w = u16::try_from(cells)
            .unwrap_or(u16::MAX)
            .min(body.right() - x);
        buffer.set_style(Rect::new(x, y, w, 1), style);
    }

    /// The editor, under the last line the comment covers, where the box of the saved comment goes.
    /// The lines or characters it covers are marked while it is open.
    fn draw_compose(&self, frame: &mut Frame, body: Rect, compose: &Compose) {
        let gutter = u16::try_from(self.gutter()).unwrap_or(0);
        let text = Rect {
            x: body.x + gutter,
            width: body.width.saturating_sub(gutter),
            ..body
        };
        let selection = Style::new().bg(self.theme.selection);
        for at in self.scroll..(self.scroll + usize::from(body.height)).min(self.layout.rows.len())
        {
            let Some(bytes) = self.covered(at, &compose.lines, compose.chars) else {
                continue;
            };
            let row = at - self.scroll;
            if compose.chars.is_some() {
                self.paint(frame.buffer_mut(), body, at, bytes, selection);
            } else {
                highlight(frame.buffer_mut(), body, row, selection);
            }
            let y = body.y + u16::try_from(row).unwrap_or(0);
            if let Some(cell) = frame.buffer_mut().cell_mut((body.x, y))
                && cell.symbol() == " "
            {
                cell.set_symbol("▌").set_fg(self.theme.warning);
            }
        }
        let (left, wide) = note_box(usize::from(text.width), DiffLayout::Unified, None);
        let slot = Rect {
            x: text.x + u16::try_from(left).unwrap_or(0),
            width: u16::try_from(wide).unwrap_or(text.width),
            ..text
        };
        let last = self.layout.rows_of(compose.lines.end.saturating_sub(1)).end;
        let at = last.saturating_sub(1 + self.scroll);
        let height = compose
            .editor
            .height(slot.width, (body.height * 2 / 3).max(4));
        compose.editor.draw(
            frame,
            editor_rect(slot, at, height),
            compose.title,
            &compose.place,
            &self.theme,
        );
    }

    /// The question the pane is waiting on.
    fn draw_prompt(&self, frame: &mut Frame, prompt: Prompt) {
        let theme = &self.theme;
        let count = self.comments.len();
        let accent = key_style(theme);
        let choice = |key: &'static str, what: &'static str, style: Style| {
            Line::from(vec![Span::styled(key, style), Span::raw(what)])
        };
        let (title, lines) = match prompt {
            Prompt::Reload => (
                format!(
                    " discard {count} comment{} and load the newest message? ",
                    plural(count)
                ),
                vec![
                    choice("[y]", " yes", accent.fg(theme.success)),
                    choice("[n]", " no", accent.fg(theme.removed)),
                ],
            ),
            Prompt::Quit => (
                format!(" {count} unsent comment{} ", plural(count)),
                vec![
                    choice("[s]", " send them, then quit", accent),
                    choice("[d]", " discard them and quit", accent),
                    choice("[esc]", " stay", accent),
                ],
            ),
        };
        draw_popup(frame, theme, title, lines);
    }
}

/// The body, the warning line and the status line of a pane `area` big.
fn split(area: Rect) -> [Rect; 3] {
    Split::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area)
}

/// The byte of the last character of `text`, or 0 when it is empty.
fn last_char(text: &str) -> usize {
    text.char_indices().last().map_or(0, |(at, _)| at)
}

/// The tokens of each line as Markdown, or none when no grammar is built in.
fn markdown_tokens(lines: &[String]) -> Vec<Vec<Token>> {
    let Some(language) = syntax::language("message.md") else {
        return Vec::new();
    };
    let mut text = lines.join("\n");
    text.push('\n');
    syntax::highlight(&text, language)
}

/// The part of `text` in `range`, coloured by the tokens that fall in it. What lies between tokens
/// is plain.
fn text_spans(
    text: &str,
    range: &Range<usize>,
    tokens: &[Token],
    theme: &Theme,
) -> Vec<Span<'static>> {
    let part = |from: usize, to: usize| text.get(from..to).unwrap_or_default().to_owned();
    let mut spans = Vec::new();
    let mut at = range.start;
    for (token, kind) in tokens {
        let (from, to) = (token.start.max(range.start), token.end.min(range.end));
        if from >= to || from < at {
            continue;
        }
        spans.push(Span::raw(part(at, from)));
        spans.push(Span::styled(
            part(from, to),
            Style::new().fg(theme.token(*kind)),
        ));
        at = to;
    }
    spans.push(Span::raw(part(at, range.end)));
    spans
}

/// Draw the pane.
pub fn render(frame: &mut Frame, pane: &Pane) {
    let [body, warnings, status] = split(frame.area());
    match &pane.screen {
        Screen::Message(text) => draw_notice(frame, body, text, &pane.keymap, &pane.theme),
        Screen::Review => pane.draw_text(frame, body),
    }
    if pane.help {
        draw_help(frame, &pane.keymap, &pane.theme, &ACTIONS, &MOTIONS);
    }
    if let Some(prompt) = pane.prompt {
        pane.draw_prompt(frame, prompt);
    }
    let width = usize::from(frame.area().width);
    let warning = pane
        .warnings
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    frame.render_widget(
        Paragraph::new(truncate_to_width(&sanitize_terminal_text(&warning), width))
            .style(Style::new().add_modifier(Modifier::BOLD)),
        warnings,
    );
    frame.render_widget(
        Paragraph::new(pane.status_line(width))
            .style(Style::new().fg(pane.theme.text).bg(pane.theme.header)),
        status,
    );
    pane.theme.paint(frame.buffer_mut());
}

/// Draw, wait up to one tick for an event, apply it. `poll` returns the next event or `None` when
/// the tick passed, and `terminated` is the signal flag. A frame is drawn only after an event or a
/// new terminal size.
pub fn run_loop<B: Backend>(
    pane: &mut Pane,
    terminal: &mut Terminal<B>,
    mut poll: impl FnMut(Duration) -> io::Result<Option<Event>>,
    terminated: impl Fn() -> bool,
) -> Exit {
    let mut dirty = true;
    let mut drawn_size = None;
    loop {
        if terminated() {
            return Exit::Terminated;
        }
        let size = terminal.size().ok();
        if let Some(size) = size {
            pane.resize(Rect::new(0, 0, size.width, size.height));
        }
        if dirty || size != drawn_size {
            if terminal.draw(|frame| render(frame, pane)).is_err() {
                return Exit::Io;
            }
            (dirty, drawn_size) = (false, size);
        }
        let polled = poll(TICK);
        dirty |= matches!(polled, Ok(Some(_)));
        match polled {
            Err(_) => return Exit::Io,
            Ok(Some(Event::Key(key))) if key.kind != KeyEventKind::Release => pane.key(key),
            Ok(Some(Event::Mouse(mouse))) => pane.mouse(mouse),
            Ok(_) => {}
        }
        pane.run_pending(|pane| {
            dirty = true;
            let _ = terminal.draw(|frame| render(frame, pane));
        });
        if pane.quit {
            return Exit::Quit;
        }
    }
}

/// The `message-tui` entry point: the process owns the terminal until it returns.
pub fn run(env: &Env) -> ExitCode {
    let termination = Termination::install();
    restoring_panic_hook(restore_terminal);
    let _guard = Guard(Some(restore_terminal));
    if enter_terminal().is_err() {
        return ExitCode::FAILURE;
    }
    let Ok(mut terminal) = Terminal::new(CrosstermBackend::new(io::stdout())) else {
        return ExitCode::FAILURE;
    };
    let mut pane = Pane::new(env.clone());
    pane.save_pointer();
    pane.load();
    let poll = |timeout| {
        if event::poll(timeout)? {
            event::read().map(Some)
        } else {
            Ok(None)
        }
    };
    match run_loop(&mut pane, &mut terminal, poll, || termination.requested()) {
        Exit::Quit | Exit::Terminated => ExitCode::SUCCESS,
        Exit::Io => ExitCode::FAILURE,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests;
