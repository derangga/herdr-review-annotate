//! The multi-line comment editor, as a widget the pane draws inside a rectangle.
//!
//! It owns a text buffer and a cursor and nothing else. A key press returns `Outcome`: keep
//! going, save this text, or cancel. Where the text goes, and what it is attached to, is the
//! caller's business.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use crate::edit_keys::{EditAction, line_end, line_start, resolve_edit_key, word_end, word_start};
use crate::theme::Theme;
use crate::tui::sanitize_terminal_text;
use crate::width::{char_width, string_width, tail_to_width};

/// The keys on the bottom border. They are fixed, as `edit_keys.rs` defines them.
const KEYS: &str = "^S save  Esc cancel";

/// What an empty editor shows where the text will go.
const PLACEHOLDER: &str = "Write a note…";

/// What a key press did to the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    /// The trimmed text, which is not empty. The editor stays as it is until the caller drops it,
    /// so a save that fails loses nothing.
    Save(String),
    Cancel,
}

/// Editor state, independent of the terminal backend.
#[derive(Debug, Default)]
pub struct Editor {
    comment: Vec<char>,
    cursor: usize,
    status: String,
}

impl Editor {
    /// An editor holding `text`, with the cursor at its end. Control characters are removed, so
    /// stored text cannot drive the terminal.
    pub fn with_text(text: &str) -> Self {
        let comment = sanitize_terminal_text(text).chars().collect::<Vec<_>>();
        Self {
            cursor: comment.len(),
            comment,
            status: String::new(),
        }
    }

    /// The text typed so far, trimmed.
    pub fn text(&self) -> String {
        self.comment.iter().collect::<String>().trim().to_owned()
    }

    /// Show `message` in place of the key hints until the next key. The text stays.
    pub fn fail(&mut self, message: impl Into<String>) {
        self.status = message.into();
    }

    /// Rows the widget wants at `width` cells wide, borders included, at most `max`.
    pub fn height(&self, width: u16, max: u16) -> u16 {
        let inner = usize::from(width.saturating_sub(2)).max(1);
        let lines = layout_comment(&self.comment, self.cursor, inner)
            .lines
            .len();
        u16::try_from(lines + 2).unwrap_or(u16::MAX).min(max)
    }

    /// Draw the editor in `area` as a rounded box. The top border reads `title` and then `place`,
    /// which is cut from the left when the two do not fit. The bottom border holds the keys, or
    /// why the last save failed. The terminal cursor goes where the text cursor is.
    pub fn draw(&self, frame: &mut Frame, area: Rect, title: &str, place: &str, theme: &Theme) {
        let border = Style::new().fg(theme.warning);
        let title = sanitize_terminal_text(title);
        // Two corners and a space on each side of the text.
        let room = usize::from(area.width).saturating_sub(4 + string_width(&title));
        let place = tail_to_width(&sanitize_terminal_text(place), room);
        let footer = if self.status.is_empty() {
            Line::styled(format!(" {KEYS} "), theme.dim())
        } else {
            Line::styled(
                format!(" {} ", sanitize_terminal_text(&self.status)),
                Style::new().fg(theme.removed),
            )
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(border)
            .title(Line::styled(
                format!(" {title}{place} "),
                border.add_modifier(Modifier::BOLD),
            ))
            .title_bottom(footer.right_aligned());
        let inner = block.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);
        if inner.is_empty() {
            return;
        }
        let rows = usize::from(inner.height);
        let layout = layout_comment(&self.comment, self.cursor, usize::from(inner.width));
        let start = layout.cursor_row.saturating_sub(rows - 1);
        let lines = layout
            .lines
            .into_iter()
            .skip(start)
            .take(rows)
            .map(Line::from)
            .collect::<Vec<_>>();
        if self.comment.is_empty() {
            frame.render_widget(Line::styled(PLACEHOLDER, theme.dim()), inner);
        } else {
            frame.render_widget(Paragraph::new(lines), inner);
        }
        let x = u16::try_from(layout.cursor_col).ok().map(|x| inner.x + x);
        let y = u16::try_from(layout.cursor_row - start)
            .ok()
            .map(|y| inner.y + y);
        if let (Some(x), Some(y)) = (x, y)
            && x < inner.x + inner.width
            && y < inner.y + inner.height
        {
            frame.set_cursor_position(Position::new(x, y));
        }
    }

    /// Handle one keyboard event.
    pub fn handle_key(&mut self, key: KeyEvent) -> Outcome {
        if key.kind == KeyEventKind::Release {
            return Outcome::Continue;
        }
        self.status.clear();
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if control => return Outcome::Cancel,
            KeyCode::Char('s') if control => return self.save(),
            _ => {}
        }
        if let Some(action) = resolve_edit_key(&key) {
            self.apply_edit_action(action);
            return Outcome::Continue;
        }
        match key.code {
            KeyCode::Esc => return Outcome::Cancel,
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.comment.remove(self.cursor);
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.comment.len() {
                    self.comment.remove(self.cursor);
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.comment.len()),
            KeyCode::Up => self.move_cursor_vertical(-1),
            KeyCode::Down => self.move_cursor_vertical(1),
            KeyCode::Home => self.cursor = line_start(&self.comment, self.cursor),
            KeyCode::End => self.cursor = line_end(&self.comment, self.cursor),
            KeyCode::Enter => self.insert('\n'),
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(character);
            }
            _ => {}
        }
        Outcome::Continue
    }

    fn save(&mut self) -> Outcome {
        let text = self.text();
        if text.is_empty() {
            "Write a comment before saving.".clone_into(&mut self.status);
            Outcome::Continue
        } else {
            Outcome::Save(text)
        }
    }

    /// Apply a word or line action.
    fn apply_edit_action(&mut self, action: EditAction) {
        match action {
            EditAction::WordLeft => self.cursor = word_start(&self.comment, self.cursor),
            EditAction::WordRight => self.cursor = word_end(&self.comment, self.cursor),
            EditAction::LineStart => self.cursor = line_start(&self.comment, self.cursor),
            EditAction::LineEnd => self.cursor = line_end(&self.comment, self.cursor),
            EditAction::DeleteWord => {
                let start = word_start(&self.comment, self.cursor);
                self.comment.drain(start..self.cursor);
                self.cursor = start;
            }
            EditAction::DeleteLine => {
                let start = line_start(&self.comment, self.cursor);
                self.comment.drain(start..self.cursor);
                self.cursor = start;
            }
        }
    }

    fn insert(&mut self, character: char) {
        self.comment.insert(self.cursor, character);
        self.cursor += 1;
    }

    fn move_cursor_vertical(&mut self, delta: isize) {
        let before = self.comment.iter().take(self.cursor).collect::<String>();
        let row = before.split('\n').count().saturating_sub(1);
        let col = before.rsplit('\n').next().map_or(0, string_width);
        let joined = self.comment.iter().collect::<String>();
        let lines = joined.split('\n').collect::<Vec<_>>();
        let target_row = row
            .saturating_add_signed(delta)
            .min(lines.len().saturating_sub(1));
        let mut next = lines
            .iter()
            .take(target_row)
            .map(|line| line.chars().count() + 1)
            .sum::<usize>();
        let mut used = 0;
        for character in lines.get(target_row).copied().unwrap_or_default().chars() {
            let width = char_width(character);
            if used + width > col {
                break;
            }
            used += width;
            next += 1;
        }
        self.cursor = next;
    }
}

/// The comment laid out in terminal cells, and where the cursor is in it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CommentLayout {
    lines: Vec<String>,
    cursor_row: usize,
    cursor_col: usize,
}

fn layout_comment(comment: &[char], cursor: usize, width: usize) -> CommentLayout {
    let safe_width = width.max(1);
    let mut lines = vec![String::new()];
    let mut row = 0;
    let mut col = 0;
    let mut cursor_row = 0;
    let mut cursor_col = 0;
    for index in 0..=comment.len() {
        let cells = comment.get(index).copied().map_or(0, char_width);
        if col > 0 && col + cells > safe_width {
            lines.push(String::new());
            row += 1;
            col = 0;
        }
        if index == cursor {
            cursor_row = row;
            cursor_col = col;
        }
        let Some(character) = comment.get(index).copied() else {
            break;
        };
        if character == '\n' {
            lines.push(String::new());
            row += 1;
            col = 0;
        } else {
            if let Some(line) = lines.get_mut(row) {
                line.push(character);
            }
            col += cells;
        }
    }
    CommentLayout {
        lines,
        cursor_row,
        cursor_col,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn type_text(editor: &mut Editor, text: &str) {
        for character in text.chars() {
            editor.handle_key(KeyEvent::from(KeyCode::Char(character)));
        }
    }

    fn ctrl(character: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL)
    }

    fn typed(editor: &Editor) -> String {
        editor.comment.iter().collect()
    }

    #[test]
    fn editing_keys_work_on_wide_characters() {
        let mut editor = Editor::default();
        assert_eq!(
            editor.handle_key(KeyEvent::from(KeyCode::Char('한'))),
            Outcome::Continue
        );
        type_text(&mut editor, "a");
        assert_eq!(editor.comment, ['한', 'a']);
        editor.handle_key(KeyEvent::from(KeyCode::Left));
        editor.handle_key(KeyEvent::from(KeyCode::Backspace));
        assert_eq!(editor.comment, ['a']);
    }

    #[test]
    fn word_and_line_keys_move_and_kill_by_their_boundaries() {
        let mut editor = Editor::default();
        type_text(&mut editor, "alpha beta");
        editor.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(editor.cursor, 6);
        editor.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT));
        assert_eq!(editor.cursor, 0);
        editor.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT));
        assert_eq!(editor.cursor, 5);
        editor.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(editor.cursor, 10);
        editor.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SUPER));
        assert_eq!(editor.cursor, 0);
        editor.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::SUPER));
        assert_eq!(editor.cursor, 10);
        editor.handle_key(ctrl('a'));
        assert_eq!(editor.cursor, 0);
        editor.handle_key(ctrl('e'));
        assert_eq!(editor.cursor, 10);
        editor.handle_key(ctrl('w'));
        assert_eq!(typed(&editor), "alpha ");
        assert_eq!(editor.cursor, 6);
        editor.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT));
        assert_eq!(typed(&editor), "");
        assert_eq!(editor.cursor, 0);
    }

    #[test]
    fn delete_line_kills_only_the_current_line() {
        let mut editor = Editor::default();
        type_text(&mut editor, "one two");
        editor.handle_key(KeyEvent::from(KeyCode::Enter));
        type_text(&mut editor, "한글 three");
        editor.handle_key(ctrl('u'));
        assert_eq!(typed(&editor), "one two\n");
        assert_eq!(editor.cursor, 8);
        editor.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(editor.cursor, 7);
        editor.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(editor.cursor, 4);
        editor.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT));
        assert_eq!(editor.cursor, 7);
        editor.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT));
        assert_eq!(editor.cursor, 8);
    }

    #[test]
    fn up_and_down_keep_the_column_and_home_and_end_stay_on_the_line() {
        let mut editor = Editor::with_text("abc\nd\nefgh");
        editor.handle_key(KeyEvent::from(KeyCode::Up));
        editor.handle_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(editor.cursor, 1);
        editor.handle_key(KeyEvent::from(KeyCode::End));
        assert_eq!(editor.cursor, 3);
        editor.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(editor.cursor, 5);
        editor.handle_key(KeyEvent::from(KeyCode::Home));
        assert_eq!(editor.cursor, 4);
    }

    #[test]
    fn escape_and_control_c_cancel() {
        for key in [KeyEvent::from(KeyCode::Esc), ctrl('c')] {
            let mut editor = Editor::with_text("draft");
            assert_eq!(editor.handle_key(key), Outcome::Cancel);
            assert_eq!(editor.text(), "draft");
        }
    }

    #[test]
    fn control_s_returns_the_trimmed_text_and_an_empty_one_says_so() {
        let mut empty = Editor::default();
        type_text(&mut empty, "  \n ");
        assert_eq!(empty.handle_key(ctrl('s')), Outcome::Continue);
        assert_eq!(empty.status, "Write a comment before saving.");
        let mut editor = Editor::default();
        type_text(&mut editor, " fix this ");
        assert_eq!(
            editor.handle_key(ctrl('s')),
            Outcome::Save("fix this".into())
        );
        // Saving does not clear the buffer: a failed write keeps what was typed.
        editor.fail("review is busy");
        assert_eq!(editor.status, "review is busy");
        assert_eq!(typed(&editor), " fix this ");
        editor.handle_key(KeyEvent::from(KeyCode::Right));
        assert_eq!(editor.status, "");
    }

    #[test]
    fn stored_text_loses_its_control_characters() {
        let editor = Editor::with_text("a\u{1b}[2Jb\tc");
        assert_eq!(typed(&editor), "a[2Jb    c");
    }

    fn rows(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect::<String>()
            })
            .collect()
    }

    /// The editor drawn alone in a pane `width` cells wide and four rows tall.
    fn drawn(editor: &Editor, width: u16, title: &str, place: &str) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, 4)).unwrap();
        terminal
            .draw(|frame| editor.draw(frame, frame.area(), title, place, &Theme::default()))
            .unwrap();
        terminal
    }

    #[test]
    fn the_widget_draws_inside_its_rect_and_nowhere_else() {
        let mut editor = Editor::default();
        type_text(&mut editor, "first\nsecond");
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal
            .draw(|frame| {
                let fill = vec![Line::from("x".repeat(40)); 10];
                frame.render_widget(Paragraph::new(fill), frame.area());
                let area = Rect::new(2, 3, 30, 4);
                editor.draw(frame, area, "Draft note - ", "a.rs R1", &Theme::default());
            })
            .unwrap();
        let rows = rows(&terminal);
        assert_eq!(rows[2], "x".repeat(40));
        assert_eq!(rows[7], "x".repeat(40));
        assert!(
            rows[3].starts_with("xx╭ Draft note - a.rs R1 ─"),
            "{}",
            rows[3]
        );
        assert!(rows[3].ends_with("╮xxxxxxxx"), "{}", rows[3]);
        assert!(rows[4].starts_with("xx│first"), "{}", rows[4]);
        assert!(rows[5].contains("second"));
        assert!(rows[6].starts_with("xx╰─"), "{}", rows[6]);
        assert!(
            rows[6].ends_with(" ^S save  Esc cancel ╯xxxxxxxx"),
            "{}",
            rows[6]
        );
        terminal
            .backend_mut()
            .assert_cursor_position(Position::new(9, 5));
        // The border is the theme's warning colour.
        let theme = Theme::default();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(2, 3)].fg, theme.warning);
        assert_eq!(buffer[(2, 6)].fg, theme.warning);
    }

    #[test]
    fn an_empty_editor_shows_a_dim_placeholder_with_the_cursor_on_its_first_letter() {
        let mut editor = Editor::default();
        let mut terminal = drawn(&editor, 40, "Reply to u1", "");
        let shown = rows(&terminal);
        assert!(shown[0].starts_with("╭ Reply to u1 ─"), "{}", shown[0]);
        assert!(shown[1].starts_with("│Write a note…  "), "{}", shown[1]);
        assert_eq!(
            terminal.backend().buffer()[(1, 1)].fg,
            Theme::default().subtle
        );
        terminal
            .backend_mut()
            .assert_cursor_position(Position::new(1, 1));
        type_text(&mut editor, "x");
        let shown = rows(&drawn(&editor, 40, "Reply to u1", ""));
        assert!(shown[1].starts_with("│x  "), "{}", shown[1]);
    }

    #[test]
    fn a_long_place_is_cut_from_the_left_and_the_title_stays() {
        let editor = Editor::default();
        let place = "src/routes/newsletter_subscription.js R10";
        let shown = rows(&drawn(&editor, 40, "Draft note - ", place));
        assert_eq!(shown[0], "╭ Draft note - …er_subscription.js R10 ╮");
        let shown = rows(&drawn(&editor, 60, "Draft note - ", place));
        assert!(
            shown[0].starts_with(&format!("╭ Draft note - {place} ─")),
            "{}",
            shown[0]
        );
    }

    #[test]
    fn a_failed_save_replaces_the_keys_on_the_bottom_border_until_the_next_key() {
        let mut editor = Editor::with_text("keep me");
        editor.fail("review is busy, press again");
        let terminal = drawn(&editor, 50, "Edit u1", "");
        let shown = rows(&terminal);
        assert!(
            shown[3].ends_with(" review is busy, press again ╯"),
            "{}",
            shown[3]
        );
        assert!(!shown[3].contains("^S save"), "{}", shown[3]);
        assert_eq!(
            terminal.backend().buffer()[(30, 3)].fg,
            Theme::default().removed
        );
        editor.handle_key(KeyEvent::from(KeyCode::Right));
        let shown = rows(&drawn(&editor, 50, "Edit u1", ""));
        assert!(shown[3].ends_with(" ^S save  Esc cancel ╯"), "{}", shown[3]);
        assert!(shown[1].contains("keep me"));
    }

    #[test]
    fn a_long_comment_scrolls_to_keep_the_cursor_visible() {
        let mut editor = Editor::default();
        type_text(&mut editor, "l1\nl2\nl3\nl4\nl5");
        let mut terminal = Terminal::new(TestBackend::new(20, 4)).unwrap();
        terminal
            .draw(|frame| editor.draw(frame, frame.area(), "c", "", &Theme::default()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = |y| {
            (0..20)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect::<String>()
        };
        assert!(text(1).contains("l4") && text(2).contains("l5"));
        assert_eq!(editor.height(20, 6), 6);
        assert_eq!(editor.height(20, 4), 4);
        assert_eq!(Editor::default().height(20, 6), 3);
    }
}
