use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::message::MessageComment;

const SESSION: &str = "0b9c6f1e-1111-4222-8333-444455556666";

/// A temporary `HOME` holding a transcript, and the Herdr that answers for the agent in it.
struct Fixture {
    home: PathBuf,
    herdr: Rc<RefCell<Herdr>>,
}

#[derive(Default)]
struct Herdr {
    calls: Vec<String>,
    /// The terminal `agent get` reports.
    terminal: &'static str,
    status: &'static str,
    agent: &'static str,
    gone: bool,
    /// `agent prompt` is refused.
    prompt_refused: bool,
}

impl Fixture {
    fn new(name: &str, message: &str) -> Self {
        let home = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "herdr-review-message-tui-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&home);
        let fixture = Self {
            home,
            herdr: Rc::new(RefCell::new(Herdr {
                terminal: "term_1",
                status: "idle",
                agent: "claude",
                ..Herdr::default()
            })),
        };
        fixture.write_message("m1", message);
        fixture
    }

    fn write_message(&self, id: &str, message: &str) {
        let projects = self.home.join(".claude").join("projects").join("p");
        std::fs::create_dir_all(&projects).unwrap();
        let line = serde_json::json!({
            "type": "assistant",
            "isSidechain": false,
            "message": {"id": id, "model": "claude-x", "content": [{"type": "text", "text": message}]},
        });
        std::fs::write(projects.join(format!("{SESSION}.jsonl")), line.to_string()).unwrap();
    }

    fn env(&self, vars: &[(&str, &str)]) -> Env {
        let home = self.home.display().to_string();
        let vars = [
            ("HOME", home.as_str()),
            ("HERDR_PANE_ID", "w1:p9"),
            ("REVIEW_DELIVER_TO", "w1:p2"),
            ("REVIEW_DELIVER_TERM", "term_1"),
        ]
        .into_iter()
        .chain(vars.iter().copied())
        .map(|(n, v)| (n.to_owned(), v.to_owned()));
        Env::new(vars, self.home.clone())
    }

    fn pane(&self) -> Pane {
        self.pane_in(self.env(&[]))
    }

    fn pane_in(&self, env: Env) -> Pane {
        let mut pane = Pane::new(env);
        let herdr = Rc::clone(&self.herdr);
        pane.herdr = HerdrCall::new(move |args| herdr.borrow_mut().answer(args));
        pane
    }
}

impl Herdr {
    fn answer(&mut self, args: &[String]) -> Result<String, String> {
        self.calls.push(args.join(" "));
        if self.gone {
            return Err(
                r#"{"error":{"code":"agent_not_found","message":"agent target w1:p2 not found"}}"#
                    .to_owned(),
            );
        }
        if self.prompt_refused && args.get(1).is_some_and(|word| word == "prompt") {
            return Err(
                r#"{"error":{"code":"agent_blocked","message":"agent w1:p2 is blocked"}}"#
                    .to_owned(),
            );
        }
        Ok(format!(
            r#"{{"result":{{"agent":{{"agent":"{}","agent_status":"{}","cwd":"/work","pane_id":"w1:p2","terminal_id":"{}","agent_session":{{"kind":"id","value":"{SESSION}"}}}}}}}}"#,
            self.agent, self.status, self.terminal
        ))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn press(pane: &mut Pane, keys: &str) {
    for c in keys.chars() {
        let mods = if c.is_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        pane.key(KeyEvent::new(KeyCode::Char(c), mods));
    }
}

fn code(pane: &mut Pane, code: KeyCode) {
    pane.key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn drawn(pane: &mut Pane, width: u16, height: u16) -> (Vec<String>, ratatui::buffer::Buffer) {
    pane.resize(Rect::new(0, 0, width, height));
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render(frame, pane)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let rows = buffer
        .content
        .chunks(usize::from(width))
        .map(|row| {
            row.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        })
        .collect();
    (rows, buffer)
}

fn screen(pane: &mut Pane, width: u16, height: u16) -> String {
    drawn(pane, width, height).0.join("\n")
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

const PROSE: &str =
    "alpha beta gamma delta epsilon zeta eta theta iota kappa\nsecond line\n\nfourth line";

#[test]
fn a_wrapped_line_is_several_rows_and_the_cursor_moves_over_it_as_one() {
    let fixture = Fixture::new("wrapped", PROSE);
    let mut pane = fixture.pane();
    pane.load();
    // 3 columns of line number and a space, then 16 of text: the first line takes four rows.
    let (rows, _) = drawn(&mut pane, 20, 12);
    assert_eq!(rows[0].trim_end(), " 1 alpha beta gamma");
    assert!(rows[1].starts_with("   "), "{rows:?}");
    assert!(pane.layout.rows_of(0).len() > 1);
    assert_eq!(pane.layout.rows_of(1).len(), 1);
    assert_eq!(pane.cursor, 0);
    press(&mut pane, "j");
    assert_eq!(pane.cursor, 1);
    press(&mut pane, "jj");
    assert_eq!(pane.cursor, 3);
    press(&mut pane, "j");
    assert_eq!(pane.cursor, 3, "stops on the last line");
    press(&mut pane, "kkk");
    assert_eq!(pane.cursor, 0);
}

#[test]
fn the_line_numbers_are_on_the_first_row_of_a_line_only() {
    let fixture = Fixture::new("numbers", PROSE);
    let mut pane = fixture.pane();
    pane.load();
    let (rows, _) = drawn(&mut pane, 20, 12);
    let numbered = rows
        .iter()
        .take(8)
        .filter(|row| row.trim_start().starts_with(|c: char| c.is_ascii_digit()))
        .count();
    assert_eq!(numbered, 4);
    assert!(rows.iter().any(|row| row.starts_with(" 4 fourth line")));
}

#[test]
fn page_keys_scroll_by_rows() {
    let text = (1..=30)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let fixture = Fixture::new("pages", &text);
    let mut pane = fixture.pane();
    pane.load();
    // 12 rows: 10 of text and the two bars.
    drawn(&mut pane, 40, 12);
    assert_eq!((pane.scroll, pane.cursor), (0, 0));
    pane.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    assert_eq!((pane.scroll, pane.cursor), (10, 10));
    pane.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    pane.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    assert_eq!(pane.scroll, 20, "the last page ends with the last row");
    assert_eq!(pane.cursor, 29);
    pane.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!((pane.scroll, pane.cursor), (10, 19));
    code(&mut pane, KeyCode::PageUp);
    assert_eq!((pane.scroll, pane.cursor), (0, 9));
}

#[test]
fn moving_past_the_window_scrolls_it_and_a_tall_line_shows_its_start() {
    let text = format!("{}\nshort", "word ".repeat(60));
    let fixture = Fixture::new("tall", &text);
    let mut pane = fixture.pane();
    pane.load();
    drawn(&mut pane, 14, 6);
    assert!(pane.layout.rows_of(0).len() > 4);
    press(&mut pane, "j");
    assert_eq!(pane.cursor, 1);
    assert_eq!(pane.scroll, pane.layout.rows_of(1).end - 4);
    press(&mut pane, "k");
    assert_eq!((pane.cursor, pane.scroll), (0, 0));
}

#[test]
fn a_failed_reload_keeps_the_message_and_says_why() {
    let fixture = Fixture::new("reload", PROSE);
    let mut pane = fixture.pane();
    pane.load();
    pane.resize(Rect::new(0, 0, 60, 8));
    press(&mut pane, "jj");
    fixture.herdr.borrow_mut().gone = true;
    press(&mut pane, "R");
    assert_eq!(pane.screen, Screen::Review);
    assert_eq!(pane.cursor, 2, "the cursor stays");
    assert_eq!(
        pane.status,
        Some((
            Tone::Failure,
            "No agent is running in that pane.".to_owned()
        ))
    );
    let text = screen(&mut pane, 60, 8);
    assert!(text.contains("second line"), "{text}");
    assert!(text.contains("No agent is running in that pane."), "{text}");
}

#[test]
fn a_reload_shows_a_newer_message_from_its_start() {
    let fixture = Fixture::new("newer", PROSE);
    let mut pane = fixture.pane();
    pane.load();
    press(&mut pane, "jj");
    fixture.write_message("m2", "a newer message");
    press(&mut pane, "R");
    assert_eq!(pane.message.as_ref().unwrap().lines, ["a newer message"]);
    assert_eq!(pane.cursor, 0);
    fixture.write_message("m3", "a newer message\nmore");
    press(&mut pane, "R");
    assert_eq!(pane.message.as_ref().unwrap().lines.len(), 2);
    press(&mut pane, "jR");
    assert_eq!(pane.cursor, 1, "the same message keeps the cursor");
}

#[test]
fn start_up_failures_are_the_message_screen_and_reload_and_quit_still_work() {
    let fixture = Fixture::new("failures", PROSE);

    let mut pane = fixture.pane_in(Env::new([], fixture.home.clone()));
    pane.load();
    assert!(matches!(&pane.screen, Screen::Message(text) if text.contains("message action")));

    fixture.herdr.borrow_mut().gone = true;
    let mut pane = fixture.pane();
    pane.load();
    assert_eq!(
        pane.screen,
        Screen::Message("No agent is running in that pane.".to_owned())
    );
    let text = screen(&mut pane, 60, 8);
    assert!(text.contains("reload, q quit"), "{text}");

    fixture.herdr.borrow_mut().gone = false;
    fixture.herdr.borrow_mut().terminal = "term_other";
    pane.load();
    assert!(matches!(&pane.screen, Screen::Message(text) if text.contains("agent is gone")));

    fixture.herdr.borrow_mut().terminal = "term_1";
    fixture.herdr.borrow_mut().agent = "codex";
    pane.load();
    assert!(matches!(&pane.screen, Screen::Message(text) if text.contains("Claude Code only")));

    fixture.herdr.borrow_mut().agent = "claude";
    press(&mut pane, "R");
    assert_eq!(pane.screen, Screen::Review);
    press(&mut pane, "q");
    assert!(pane.quit);
}

#[test]
fn the_footer_and_the_help_list_only_what_the_pane_does() {
    let fixture = Fixture::new("keys", PROSE);
    let mut pane = fixture.pane();
    pane.load();
    let text = screen(&mut pane, 100, 30);
    let last = text.lines().last().unwrap();
    assert!(last.contains("c comment"), "{last}");
    assert!(last.contains("S send"), "{last}");
    assert!(last.contains("R reload"), "{last}");
    assert!(last.contains("? help"), "{last}");
    assert!(last.contains("q quit"), "{last}");
    for other in ["sidebar", "resolve", "hunk", "spec"] {
        assert!(!last.contains(other), "{last}");
    }

    press(&mut pane, "?");
    let text = screen(&mut pane, 100, 30);
    for action in ACTIONS {
        assert!(text.contains(action.describe()), "{}", action.name());
    }
    for action in Action::ALL.into_iter().filter(|a| !ACTIONS.contains(a)) {
        assert!(
            !text.contains(action.describe()),
            "{} is not the pane's",
            action.name()
        );
    }
    press(&mut pane, "x");
    assert!(!pane.help, "any key closes it");
}

#[test]
fn a_key_the_pane_does_not_act_on_does_nothing() {
    let fixture = Fixture::new("ignored", PROSE);
    let mut pane = fixture.pane();
    pane.load();
    // `r` is reply, `x` is resolve, `f` is the sidebar.
    press(&mut pane, "rxfbt");
    assert_eq!((pane.cursor, pane.select, pane.quit), (0, None, false));
    assert_eq!(pane.status, None);
}

#[test]
fn the_status_line_names_the_agent_and_says_when_it_was_working() {
    let fixture = Fixture::new("status", PROSE);
    let mut pane = fixture.pane();
    pane.load();
    let text = screen(&mut pane, 100, 8);
    let last = text.lines().last().unwrap();
    assert!(
        last.starts_with(" MESSAGE  \u{2192} claude w1:p2"),
        "{last}"
    );
    assert!(!last.contains("working"));

    fixture.herdr.borrow_mut().status = "working";
    press(&mut pane, "R");
    let text = screen(&mut pane, 100, 8);
    assert!(text.lines().last().unwrap().contains(" working "), "{text}");
}

#[test]
fn visual_mode_selects_lines_and_escape_leaves_it() {
    let fixture = Fixture::new("visual", PROSE);
    let mut pane = fixture.pane();
    pane.load();
    press(&mut pane, "jv");
    assert_eq!(pane.select, Some(1));
    press(&mut pane, "jj");
    assert_eq!(pane.range(), 1..4);
    let text = screen(&mut pane, 100, 8);
    let last = text.lines().last().unwrap();
    assert!(last.contains("VISUAL"), "{last}");
    assert!(last.contains("lines 2-4 (3 lines)"), "{last}");
    assert!(last.contains("v/esc cancel"), "{last}");
    press(&mut pane, "k");
    assert_eq!(pane.range(), 1..3);
    code(&mut pane, KeyCode::Esc);
    assert_eq!(pane.select, None);
    assert_eq!(pane.range(), 2..3);
    press(&mut pane, "vv");
    assert_eq!(pane.select, None);
}

#[test]
fn the_selected_lines_and_the_cursor_are_tinted() {
    let fixture = Fixture::new("tint", PROSE);
    let mut pane = fixture.pane();
    pane.load();
    pane.resize(Rect::new(0, 0, 80, 10));
    press(&mut pane, "jvj");
    let (_, buffer) = drawn(&mut pane, 80, 10);
    let bg = |row: u16| buffer[(10, row)].bg;
    // Each line takes one row at this width. The range is lines 2 and 3, and the cursor is on 3.
    assert_ne!(bg(0), pane.theme.selection);
    assert_eq!(bg(1), pane.theme.selection);
    assert_eq!(bg(2), pane.theme.cursor);
}

#[test]
fn the_wheel_scrolls_and_a_click_or_a_drag_selects_lines() {
    let text = (1..=30)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let fixture = Fixture::new("mouse", &text);
    let mut pane = fixture.pane();
    pane.load();
    drawn(&mut pane, 40, 12);
    pane.mouse(mouse(MouseEventKind::ScrollDown, 5, 3));
    assert_eq!((pane.scroll, pane.cursor), (3, 3));
    pane.mouse(mouse(MouseEventKind::ScrollUp, 5, 3));
    assert_eq!(pane.scroll, 0);
    pane.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 4));
    assert_eq!((pane.cursor, pane.select), (4, None));
    pane.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 5, 7));
    assert_eq!((pane.cursor, pane.select), (7, Some(4)));
    pane.mouse(mouse(MouseEventKind::Up(MouseButton::Left), 5, 7));
    pane.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 5, 9));
    assert_eq!(pane.cursor, 7, "a drag with no press selects nothing");
    pane.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 2));
    assert_eq!(pane.select, None, "a click drops the range");
    pane.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 5, 11));
    assert_eq!(
        (pane.select, pane.scroll),
        (Some(2), 1),
        "below the text scrolls"
    );
}

#[test]
fn the_pointer_file_names_this_pane_for_the_agents_terminal() {
    let fixture = Fixture::new("pointer", PROSE);
    let mut pane = fixture.pane();
    pane.save_pointer();
    let path = fixture
        .home
        .join(".local/state/herdr-review/message/term_1");
    assert_eq!(std::fs::read_to_string(path).unwrap(), "w1:p9");
    assert!(pane.warnings.is_empty());
}

#[test]
fn a_pointer_that_cannot_be_written_is_a_warning() {
    let fixture = Fixture::new("pointer-fails", PROSE);
    // A file where the directory should be.
    let state = fixture.home.join(".local/state");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("herdr-review"), "").unwrap();
    let mut pane = fixture.pane();
    pane.save_pointer();
    assert_eq!(pane.warnings.len(), 1);
    pane.load();
    assert_eq!(pane.screen, Screen::Review, "the pane still opens");
}

#[test]
fn the_message_is_cleaned_of_terminal_control_characters() {
    let fixture = Fixture::new("sanitize", "a\u{1b}[31mred\tb");
    let mut pane = fixture.pane();
    pane.load();
    assert_eq!(pane.message.as_ref().unwrap().lines, ["a[31mred    b"]);
}

#[cfg(feature = "syntax")]
#[test]
fn markdown_is_coloured_with_the_theme() {
    let fixture = Fixture::new(
        "colours",
        "# Title\n\n`code` and **bold**\n\n```rust\nfn main() {}\n```",
    );
    let mut pane = fixture.pane();
    pane.load();
    assert!(
        pane.tokens.iter().any(|line| !line.is_empty()),
        "the Markdown grammar resolves"
    );
    let (rows, buffer) = drawn(&mut pane, 40, 12);
    let fg_of = |row: usize, text: &str| {
        let column = u16::try_from(rows[row].find(text).unwrap()).unwrap();
        buffer[(column, u16::try_from(row).unwrap())].fg
    };
    let row = |text: &str| rows.iter().position(|row| row.contains(text)).unwrap();
    assert_ne!(fg_of(row("Title"), "Title"), pane.theme.text, "the heading");
    assert_ne!(
        fg_of(row("fn main"), "fn"),
        pane.theme.text,
        "code in a fence"
    );
    assert_eq!(
        fg_of(row("code` and"), "and"),
        pane.theme.text,
        "plain text"
    );
}

#[cfg(not(feature = "syntax"))]
#[test]
fn without_the_syntax_feature_the_text_is_plain() {
    let fixture = Fixture::new("plain", "# Title\n\n`code`");
    let mut pane = fixture.pane();
    pane.load();
    assert!(pane.tokens.is_empty());
    let (rows, buffer) = drawn(&mut pane, 40, 12);
    let column = u16::try_from(rows[0].find('#').unwrap()).unwrap();
    assert_eq!(buffer[(column, 0)].fg, pane.theme.text);
}

#[test]
fn a_resize_wraps_again_and_keeps_the_first_line_on_screen() {
    let fixture = Fixture::new("resize", PROSE);
    let mut pane = fixture.pane();
    pane.load();
    drawn(&mut pane, 20, 5);
    press(&mut pane, "jj");
    let before = pane.layout.line_at(pane.scroll);
    drawn(&mut pane, 60, 5);
    assert_eq!(pane.layout.rows_of(0).len(), 1);
    assert_eq!(pane.layout.line_at(pane.scroll), before);
    assert_eq!(pane.cursor, 2);
}

#[test]
fn spaces_a_wrap_drops_do_not_shift_the_row_ranges() {
    let lines = [
        "aa  bb   cc  dd".to_owned(),
        "  indented words here".to_owned(),
    ];
    let layout = Layout::build(&lines, 6, &[]);
    let pieces = |line: usize| {
        layout
            .rows
            .iter()
            .filter(|row| row.line == line)
            .map(|row| match &row.kind {
                Kind::Text(range) => &lines[line][range.clone()],
                Kind::Note { .. } => panic!("no notes were given"),
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(pieces(0), ["aa  bb", "  cc", "dd"]);
    let second = pieces(1);
    assert_eq!(second, ["indent", "ed", "words", "here"]);
}

#[test]
fn the_loop_draws_on_change_and_quits_on_q() {
    let fixture = Fixture::new("loop", PROSE);
    let mut pane = fixture.pane();
    pane.load();
    let events = RefCell::new(
        vec![
            None,
            Some(Event::Key(KeyEvent::new(
                KeyCode::Char('j'),
                KeyModifiers::NONE,
            ))),
            Some(Event::Key(KeyEvent::new(
                KeyCode::Char('q'),
                KeyModifiers::NONE,
            ))),
        ]
        .into_iter(),
    );
    let polls = Cell::new(0);
    let poll = |_| {
        polls.set(polls.get() + 1);
        Ok(events.borrow_mut().next().flatten())
    };
    let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    let exit = run_loop(&mut pane, &mut terminal, poll, || false);
    assert_eq!(exit, Exit::Quit);
    assert_eq!(polls.get(), 3);
    assert_eq!(pane.cursor, 1);
}

#[test]
fn a_signal_ends_the_loop() {
    let fixture = Fixture::new("signal", PROSE);
    let mut pane = fixture.pane();
    pane.load();
    let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
    let exit = run_loop(&mut pane, &mut terminal, |_| Ok(None), || true);
    assert_eq!(exit, Exit::Terminated);
}

const NOTE: &str = "line one\nline two\nline three\nline four\nline five";

/// Add a comment on the lines under the cursor, or the selected range.
fn comment(pane: &mut Pane, text: &str) {
    press(pane, "c");
    press(pane, text);
    pane.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
}

fn opened(name: &str, message: &str) -> (Fixture, Pane) {
    let fixture = Fixture::new(name, message);
    let mut pane = fixture.pane();
    pane.load();
    pane.resize(Rect::new(0, 0, 60, 24));
    (fixture, pane)
}

fn row_of(rows: &[String], text: &str) -> usize {
    rows.iter().position(|row| row.contains(text)).unwrap()
}

#[test]
fn a_range_comment_draws_as_a_box_under_the_last_line_it_covers() {
    let (_fixture, mut pane) = opened("range", NOTE);
    press(&mut pane, "jvj");
    comment(&mut pane, "why two?");
    assert_eq!(
        pane.comments,
        [MessageComment {
            start: 2,
            end: 3,
            body: "why two?".to_owned()
        }]
    );
    let (rows, _) = drawn(&mut pane, 60, 24);
    let (three, four) = (row_of(&rows, "line three"), row_of(&rows, "line four"));
    assert!(
        rows[three + 1].contains("╭ ● Your note · lines 2-3"),
        "{rows:#?}"
    );
    assert!(rows[three + 3].contains("why two?"));
    assert!(rows[three + 4].contains("╰"));
    assert!(rows[three + 4].contains("e edit  d delete"));
    assert_eq!(four, three + 5, "the next line comes after the box");
    assert!(!rows[row_of(&rows, "line two") + 1].contains("╭"));
    assert_eq!((pane.cursor, pane.select), (2, None));
    let status = rows.last().unwrap();
    assert!(status.contains(" 1 comment "), "{status}");
}

#[test]
fn two_comments_on_the_same_lines_both_draw() {
    let (_fixture, mut pane) = opened("two", NOTE);
    press(&mut pane, "j");
    comment(&mut pane, "first");
    comment(&mut pane, "second");
    assert_eq!(pane.comments.len(), 2);
    let (rows, _) = drawn(&mut pane, 60, 24);
    let first = row_of(&rows, "first");
    let second = row_of(&rows, "second");
    assert!(
        first < second && second < row_of(&rows, "line three"),
        "{rows:#?}"
    );
    assert!(rows.last().unwrap().contains(" 2 comments "));
}

#[test]
fn a_comment_is_placed_by_its_last_line_even_when_written_first() {
    let (_fixture, mut pane) = opened("order", NOTE);
    press(&mut pane, "jjjj");
    comment(&mut pane, "on five");
    press(&mut pane, "kkk");
    comment(&mut pane, "on two");
    let (rows, _) = drawn(&mut pane, 60, 24);
    assert!(row_of(&rows, "on two") < row_of(&rows, "line three"));
    assert!(row_of(&rows, "on five") > row_of(&rows, "line five"));
}

#[test]
fn edit_and_delete_change_only_the_focused_comment() {
    let (_fixture, mut pane) = opened("edit", NOTE);
    comment(&mut pane, "one");
    press(&mut pane, "jj");
    comment(&mut pane, "three");
    press(&mut pane, "k");
    assert_eq!(pane.focused(), None);
    press(&mut pane, "e");
    assert_eq!(
        pane.status.as_ref().unwrap().1,
        "no comment here, n finds the next one"
    );
    press(&mut pane, "n");
    assert_eq!((pane.cursor, pane.focused()), (2, Some(1)));
    press(&mut pane, "e");
    assert!(pane.compose.is_some());
    press(&mut pane, " changed");
    pane.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    let bodies = pane
        .comments
        .iter()
        .map(|c| c.body.as_str())
        .collect::<Vec<_>>();
    assert_eq!(bodies, ["one", "three changed"]);
    press(&mut pane, "d");
    let bodies = pane
        .comments
        .iter()
        .map(|c| c.body.as_str())
        .collect::<Vec<_>>();
    assert_eq!(bodies, ["one"]);
    assert_eq!(pane.status.as_ref().unwrap().1, "deleted the comment");
    let (rows, _) = drawn(&mut pane, 60, 24);
    assert!(
        rows.iter()
            .any(|row| row.contains("one") && row.contains('│'))
    );
    assert!(!rows.iter().any(|row| row.contains("three changed")));
}

#[test]
fn escape_leaves_the_editor_without_a_comment_and_an_empty_one_saves_nothing() {
    let (_fixture, mut pane) = opened("cancel", NOTE);
    press(&mut pane, "c");
    press(&mut pane, "draft");
    code(&mut pane, KeyCode::Esc);
    assert!(pane.compose.is_none() && pane.comments.is_empty());
    press(&mut pane, "c");
    pane.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(pane.comments.is_empty(), "an empty note is not saved");
}

#[test]
fn the_editor_is_drawn_under_the_lines_it_comments_on() {
    let (_fixture, mut pane) = opened("editor", NOTE);
    press(&mut pane, "jvj");
    press(&mut pane, "c");
    press(&mut pane, "hello");
    let (rows, buffer) = drawn(&mut pane, 60, 24);
    let three = row_of(&rows, "line three");
    assert!(
        rows[three + 1].contains("Draft note - lines 2-3"),
        "{rows:#?}"
    );
    assert!(rows.iter().any(|row| row.contains("hello")));
    assert_eq!(pane.compose.as_ref().unwrap().lines, 1..3);
    // A bar marks the covered lines in the first cell of the gutter, and the numbers stay.
    assert_eq!(buffer[(0, 0)].symbol(), " ");
    assert_eq!(buffer[(0, 1)].symbol(), "▌");
    assert_eq!(buffer[(1, 1)].symbol(), "2");
    let two = u16::try_from(row_of(&rows, "line two")).unwrap();
    assert_eq!(buffer[(5, two)].bg, pane.theme.selection);
    // Keys go to the editor, and the mouse does nothing.
    press(&mut pane, "qj");
    assert!(!pane.quit);
    pane.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 0));
    assert_eq!(pane.cursor, 2);
}

#[test]
fn n_and_capital_n_move_between_comments() {
    let (_fixture, mut pane) = opened("jump", NOTE);
    press(&mut pane, "n");
    assert_eq!(pane.status.as_ref().unwrap().1, "no comments yet");
    comment(&mut pane, "a");
    press(&mut pane, "jjj");
    comment(&mut pane, "b");
    press(&mut pane, "gg");
    code(&mut pane, KeyCode::Up);
    code(&mut pane, KeyCode::Up);
    code(&mut pane, KeyCode::Up);
    code(&mut pane, KeyCode::Up);
    assert_eq!(pane.cursor, 0);
    press(&mut pane, "n");
    assert_eq!(
        pane.cursor, 3,
        "the comment on the cursor's own line is skipped"
    );
    press(&mut pane, "n");
    assert_eq!(pane.status.as_ref().unwrap().1, "no next comment");
    press(&mut pane, "N");
    assert_eq!(pane.cursor, 0);
    press(&mut pane, "N");
    assert_eq!(pane.status.as_ref().unwrap().1, "no previous comment");
}

#[test]
fn n_steps_through_comments_on_one_line_in_the_order_written() {
    let (_fixture, mut pane) = opened("same-line", NOTE);
    comment(&mut pane, "first");
    comment(&mut pane, "second");
    press(&mut pane, "N");
    assert_eq!(pane.focused(), Some(0));
    press(&mut pane, "n");
    assert_eq!(pane.focused(), Some(1));
    // The tinted box is the focused one.
    let (rows, buffer) = drawn(&mut pane, 60, 24);
    let second = u16::try_from(row_of(&rows, "second")).unwrap();
    let first = u16::try_from(row_of(&rows, "first")).unwrap();
    assert_eq!(buffer[(10, second)].bg, pane.theme.cursor);
    assert_ne!(buffer[(10, first)].bg, pane.theme.cursor);
}

#[test]
fn a_box_below_the_window_is_scrolled_into_view() {
    let text = (1..=30)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (_fixture, mut pane) = opened("reveal", &text);
    pane.resize(Rect::new(0, 0, 60, 12));
    press(&mut pane, "jjjjjjjjj");
    comment(&mut pane, "body");
    let (rows, _) = drawn(&mut pane, 60, 12);
    assert!(rows.iter().any(|row| row.contains("╰")), "{rows:#?}");
    assert!(rows.iter().any(|row| row.contains("line 10")));
}

#[test]
fn reload_with_comments_asks_first() {
    let (fixture, mut pane) = opened("reload-asks", NOTE);
    comment(&mut pane, "mine");
    fixture.write_message("m2", "a newer message");
    press(&mut pane, "R");
    assert_eq!(pane.prompt, Some(Prompt::Reload));
    let text = screen(&mut pane, 100, 24);
    assert!(
        text.contains("discard 1 comment and load the newest message?"),
        "{text}"
    );
    // Other keys wait.
    press(&mut pane, "jq");
    assert_eq!(pane.prompt, Some(Prompt::Reload));
    assert!(!pane.quit);
    press(&mut pane, "n");
    assert_eq!(pane.prompt, None);
    assert_eq!(pane.comments.len(), 1);
    assert_eq!(pane.message.as_ref().unwrap().id, "m1");

    press(&mut pane, "R");
    code(&mut pane, KeyCode::Esc);
    assert_eq!(pane.message.as_ref().unwrap().id, "m1");

    press(&mut pane, "R");
    press(&mut pane, "y");
    assert_eq!(pane.prompt, None);
    assert!(pane.comments.is_empty());
    assert_eq!(pane.message.as_ref().unwrap().lines, ["a newer message"]);
    let text = screen(&mut pane, 100, 24);
    assert!(!text.contains("1 comment"), "{text}");
}

#[test]
fn a_failed_reload_after_yes_keeps_the_comments() {
    let (fixture, mut pane) = opened("reload-fails", NOTE);
    comment(&mut pane, "mine");
    fixture.herdr.borrow_mut().gone = true;
    press(&mut pane, "R");
    press(&mut pane, "y");
    assert_eq!(pane.comments.len(), 1);
    assert_eq!(pane.screen, Screen::Review);
    assert_eq!(pane.status.as_ref().unwrap().0, Tone::Failure);
    let text = screen(&mut pane, 100, 24);
    assert!(text.contains("mine"), "{text}");
}

#[test]
fn quit_with_comments_asks_and_without_leaves() {
    let (_fixture, mut pane) = opened("quit-asks", NOTE);
    comment(&mut pane, "mine");
    press(&mut pane, "q");
    assert_eq!(pane.prompt, Some(Prompt::Quit));
    assert!(!pane.quit);
    let text = screen(&mut pane, 100, 24);
    assert!(text.contains("1 unsent comment"), "{text}");
    assert!(text.contains("[d] discard them and quit"), "{text}");
    code(&mut pane, KeyCode::Esc);
    assert!(pane.prompt.is_none() && !pane.quit);
    press(&mut pane, "qd");
    assert!(pane.quit);

    let (_fixture, mut pane) = opened("quit-plain", NOTE);
    press(&mut pane, "q");
    assert!(pane.quit && pane.prompt.is_none());
}

#[test]
fn the_boxes_wrap_again_when_the_pane_is_resized() {
    let (_fixture, mut pane) = opened("box-resize", NOTE);
    comment(
        &mut pane,
        "a long note that has to wrap inside a narrow box of text",
    );
    let narrow = pane.layout.box_of(0).len();
    pane.resize(Rect::new(0, 0, 120, 24));
    assert!(pane.layout.box_of(0).len() < narrow);
    assert_eq!(
        pane.layout.box_of(0).end,
        pane.layout.rows_of(0).end + pane.layout.box_of(0).len()
    );
}

#[test]
fn comment_keys_do_nothing_on_the_message_screen() {
    let fixture = Fixture::new("no-message", NOTE);
    fixture.herdr.borrow_mut().gone = true;
    let mut pane = fixture.pane();
    pane.load();
    press(&mut pane, "cned");
    assert!(pane.compose.is_none() && pane.comments.is_empty());
}

/// The Herdr calls that were not `agent get`.
fn calls_but_get(fixture: &Fixture) -> Vec<String> {
    let calls = fixture.herdr.borrow().calls.clone();
    calls
        .into_iter()
        .filter(|call| !call.starts_with("agent get"))
        .collect()
}

/// Press `S` and run what it queued, as the loop does.
fn send(pane: &mut Pane) {
    press(pane, "S");
    pane.run_pending(|_| {});
}

#[test]
fn send_prompts_the_agent_with_the_comments_and_the_pane_leaves() {
    let (fixture, mut pane) = opened("send", NOTE);
    press(&mut pane, "jvj");
    comment(&mut pane, "Why two?");
    press(&mut pane, "jj");
    comment(&mut pane, "Drop this.\nPlease.");
    send(&mut pane);
    assert!(pane.quit);
    assert_eq!(
        pane.comments.len(),
        2,
        "nothing is cleared, the pane just exits"
    );
    let calls = calls_but_get(&fixture);
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(
        calls[0],
        "agent prompt w1:p2 Feedback on your last message. Address each point.\n\
         \n\
         - lines 2-3:\n\
         \x20 > line two\n\
         \x20 > line three\n\
         \x20 Why two?\n\
         - line 5:\n\
         \x20 > line five\n\
         \x20 Drop this.\n\
         \x20 Please."
    );
}

#[test]
fn sending_is_drawn_before_the_herdr_call() {
    let (fixture, mut pane) = opened("sending", NOTE);
    comment(&mut pane, "x");
    press(&mut pane, "S");
    assert_eq!(pane.status, Some((Tone::Notice, "sending".to_owned())));
    assert!(calls_but_get(&fixture).is_empty(), "nothing was sent yet");
    let mut drawn_status = None;
    pane.run_pending(|pane| {
        drawn_status = pane.message_text().map(str::to_owned);
        assert!(calls_but_get(&fixture).is_empty(), "the frame comes first");
    });
    assert_eq!(drawn_status.as_deref(), Some("sending"));
    assert_eq!(calls_but_get(&fixture).len(), 1);
    pane.run_pending(|_| panic!("the send ran twice"));
}

#[test]
fn a_refused_send_keeps_the_pane_and_the_comments_and_says_why() {
    let (fixture, mut pane) = opened("refused", NOTE);
    comment(&mut pane, "mine");
    fixture.herdr.borrow_mut().status = "blocked";
    send(&mut pane);
    assert!(!pane.quit);
    assert_eq!(pane.comments.len(), 1);
    let (tone, text) = pane.status.clone().unwrap();
    assert_eq!(tone, Tone::Failure);
    assert_eq!(
        text,
        "The agent is waiting on a prompt. Nothing was sent; your comments are still unsent."
    );
    let calls = calls_but_get(&fixture);
    assert_eq!(
        calls.len(),
        1,
        "no prompt, only the notification: {calls:?}"
    );
    assert!(calls[0].starts_with("notification show review: not sent --body The agent is waiting"));
    let shown = screen(&mut pane, 100, 24);
    assert!(
        shown.contains("The agent is waiting on a prompt."),
        "{shown}"
    );
    assert!(shown.contains("mine"), "the comment is still on screen");

    // Herdr's own refusal at the prompt reads the same, and the next press can succeed.
    fixture.herdr.borrow_mut().status = "idle";
    fixture.herdr.borrow_mut().prompt_refused = true;
    send(&mut pane);
    assert!(!pane.quit);
    assert!(
        pane.status
            .as_ref()
            .unwrap()
            .1
            .starts_with("The agent is waiting on a prompt.")
    );
    fixture.herdr.borrow_mut().prompt_refused = false;
    send(&mut pane);
    assert!(pane.quit, "pressing again sends it");
}

#[test]
fn a_changed_terminal_refuses_before_anything_is_typed() {
    let (fixture, mut pane) = opened("changed", NOTE);
    comment(&mut pane, "mine");
    fixture.herdr.borrow_mut().terminal = "term_other";
    send(&mut pane);
    assert!(!pane.quit);
    assert_eq!(pane.comments.len(), 1);
    assert_eq!(
        pane.status.as_ref().unwrap().1,
        "The agent is gone: that pane runs something else now. \
         Nothing was sent; your comments are still unsent."
    );
    assert!(
        calls_but_get(&fixture)
            .iter()
            .all(|call| !call.starts_with("agent prompt")),
        "nothing was typed"
    );

    fixture.herdr.borrow_mut().terminal = "term_1";
    fixture.herdr.borrow_mut().gone = true;
    send(&mut pane);
    assert!(
        pane.status
            .as_ref()
            .unwrap()
            .1
            .starts_with("No agent is running in that pane.")
    );
    assert!(!pane.quit);
}

#[test]
fn send_with_no_comments_says_so_and_calls_nothing() {
    let (fixture, mut pane) = opened("nothing", NOTE);
    send(&mut pane);
    assert_eq!(
        pane.status,
        Some((Tone::Notice, "nothing to send".to_owned()))
    );
    assert!(!pane.quit);
    assert!(calls_but_get(&fixture).is_empty());
}

#[test]
fn the_quit_prompt_can_send_and_then_the_pane_leaves() {
    let (fixture, mut pane) = opened("quit-sends", NOTE);
    comment(&mut pane, "mine");
    press(&mut pane, "q");
    let text = screen(&mut pane, 100, 24);
    assert!(text.contains("[s] send them, then quit"), "{text}");
    press(&mut pane, "s");
    assert_eq!(pane.prompt, None);
    pane.run_pending(|_| {});
    assert!(pane.quit);
    assert_eq!(calls_but_get(&fixture).len(), 1);

    // A refusal leaves the pane open, with the comments.
    let (fixture, mut pane) = opened("quit-refused", NOTE);
    comment(&mut pane, "mine");
    fixture.herdr.borrow_mut().status = "blocked";
    press(&mut pane, "qs");
    pane.run_pending(|_| {});
    assert!(!pane.quit);
    assert_eq!(pane.comments.len(), 1);
}

#[test]
fn the_loop_sends_and_exits() {
    let (fixture, mut pane) = opened("loop-send", NOTE);
    comment(&mut pane, "mine");
    let events = RefCell::new(
        vec![Some(Event::Key(KeyEvent::new(
            KeyCode::Char('S'),
            KeyModifiers::SHIFT,
        )))]
        .into_iter(),
    );
    let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
    let exit = run_loop(
        &mut pane,
        &mut terminal,
        |_| Ok(events.borrow_mut().next().flatten()),
        || false,
    );
    assert_eq!(exit, Exit::Quit);
    assert_eq!(calls_but_get(&fixture).len(), 1);
}
